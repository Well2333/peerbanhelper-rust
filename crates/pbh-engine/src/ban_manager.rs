//! BanManager + Ban Wave 调度循环。对应上游 `DownloaderServerImpl` 的 banWave。
//!
//! 一轮 wave：移除到期封禁 → 对每个下载器(登录→拉 torrents→拉 peers→逐 peer 跑模块→命中即封) → 下发封禁列表。

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use std::net::IpAddr;

use pbh_domain::{BanMetadata, CheckResult, Peer, PeerAction, PeerAddress, Torrent};
use pbh_downloader::DownloaderManager;
use pbh_rules::{IpMatcher, RuleFeatureModule};
use pbh_storage::{Db, NewBanHistory};

use crate::BanList;

static BAN_SEQ: AtomicU64 = AtomicU64::new(0);

/// 封禁管理 + ban wave 执行。
pub struct BanManager {
    ban_list: Arc<BanList>,
    downloaders: Arc<DownloaderManager>,
    modules: Vec<Arc<dyn RuleFeatureModule>>,
    db: Db,
    global_ban_duration: i64,
    /// 旁路名单（这些地址来的 peer 不检查）。
    ignore: IpMatcher<()>,
    /// 防止 wave 重叠。
    running: AtomicBool,
}

/// run_once 的重叠保护 RAII：退出时清标志。持有 `&AtomicBool`（Send），可跨 await。
struct WaveGuard<'a>(&'a AtomicBool);
impl Drop for WaveGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

impl BanManager {
    pub fn new(
        ban_list: Arc<BanList>,
        downloaders: Arc<DownloaderManager>,
        modules: Vec<Arc<dyn RuleFeatureModule>>,
        db: Db,
        global_ban_duration: i64,
        ignore_addresses: &[String],
    ) -> Arc<Self> {
        let mut ignore = IpMatcher::new();
        for a in ignore_addresses {
            ignore.insert(a, ());
        }
        Arc::new(BanManager {
            ban_list,
            downloaders,
            modules,
            db,
            global_ban_duration,
            ignore,
            running: AtomicBool::new(false),
        })
    }

    pub fn ban_list(&self) -> &Arc<BanList> {
        &self.ban_list
    }

    pub fn global_ban_duration(&self) -> i64 {
        self.global_ban_duration
    }

    /// 手动封禁单个 IP。下次 wave 下发到下载器。
    pub fn manual_ban(&self, ip: &str, duration_ms: i64) -> bool {
        let Ok(addr) = ip.trim().parse::<IpAddr>() else {
            return false;
        };
        let now = now_ms();
        let dur = if duration_ms > 0 {
            duration_ms
        } else {
            self.global_ban_duration
        };
        let meta = BanMetadata {
            context: "manual".into(),
            random_id: gen_id(),
            peer: PeerAddress::new(addr, 0),
            ban_at: now,
            unban_at: now.saturating_add(dur),
            ban_for_disconnect: false,
            exclude_from_report: false,
            exclude_from_display: false,
            rule: "manual".into(),
            description: "手动封禁".into(),
        };
        self.ban_list.ban(ip, meta)
    }

    /// 手动解封。
    pub fn manual_unban(&self, ip: &str) -> bool {
        self.ban_list.unban(ip).is_some()
    }

    /// 对单个 peer 跑所有模块，合并结果（Skip 短路）。
    fn run_modules(&self, torrent: &Torrent, peer: &Peer) -> CheckResult {
        let mut result = CheckResult::pass("none");
        for m in &self.modules {
            let r = m.should_ban(torrent, peer);
            result = result.merge(r);
            if result.action == PeerAction::Skip {
                break;
            }
        }
        result
    }

    /// 执行一轮 ban wave。
    pub async fn run_once(&self) {
        if self
            .running
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            tracing::warn!("上一轮 ban wave 仍在运行，跳过本次");
            return;
        }
        let _guard = WaveGuard(&self.running);
        let now = now_ms();
        let expired = self.ban_list.remove_expired(now);
        if !expired.is_empty() {
            tracing::info!("解封 {} 个到期封禁", expired.len());
        }

