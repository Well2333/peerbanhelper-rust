//! 路由与处理器。

use std::time::{SystemTime, UNIX_EPOCH};

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, Request, State};
use axum::http::{header, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::json;

use crate::{ApiResp, Page, WebState};
use pbh_config::ProfileConfig;
use pbh_downloader::DownloaderConfig;

/// 组装路由。`/` 与 `/api/auth/login` 公开，其余 `/api/*` 需 Bearer token。
pub fn router(state: WebState) -> Router {
    let protected = Router::new()
        .route("/api/status", get(status))
        .route(
            "/api/downloaders",
            get(list_downloaders).put(upsert_downloader),
        )
        .route("/api/downloaders/:id", delete(delete_downloader))
        .route("/api/downloaders/test", post(test_downloader))
        .route("/api/bans", get(list_bans).put(add_ban))
        .route("/api/bans/:ip", delete(remove_ban))
        .route("/api/bans/history", get(ban_history))
        .route("/api/config/profile", get(get_profile).put(put_profile))
        .route("/api/config/app", get(get_app_config).put(put_app_config))
        .route("/api/sub/rules", get(list_sub_rules).put(upsert_sub_rule))
        .route("/api/sub/rules/:id", delete(delete_sub_rule))
        .route("/api/sub/logs", get(sub_rule_logs))
        .route("/api/logs", get(get_logs))
        .route("/api/geoip/update", post(geoip_update))
        .route("/api/update/check", get(update_check))
        .route_layer(middleware::from_fn_with_state(state.clone(), auth));

    Router::new()
        .route("/", get(index))
        .route("/api/auth/login", post(login))
        // 公开纯文本封禁列表,供下载器/外部消费（无需鉴权）。
        .route("/blocklist/ip", get(blocklist_ip))
        // WS 实时日志流（浏览器 WS 不能设头,token 走 query）。
        .route("/api/logs/stream", get(logs_stream))
        .merge(protected)
        .with_state(state)
}

#[derive(Deserialize)]
struct WsQuery {
    token: String,
    #[serde(default)]
    offset: u64,
}

/// WS 升级 + token 校验（query 参数）。
async fn logs_stream(
    ws: WebSocketUpgrade,
    State(st): State<WebState>,
    Query(q): Query<WsQuery>,
) -> Response {
    let token = st.config.current().app.server.token.clone();
    if token.is_empty() || q.token != token {
        return unauthorized();
    }
    ws.on_upgrade(move |socket| log_socket(socket, st, q.offset))
}

/// 推送 `seq > offset` 的日志,之后周期推送新增（700ms 轮询环形缓冲）。
async fn log_socket(mut socket: WebSocket, st: WebState, mut last: u64) {
    let mut tick = tokio::time::interval(std::time::Duration::from_millis(700));
    loop {
        tokio::select! {
            _ = tick.tick() => {
                for e in st.logs.since(last) {
                    last = last.max(e.seq);
                    let msg = json!({
                        "seq": e.seq, "time_ms": e.time_ms, "level": e.level, "message": e.message
                    }).to_string();
                    if socket.send(Message::Text(msg)).await.is_err() {
                        return;
                    }
                }
            }
            inbound = socket.recv() => {
                match inbound {
                    None | Some(Ok(Message::Close(_))) | Some(Err(_)) => return,
                    _ => {}
                }
            }
        }
    }
}

/// 纯文本导出当前封禁的 IP/CIDR（每行一条）。
async fn blocklist_ip(State(st): State<WebState>) -> Response {
    let lines: Vec<String> = st
        .ban_manager
        .ban_list()
        .snapshot()
        .into_iter()
        .map(|(net, _)| net)
        .collect();
    (
        [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
        lines.join("\n"),
    )
        .into_response()
}

async fn index() -> Html<&'static str> {
    Html(include_str!("../assets/index.html"))
}

// ---------------- 鉴权 ----------------

async fn auth(State(st): State<WebState>, req: Request, next: Next) -> Response {
    let token = st.config.current().app.server.token.clone();
    let ok = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .map(|t| !token.is_empty() && t == token)
        .unwrap_or(false);
    if ok {
        next.run(req).await
    } else {
        unauthorized()
    }
}

fn unauthorized() -> Response {
    (StatusCode::UNAUTHORIZED, Json(ApiResp::<()>::err("未授权"))).into_response()
}

fn bad_request(msg: impl Into<String>) -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(ApiResp::<()>::err(msg.into())),
    )
        .into_response()
}

