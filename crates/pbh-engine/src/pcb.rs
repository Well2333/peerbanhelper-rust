//! ProgressCheatBlocker（PCB）—— 进度作弊检测 / 反吸血核心。
//! 对应上游 `module/impl/rule/ProgressCheatBlocker.java`。
//!
//! 追踪每个 peer 给我方上传的累计字节，与其自报进度比对，识别「谎报进度白嫖」的吸血客户端。
//! 四道子检查（严格短路）：fast-pcb-test → excessive(过量上传) → difference(进度差异, ban-delay 窗口) → rewind(进度回退)。
//!
//! 双视图状态：单 IP（`pcb_address`）+ 前缀段（`pcb_range`，跨 IP 聚合，防止换 IP 绕过）。
//! 本文件是**内存核心 + 判定状态机**；DB 持久化（载入/批刷/清理/解封重置）在后续提交附加。

use std::net::IpAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use ip_network::IpNetwork;
use moka::sync::Cache;
use parking_lot::Mutex;
use pbh_domain::{CheckResult, Peer, PeerAction, Torrent};
use pbh_rules::RuleFeatureModule;
use pbh_storage::{Db, PcbAddressRow, PcbAnalysis, PcbRangeRow};

const MODULE: &str = "ProgressCheatBlocker";

/// PCB 配置（profile.yml `module.progress-cheat-blocker`）。
#[derive(Debug, Clone)]
pub struct PcbConfig {
    /// 小于此大小（字节）的种子不检查。
    pub minimum_size: i64,
    /// 进度差异最大允许值（0.1 = 10%）。
    pub maximum_difference: f64,
    /// 进度回退最大允许值（0.07）；<=0 禁用回退检查。
    pub rewind_maximum_difference: f64,
    /// 是否启用过量上传检查。
    pub block_excessive: bool,
    /// 过量倍率（1.5 = 允许上传超过基准 50%）。
    pub excessive_threshold: f64,
    pub ipv4_prefix: u8,
    pub ipv6_prefix: u8,
    pub ban_duration: i64,
    /// ban-delay 观察窗口（ms）。
    pub max_wait_duration: i64,
    /// fast-pcb-test 触发比例（computedUploaded >= pct*size）；<=0 禁用。
    pub fast_pcb_test_percentage: f64,
    /// fast-pcb-test 的 BAN_FOR_DISCONNECT 时长（ms）。
    pub fast_pcb_test_block_duration: i64,
    /// 是否持久化到 DB（重启续算）。
    pub enable_persist: bool,
    /// 持久化数据保留时长（ms）。
    pub persist_duration: i64,
}

impl Default for PcbConfig {
    fn default() -> Self {
        PcbConfig {
            minimum_size: 50_000_000,
            maximum_difference: 0.1,
            rewind_maximum_difference: 0.07,
            block_excessive: true,
            excessive_threshold: 1.5,
            ipv4_prefix: 32,
            ipv6_prefix: 56,
            ban_duration: 2_592_000_000, // 30 天
            max_wait_duration: 30_000,
            fast_pcb_test_percentage: 0.1,
            fast_pcb_test_block_duration: 15_000,
            enable_persist: true,
            persist_duration: 1_209_600_000, // 14 天
        }
    }
}

/// 单条 PCB 状态（address 或 range 共用同一组分析字段）。
#[derive(Debug, Clone)]
pub struct PcbEntry {
    pub last_report_progress: f64,
    pub last_report_uploaded: i64,
    pub tracking_uploaded_increase_total: i64,
    pub rewind_counter: i64,
    pub progress_difference_counter: i64,
    pub first_time_seen: i64,
    pub last_time_seen: i64,
    /// ban-delay 窗口结束时刻（0 = 无窗口）。
    pub ban_delay_window_end_at: i64,
    /// fast-pcb-test 执行时刻（0 = 未执行）。
    pub fast_pcb_test_execute_at: i64,
    pub last_torrent_completed_size: i64,
    /// 自上次落库后是否被改动。
    pub dirty: bool,
}