        let downloaders = self.downloaders.list();
        for d in downloaders {
            if d.is_paused() {
                continue;
            }
            match d.login().await {
                Ok(o) if o.success => {}
                Ok(o) => {
                    tracing::warn!(downloader = d.name(), "登录失败: {}", o.message);
                    continue;
                }
                Err(e) => {
                    tracing::warn!(downloader = d.name(), "登录错误: {e}");
                    continue;
                }
            }
            let torrents = match d.get_torrents().await {
                Ok(t) => t,
                Err(e) => {
                    tracing::warn!(downloader = d.name(), "拉取种子失败: {e}");
                    continue;
                }
            };
            let mut newly: Vec<String> = Vec::new();
            for t in &torrents {
                let peers = match d.get_peers(t).await {
                    Ok(p) => p,
                    Err(e) => {
                        tracing::warn!(torrent = %t.name, "拉取 peer 失败: {e}");
                        continue;
                    }
                };
                for p in &peers {
                    if self.ignore.contains(p.address.ip) || self.ban_list.contains(p.address.ip) {
                        continue;
                    }
                    let r = self.run_modules(t, p);
                    if matches!(r.action, PeerAction::Ban | PeerAction::BanForDisconnect) {
                        self.record_ban(d.id(), t, p, &r, now, &mut newly).await;
                    }
                }
            }

            // 下发当前完整封禁列表（+本轮新增供增量）。
            let full: Vec<String> = self
                .ban_list
                .snapshot()
                .into_iter()
                .map(|(net, _)| net)
                .collect();
            if let Err(e) = d.apply_ban_list(&full, &newly, false).await {
                tracing::warn!(downloader = d.name(), "下发封禁失败: {e}");
            }
        }
    }

    async fn record_ban(
        &self,
        downloader_id: &str,
        t: &Torrent,
        p: &Peer,
        r: &CheckResult,
        now: i64,
        newly: &mut Vec<String>,
    ) {
        let dur = if r.duration_ms > 0 {
            r.duration_ms
        } else {
            self.global_ban_duration
        };
        let meta = BanMetadata {
            context: r.module.to_string(),
            random_id: gen_id(),
            peer: p.address.clone(),
            ban_at: now,
            unban_at: now.saturating_add(dur),
            ban_for_disconnect: r.action == PeerAction::BanForDisconnect,
            exclude_from_report: false,
            exclude_from_display: false,
            rule: r.rule.clone(),
            description: r.reason.clone(),
        };
        if !self.ban_list.ban(&p.address.ip.to_string(), meta) {
            return;
        }
        newly.push(p.address.raw_ip.clone());
        tracing::info!(
            module = r.module,
            "封禁 {} ({}) — {}",
            p.address.raw_ip,
            p.client_name.as_deref().unwrap_or("?"),
            r.reason
        );
        // 历史落库（失败仅记日志，不影响封禁）。
        let torrent_id = match self
            .db
            .upsert_torrent(&t.hash, &t.name, t.size, Some(t.private_torrent))
            .await
        {
            Ok(id) => id,
            Err(e) => {
                tracing::warn!("写入种子失败: {e}");
                return;
            }
        };
        let row = NewBanHistory {
            ban_at: now,
            unban_at: meta_unban(now, dur),
            ip: p.address.ip.to_string(),
            port: p.address.port as i64,
            peer_id: p.peer_id.clone(),
            client_name: p.client_name.clone(),
            peer_progress: p.progress,
            downloader_progress: t.progress,
            torrent_id,
            module_name: r.module.to_string(),
            rule_name: r.rule.clone(),
            description: r.reason.clone(),
            downloader: downloader_id.to_string(),
        };
        if let Err(e) = self.db.insert_ban_history(&row).await {
            tracing::warn!("写入封禁历史失败: {e}");
        }
    }

    /// 启动后台 ban wave 循环（固定延迟）。
    pub fn spawn_loop(self: Arc<Self>, interval_ms: u64) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            let period = Duration::from_millis(interval_ms.max(1000));
            let mut tick = tokio::time::interval(period);
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                tick.tick().await;
                self.run_once().await;
            }
        })
    }
}

fn meta_unban(now: i64, dur: i64) -> i64 {
    now.saturating_add(dur)
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn gen_id() -> String {
    let seq = BAN_SEQ.fetch_add(1, Ordering::Relaxed);
    let t = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{t:x}{seq:x}")
}