// ---------------- 处理器 ----------------

#[derive(Deserialize)]
struct LoginBody {
    token: String,
}

async fn login(State(st): State<WebState>, Json(b): Json<LoginBody>) -> Response {
    let token = st.config.current().app.server.token.clone();
    if !token.is_empty() && b.token == token {
        ApiResp::ok_empty().into_response()
    } else {
        unauthorized()
    }
}

async fn status(State(st): State<WebState>) -> Response {
    let cfg = st.config.current();
    let s = st.ban_manager.stats();
    let login = st.ban_manager.downloader_status();
    let downloader_list: Vec<_> = st
        .downloaders
        .configs()
        .into_iter()
        .map(|c| {
            json!({
                "id": c.id,
                "name": c.name,
                "type": c.kind,
                "endpoint": c.endpoint,
                "paused": c.paused,
                "online": login.get(&c.id).copied(),
            })
        })
        .collect();
    ApiResp::ok(json!({
        "version": env!("CARGO_PKG_VERSION"),
        "downloaders": st.downloaders.count(),
        "modules": st.ban_manager.module_count(),
        "banned": st.ban_manager.ban_list().len(),
        "check_interval": cfg.profile.check_interval,
        "ban_duration": cfg.profile.ban_duration,
        "stats": {
            "checked_peers": s.checked_peers,
            "banned_peers": s.banned_peers,
            "unbanned_peers": s.unbanned_peers,
            "waves": s.waves,
            "last_wave_at": s.last_wave_at,
            "last_wave_ms": s.last_wave_ms,
        },
        "downloader_list": downloader_list,
    }))
    .into_response()
}

// ---------------- 规则配置（profile.yml）----------------

async fn get_profile(State(st): State<WebState>) -> Response {
    let profile = st.config.current().profile.clone();
    let yaml = match serde_yaml::to_string(&profile) {
        Ok(y) => y,
        Err(e) => return bad_request(format!("序列化 profile 失败: {e}")),
    };
    let profile_json = serde_json::to_value(&profile).unwrap_or(serde_json::Value::Null);
    ApiResp::ok(json!({ "yaml": yaml, "profile": profile_json })).into_response()
}

#[derive(Deserialize)]
struct ProfileBody {
    #[serde(default)]
    yaml: Option<String>,
    #[serde(default)]
    profile: Option<serde_json::Value>,
}

async fn put_profile(State(st): State<WebState>, Json(b): Json<ProfileBody>) -> Response {
    // 解析校验：优先使用 profile JSON，回退到 yaml。
    let profile: ProfileConfig = if let Some(v) = b.profile {
        match serde_json::from_value(v) {
            Ok(p) => p,
            Err(e) => return bad_request(format!("profile JSON 解析失败: {e}")),
        }
    } else if let Some(y) = b.yaml {
        match serde_yaml::from_str(&y) {
            Ok(p) => p,
            Err(e) => return bad_request(format!("YAML 解析失败: {e}")),
        }
    } else {
        return bad_request("缺少 yaml 或 profile");
    };
    // 写盘 + 热重载配置。
    if let Err(e) = st.config.save_profile(&profile) {
        return bad_request(format!("保存失败: {e}"));
    }
    // 重建规则模块（即时生效，无需重启）。
    let p = st.config.current().profile.clone();
    let modules = pbh_engine::build_modules(
        &p,
        p.ban_duration,
        st.ban_manager.ban_list(),
        &st.db,
        &st.geoip,
        &st.btn.current_state(),
        &st.config.current().app.network.proxy,
    );
    let n = modules.len();
    st.ban_manager.rebuild_modules(modules);
    ApiResp::ok(json!({ "modules": n })).into_response()
}

// ---------------- 应用配置（config.yml 可编辑子集）----------------

async fn get_app_config(State(st): State<WebState>) -> Response {
    let a = st.config.current().app.clone();
    ApiResp::ok(json!({
        "btn": {
            "enabled": a.btn.enabled,
            "config_url": a.btn.config_url,
            "submit": a.btn.submit,
            "app_id": a.btn.app_id,
            "app_secret": a.btn.app_secret,
        },
        "ip_database": {
            "account_id": a.ip_database.account_id,
            "license_key": a.ip_database.license_key,
            "auto_update": a.ip_database.auto_update,
        },
        "network": { "proxy": a.network.proxy },
        "persist": { "ban_logs_keep_days": a.persist.ban_logs_keep_days },
    }))
    .into_response()
}

