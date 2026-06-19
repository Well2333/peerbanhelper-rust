//! pbh-downloader —— 下载器抽象 + qBittorrent / qBittorrentEE。对应上游 `downloader/**`。
//!
//! 本期仅 qB + qBEE，但保留 `Downloader` trait + 工厂注册以维持可扩展性。
//! 协议细节见 `memory/design/architecture-analysis.md` §2.2，封禁串须与上游字节级一致。
//! v2 精简：trait 只含与封禁相关的能力（无 speed-limiter / listen_port / NAT）。

mod qbittorrent;

use std::collections::BTreeSet;
use std::sync::Arc;

use async_trait::async_trait;
use pbh_domain::{Peer, Torrent};
use serde::{Deserialize, Serialize};

pub use qbittorrent::QBittorrentClient;

/// 下载器交互错误。
#[derive(Debug, thiserror::Error)]
pub enum DownloaderError {
    #[error("HTTP 错误: {0}")]
    Http(#[from] reqwest::Error),
    #[error("下载器 API 错误: {0}")]
    Api(String),
    #[error("配置错误: {0}")]
    Config(String),
}

pub type Result<T> = std::result::Result<T, DownloaderError>;

/// 下载器特性标志。对应上游 `DownloaderFeatureFlag`（v2 子集）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeatureFlag {
    /// 可解封（全量替换即隐含支持）。
    UnbanIp,
    /// 支持下发 CIDR 段封禁（qB ≥ 5.3.0）。
    RangeBanIp,
}

/// 登录结果。
#[derive(Debug, Clone)]
pub struct LoginOutcome {
    pub success: bool,
    pub message: String,
}

impl LoginOutcome {
    pub fn ok() -> Self {
        LoginOutcome {
            success: true,
            message: "ok".into(),
        }
    }
    pub fn fail(msg: impl Into<String>) -> Self {
        LoginOutcome {
            success: false,
            message: msg.into(),
        }
    }
}

/// HTTP Basic 认证（位于 qB 登录之外，用于反代）。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BasicAuth {
    pub user: String,
    pub pass: String,
}

/// 下载器配置（v2 自有格式，YAML/JSON 均 kebab-case）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct DownloaderConfig {
    /// 唯一 id（管理器分配/持久化）。
    pub id: String,
    /// 类型：`qbittorrent` / `qbittorrentee`。
    #[serde(rename = "type")]
    pub kind: String,
    pub name: String,
    /// 基础 URL（运行时追加 `/api/v2`）。
    pub endpoint: String,
    pub username: String,
    pub password: String,
    /// qB ≥ 5.2.0 的 Bearer api-key；非空则跳过 `/auth/login`。
    pub api_key: String,
    /// HTTP Basic（反代）；user 为空表示不启用。
    pub basic_auth: BasicAuth,
    /// 增量封禁（`/transfer/banPeers`）。
    pub increment_ban: bool,
    /// EE：走 shadowban API。
    pub use_shadow_ban: bool,
    /// 校验 TLS 证书。
    pub verify_ssl: bool,
    /// 排除私有种子。
    pub ignore_private: bool,
    /// 暂停（不参与 ban wave）。
    pub paused: bool,
}

impl Default for DownloaderConfig {
    fn default() -> Self {
        DownloaderConfig {
            id: String::new(),
            kind: "qbittorrent".into(),
            name: String::new(),
            endpoint: String::new(),
            username: String::new(),
            password: String::new(),
            api_key: String::new(),
            basic_auth: BasicAuth::default(),
            increment_ban: false,
            use_shadow_ban: false,
            verify_ssl: true,
            ignore_private: false,
            paused: false,
        }
    }
}

/// 下载器抽象（v2 精简：仅封禁相关能力）。
#[async_trait]
pub trait Downloader: Send + Sync {
    fn id(&self) -> &str;
    fn name(&self) -> &str;
    fn type_label(&self) -> &str;
    fn feature_flags(&self) -> Vec<FeatureFlag>;
    fn is_paused(&self) -> bool;

    /// 登录（含会话建立与必要的偏好设置）。
    async fn login(&self) -> Result<LoginOutcome>;
    /// 活动种子列表（按 `ignore_private` 过滤私有种子）。
    async fn get_torrents(&self) -> Result<Vec<Torrent>>;
    /// 某种子的 peer 列表（已过滤 Web/onion/i2p 等）。
    async fn get_peers(&self, torrent: &Torrent) -> Result<Vec<Peer>>;
    /// 应用封禁。`full_banned`=全部封禁网络字符串(全量模式);`newly_added_peers`=新增 peer 的 `ip:port`(增量模式);
    /// `apply_full`=强制全量替换。
    async fn apply_ban_list(
        &self,
        full_banned: &[String],
        newly_added_peers: &[String],
        apply_full: bool,
    ) -> Result<()>;
}