impl PcbEntry {
    pub fn new(now: i64) -> Self {
        PcbEntry {
            last_report_progress: 0.0,
            last_report_uploaded: 0,
            tracking_uploaded_increase_total: 0,
            rewind_counter: 0,
            progress_difference_counter: 0,
            first_time_seen: now,
            last_time_seen: now,
            ban_delay_window_end_at: 0,
            fast_pcb_test_execute_at: 0,
            last_torrent_completed_size: 0,
            dirty: false,
        }
    }
}

fn ban(duration: i64, action: PeerAction, rule: &str, reason: String) -> CheckResult {
    CheckResult {
        module: MODULE,
        action,
        duration_ms: duration,
        rule: rule.into(),
        reason,
    }
}

// ---------------- ban-delay 窗口状态机（作用于 addr + range 两者）----------------

fn window_scheduled(addr: &PcbEntry, range: &PcbEntry) -> bool {
    addr.ban_delay_window_end_at > 0 || range.ban_delay_window_end_at > 0
}
fn window_expired(addr: &PcbEntry, range: &PcbEntry, now: i64) -> bool {
    (addr.ban_delay_window_end_at > 0 && addr.ban_delay_window_end_at < now)
        || (range.ban_delay_window_end_at > 0 && range.ban_delay_window_end_at < now)
}
fn schedule_window(addr: &mut PcbEntry, range: &mut PcbEntry, now: i64, dur: i64) {
    if addr.ban_delay_window_end_at == 0 {
        addr.ban_delay_window_end_at = now + dur;
        addr.dirty = true;
    }
    if range.ban_delay_window_end_at == 0 {
        range.ban_delay_window_end_at = now + dur;
        range.dirty = true;
    }
}
fn reset_window(addr: &mut PcbEntry, range: &mut PcbEntry) {
    if addr.ban_delay_window_end_at != 0 {
        addr.ban_delay_window_end_at = 0;
        addr.dirty = true;
    }
    if range.ban_delay_window_end_at != 0 {
        range.ban_delay_window_end_at = 0;
        range.dirty = true;
    }
}

fn file_too_small(cfg: &PcbConfig, torrent: &Torrent) -> bool {
    torrent.size < cfg.minimum_size
}
fn is_uploading(peer: &Peer) -> bool {
    peer.upload_speed > 0 || peer.uploaded > 0
}

/// 纯判定核心：对一对 (addr, range) 状态执行 PCB 检查并更新状态，返回结果。
///
/// 与上游 `finally` 块一致：无论是否封禁，末尾都会刷新 last_report/last_time 等字段。
pub fn evaluate(
    cfg: &PcbConfig,
    addr: &mut PcbEntry,
    range: &mut PcbEntry,
    torrent: &Torrent,
    peer: &Peer,
    now: i64,
) -> CheckResult {
    if peer.is_handshaking() {
        return CheckResult::pass(MODULE);
    }

    // 上传增量（处理重连/重置/-1：报告值变小则取报告值本身，再 clamp 到 >=0）。
    let incr = if peer.uploaded < addr.last_report_uploaded {
        peer.uploaded
    } else {
        peer.uploaded - addr.last_report_uploaded
    }
    .max(0);
    addr.tracking_uploaded_increase_total += incr;
    range.tracking_uploaded_increase_total += incr;
    if incr > 0 {
        addr.dirty = true;
        range.dirty = true;
    }

    // 实际上传量 = max(报告值, 单IP累计, 段累计)——重连归零也无法逃避。
    let computed_uploaded = peer
        .uploaded
        .max(addr.tracking_uploaded_increase_total)
        .max(range.tracking_uploaded_increase_total);

    let result = run_checks(cfg, addr, range, torrent, peer, now, computed_uploaded);

    // finalize（始终执行）。
    addr.last_report_uploaded = peer.uploaded;
    range.last_report_uploaded = peer.uploaded;
    if peer.progress > 0.0 {
        addr.last_report_progress = peer.progress;
        range.last_report_progress = peer.progress;
    }
    let cs = torrent.completed_size;
    addr.last_torrent_completed_size = cs.max(addr.last_torrent_completed_size);
    range.last_torrent_completed_size = cs.max(range.last_torrent_completed_size);
    addr.last_time_seen = now;
    range.last_time_seen = now;
    addr.dirty = true;
    range.dirty = true;

    result
}