#[derive(Deserialize)]
struct AppConfigBody {
    btn: Option<BtnBody>,
    ip_database: Option<IpDbBody>,
    network: Option<NetBody>,
    persist: Option<PersistBody>,
}

#[derive(Deserialize)]
struct BtnBody {
    enabled: bool,
    config_url: String,
    submit: bool,
    app_id: String,
    app_secret: String,
}

#[derive(Deserialize)]
struct IpDbBody {
    account_id: String,
    license_key: String,
    auto_update: bool,
}

#[derive(Deserialize)]
struct NetBody {
    proxy: String,
}

#[derive(Deserialize)]
struct PersistBody {
    ban_logs_keep_days: i64,
}

async fn put_app_config(State(st): State<WebState>, Json(b): Json<AppConfigBody>) -> Response {
    let mut app = st.config.current().app.clone();
    if let Some(x) = b.btn {
        app.btn.enabled = x.enabled;
        app.btn.config_url = x.config_url;
        app.btn.submit = x.submit;
        app.btn.app_id = x.app_id;
        app.btn.app_secret = x.app_secret;
    }
    if let Some(x) = b.ip_database {
        app.ip_database.account_id = x.account_id;
        app.ip_database.license_key = x.license_key;
        app.ip_database.auto_update = x.auto_update;
    }
    if let Some(x) = b.network {
        app.network.proxy = x.proxy;
    }
    if let Some(x) = b.persist {
        app.persist.ban_logs_keep_days = x.ban_logs_keep_days;
    }
    if let Err(e) = st.config.save_app(&app) {
        return bad_request(format!("保存失败: {e}"));
    }
    // 热加载：重启 BTN（应用新 enabled/凭证/代理），再重建规则模块。
    let dur = st.config.current().profile.ban_duration;
    st.btn.apply(&app, dur);
    let p = st.config.current().profile.clone();
    let modules = pbh_engine::build_modules(
        &p,
        p.ban_duration,
        st.ban_manager.ban_list(),
        &st.db,
        &st.geoip,
        &st.btn.current_state(),
        &app.network.proxy,
    );
    let n = modules.len();
    st.ban_manager.rebuild_modules(modules);
    ApiResp::ok(json!({ "modules": n })).into_response()
}

async fn list_downloaders(State(st): State<WebState>) -> Response {
    ApiResp::ok(st.downloaders.configs()).into_response()
}

async fn upsert_downloader(
    State(st): State<WebState>,
    Json(mut cfg): Json<DownloaderConfig>,
) -> Response {
    if cfg.id.trim().is_empty() {
        cfg.id = gen_id();
    }
    match st.downloaders.upsert(cfg) {
        Ok(()) => ApiResp::ok_empty().into_response(),
        Err(e) => bad_request(e.to_string()),
    }
}

async fn delete_downloader(State(st): State<WebState>, Path(id): Path<String>) -> Response {
    match st.downloaders.remove(&id) {
        Ok(_) => ApiResp::ok_empty().into_response(),
        Err(e) => bad_request(e.to_string()),
    }
}

async fn test_downloader(Json(cfg): Json<DownloaderConfig>) -> Response {
    match pbh_downloader::build_downloader(cfg) {
        Ok(d) => match d.login().await {
            Ok(o) => {
                ApiResp::ok(json!({"success": o.success, "message": o.message})).into_response()
            }
            Err(e) => {
                ApiResp::ok(json!({"success": false, "message": e.to_string()})).into_response()
            }
        },
        Err(e) => bad_request(e.to_string()),
    }
}

async fn list_bans(State(st): State<WebState>) -> Response {
    let items: Vec<_> = st
        .ban_manager
        .ban_list()
        .snapshot()
        .into_iter()
        .map(|(net, m)| {
            // GeoIP 可用时按需补地理（国家/城市/ASN）。
            let geo = st.geoip.query(m.peer.ip).map(|d| {
                let parts: Vec<String> = [d.country_iso, d.city_name, d.as_organization]
                    .into_iter()
                    .flatten()
                    .collect();
                parts.join(" / ")
            });
            json!({
                "address": net,
                "ip": m.peer.ip.to_string(),
                "port": m.peer.port,
                "module": m.context,
                "rule": m.rule,
                "description": m.description,
                "ban_at": m.ban_at,
                "unban_at": m.unban_at,
                "geo": geo,
            })
        })
        .collect();
    ApiResp::ok(json!({ "total": items.len(), "items": items })).into_response()
}

