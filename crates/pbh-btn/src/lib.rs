//! pbh-btn —— BTN 云端威胁情报网络。
//!
//! 对应 Java `btn/**`、`module/impl/rule/BtnNetworkOnline.java`、`util/pow/**`。
//! 协议细节见 `memory/design/architecture-analysis.md` §2.4。
//!
//! 实现：config 拉取 + ability 调度（下行 rules/denylist/allowlist 更新共享状态 → `BtnNetworkOnline`
//! 应用封禁；上行 submit_bans gzip + DB 游标）、PoW 求解、种子隐私哈希、固定头注入。
//!
//! **注意**：BTN 需用户的 app-id/app-secret + BTN 服务端;无凭证时不启用。本实现以单测覆盖
//! PoW/哈希/序列化/规则应用,**未对真实 BTN 服务端做联网验证**（需用户账号）。

pub mod client;
pub mod hash;
pub mod model;
pub mod online;
pub mod pow;

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use parking_lot::RwLock;
use pbh_storage::Db;

pub use client::{gzip, BtnClient};
pub use hash::hashed_identifier;
pub use model::{BtnBan, BtnConfigResponse, BtnRuleset, SubmitBansBody};
pub use online::{apply_allowlist, apply_denylist, apply_ruleset, BtnNetworkOnline, BtnState};
pub use pow::{solve as pow_solve, PowChallenge};

/// 共享 BTN 威胁情报状态（调度器写、模块读）。
pub type SharedBtnState = Arc<RwLock<BtnState>>;

/// 新建共享状态。
pub fn new_state() -> SharedBtnState {
    Arc::new(RwLock::new(BtnState::default()))
}

/// BTN 协议实现版本（对应 Java `PBH_BTN_PROTOCOL_IMPL_VERSION`）。
pub const PROTOCOL_IMPL_VERSION: u32 = 20;
/// 可读协议版本（对应 `PBH_BTN_PROTOCOL_READABLE_VERSION`）。
pub const PROTOCOL_READABLE_VERSION: &str = "2.0.1";

/// BTN 运行配置（来自 `config.yml` 的 `btn:`）。
#[derive(Debug, Clone)]
pub struct BtnRuntimeConfig {
    pub config_url: String,
    pub app_id: String,
    pub app_secret: String,
    pub installation_id: String,
    pub submit: bool,
    /// BTN 封禁时长。
    pub ban_duration: i64,
}

/// 启动 BTN 后台调度（拉 config → 下行更新状态 + 上行提交）。返回停止标志。
pub fn spawn(cfg: BtnRuntimeConfig, db: Db, state: Arc<RwLock<BtnState>>) -> Arc<AtomicBool> {
    let shutdown = Arc::new(AtomicBool::new(false));
    let sd = shutdown.clone();
    tokio::spawn(async move {
        let client = BtnClient::new(
            cfg.app_id.clone(),
            cfg.app_secret.clone(),
            cfg.installation_id.clone(),
        );
        loop {
            if sd.load(Ordering::Relaxed) {
                return;
            }
            match client.fetch_config(&cfg.config_url).await {
                Ok(config) => {
                    tracing::info!(
                        "BTN config 已拉取（{} 个 ability,{}）",
                        config.ability.len(),
                        if config.is_legacy() {
                            "legacy"
                        } else {
                            "modern"
                        }
                    );
                    run_abilities(&client, &config, &cfg, &db, &state, &sd).await;
                }
                Err(e) => {
                    tracing::warn!("BTN config 拉取失败: {e};600s 后重试");
                    sleep_checked(&sd, 600).await;
                }
            }
        }
    });
    shutdown
}

