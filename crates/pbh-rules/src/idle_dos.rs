//! IdleConnectionDosProtection —— 空闲连接 DoS 防护。
//! 对应上游 `module/impl/rule/IdleConnectionDosProtection.java`。
//!
//! 一些客户端建立连接后长期不传输数据（既不上传也不下载、进度不变），白占连接位形成轻量 DoS。
//! 本模块跟踪每个 peer 的「空闲起始时刻」与上次字节数快照，空闲超过 `max-allowed-idle-time` 即封禁。
//! 上游用 `expireAfterAccess` + 批回调清理无效条目；这里用 `moka` 的 time-to-idle 自动驱逐替代。

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use moka::sync::Cache;
use pbh_domain::{CheckResult, Peer, PeerAction, Torrent};

use crate::module::RuleFeatureModule;

/// 保护模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProtectMode {
    /// 0：由 peer flags 决定——做种总是保护；下载时若 peer 有兴趣位(d/D/u/U)则放行。
    ByPeerFlags,
    /// 1：仅做种任务保护，下载任务不检查。
    SeedingOnly,
    /// 2：做种与下载都保护（最激进，易误判）。
    Always,
}

impl ProtectMode {
    pub fn from_u8(v: u8) -> Self {
        match v {
            1 => ProtectMode::SeedingOnly,
            2 => ProtectMode::Always,
            _ => ProtectMode::ByPeerFlags,
        }
    }
}

#[derive(Debug, Clone)]
struct ConnInfo {
    idle_start_ms: i64,
    percentage: f64,
    uploaded: i64,
    downloaded: i64,
}

/// 空闲连接 DoS 防护模块。
pub struct IdleConnectionDosProtection {
    ban_duration: i64,
    max_allowed_idle_ms: i64,
    idle_speed_threshold: i64,
    min_status_change_pct: f64,
    reset_on_status_change: bool,
    mode: ProtectMode,
    cache: Cache<String, ConnInfo>,
}

impl IdleConnectionDosProtection {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        ban_duration: i64,
        max_allowed_idle_ms: i64,
        idle_speed_threshold: i64,
        min_status_change_pct: f64,
        reset_on_status_change: bool,
        mode: ProtectMode,
    ) -> Self {
        // time-to-idle 给足够余量（> 最大空闲时长），peer 持续出现即续期，消失后自动驱逐。
        let tti = Duration::from_millis((max_allowed_idle_ms.max(60_000) as u64).saturating_mul(2));
        IdleConnectionDosProtection {
            ban_duration,
            max_allowed_idle_ms,
            idle_speed_threshold,
            min_status_change_pct,
            reset_on_status_change,
            mode,
            cache: Cache::builder()
                .max_capacity(16_384)
                .time_to_idle(tti)
                .build(),
        }
    }

    /// 是否对该 (torrent, peer) 启用保护。
    fn protected(&self, torrent: &Torrent, peer: &Peer) -> bool {
        match self.mode {
            ProtectMode::Always => true,
            ProtectMode::SeedingOnly => torrent.is_seeding(),
            ProtectMode::ByPeerFlags => {
                if torrent.is_seeding() {
                    return true;
                }
                // 下载中：peer 表达了兴趣（在传输）则放行，不视为空闲。
                match &peer.flags {
                    Some(f) => !(f.interesting || f.remote_interested),
                    None => true,
                }
            }
        }
    }
}