#[derive(Deserialize)]
struct AddBan {
    ip: String,
    #[serde(default)]
    duration_ms: i64,
}

async fn add_ban(State(st): State<WebState>, Json(b): Json<AddBan>) -> Response {
    if st.ban_manager.manual_ban(&b.ip, b.duration_ms) {
        ApiResp::ok_empty().into_response()
    } else {
        bad_request("无效 IP")
    }
}

async fn remove_ban(State(st): State<WebState>, Path(ip): Path<String>) -> Response {
    st.ban_manager.manual_unban(&ip);
    ApiResp::ok_empty().into_response()
}

#[derive(Deserialize)]
struct PageQuery {
    page: Option<i64>,
    #[serde(rename = "pageSize")]
    page_size: Option<i64>,
}

async fn ban_history(State(st): State<WebState>, Query(q): Query<PageQuery>) -> Response {
    let page = q.page.unwrap_or(1).max(1);
    let size = q.page_size.unwrap_or(20).clamp(1, 200);
    let offset = (page - 1) * size;
    let items = st
        .db
        .query_ban_history(size, offset)
        .await
        .unwrap_or_default();
    let total = st.db.count_ban_history().await.unwrap_or(0);
    ApiResp::ok(Page {
        page,
        size,
        total,
        items,
    })
    .into_response()
}

// ---------------- IP 黑名单订阅管理 ----------------

/// 列出订阅：以 profile.yml 配置为准（id/name/url/enabled），合并 DB 状态（条数/最后更新）。
async fn list_sub_rules(State(st): State<WebState>) -> Response {
    let profile = st.config.current().profile.clone();
    let configured = profile_sub_rules(&profile);
    let db_rows = st.db.list_rule_subs().await.unwrap_or_default();
    let items: Vec<_> = configured
        .into_iter()
        .map(|(rule_id, name, url, enabled)| {
            let st_row = db_rows.iter().find(|r| r.rule_id == rule_id);
            json!({
                "rule_id": rule_id,
                "rule_name": name,
                "sub_url": url,
                "enabled": enabled,
                "ent_count": st_row.and_then(|r| r.ent_count),
                "last_update": st_row.and_then(|r| r.last_update),
            })
        })
        .collect();
    ApiResp::ok(items).into_response()
}

#[derive(Deserialize)]
struct SubRuleBody {
    rule_id: String,
    name: String,
    url: String,
    #[serde(default = "yes")]
    enabled: bool,
}
fn yes() -> bool {
    true
}

/// 新增/更新一条订阅（写入 profile.yml 的 ip-address-blocker-rules.rules）。
async fn upsert_sub_rule(State(st): State<WebState>, Json(b): Json<SubRuleBody>) -> Response {
    if b.rule_id.trim().is_empty() || b.url.trim().is_empty() {
        return bad_request("rule_id 与 url 必填");
    }
    if b.rule_id.contains('.') {
        return bad_request("rule_id 不可含 '.'");
    }
    let mut profile = st.config.current().profile.clone();
    set_sub_rule(&mut profile, &b);
    match save_and_rebuild(&st, profile).await {
        Ok(n) => ApiResp::ok(json!({ "modules": n })).into_response(),
        Err(e) => bad_request(e),
    }
}

/// 删除一条订阅。
async fn delete_sub_rule(State(st): State<WebState>, Path(id): Path<String>) -> Response {
    let mut profile = st.config.current().profile.clone();
    remove_sub_rule(&mut profile, &id);
    let _ = st.db.delete_rule_sub(&id).await;
    match save_and_rebuild(&st, profile).await {
        Ok(n) => ApiResp::ok(json!({ "modules": n })).into_response(),
        Err(e) => bad_request(e),
    }
}