#[allow(clippy::too_many_arguments)]
fn run_checks(
    cfg: &PcbConfig,
    addr: &mut PcbEntry,
    range: &mut PcbEntry,
    torrent: &Torrent,
    peer: &Peer,
    now: i64,
    computed_uploaded: i64,
) -> CheckResult {
    if torrent.size <= 0 || !is_uploading(peer) {
        return CheckResult::pass(MODULE);
    }

    // 1) fast-pcb-test：达到比例即短封强制断连复测（一次性）。
    if cfg.fast_pcb_test_percentage > 0.0
        && !file_too_small(cfg, torrent)
        && (addr.fast_pcb_test_execute_at == 0 || range.fast_pcb_test_execute_at == 0)
        && computed_uploaded as f64 >= cfg.fast_pcb_test_percentage * torrent.size as f64
    {
        addr.fast_pcb_test_execute_at = now;
        range.fast_pcb_test_execute_at = now;
        addr.dirty = true;
        range.dirty = true;
        return ban(
            cfg.fast_pcb_test_block_duration,
            PeerAction::BanForDisconnect,
            "pcb:fast-test",
            "快速进度作弊复测：强制断连".into(),
        );
    }

    // 2) excessive：上传量超过基准的 N 倍。
    if cfg.block_excessive {
        let computed_completed = torrent
            .completed_size
            .max(range.last_torrent_completed_size)
            .max(addr.last_torrent_completed_size);
        // Case 1：上传超过种子总大小。
        if computed_uploaded > torrent.size {
            let threshold = torrent.size.max(cfg.minimum_size) as f64 * cfg.excessive_threshold;
            if computed_uploaded as f64 > threshold {
                reset_window(addr, range);
                return ban(
                    cfg.ban_duration,
                    PeerAction::Ban,
                    "pcb:excessive",
                    format!(
                        "过量上传 {computed_uploaded} 字节 > 种子 {} 的 {} 倍",
                        torrent.size, cfg.excessive_threshold
                    ),
                );
            }
        }
        // Case 2：未完成任务，上传超过已完成量的 N 倍。
        if computed_completed > 0 && computed_uploaded > computed_completed {
            let threshold =
                computed_completed.max(cfg.minimum_size) as f64 * cfg.excessive_threshold;
            if computed_uploaded as f64 > threshold {
                reset_window(addr, range);
                return ban(
                    cfg.ban_duration,
                    PeerAction::Ban,
                    "pcb:excessive-incomplete",
                    format!(
                        "过量上传 {computed_uploaded} 字节 > 已完成 {computed_completed} 的 {} 倍",
                        cfg.excessive_threshold
                    ),
                );
            }
        }
    }

    // 3) difference：我方推算进度明显高于 peer 自报进度（ban-delay 窗口）。
    let computed_progress = computed_uploaded as f64 / torrent.size as f64;
    if computed_progress > peer.progress {
        let diff = computed_progress - peer.progress;
        if diff > cfg.maximum_difference && !file_too_small(cfg, torrent) {
            if !window_scheduled(addr, range) {
                schedule_window(addr, range, now, cfg.max_wait_duration);
                return CheckResult::pass(MODULE);
            } else if window_expired(addr, range, now) {
                addr.progress_difference_counter += 1;
                range.progress_difference_counter += 1;
                reset_window(addr, range);
                return ban(
                    cfg.ban_duration,
                    PeerAction::Ban,
                    "pcb:difference",
                    format!(
                        "进度差异 {:.3}：自报 {:.3} 实推 {:.3}",
                        diff, peer.progress, computed_progress
                    ),
                );
            } else {
                return CheckResult::pass(MODULE);
            }
        }
    }

    // 4) rewind：进度回退。
    if cfg.rewind_maximum_difference > 0.0 && !file_too_small(cfg, torrent) {
        let last_report = addr.last_report_progress.max(range.last_report_progress);
        let rewind = last_report - peer.progress;
        if rewind > cfg.rewind_maximum_difference {
            if peer.progress > 0.0 || window_expired(addr, range, now) {
                addr.rewind_counter += 1;
                range.rewind_counter += 1;
                reset_window(addr, range);
                return ban(
                    cfg.ban_duration,
                    PeerAction::Ban,
                    "pcb:rewind",
                    format!(
                        "进度回退 {rewind:.3}：{last_report:.3} → {:.3}",
                        peer.progress
                    ),
                );
            } else if !window_scheduled(addr, range) {
                schedule_window(addr, range, now, cfg.max_wait_duration);
            }
        }
    }

    CheckResult::pass(MODULE)
}