/// 按各 ability 的 interval 周期执行下行/上行。直到停止标志置位。
async fn run_abilities(
    client: &BtnClient,
    config: &BtnConfigResponse,
    cfg: &BtnRuntimeConfig,
    db: &Db,
    state: &Arc<RwLock<BtnState>>,
    sd: &Arc<AtomicBool>,
) {
    let mut last: HashMap<String, i64> = HashMap::new();
    let mut rules_rev = String::new();
    let mut deny_rev = String::new();
    let mut allow_rev = String::new();
    // config 每 ~1h 重拉一次（让外层 loop 重新 fetch_config）。
    let config_started = now_ms();
    loop {
        if sd.load(Ordering::Relaxed) {
            return;
        }
        let now = now_ms();
        for (key, ab) in &config.ability {
            let interval = ab.interval.unwrap_or(3_600_000).max(1000);
            let due = last.get(key).is_none_or(|&t| now - t >= interval);
            if !due {
                continue;
            }
            let Some(endpoint) = ab.endpoint.as_deref() else {
                continue;
            };
            match key.as_str() {
                "rule_peer_identity" | "rules" => {
                    match client.fetch_rules(endpoint, &rules_rev).await {
                        Ok(Some(rs)) => {
                            rules_rev = rs.version.clone().unwrap_or_default();
                            apply_ruleset(state, &rs);
                            tracing::info!("BTN 规则集已更新 (rev={rules_rev})");
                        }
                        Ok(None) => {}
                        Err(e) => tracing::warn!("BTN 规则拉取失败: {e}"),
                    }
                }
                "ip_denylist" => match client.fetch_ip_list(endpoint, &deny_rev).await {
                    Ok(Some((text, ver))) => {
                        deny_rev = ver;
                        apply_denylist(state, &text);
                        tracing::info!("BTN 黑名单已更新 ({} 字节)", text.len());
                    }
                    Ok(None) => {}
                    Err(e) => tracing::warn!("BTN 黑名单拉取失败: {e}"),
                },
                "ip_allowlist" => match client.fetch_ip_list(endpoint, &allow_rev).await {
                    Ok(Some((text, ver))) => {
                        allow_rev = ver;
                        apply_allowlist(state, &text);
                        tracing::info!("BTN 白名单已更新 ({} 字节)", text.len());
                    }
                    Ok(None) => {}
                    Err(e) => tracing::warn!("BTN 白名单拉取失败: {e}"),
                },
                "submit_bans" if cfg.submit => {
                    submit_bans(client, endpoint, db).await;
                }
                _ => {}
            }
            last.insert(key.clone(), now);
        }
        // config 老化 → 退出让外层重拉。
        if now_ms() - config_started > 3_600_000 {
            return;
        }
        sleep_checked(sd, 30).await;
    }
}

/// 上行提交 BTN 模块产生的封禁（按 history.id 游标分页 + gzip）。
async fn submit_bans(client: &BtnClient, url: &str, db: &Db) {
    const CURSOR_KEY: &str = "BtnAbilitySubmitBans.cursor";
    let cursor: i64 = db
        .meta_get(CURSOR_KEY)
        .await
        .ok()
        .flatten()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let rows = match db.query_btn_bans(cursor, 100).await {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!("BTN submit_bans 查询失败: {e}");
            return;
        }
    };
    if rows.is_empty() {
        return;
    }
    let max_id = rows.iter().map(|r| r.id).max().unwrap_or(cursor);
    let bans: Vec<BtnBan> = rows
        .into_iter()
        .map(|r| BtnBan {
            ban_at: r.ban_at,
            peer_ip: r.ip,
            peer_port: r.port,
            peer_id: r.peer_id,
            peer_client_name: r.client_name,
            peer_progress: r.peer_progress,
            torrent_identifier: hashed_identifier(&r.info_hash),
            torrent_size: r.torrent_size,
            module: r.module_name,
            rule: r.rule_name,
            description: r.description,
        })
        .collect();
    let n = bans.len();
    let Ok(body) = serde_json::to_string(&SubmitBansBody { bans }) else {
        return;
    };
    match client.submit_gzip(url, &body).await {
        Ok(()) => {
            let _ = db.meta_set(CURSOR_KEY, &max_id.to_string()).await;
            tracing::info!("BTN 已上报 {n} 条封禁 (游标→{max_id})");
        }
        Err(e) => tracing::warn!("BTN submit_bans 上报失败: {e}"),
    }
}

async fn sleep_checked(sd: &Arc<AtomicBool>, secs: u64) {
    for _ in 0..secs {
        if sd.load(Ordering::Relaxed) {
            return;
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}