/// 从 profile 提取已配置订阅 (id, name, url, enabled)。
fn profile_sub_rules(profile: &ProfileConfig) -> Vec<(String, String, String, bool)> {
    let mut out = Vec::new();
    let Some(rules) = profile
        .module_section("ip-address-blocker-rules")
        .and_then(|s| s.get("rules"))
        .and_then(|v| v.as_mapping())
    else {
        return out;
    };
    for (k, v) in rules {
        let Some(id) = k.as_str() else { continue };
        out.push((
            id.to_string(),
            v.get("name")
                .and_then(|n| n.as_str())
                .unwrap_or(id)
                .to_string(),
            v.get("url")
                .and_then(|u| u.as_str())
                .unwrap_or("")
                .to_string(),
            v.get("enabled").and_then(|e| e.as_bool()).unwrap_or(true),
        ));
    }
    out
}

fn set_sub_rule(profile: &mut ProfileConfig, b: &SubRuleBody) {
    use serde_yaml::{Mapping, Value};
    let sec = profile
        .module
        .entry(Value::from("ip-address-blocker-rules"))
        .or_insert_with(|| Value::Mapping(Mapping::new()));
    let Value::Mapping(sec) = sec else { return };
    sec.insert(Value::from("enabled"), Value::from(true)); // 有订阅则启用模块
    let rules = sec
        .entry(Value::from("rules"))
        .or_insert_with(|| Value::Mapping(Mapping::new()));
    let Value::Mapping(rules) = rules else { return };
    let mut r = Mapping::new();
    r.insert(Value::from("enabled"), Value::from(b.enabled));
    r.insert(Value::from("name"), Value::from(b.name.clone()));
    r.insert(Value::from("url"), Value::from(b.url.clone()));
    rules.insert(Value::from(b.rule_id.clone()), Value::Mapping(r));
}

fn remove_sub_rule(profile: &mut ProfileConfig, id: &str) {
    use serde_yaml::Value;
    if let Some(Value::Mapping(sec)) = profile.module.get_mut("ip-address-blocker-rules") {
        let empty = if let Some(Value::Mapping(rules)) = sec.get_mut("rules") {
            rules.remove(id);
            rules.is_empty()
        } else {
            true
        };
        // 删到空 → 禁用模块,避免残留空的 IPBlackRuleList。
        if empty {
            sec.insert(Value::from("enabled"), Value::from(false));
        }
    }
}

/// 保存 profile + 重建规则模块（订阅即时下载）。返回模块数。
async fn save_and_rebuild(
    st: &WebState,
    profile: ProfileConfig,
) -> std::result::Result<usize, String> {
    st.config
        .save_profile(&profile)
        .map_err(|e| e.to_string())?;
    let p = st.config.current().profile.clone();
    let modules = pbh_engine::build_modules(
        &p,
        p.ban_duration,
        st.ban_manager.ban_list(),
        &st.db,
        &st.geoip,
        &st.btn.current_state(),
        &st.config.current().app.network.proxy,
    );
    let n = modules.len();
    st.ban_manager.rebuild_modules(modules);
    Ok(n)
}

#[derive(Deserialize)]
struct SubLogQuery {
    id: String,
}

async fn sub_rule_logs(State(st): State<WebState>, Query(q): Query<SubLogQuery>) -> Response {
    match st.db.query_rule_sub_logs(&q.id, 30).await {
        Ok(rows) => ApiResp::ok(rows).into_response(),
        Err(e) => bad_request(e.to_string()),
    }
}

#[derive(Deserialize)]
struct LogQuery {
    after: Option<u64>,
}

async fn get_logs(State(st): State<WebState>, Query(q): Query<LogQuery>) -> Response {
    let items: Vec<_> = st
        .logs
        .since(q.after.unwrap_or(0))
        .into_iter()
        .map(
            |e| json!({"seq": e.seq, "time_ms": e.time_ms, "level": e.level, "message": e.message}),
        )
        .collect();
    ApiResp::ok(json!({ "items": items })).into_response()
}

async fn geoip_update(State(st): State<WebState>) -> Response {
    let _guard = match st.geoip_lock.try_lock() {
        Ok(g) => g,
        Err(_) => return ApiResp::ok(json!({ "changed": false, "loaded": st.geoip.is_loaded(), "busy": true })).into_response(),
    };
    let app = st.config.current().app.clone();
    let dir = st.paths.data_dir().join("geoip");
    let client = pbh_net::build_client(&app.network.proxy, std::time::Duration::from_secs(60));
    let changed = pbh_geoip::download::ensure_databases(
        &client,
        &dir,
        true,
        &app.ip_database.account_id,
        &app.ip_database.license_key,
    ).await;
    if changed || !st.geoip.is_loaded() {
        if let Some(p) = pbh_geoip::MaxmindProvider::load_from_dir(&dir) {
            st.geoip.install(std::sync::Arc::new(p) as std::sync::Arc<dyn pbh_geoip::GeoIpProvider>);
            tracing::info!("GeoIP 库已手动更新并热加载");
        }
    }
    ApiResp::ok(json!({ "changed": changed, "loaded": st.geoip.is_loaded() })).into_response()
}