// ---------------- 模块（缓存包装）----------------

type EntryCache = Cache<String, Arc<Mutex<PcbEntry>>>;

/// 进度作弊检测模块。内存核心 + 可选 DB 持久化（重启续算 / 批刷 / 8h 清理）。
pub struct ProgressCheatBlocker {
    cfg: PcbConfig,
    /// `torrent_id|ip:port` → 单 IP 状态。
    addr_cache: EntryCache,
    /// `torrent_id|prefix` → 段聚合状态。
    range_cache: EntryCache,
    /// 后台维护任务停止标志（Drop 时置位）。
    shutdown: Arc<AtomicBool>,
}

impl ProgressCheatBlocker {
    pub fn new(cfg: PcbConfig) -> Self {
        ProgressCheatBlocker {
            cfg,
            addr_cache: Cache::builder().max_capacity(8192).build(),
            range_cache: Cache::builder().max_capacity(8192).build(),
            shutdown: Arc::new(AtomicBool::new(false)),
        }
    }

    /// 带持久化构造：启动后台任务（首次载入历史 → 周期批刷 → 8h 清理），返回 `Arc`。
    pub fn with_persistence(cfg: PcbConfig, db: Db) -> Arc<Self> {
        let persist_duration = cfg.persist_duration;
        let me = Arc::new(Self::new(cfg));
        let addr = me.addr_cache.clone();
        let range = me.range_cache.clone();
        let shutdown = me.shutdown.clone();
        // 任务只持有缓存/db/flag 的克隆，不持有模块 Arc —— 模块被替换(热重载)即 Drop→停任务。
        tokio::spawn(async move {
            let since = now_ms() - persist_duration;
            load_into(&db, &addr, &range, since).await;
            let mut last_cleanup = now_ms();
            loop {
                // 60s 一刷,期间每秒检查停止标志。
                for _ in 0..60 {
                    if shutdown.load(Ordering::Relaxed) {
                        flush(&db, &addr, &range).await;
                        return;
                    }
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
                flush(&db, &addr, &range).await;
                if now_ms() - last_cleanup > 8 * 3600 * 1000 {
                    match db.cleanup_pcb(now_ms() - persist_duration).await {
                        Ok(n) if n > 0 => tracing::info!("PCB 清理过期记录 {n} 条"),
                        Ok(_) => {}
                        Err(e) => tracing::warn!("PCB 清理失败: {e}"),
                    }
                    last_cleanup = now_ms();
                }
            }
        });
        me
    }
}

impl Drop for ProgressCheatBlocker {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
    }
}

