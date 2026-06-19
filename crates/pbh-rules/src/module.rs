//! 规则模块 trait + 离线规则模块（不依赖网络/GeoIP/BanList）。
//!
//! 对应上游 `module/RuleFeatureModule` 与 `module/impl/rule/*`。
//! 需要 BanList 的 AutoRangeBan、需要 GeoIP 的 IPBlackList 等放到各自里程碑/crate。

use pbh_domain::{CheckResult, Peer, PeerAction, Torrent};

use crate::matcher::{MatchOutcome, RuleSet};

/// 规则模块统一接口。每次 ban wave 对每个 (torrent, peer) 调用一次。
pub trait RuleFeatureModule: Send + Sync {
    /// 模块标识（用于日志/封禁记录）。
    fn name(&self) -> &'static str;
    /// 配置名（`profile.yml` 的 `module.<configName>`）。
    fn config_name(&self) -> &'static str;
    /// 检查单个 peer。
    fn should_ban(&self, torrent: &Torrent, peer: &Peer) -> CheckResult;
}

fn ban(module: &'static str, duration_ms: i64, rule: String, reason: String) -> CheckResult {
    CheckResult {
        module,
        action: PeerAction::Ban,
        duration_ms,
        rule,
        reason,
    }
}

// ---------------- PeerIdBlacklist ----------------

/// 按 BT PeerID 匹配封禁。对应上游 `PeerIdBlacklist`。
pub struct PeerIdBlacklist {
    rules: RuleSet,
    ban_duration: i64,
}

impl PeerIdBlacklist {
    pub fn new(rules: RuleSet, ban_duration: i64) -> Self {
        PeerIdBlacklist {
            rules,
            ban_duration,
        }
    }
}

impl RuleFeatureModule for PeerIdBlacklist {
    fn name(&self) -> &'static str {
        "PeerIdBlacklist"
    }
    fn config_name(&self) -> &'static str {
        "peer-id-blacklist"
    }
    fn should_ban(&self, _torrent: &Torrent, peer: &Peer) -> CheckResult {
        let Some(pid) = peer.peer_id.as_deref() else {
            return CheckResult::pass(self.name());
        };
        if self.rules.match_input(pid) == MatchOutcome::True {
            ban(
                self.name(),
                self.ban_duration,
                "peer-id-blacklist".into(),
                format!("PeerID 命中黑名单: {pid}"),
            )
        } else {
            CheckResult::pass(self.name())
        }
    }
}

// ---------------- ClientNameBlacklist ----------------

/// 按客户端名匹配封禁。对应上游 `ClientNameBlacklist`。
pub struct ClientNameBlacklist {
    rules: RuleSet,
    ban_duration: i64,
}

impl ClientNameBlacklist {
    pub fn new(rules: RuleSet, ban_duration: i64) -> Self {
        ClientNameBlacklist {
            rules,
            ban_duration,
        }
    }
}

impl RuleFeatureModule for ClientNameBlacklist {
    fn name(&self) -> &'static str {
        "ClientNameBlacklist"
    }
    fn config_name(&self) -> &'static str {
        "client-name-blacklist"
    }
    fn should_ban(&self, _torrent: &Torrent, peer: &Peer) -> CheckResult {
        let Some(name) = peer.client_name.as_deref() else {
            return CheckResult::pass(self.name());
        };
        if self.rules.match_input(name) == MatchOutcome::True {
            ban(
                self.name(),
                self.ban_duration,
                "client-name-blacklist".into(),
                format!("客户端名命中黑名单: {name}"),
            )
        } else {
            CheckResult::pass(self.name())
        }
    }
}

// ---------------- AntiVampire ----------------

/// 反吸血（迅雷预设）。对应上游 `AntiVampire`。
pub struct AntiVampire {
    ban_duration: i64,
    xunlei_enabled: bool,
}

impl AntiVampire {
    pub fn new(ban_duration: i64, xunlei_enabled: bool) -> Self {
        AntiVampire {
            ban_duration,
            xunlei_enabled,
        }
    }
}

impl RuleFeatureModule for AntiVampire {
    fn name(&self) -> &'static str {
        "AntiVampire"
    }
    fn config_name(&self) -> &'static str {
        "anti-vampire"
    }
    fn should_ban(&self, torrent: &Torrent, peer: &Peer) -> CheckResult {
        if !self.xunlei_enabled {
            return CheckResult::pass(self.name());
        }
        let pid = peer.peer_id.as_deref().unwrap_or("").to_lowercase();
        let name = peer.client_name.as_deref().unwrap_or("").to_lowercase();
        let is_xunlei = pid.starts_with("-xl") || name.starts_with("xunlei");
        if !is_xunlei {
            return CheckResult::pass(self.name());
        }
        let is_0019 =
            pid.starts_with("-xl0019") || name.contains("0019") || name.contains("0.0.1.9");
        // 0019 变体：仅在我方做种时封（下载时允许其参与）。
        if is_0019 && !torrent.is_seeding() {
            return CheckResult::pass(self.name());
        }
        ban(
            self.name(),
            self.ban_duration,
            "anti-vampire:xunlei".into(),
            "迅雷吸血客户端".into(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::matcher::{Matcher, StringRule};
    use pbh_domain::PeerAddress;

    fn peer(peer_id: Option<&str>, client: Option<&str>) -> Peer {
        Peer {
            address: PeerAddress::new("1.2.3.4".parse().unwrap(), 6881),
            peer_id: peer_id.map(String::from),
            client_name: client.map(String::from),
            download_speed: 1,
            upload_speed: 1,
            downloaded: 0,
            uploaded: 0,
            progress: 0.5,
            flags: None,
        }
    }

    fn torrent(progress: f64) -> Torrent {
        Torrent {
            id: "h".into(),
            hash: "h".into(),
            name: "t".into(),
            progress,
            size: 100,
            completed_size: -1,
            private_torrent: false,
        }
    }

    #[test]
    fn peer_id_blacklist_bans_match() {
        let rs = RuleSet::new(vec![StringRule::new(Matcher::StartsWith("-XL".into()))]);
        let m = PeerIdBlacklist::new(rs, 1000);
        assert_eq!(
            m.should_ban(&torrent(0.0), &peer(Some("-XL0019-"), None))
                .action,
            PeerAction::Ban
        );
        assert_eq!(
            m.should_ban(&torrent(0.0), &peer(Some("-qB4250-"), None))
                .action,
            PeerAction::NoAction
        );
    }

    #[test]
    fn anti_vampire_xunlei_0019_only_when_seeding() {
        let m = AntiVampire::new(1000, true);
        // 0019 下载中 → 放行。
        assert_eq!(
            m.should_ban(&torrent(0.5), &peer(Some("-XL0019-"), None))
                .action,
            PeerAction::NoAction
        );
        // 0019 做种中 → 封。
        assert_eq!(
            m.should_ban(&torrent(1.0), &peer(Some("-XL0019-"), None))
                .action,
            PeerAction::Ban
        );
        // 非 0019 迅雷 → 始终封。
        assert_eq!(
            m.should_ban(&torrent(0.5), &peer(Some("-XL0012-"), None))
                .action,
            PeerAction::Ban
        );
        // 非迅雷 → 放行。
        assert_eq!(
            m.should_ban(&torrent(0.5), &peer(Some("-qB4250-"), None))
                .action,
            PeerAction::NoAction
        );
    }
}
