//! qBittorrent + qBittorrentEE 客户端。对应上游 `downloader/impl/qbittorrent/**`。

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use pbh_domain::{Peer, PeerAddress, PeerFlag, Torrent};
use reqwest::{Method, RequestBuilder};
use serde::Deserialize;

use crate::{
    join_full_ban_string, join_increment_peers, Downloader, DownloaderConfig, DownloaderError,
    FeatureFlag, LoginOutcome, Result,
};

const PAGE_SIZE: u32 = 100;
const USER_AGENT: &str = concat!("PeerBanHelper-Rust/", env!("CARGO_PKG_VERSION"));

/// qB 客户端（stock 与 EE 共用，EE 经 `use_shadow_ban` 切换封禁策略）。
pub struct QBittorrentClient {
    config: DownloaderConfig,
    http: reqwest::Client,
    api: String,
    enhanced: bool,
    range_ban: AtomicBool,
}

impl QBittorrentClient {
    pub fn new(config: DownloaderConfig) -> Result<Self> {
        let enhanced = config.kind.eq_ignore_ascii_case("qbittorrentee");
        let api = format!("{}/api/v2", config.endpoint.trim_end_matches('/'));
        let http = reqwest::Client::builder()
            .cookie_store(true)
            .gzip(true)
            .danger_accept_invalid_certs(!config.verify_ssl)
            .danger_accept_invalid_hostnames(!config.verify_ssl)
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .user_agent(USER_AGENT)
            .build()?;
        Ok(QBittorrentClient {
            config,
            http,
            api,
            enhanced,
            range_ban: AtomicBool::new(false),
        })
    }

    fn shadow(&self) -> bool {
        self.enhanced && self.config.use_shadow_ban
    }

    /// 构造请求，附加 basic-auth 与 api-key Bearer。
    fn req(&self, method: Method, path: &str) -> RequestBuilder {
        let mut rb = self.http.request(method, format!("{}{}", self.api, path));
        let ba = &self.config.basic_auth;
        if !ba.user.is_empty() {
            rb = rb.basic_auth(&ba.user, Some(&ba.pass));
        }
        if !self.config.api_key.is_empty() && !path.contains("/auth/") {
            rb = rb.bearer_auth(&self.config.api_key);
        }
        rb
    }

    async fn get_text(&self, path: &str) -> Result<String> {
        let resp = self.req(Method::GET, path).send().await?;
        let status = resp.status();
        let body = resp.text().await?;
        if !status.is_success() {
            return Err(status_error(path, status, &body));
        }
        Ok(body)
    }

    async fn post_form(&self, path: &str, form: &[(&str, &str)]) -> Result<String> {
        let resp = self.req(Method::POST, path).form(form).send().await?;
        let status = resp.status();
        let body = resp.text().await?;
        if !status.is_success() {
            return Err(status_error(path, status, &body));
        }
        Ok(body)
    }

    /// 读取并解析版本，刷新 `range_ban` 标志。
    async fn refresh_version(&self) -> Result<()> {
        let ver = self.get_text("/app/version").await?;
        if let Some(v) = parse_version(&ver) {
            let supports = v >= semver::Version::new(5, 3, 0);
            self.range_ban.store(supports, Ordering::Relaxed);
        }
        Ok(())
    }

    /// 设置 `enable_multi_connections_from_same_ip=false`（与上游一致的登录副作用）。
    async fn apply_login_prefs(&self) -> Result<()> {
        let json = r#"{"enable_multi_connections_from_same_ip":false}"#;
        self.post_form("/app/setPreferences", &[("json", json)])
            .await?;
        Ok(())
    }
}

#[async_trait]
impl Downloader for QBittorrentClient {
    fn id(&self) -> &str {
        &self.config.id
    }
    fn name(&self) -> &str {
        &self.config.name
    }
    fn type_label(&self) -> &str {
        if self.enhanced {
            "qBittorrentEE"
        } else {
            "qBittorrent"
        }
    }
    fn feature_flags(&self) -> Vec<FeatureFlag> {
        let mut f = vec![FeatureFlag::UnbanIp];
        if self.range_ban.load(Ordering::Relaxed) {
            f.push(FeatureFlag::RangeBanIp);
        }
        f
    }
    fn is_paused(&self) -> bool {
        self.config.paused
    }