impl RuleFeatureModule for ProgressCheatBlocker {
    fn name(&self) -> &'static str {
        MODULE
    }
    fn config_name(&self) -> &'static str {
        "progress-cheat-blocker"
    }
    fn should_ban(&self, torrent: &Torrent, peer: &Peer) -> CheckResult {
        let now = now_ms();
        let ip = peer.address.ip;
        let prefix = if ip.is_ipv4() {
            self.cfg.ipv4_prefix
        } else {
            self.cfg.ipv6_prefix
        };
        let block = match IpNetwork::new_truncate(ip, prefix) {
            Ok(b) => b.to_string(),
            Err(_) => ip.to_string(),
        };
        let akey = format!("{}|{}", torrent.id, peer.address.cache_key());
        let rkey = format!("{}|{block}", torrent.id);
        let addr = self
            .addr_cache
            .get_with(akey, || Arc::new(Mutex::new(PcbEntry::new(now))));
        let range = self
            .range_cache
            .get_with(rkey, || Arc::new(Mutex::new(PcbEntry::new(now))));
        // 统一加锁顺序（addr → range），range 为段共享。
        let mut a = addr.lock();
        let mut r = range.lock();
        evaluate(&self.cfg, &mut a, &mut r, torrent, peer, now)
    }

    fn on_unban(&self, ip: IpAddr) {
        // 解封后清除该 IP 的单点跟踪状态,给其重新开始的机会（段聚合状态保留）。
        let needle = if ip.is_ipv6() {
            format!("|[{ip}]:")
        } else {
            format!("|{ip}:")
        };
        let keys: Vec<String> = self
            .addr_cache
            .iter()
            .map(|(k, _)| (*k).clone())
            .filter(|k| k.contains(&needle))
            .collect();
        for k in keys {
            self.addr_cache.invalidate(&k);
        }
    }
}

// ---------------- 持久化辅助 ----------------

fn to_analysis(e: &PcbEntry) -> PcbAnalysis {
    PcbAnalysis {
        last_report_progress: e.last_report_progress,
        last_report_uploaded: e.last_report_uploaded,
        tracking_uploaded_increase_total: e.tracking_uploaded_increase_total,
        rewind_counter: e.rewind_counter,
        progress_difference_counter: e.progress_difference_counter,
        first_time_seen: e.first_time_seen,
        last_time_seen: e.last_time_seen,
        ban_delay_window_end_at: e.ban_delay_window_end_at,
        fast_pcb_test_execute_at: e.fast_pcb_test_execute_at,
        last_torrent_completed_size: e.last_torrent_completed_size,
    }
}

fn entry_from_analysis(a: &PcbAnalysis) -> PcbEntry {
    PcbEntry {
        last_report_progress: a.last_report_progress,
        last_report_uploaded: a.last_report_uploaded,
        tracking_uploaded_increase_total: a.tracking_uploaded_increase_total,
        rewind_counter: a.rewind_counter,
        progress_difference_counter: a.progress_difference_counter,
        first_time_seen: a.first_time_seen,
        last_time_seen: a.last_time_seen,
        ban_delay_window_end_at: a.ban_delay_window_end_at,
        fast_pcb_test_execute_at: a.fast_pcb_test_execute_at,
        last_torrent_completed_size: a.last_torrent_completed_size,
        dirty: false,
    }
}

/// 还原 raw_ip 串（与 `PeerAddress::cache_key` 同格式）。
fn raw_ip(ip: &str, port: i64) -> String {
    if ip.contains(':') {
        format!("[{ip}]:{port}")
    } else {
        format!("{ip}:{port}")
    }
}

/// 解析 addr key `torrent_id|rawip` → (ip, port, torrent_id)。
fn parse_addr_key(key: &str) -> Option<(String, i64, String)> {
    let (tid, rawip) = key.split_once('|')?;
    let (ip, port) = if let Some(rest) = rawip.strip_prefix('[') {
        // v6: [addr]:port
        let idx = rest.find("]:")?;
        (rest[..idx].to_string(), rest[idx + 2..].parse().ok()?)
    } else {
        let (ip, port) = rawip.rsplit_once(':')?;
        (ip.to_string(), port.parse().ok()?)
    };
    Some((ip, port, tid.to_string()))
}

