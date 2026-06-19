//! BanManager + Ban Wave 调度循环。对应上游 `DownloaderServerImpl` 的 banWave。
//!
//! 一轮 wave：移除到期封禁 → 对每个下载器(登录→拉 torrents→拉 peers→逐 peer 跑模块→命中即封) → 下发封禁列表。

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use std::net::IpAddr;

use pbh_domain::{BanMetadata, CheckResult, Peer, PeerAction, PeerAddress, Torrent};
use pbh_downloader::DownloaderManager;
use pbh_rules::{IpMatcher, RuleFeatureModule};
use pbh_storage::{Db, NewBanHistory};

use crate::BanList;

static BAN_SEQ: AtomicU64 = AtomicU64::new(0);

/// 运行期累计统计（仪表盘用）。原子计数，进程生命周期内累加。
#[derive(Default)]
pub struct Stats {
    /// 累计检查过的 peer 次数。
    pub checked_peers: AtomicU64,
    /// 累计封禁次数。
    pub banned_peers: AtomicU64,
    /// 累计（到期）解封次数。
    pub unbanned_peers: AtomicU64,
    /// 完成的 ban wave 轮数。
    pub waves: AtomicU64,
    /// 上一轮 wave 完成时刻（epoch ms）。
    pub last_wave_at: AtomicU64,
    /// 上一轮 wave 耗时（ms）。
    pub last_wave_ms: AtomicU64,
}

/// 统计快照（可序列化给前端）。
#[derive(Debug, Clone, Default)]
pub struct StatsSnapshot {
    pub checked_peers: u64,
    pub banned_peers: u64,
    pub unbanned_peers: u64,
    pub waves: u64,
    pub last_wave_at: u64,
    pub last_wave_ms: u64,
}

/// 封禁管理 + ban wave 执行。
pub struct BanManager {
    ban_list: Arc<BanList>,
    downloaders: Arc<DownloaderManager>,
    /// 启用的规则模块。`RwLock` 以支持配置热重载（PUT /api/config/profile 后重建）。
    modules: RwLock<Vec<Arc<dyn RuleFeatureModule>>>,
    db: Db,
    global_ban_duration: i64,
    /// 旁路名单（这些地址来的 peer 不检查）。
    ignore: IpMatcher<()>,
    /// 防止 wave 重叠。
    running: AtomicBool,
    /// 运行统计。
    stats: Stats,
    /// 每个下载器上轮登录是否成功（id → ok）。
    login_status: RwLock<HashMap<String, bool>>,
    /// 是否把当前 swarm 记入 `tracked_swarm`（供 BTN 上行 submit_swarm）。
    track_swarm: bool,
    /// GeoIP 可选注入（封禁历史回填 peer_geoip）。
    geoip: Option<Arc<dyn pbh_geoip::GeoIpProvider>>,
    /// 上次 banlist 快照落库时刻（epoch ms）。
    last_snapshot_at: AtomicU64,
}

/// run_once 的重叠保护 RAII：退出时清标志。持有 `&AtomicBool`（Send），可跨 await。
struct WaveGuard<'a>(&'a AtomicBool);
impl Drop for WaveGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