/// 工厂：按配置 `type` 构造下载器（表驱动，便于扩展）。
pub fn build_downloader(config: DownloaderConfig) -> Result<Arc<dyn Downloader>> {
    match config.kind.to_lowercase().as_str() {
        "qbittorrent" | "qbittorrentee" => Ok(Arc::new(QBittorrentClient::new(config)?)),
        other => Err(DownloaderError::Config(format!(
            "不支持的下载器类型: {other}"
        ))),
    }
}

// ---------------- 封禁串拼装（纯逻辑，可测） ----------------

/// 拼装全量封禁串：规范化(compressed) + 去重 + 换行分隔。
/// `support_range=false` 时丢弃带前缀的 CIDR（老 qB 只支持单 IP），仅保留主机地址。
pub fn join_full_ban_string(networks: &[String], support_range: bool) -> String {
    let mut seen = BTreeSet::new();
    let mut out: Vec<String> = Vec::new();
    for raw in networks {
        if let Some(s) = normalize_entry(raw, support_range) {
            if seen.insert(s.clone()) {
                out.push(s);
            }
        }
    }
    out.join("\n")
}

/// 拼装增量封禁串：`ip:port` 用 `|` 分隔 + 去重。
pub fn join_increment_peers(peer_rawips: &[String]) -> String {
    let mut seen = BTreeSet::new();
    let mut out: Vec<String> = Vec::new();
    for p in peer_rawips {
        if seen.insert(p.clone()) {
            out.push(p.clone());
        }
    }
    out.join("|")
}

/// 把一个 IP/CIDR 字符串规范化为 qB `banned_IPs` 接受的形式。
fn normalize_entry(raw: &str, support_range: bool) -> Option<String> {
    let raw = raw.trim();
    // CIDR？
    if let Ok(net) = raw.parse::<ipnet::IpNet>() {
        let host_len = match net {
            ipnet::IpNet::V4(_) => 32,
            ipnet::IpNet::V6(_) => 128,
        };
        if net.prefix_len() == host_len {
            // 单主机：发地址本身。
            return Some(net.addr().to_string());
        }
        if support_range {
            return Some(format!("{}/{}", net.network(), net.prefix_len()));
        }
        // 老 qB 不支持段封禁：丢弃（避免发出会被忽略/报错的条目）。
        return None;
    }
    // 裸 IP。
    if let Ok(ip) = raw.parse::<std::net::IpAddr>() {
        return Some(ip.to_string());
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_ban_single_ips() {
        let s = join_full_ban_string(
            &["1.2.3.4".into(), "1.2.3.4".into(), "5.6.7.8".into()],
            true,
        );
        assert_eq!(s, "1.2.3.4\n5.6.7.8"); // 去重 + 换行
    }

    #[test]
    fn full_ban_cidr_gated_by_range_support() {
        let nets = vec!["10.0.0.0/24".to_string(), "1.2.3.4".to_string()];
        let with = join_full_ban_string(&nets, true);
        assert!(with.contains("10.0.0.0/24"));
        assert!(with.contains("1.2.3.4"));
        let without = join_full_ban_string(&nets, false);
        assert_eq!(without, "1.2.3.4");
    }

    #[test]
    fn ipv6_compressed_and_slash32_host() {
        assert_eq!(
            join_full_ban_string(&["9.9.9.9/32".into()], true),
            "9.9.9.9"
        );
        let s = join_full_ban_string(&["2001:0db8:0000:0000:0000:0000:0000:0001".into()], true);
        assert_eq!(s, "2001:db8::1");
    }

    #[test]
    fn increment_pipe_joined_distinct() {
        let s = join_increment_peers(&[
            "1.2.3.4:6881".into(),
            "1.2.3.4:6881".into(),
            "[2001:db8::1]:6881".into(),
        ]);
        assert_eq!(s, "1.2.3.4:6881|[2001:db8::1]:6881");
    }

    #[test]
    fn config_yaml_roundtrip_kebab() {
        let c = DownloaderConfig {
            id: "d1".into(),
            kind: "qbittorrentee".into(),
            name: "我的 qB".into(),
            endpoint: "http://127.0.0.1:8080".into(),
            increment_ban: true,
            ..Default::default()
        };
        let y = serde_yaml::to_string(&c).unwrap();
        assert!(y.contains("increment-ban"));
        assert!(y.contains("type: qbittorrentee"));
        let back: DownloaderConfig = serde_yaml::from_str(&y).unwrap();
        assert_eq!(back.kind, "qbittorrentee");
        assert!(back.increment_ban);
        assert!(back.verify_ssl);
    }

    #[test]
    fn factory_rejects_unknown() {
        let c = DownloaderConfig {
            kind: "transmission".into(),
            ..Default::default()
        };
        assert!(build_downloader(c).is_err());
    }
}