/// latest 是否比 current 新(语义化:major.minor.patch,容忍前导 v 与多余段)。
fn version_newer(current: &str, latest: &str) -> bool {
    fn parse(s: &str) -> Vec<u64> {
        s.trim()
            .trim_start_matches('v')
            .split('.')
            .map(|p| p.chars().take_while(|c| c.is_ascii_digit()).collect::<String>())
            .map(|d| d.parse::<u64>().unwrap_or(0))
            .collect()
    }
    let (a, b) = (parse(current), parse(latest));
    for i in 0..a.len().max(b.len()) {
        let x = a.get(i).copied().unwrap_or(0);
        let y = b.get(i).copied().unwrap_or(0);
        if y != x {
            return y > x;
        }
    }
    false
}

async fn update_check(State(st): State<WebState>) -> Response {
    let current = env!("CARGO_PKG_VERSION");
    let app = st.config.current().app.clone();
    let client = pbh_net::build_client(&app.network.proxy, std::time::Duration::from_secs(15));
    let url = "https://api.github.com/repos/Well2333/peerbanhelper-rust/releases/latest";
    let resp = client.get(url).send().await;
    match resp {
        Ok(r) if r.status().is_success() => match r.json::<serde_json::Value>().await {
            Ok(j) => {
                let latest = j.get("tag_name").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let html_url = j.get("html_url").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let newer = !latest.is_empty() && version_newer(current, &latest);
                ApiResp::ok(json!({ "current": current, "latest": latest, "newer": newer, "html_url": html_url })).into_response()
            }
            Err(e) => bad_request(format!("解析失败: {e}")),
        },
        Ok(r) => bad_request(format!("GitHub 返回 {}", r.status())),
        Err(e) => bad_request(format!("请求失败: {e}")),
    }
}

fn gen_id() -> String {
    let t = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("d{t:x}")
}

#[cfg(test)]
mod profile_json_tests {
    use pbh_config::ProfileConfig;

    #[test]
    fn profile_json_roundtrip_preserves_module_keys() {
        let yaml = "check-interval: 4000\nban-duration: 600000\nignore-peers-from-addresses: []\nmodule:\n  peer-id-blacklist:\n    enabled: true\n    banned-peer-id:\n    - method: STARTS_WITH\n      content: -XL\n";
        let p: ProfileConfig = serde_yaml::from_str(yaml).unwrap();
        // ProfileConfig -> serde_json::Value -> ProfileConfig
        let jv = serde_json::to_value(&p).unwrap();
        let back: ProfileConfig = serde_json::from_value(jv).unwrap();
        assert_eq!(back.check_interval, 4000);
        assert_eq!(back.ban_duration, 600000);
        // module key preserved
        assert!(back.module_section("peer-id-blacklist").is_some());
        let sec = back.module_section("peer-id-blacklist").unwrap();
        assert_eq!(sec.get("enabled").unwrap().as_bool(), Some(true));
    }
}

#[cfg(test)]
mod update_tests {
    use super::version_newer;
    #[test]
    fn semver_compare() {
        assert!(version_newer("0.1.0", "0.2.0"));
        assert!(version_newer("0.1.0", "1.0.0"));
        assert!(!version_newer("0.2.0", "0.1.0"));
        assert!(!version_newer("0.1.0", "0.1.0"));
        assert!(version_newer("v0.1.0", "v0.2.0")); // 容忍 v 前缀
        // 非对称 v 前缀(GitHub tag 常带 v,CARGO_PKG_VERSION 不带)
        assert!(version_newer("0.1.0", "v0.2.0"));
        assert!(!version_newer("v0.2.0", "0.1.0"));
        // 预发布后缀(take_while 只取数字)
        assert!(version_newer("1.0.0", "1.0.1-rc1"));
        assert!(!version_newer("1.0.0", "1.0.0-rc1"));
        // 不等长版本段
        assert!(version_newer("1.0.0", "1.0.0.1"));
        assert!(!version_newer("1.0", "1.0.0"));
    }
}