    async fn login(&self) -> Result<LoginOutcome> {
        // api-key 模式：无需 /auth/login，直接校验版本。
        if self.config.api_key.is_empty() {
            let body = self
                .post_form(
                    "/auth/login",
                    &[
                        ("username", &self.config.username),
                        ("password", &self.config.password),
                    ],
                )
                .await?;
            if !body.trim().eq_ignore_ascii_case("Ok.") {
                // /auth/login 未返回 "Ok." 不一定是真失败：qB 可对「本机/白名单子网」启用
                // "跳过身份验证"，此时登录接口仍按账密返回 "Fails."，但其它需鉴权接口照常可用
                // （这正是"账密为空却能连上、但测试连接报错"的成因）。用 /app/version 探活确认：
                // 能取到版本 → 会话有效(绕过生效)，继续；否则才判定为真正的登录失败。
                match self.get_text("/app/version").await {
                    Ok(v) if !v.trim().is_empty() => {
                        tracing::info!(
                            "下载器[{}] /auth/login 返回 \"{}\"，但 /app/version 可访问（qB 疑似对本机/白名单子网跳过鉴权），按已登录继续。",
                            self.config.id,
                            body.trim()
                        );
                    }
                    Ok(_) => {
                        return Ok(LoginOutcome::fail(login_reject_hint(
                            body.trim(),
                            "/app/version 返回空响应",
                        )));
                    }
                    Err(e) => {
                        return Ok(LoginOutcome::fail(login_reject_hint(
                            body.trim(),
                            &e.to_string(),
                        )));
                    }
                }
            }
        }
        self.refresh_version().await?;
        self.apply_login_prefs().await?;

        // EE shadowban 模式需服务端启用该功能。
        if self.shadow() {
            let prefs = self.get_text("/app/preferences").await?;
            let enabled = serde_json::from_str::<QbPreferences>(&prefs)
                .map(|p| p.shadow_ban_enabled)
                .unwrap_or(false);
            if !enabled {
                return Ok(LoginOutcome::fail(
                    "qBittorrentEE 未启用 shadow ban，请在客户端开启或关闭 use-shadow-ban",
                ));
            }
        }
        Ok(LoginOutcome::ok())
    }

    async fn get_torrents(&self) -> Result<Vec<Torrent>> {
        let mut out = Vec::new();
        let mut seen = std::collections::HashSet::new();
        let mut offset = 0u32;
        loop {
            let path = format!("/torrents/info?filter=active&limit={PAGE_SIZE}&offset={offset}");
            let body = self.get_text(&path).await?;
            let page: Vec<QbTorrent> = serde_json::from_str(&body)
                .map_err(|e| DownloaderError::Api(format!("解析 torrents 失败: {e}")))?;
            let n = page.len();
            for t in page {
                if !seen.insert(t.hash.clone()) {
                    continue;
                }
                let is_private = t.is_private.unwrap_or(false);
                if self.config.ignore_private && is_private {
                    continue;
                }
                out.push(Torrent {
                    id: t.hash.clone(),
                    hash: t.hash,
                    name: t.name,
                    progress: t.progress,
                    size: t.total_size,
                    completed_size: t.completed.unwrap_or(-1), // qB /torrents/info 的 completed
                    private_torrent: is_private,
                });
            }
            if n < PAGE_SIZE as usize {
                break;
            }
            offset += PAGE_SIZE;
        }
        Ok(out)
    }

    async fn get_peers(&self, torrent: &Torrent) -> Result<Vec<Peer>> {
        let body = self
            .get_text(&format!("/sync/torrentPeers?hash={}", torrent.hash))
            .await?;
        let root: serde_json::Value = serde_json::from_str(&body)
            .map_err(|e| DownloaderError::Api(format!("解析 peers 失败: {e}")))?;
        let mut out = Vec::new();
        let Some(peers) = root.get("peers").and_then(|p| p.as_object()) else {
            return Ok(out);
        };
        for (key, val) in peers {
            // 过滤 Web/onion/i2p。
            if key.contains(".onion") || key.contains(".i2p") {
                continue;
            }
            let p: QbPeer = match serde_json::from_value(val.clone()) {
                Ok(p) => p,
                Err(_) => continue,
            };
            let conn = p.connection.to_ascii_lowercase();
            if conn == "http" || conn == "https" || conn == "web" {
                continue;
            }
            if p.ip.trim().is_empty() {
                continue;
            }
            if self.shadow() && p.shadowbanned.unwrap_or(false) {
                continue;
            }
            let Ok(ip) = p.ip.trim().parse::<std::net::IpAddr>() else {
                continue;
            };
            let mut address = PeerAddress::new(ip, p.port);
            address.raw_ip = key.clone(); // 封禁回传必须用下载器给的原始键
            out.push(Peer {
                address,
                peer_id: non_empty(p.peer_id_client),
                client_name: non_empty(p.client),
                download_speed: p.dl_speed,
                upload_speed: p.up_speed,
                downloaded: p.downloaded,
                uploaded: p.uploaded,
                progress: p.progress,
                flags: Some(PeerFlag::parse(&p.flags)),
            });
        }
        Ok(out)
    }

