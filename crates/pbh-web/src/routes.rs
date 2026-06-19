//! 路由与处理器。

use std::time::{SystemTime, UNIX_EPOCH};

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
        .route("/api/downloaders/{id}", delete(delete_downloader))
        .route("/api/downloaders/test", post(test_downloader))
        .route("/api/bans", get(list_bans).put(add_ban))
        .route("/api/bans/{ip}", delete(remove_ban))
        .route("/api/bans/history", get(ban_history))
        .route("/api/config/profile", get(get_profile).put(put_profile))
        .route("/api/logs", get(get_logs))
        .route_layer(middleware::from_fn_with_state(state.clone(), auth));

    Router::new()
        .route("/", get(index))
        .route("/api/auth/login", post(login))
        .merge(protected)
        .with_state(state)
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
    match serde_yaml::to_string(&profile) {
        Ok(yaml) => ApiResp::ok(json!({ "yaml": yaml })).into_response(),
        Err(e) => bad_request(format!("序列化 profile 失败: {e}")),
    }
}

#[derive(Deserialize)]
struct ProfileBody {
    yaml: String,
}

async fn put_profile(State(st): State<WebState>, Json(b): Json<ProfileBody>) -> Response {
    // 解析校验。
    let profile: ProfileConfig = match serde_yaml::from_str(&b.yaml) {
        Ok(p) => p,
        Err(e) => return bad_request(format!("YAML 解析失败: {e}")),
    };
    // 写盘 + 热重载配置。
    if let Err(e) = st.config.save_profile(&profile) {
        return bad_request(format!("保存失败: {e}"));
    }
    // 重建规则模块（即时生效，无需重启）。
    let p = st.config.current().profile.clone();
    let modules = pbh_engine::build_modules(&p, p.ban_duration);
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
            json!({
                "address": net,
                "ip": m.peer.ip.to_string(),
                "port": m.peer.port,
                "module": m.context,
                "rule": m.rule,
                "description": m.description,
                "ban_at": m.ban_at,
                "unban_at": m.unban_at,
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

fn gen_id() -> String {
    let t = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("d{t:x}")
}