/// 解析 range key `torrent_id|prefix` → (ip_range, torrent_id)。
fn parse_range_key(key: &str) -> Option<(String, String)> {
    let (tid, prefix) = key.split_once('|')?;
    Some((prefix.to_string(), tid.to_string()))
}

/// 把脏条目批量落库（清 dirty）。
async fn flush(db: &Db, addr: &EntryCache, range: &EntryCache) {
    let mut arows = Vec::new();
    for (k, v) in addr.iter() {
        let mut e = v.lock();
        if !e.dirty {
            continue;
        }
        if let Some((ip, port, tid)) = parse_addr_key(&k) {
            arows.push(PcbAddressRow {
                ip,
                port,
                torrent_id: tid,
                a: to_analysis(&e),
            });
            e.dirty = false;
        }
    }
    if let Err(e) = db.upsert_pcb_addresses(&arows).await {
        tracing::warn!("PCB addr 落库失败: {e}");
    }
    let mut rrows = Vec::new();
    for (k, v) in range.iter() {
        let mut e = v.lock();
        if !e.dirty {
            continue;
        }
        if let Some((ip_range, tid)) = parse_range_key(&k) {
            rrows.push(PcbRangeRow {
                ip_range,
                torrent_id: tid,
                a: to_analysis(&e),
            });
            e.dirty = false;
        }
    }
    if let Err(e) = db.upsert_pcb_ranges(&rrows).await {
        tracing::warn!("PCB range 落库失败: {e}");
    }
}

/// 从 DB 载入近期状态到缓存（重启续算）。
async fn load_into(db: &Db, addr: &EntryCache, range: &EntryCache, since: i64) {
    match db.load_pcb_addresses(since).await {
        Ok(rows) => {
            let n = rows.len();
            for r in rows {
                let key = format!("{}|{}", r.torrent_id, raw_ip(&r.ip, r.port));
                addr.insert(key, Arc::new(Mutex::new(entry_from_analysis(&r.a))));
            }
            if n > 0 {
                tracing::info!("PCB 载入 {n} 条单 IP 历史状态");
            }
        }
        Err(e) => tracing::warn!("PCB 载入 addr 失败: {e}"),
    }
    match db.load_pcb_ranges(since).await {
        Ok(rows) => {
            for r in rows {
                let key = format!("{}|{}", r.torrent_id, r.ip_range);
                range.insert(key, Arc::new(Mutex::new(entry_from_analysis(&r.a))));
            }
        }
        Err(e) => tracing::warn!("PCB 载入 range 失败: {e}"),
    }
}