    async fn apply_ban_list(
        &self,
        full_banned: &[String],
        newly_added_peers: &[String],
        apply_full: bool,
    ) -> Result<()> {
        let increment = !apply_full && self.config.increment_ban && !newly_added_peers.is_empty();
        if increment {
            let peers = join_increment_peers(newly_added_peers);
            let path = if self.shadow() {
                "/transfer/shadowbanPeers"
            } else {
                "/transfer/banPeers"
            };
            self.post_form(path, &[("peers", &peers)]).await?;
        } else {
            let support_range = self.range_ban.load(Ordering::Relaxed);
            let s = join_full_ban_string(full_banned, support_range);
            let key = if self.shadow() {
                "shadow_banned_IPs"
            } else {
                "banned_IPs"
            };
            let mut obj = serde_json::Map::new();
            obj.insert(key.to_string(), serde_json::Value::String(s));
            let json = serde_json::Value::Object(obj).to_string();
            self.post_form("/app/setPreferences", &[("json", &json)])
                .await?;
        }
        Ok(())
    }
}

fn non_empty(s: String) -> Option<String> {
    if s.trim().is_empty() {
        None
    } else {
        Some(s)
    }
}

/// 组装可操作的登录失败原因：区分「账密错」与「未开白名单免鉴权」两种常见成因。
/// `body`=qB /auth/login 的原始返回（如 "Fails."）；`probe`=免鉴权探活失败详情。
fn login_reject_hint(body: &str, probe: &str) -> String {
    format!(
        "登录失败：qB /api/v2/auth/login 返回 \"{body}\"，且免鉴权探活也失败（{probe}）。请排查：\
         ① 若 qB 设了账号密码，检查该下载器填写的用户名/密码是否正确；\
         ② 若你想免密使用，请在 qB「选项 → Web UI」勾选「对本地主机上的客户端跳过身份验证」，\
         或把运行本程序的机器 IP 加入「对白名单子网中的客户端跳过身份验证」；\
         ③ 确认端点地址/端口正确、反向代理未拦截 /api/v2 路径。"
    )
}

/// 把非 2xx 响应映射为错误：401/403 → `Auth`（重试无益，应暂停），其余 → `Api`。
fn status_error(path: &str, status: reqwest::StatusCode, body: &str) -> DownloaderError {
    let msg = format!("{path} → {status}: {body}");
    if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
        DownloaderError::Auth(msg)
    } else {
        DownloaderError::Api(msg)
    }
}

/// 宽松解析版本（去前缀 `v`，补足到 x.y.z）。
fn parse_version(s: &str) -> Option<semver::Version> {
    let s = s.trim().trim_start_matches('v').trim();
    if let Ok(v) = semver::Version::parse(s) {
        return Some(v);
    }
    let mut nums: Vec<String> = s
        .split('.')
        .take(3)
        .map(|p| p.chars().take_while(|c| c.is_ascii_digit()).collect())
        .filter(|p: &String| !p.is_empty())
        .collect();
    while nums.len() < 3 {
        nums.push("0".into());
    }
    semver::Version::parse(&nums.join(".")).ok()
}

// ---------------- qB JSON DTO ----------------