impl RuleFeatureModule for IdleConnectionDosProtection {
    fn name(&self) -> &'static str {
        "IdleConnectionDosProtection"
    }
    fn config_name(&self) -> &'static str {
        "idle-connection-dos-protection"
    }
    fn should_ban(&self, torrent: &Torrent, peer: &Peer) -> CheckResult {
        let key = peer.address.cache_key();
        if !self.protected(torrent, peer) {
            self.cache.invalidate(&key);
            return CheckResult::pass(self.name());
        }
        // 速度快速路径：仍在传输 → 重置并放行。
        if peer.upload_speed > self.idle_speed_threshold
            || peer.download_speed > self.idle_speed_threshold
        {
            self.cache.invalidate(&key);
            return CheckResult::pass(self.name());
        }
        let now = now_ms();
        match self.cache.get(&key) {
            None => {
                self.cache.insert(key, fresh(now, peer));
                CheckResult::pass(self.name())
            }
            Some(info) => {
                let up_delta = byte_delta(peer.uploaded, info.uploaded);
                let dn_delta = byte_delta(peer.downloaded, info.downloaded);
                let prog_delta = (peer.progress - info.percentage).abs();
                // 有实际传输 → 重置空闲。
                if up_delta > self.idle_speed_threshold || dn_delta > self.idle_speed_threshold {
                    self.cache.insert(key, fresh(now, peer));
                    return CheckResult::pass(self.name());
                }
                // 进度变化视为活动。
                if self.reset_on_status_change && prog_delta >= self.min_status_change_pct {
                    self.cache.insert(key, fresh(now, peer));
                    return CheckResult::pass(self.name());
                }
                // 空闲超时 → 封禁。
                if now.saturating_sub(info.idle_start_ms) > self.max_allowed_idle_ms {
                    return CheckResult {
                        module: self.name(),
                        action: PeerAction::Ban,
                        duration_ms: self.ban_duration,
                        rule: "idle-connection-dos".into(),
                        reason: format!("空闲连接超过 {}ms 无传输", self.max_allowed_idle_ms),
                    };
                }
                // 仍在窗口内：保留空闲起点，仅刷新字节快照。
                self.cache.insert(
                    key,
                    ConnInfo {
                        idle_start_ms: info.idle_start_ms,
                        percentage: peer.progress,
                        uploaded: peer.uploaded,
                        downloaded: peer.downloaded,
                    },
                );
                CheckResult::pass(self.name())
            }
        }
    }
}

fn fresh(now: i64, peer: &Peer) -> ConnInfo {
    ConnInfo {
        idle_start_ms: now,
        percentage: peer.progress,
        uploaded: peer.uploaded,
        downloaded: peer.downloaded,
    }
}

/// 字节增量；任一为 -1（下载器不可报告）时按 0 处理。
fn byte_delta(now: i64, prev: i64) -> i64 {
    if now < 0 || prev < 0 {
        return 0;
    }
    (now - prev).max(0)
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pbh_domain::PeerAddress;

    fn idle_peer() -> Peer {
        Peer {
            address: PeerAddress::new("1.2.3.4".parse().unwrap(), 6881),
            peer_id: None,
            client_name: None,
            download_speed: 0,
            upload_speed: 0,
            downloaded: 0,
            uploaded: 0,
            progress: 0.0,
            flags: None,
        }
    }
    fn seeding() -> Torrent {
        Torrent {
            id: "h".into(),
            hash: "h".into(),
            name: "t".into(),
            progress: 1.0,
            size: 100,
            completed_size: 100,
            private_torrent: false,
        }
    }

    #[test]
    fn first_sight_passes_then_idle_bans() {
        // max_idle 很小，第二次检查即超时。
        let m = IdleConnectionDosProtection::new(1000, 0, 64, 0.001, true, ProtectMode::Always);
        let t = seeding();
        let p = idle_peer();
        // 首次：记录,放行。
        assert_eq!(m.should_ban(&t, &p).action, PeerAction::NoAction);
        // 第二次:无传输、无进度变化、空闲 > 0ms → 封。
        std::thread::sleep(std::time::Duration::from_millis(3));
        assert_eq!(m.should_ban(&t, &p).action, PeerAction::Ban);
    }

    #[test]
    fn active_transfer_resets() {
        let m = IdleConnectionDosProtection::new(1000, 0, 64, 0.001, true, ProtectMode::Always);
        let t = seeding();
        let mut p = idle_peer();
        assert_eq!(m.should_ban(&t, &p).action, PeerAction::NoAction);
        // 上传增加 → 活动 → 放行,不封。
        p.uploaded = 10_000;
        std::thread::sleep(std::time::Duration::from_millis(3));
        assert_eq!(m.should_ban(&t, &p).action, PeerAction::NoAction);
    }

    #[test]
    fn seeding_only_skips_download_task() {
        let m =
            IdleConnectionDosProtection::new(1000, 0, 64, 0.001, true, ProtectMode::SeedingOnly);
        let downloading = Torrent {
            progress: 0.3,
            completed_size: 30,
            ..seeding()
        };
        let p = idle_peer();
        assert_eq!(m.should_ban(&downloading, &p).action, PeerAction::NoAction);
        std::thread::sleep(std::time::Duration::from_millis(3));
        // 下载任务不保护 → 永不封。
        assert_eq!(m.should_ban(&downloading, &p).action, PeerAction::NoAction);
    }
}