pub(crate) fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pbh_domain::PeerAddress;

    fn cfg() -> PcbConfig {
        PcbConfig {
            minimum_size: 1000,            // 测试用小阈值
            fast_pcb_test_percentage: 0.0, // 默认关 fast-test,隔离测试其它检查
            ..Default::default()
        }
    }

    fn torrent(size: i64, completed: i64) -> Torrent {
        Torrent {
            id: "t".into(),
            hash: "t".into(),
            name: "t".into(),
            progress: 1.0,
            size,
            completed_size: completed,
            private_torrent: false,
        }
    }

    fn peer(uploaded: i64, progress: f64) -> Peer {
        Peer {
            address: PeerAddress::new("1.2.3.4".parse().unwrap(), 6881),
            peer_id: None,
            client_name: None,
            download_speed: 0,
            upload_speed: 100,
            downloaded: 0,
            uploaded,
            progress,
            flags: None,
        }
    }

    // 序列回放：依次喂入快照，断言每步动作。
    fn replay(cfg: &PcbConfig, steps: &[(Peer, Torrent, i64)]) -> Vec<PeerAction> {
        let mut a = PcbEntry::new(0);
        let mut r = PcbEntry::new(0);
        steps
            .iter()
            .map(|(p, t, now)| evaluate(cfg, &mut a, &mut r, t, p, *now).action)
            .collect()
    }

    #[test]
    fn excessive_upload_bans() {
        // 种子 10000，过量阈值 1.5 → 上传 > 15000 即封。
        let c = cfg();
        let out = replay(
            &c,
            &[
                (peer(5000, 1.0), torrent(10000, 10000), 1),
                (peer(20000, 1.0), torrent(10000, 10000), 2), // 20000 > 15000 → 封
            ],
        );
        assert_eq!(out[0], PeerAction::NoAction);
        assert_eq!(out[1], PeerAction::Ban);
    }

    #[test]
    fn difference_uses_ban_delay_window() {
        // 自报进度 0，但我方上传推算进度高 → 差异超标；首次仅排程窗口,窗口到期才封。
        let c = cfg();
        let t = torrent(10000, 10000);
        let out = replay(
            &c,
            &[
                // 上传 8000/10000=0.8 实推,自报 0.0,差 0.8>0.1 → 排程窗口,放行。
                (peer(8000, 0.0), t.clone(), 1),
                // 窗口未到期（now=2 < 1+30000）→ 放行。
                (peer(8000, 0.0), t.clone(), 2),
                // 窗口到期（now=40000）→ 封。
                (peer(8000, 0.0), t.clone(), 40_000),
            ],
        );
        assert_eq!(out[0], PeerAction::NoAction);
        assert_eq!(out[1], PeerAction::NoAction);
        assert_eq!(out[2], PeerAction::Ban);
    }

    #[test]
    fn rewind_bans_when_progress_active() {
        let c = cfg();
        let t = torrent(10000, 10000);
        // 先报 0.9（低上传,不触发 excessive/difference）,再回退到 0.5（回退 0.4 > 0.07,progress>0 → 封）。
        let out = replay(
            &c,
            &[(peer(10, 0.9), t.clone(), 1), (peer(20, 0.5), t.clone(), 2)],
        );
        assert_eq!(out[0], PeerAction::NoAction);
        assert_eq!(out[1], PeerAction::Ban);
    }

    #[test]
    fn handshaking_and_small_file_pass() {
        let c = cfg();
        // 握手 peer。
        let mut a = PcbEntry::new(0);
        let mut r = PcbEntry::new(0);
        let mut hp = peer(99999, 0.0);
        hp.upload_speed = 0;
        hp.download_speed = 0;
        assert_eq!(
            evaluate(&c, &mut a, &mut r, &torrent(10000, 10000), &hp, 1).action,
            PeerAction::NoAction
        );
        // 小种子（< minimum_size 1000）不触发 difference/rewind,但 excessive 仍按 max(size,min) 评估。
        let out = replay(&c, &[(peer(50, 0.0), torrent(500, 500), 1)]);
        assert_eq!(out[0], PeerAction::NoAction);
    }

    #[test]
    fn fast_pcb_test_disconnects_once() {
        let mut c = cfg();
        c.fast_pcb_test_percentage = 0.1; // 10% → 上传 >= 1000 触发
        c.excessive_threshold = 1000.0; // 抬高 excessive 阈值避免抢先命中
        let t = torrent(10000, 10000);
        let out = replay(&c, &[(peer(2000, 1.0), t.clone(), 1)]);
        assert_eq!(out[0], PeerAction::BanForDisconnect);
        // 第二次不再重复 fast-test（execute_at 已置）。
        let mut a = PcbEntry::new(0);
        let mut r = PcbEntry::new(0);
        evaluate(&c, &mut a, &mut r, &t, &peer(2000, 1.0), 1);
        let second = evaluate(&c, &mut a, &mut r, &t, &peer(2500, 1.0), 2).action;
        assert_ne!(second, PeerAction::BanForDisconnect);
    }
}