#[derive(Debug, Deserialize)]
struct QbTorrent {
    hash: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    total_size: i64,
    /// 已完成数据量（字节）。qB `/torrents/info` 提供;缺失时为 None → completed_size=-1。
    #[serde(default)]
    completed: Option<i64>,
    #[serde(default)]
    progress: f64,
    #[serde(default)]
    is_private: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct QbPeer {
    #[serde(default)]
    client: String,
    #[serde(default)]
    connection: String,
    #[serde(default)]
    dl_speed: i64,
    #[serde(default)]
    up_speed: i64,
    #[serde(default)]
    downloaded: i64,
    #[serde(default)]
    uploaded: i64,
    #[serde(default)]
    flags: String,
    #[serde(default)]
    ip: String,
    #[serde(default)]
    peer_id_client: String,
    #[serde(default)]
    port: u16,
    #[serde(default)]
    progress: f64,
    #[serde(default)]
    shadowbanned: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct QbPreferences {
    #[serde(default)]
    shadow_ban_enabled: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_parse_lenient() {
        assert_eq!(
            parse_version("v5.3.0").unwrap(),
            semver::Version::new(5, 3, 0)
        );
        assert_eq!(parse_version("4.6").unwrap(), semver::Version::new(4, 6, 0));
        assert!(parse_version("5.3.0").unwrap() >= semver::Version::new(5, 3, 0));
        assert!(parse_version("5.0.1").unwrap() < semver::Version::new(5, 3, 0));
    }

    #[test]
    fn build_client_sets_endpoint_and_kind() {
        let c = DownloaderConfig {
            id: "d".into(),
            kind: "qbittorrentee".into(),
            endpoint: "http://host:8080/".into(),
            use_shadow_ban: true,
            ..Default::default()
        };
        let cli = QBittorrentClient::new(c).unwrap();
        assert_eq!(cli.api, "http://host:8080/api/v2");
        assert_eq!(cli.type_label(), "qBittorrentEE");
        assert!(cli.shadow());
        assert!(cli.feature_flags().contains(&FeatureFlag::UnbanIp));
    }

    #[test]
    fn peer_json_parses() {
        let v: serde_json::Value = serde_json::from_str(
            r#"{"client":"qBittorrent/4.2.5","connection":"BT","ip":"1.2.3.4","port":6881,"dl_speed":10,"up_speed":20,"progress":0.5,"flags":"d U","peer_id_client":"-qB4250-"}"#,
        )
        .unwrap();
        let p: QbPeer = serde_json::from_value(v).unwrap();
        assert_eq!(p.ip, "1.2.3.4");
        assert_eq!(p.port, 6881);
        assert_eq!(p.up_speed, 20);
    }

    #[test]
    fn torrent_json_parses_completed() {
        // 含 completed → 用作 completed_size。
        let t: QbTorrent = serde_json::from_str(
            r#"{"hash":"abc","name":"x","total_size":1000,"completed":640,"progress":0.64,"is_private":true}"#,
        )
        .unwrap();
        assert_eq!(t.total_size, 1000);
        assert_eq!(t.completed, Some(640));
        // 缺 completed → None（→ completed_size=-1）。
        let t2: QbTorrent = serde_json::from_str(r#"{"hash":"d","total_size":50}"#).unwrap();
        assert_eq!(t2.completed, None);
    }

    #[test]
    fn status_error_classifies_auth() {
        use reqwest::StatusCode;
        // 401/403 → Auth（应暂停重试）。
        assert!(status_error("/auth/login", StatusCode::FORBIDDEN, "banned").is_auth());
        assert!(status_error("/auth/login", StatusCode::UNAUTHORIZED, "no").is_auth());
        // 其它（5xx/4xx 非鉴权）→ Api（可重试）。
        assert!(!status_error("/x", StatusCode::INTERNAL_SERVER_ERROR, "oops").is_auth());
        assert!(!status_error("/x", StatusCode::NOT_FOUND, "nf").is_auth());
    }

    #[test]
    fn login_reject_hint_is_actionable() {
        let h = login_reject_hint("Fails.", "403 Forbidden");
        assert!(h.contains("Fails."));
        assert!(h.contains("跳过身份验证")); // 指向 qB 白名单免鉴权设置
        assert!(h.contains("用户名/密码")); // 也提示检查账密
    }

    /// 极简 qB mock：按请求路径返回 canned 响应。`version_ok=false` 时 /app/version 返 403。
    async fn spawn_mock_qb(version_ok: bool) -> String {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else {
                    break;
                };
                let mut buf = [0u8; 4096];
                let n = sock.read(&mut buf).await.unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]);
                let first = req.lines().next().unwrap_or("");
                let (code, body) = if first.contains("/auth/login") {
                    ("200 OK", "Fails.".to_string()) // 空账密 → qB 登录接口回 Fails.
                } else if first.contains("/app/version") {
                    if version_ok {
                        ("200 OK", "v5.0.0".to_string()) // 白名单绕过：需鉴权接口照常可用
                    } else {
                        ("403 Forbidden", "Forbidden".to_string()) // 无绕过：真失败
                    }
                } else {
                    ("200 OK", String::new()) // setPreferences 等
                };
                let resp = format!(
                    "HTTP/1.1 {code}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = sock.write_all(resp.as_bytes()).await;
                let _ = sock.shutdown().await;
            }
        });
        format!("http://{addr}")
    }

    fn cfg_for(endpoint: String) -> DownloaderConfig {
        DownloaderConfig {
            id: "d".into(),
            kind: "qbittorrent".into(),
            endpoint,
            ..Default::default() // 账密留空：模拟靠 qB 白名单免鉴权
        }
    }

    #[tokio::test]
    async fn login_tolerates_qb_auth_bypass() {
        // /auth/login 回 Fails. 但 /app/version 可访问 → 应按已登录继续（修复"能连上却测试报错"）。
        let ep = spawn_mock_qb(true).await;
        let cli = QBittorrentClient::new(cfg_for(ep)).unwrap();
        let o = cli.login().await.unwrap();
        assert!(o.success, "白名单绕过下 login 应成功，实际: {}", o.message);
    }

    #[tokio::test]
    async fn login_fails_with_hint_when_no_bypass() {
        // /auth/login 回 Fails. 且 /app/version 也 403 → 真失败，且给出可操作提示。
        let ep = spawn_mock_qb(false).await;
        let cli = QBittorrentClient::new(cfg_for(ep)).unwrap();
        let o = cli.login().await.unwrap();
        assert!(!o.success);
        assert!(o.message.contains("跳过身份验证"), "应含排查提示: {}", o.message);
    }
}