impl BanManager {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        ban_list: Arc<BanList>,
        downloaders: Arc<DownloaderManager>,
        modules: Vec<Arc<dyn RuleFeatureModule>>,
        db: Db,
        global_ban_duration: i64,
        ignore_addresses: &[String],
        track_swarm: bool,
        geoip: Option<Arc<dyn pbh_geoip::GeoIpProvider>>,
    ) -> Arc<Self> {
        let mut ignore = IpMatcher::new();
        for a in ignore_addresses {
            ignore.insert(a, ());
        }
        Arc::new(BanManager {
            ban_list,
            downloaders,
            modules: RwLock::new(modules),
            db,
            global_ban_duration,
            ignore,
            running: AtomicBool::new(false),
            stats: Stats::default(),
            login_status: RwLock::new(HashMap::new()),
            track_swarm,
            geoip,
            last_snapshot_at: AtomicU64::new(now_ms() as u64),
        })
    }

    /// 把当前内存封禁表快照落库（全量替换 banlist）。
    pub async fn snapshot_to_db(&self) {
        let entries: Vec<(String, String)> = self
            .ban_list
            .snapshot()
            .into_iter()
            .filter_map(|(net, meta)| serde_json::to_string(&meta).ok().map(|j| (net, j)))
            .collect();
        if let Err(e) = self.db.save_banlist(&entries).await {
            tracing::warn!("banlist 快照失败: {e}");
        }
    }

    pub fn ban_list(&self) -> &Arc<BanList> {
        &self.ban_list
    }

    pub fn global_ban_duration(&self) -> i64 {
        self.global_ban_duration
    }

    /// 当前统计快照（仪表盘用）。
    pub fn stats(&self) -> StatsSnapshot {
        let s = &self.stats;
        StatsSnapshot {
            checked_peers: s.checked_peers.load(Ordering::Relaxed),
            banned_peers: s.banned_peers.load(Ordering::Relaxed),
            unbanned_peers: s.unbanned_peers.load(Ordering::Relaxed),
            waves: s.waves.load(Ordering::Relaxed),
            last_wave_at: s.last_wave_at.load(Ordering::Relaxed),
            last_wave_ms: s.last_wave_ms.load(Ordering::Relaxed),
        }
    }

    /// 每个下载器上轮登录状态（id → 是否成功）。
    pub fn downloader_status(&self) -> HashMap<String, bool> {
        self.login_status.read().unwrap().clone()
    }

    /// 启用的模块数量。
    pub fn module_count(&self) -> usize {
        self.modules.read().unwrap().len()
    }

    /// 热重载：用新规则集替换当前模块（PUT /api/config/profile 后调用）。
    pub fn rebuild_modules(&self, modules: Vec<Arc<dyn RuleFeatureModule>>) {
        *self.modules.write().unwrap() = modules;
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
        let removed = self.ban_list.unban(ip).is_some();
        if removed {
            if let Ok(addr) = ip.trim().parse::<IpAddr>() {
                let modules = self.modules.read().unwrap();
                for m in modules.iter() {
                    m.on_unban(addr);
                }
            }
        }
        removed
    }

    /// 对单个 peer 跑所有模块，合并结果（Skip 短路）。
    fn run_modules(&self, torrent: &Torrent, peer: &Peer) -> CheckResult {
        let mut result = CheckResult::pass("none");
        let modules = self.modules.read().unwrap();
        for m in modules.iter() {
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
        let wave_start = now_ms();
        let now = wave_start;
        let expired = self.ban_list.remove_expired(now);
        if !expired.is_empty() {
            tracing::info!("解封 {} 个到期封禁", expired.len());
            self.stats
                .unbanned_peers
                .fetch_add(expired.len() as u64, Ordering::Relaxed);
            // 通知各模块（PCB 重置该 IP 跟踪状态）。
            let modules = self.modules.read().unwrap();
            for meta in &expired {
                for m in modules.iter() {
                    m.on_unban(meta.peer.ip);
                }
            }
        }

        let downloaders = self.downloaders.list();
        for d in downloaders {
            if d.is_paused() {
                continue;
            }
            match d.login().await {
                Ok(o) if o.success => {
                    self.login_status
                        .write()
                        .unwrap()
                        .insert(d.id().to_string(), true);
                }
                Ok(o) => {
                    tracing::warn!(downloader = d.name(), "登录失败: {}", o.message);
                    self.login_status
                        .write()
                        .unwrap()
                        .insert(d.id().to_string(), false);
                    continue;
                }
                Err(e) => {
                    tracing::warn!(downloader = d.name(), "登录错误: {e}");
                    self.login_status
                        .write()
                        .unwrap()
                        .insert(d.id().to_string(), false);
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
            let mut swarm: Vec<pbh_storage::SwarmRow> = Vec::new();
            for t in &torrents {
                let peers = match d.get_peers(t).await {
                    Ok(p) => p,
                    Err(e) => {
                        tracing::warn!(torrent = %t.name, "拉取 peer 失败: {e}");
                        continue;
                    }
                };
                for p in &peers {
                    // swarm 跟踪：记录全部 peer（供 BTN 上行）。
                    if self.track_swarm {
                        swarm.push(swarm_row(d.id(), t, p, now));
                    }
                    if self.ignore.contains(p.address.ip) || self.ban_list.contains(p.address.ip) {
                        continue;
                    }
                    self.stats.checked_peers.fetch_add(1, Ordering::Relaxed);
                    let r = self.run_modules(t, p);
                    if matches!(r.action, PeerAction::Ban | PeerAction::BanForDisconnect) {
                        self.record_ban(d.id(), t, p, &r, now, &mut newly).await;
                    }
                }
            }
            if !swarm.is_empty() {
                if let Err(e) = self.db.upsert_tracked_swarm(&swarm).await {
                    tracing::warn!("swarm 记录失败: {e}");
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

        let elapsed = now_ms().saturating_sub(wave_start).max(0) as u64;
        self.stats.waves.fetch_add(1, Ordering::Relaxed);
        self.stats
            .last_wave_at
            .store(wave_start as u64, Ordering::Relaxed);
        self.stats.last_wave_ms.store(elapsed, Ordering::Relaxed);

        // 每小时把 banlist 快照落库（重启可恢复）。
        if now_ms() as u64 - self.last_snapshot_at.load(Ordering::Relaxed) > 3_600_000 {
            self.last_snapshot_at
                .store(now_ms() as u64, Ordering::Relaxed);
            self.snapshot_to_db().await;
        }
    }

    /// 从 DB 恢复未过期的封禁快照到内存 BanList。返回恢复条数。
    pub async fn restore_banlist(ban_list: &Arc<BanList>, db: &Db) -> usize {
        let now = now_ms();
        let mut n = 0;
        if let Ok(entries) = db.load_banlist().await {
            for (addr, json) in entries {
                if let Ok(meta) = serde_json::from_str::<BanMetadata>(&json) {
                    if meta.unban_at > now && ban_list.ban(&addr, meta) {
                        n += 1;
                    }
                }
            }
        }
        n
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
        self.stats.banned_peers.fetch_add(1, Ordering::Relaxed);
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
            peer_uploaded: p.uploaded,
            peer_downloaded: p.downloaded,
            peer_progress: p.progress,
            downloader_progress: t.progress,
            torrent_id,
            module_name: r.module.to_string(),
            rule_name: r.rule.clone(),
            description: r.reason.clone(),
            flags: p.flags.as_ref().map(|f| f.raw.clone()),
            downloader: downloader_id.to_string(),
            peer_geoip: self
                .geoip
                .as_ref()
                .and_then(|g| g.query(p.address.ip))
                .and_then(|geo| serde_json::to_string(&geo).ok()),
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

/// 构造一条 swarm 观测行。
fn swarm_row(downloader_id: &str, t: &Torrent, p: &Peer, now: i64) -> pbh_storage::SwarmRow {
    pbh_storage::SwarmRow {
        ip: p.address.ip.to_string(),
        port: p.address.port as i64,
        info_hash: t.hash.clone(),
        torrent_is_private: t.private_torrent,
        torrent_size: t.size,
        downloader: downloader_id.to_string(),
        downloader_progress: t.progress,
        peer_id: p.peer_id.clone(),
        client_name: p.client_name.clone(),
        peer_progress: p.progress,
        uploaded: p.uploaded.max(0),
        upload_speed: p.upload_speed.max(0),
        downloaded: p.downloaded.max(0),
        download_speed: p.download_speed.max(0),
        last_flags: p.flags.as_ref().map(|f| f.raw.clone()),
        now,
    }
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
