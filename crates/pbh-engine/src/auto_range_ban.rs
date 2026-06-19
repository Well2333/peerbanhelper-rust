//! AutoRangeBan —— 自动 IP 段封禁。对应上游 `module/impl/rule/AutoRangeBan.java`。
//!
//! 当某个 peer 所在的 /N 段（IPv4 默认 /30、IPv6 默认 /48）内已有别的 IP 被规则封禁，
//! 就把该 peer 一并封禁——多拨/同段恶意常成片出现。读 [`BanList`]，无内部状态。

use std::sync::Arc;

use ip_network::IpNetwork;
use pbh_domain::{CheckResult, Peer, PeerAction, Torrent};
use pbh_rules::RuleFeatureModule;

use crate::BanList;

/// 自动段封禁模块。
pub struct AutoRangeBan {
    ban_list: Arc<BanList>,
    ipv4_prefix: u8,
    ipv6_prefix: u8,
    ban_duration: i64,
}

impl AutoRangeBan {
    pub fn new(
        ban_list: Arc<BanList>,
        ipv4_prefix: u8,
        ipv6_prefix: u8,
        ban_duration: i64,
    ) -> Self {
        AutoRangeBan {
            ban_list,
            ipv4_prefix,
            ipv6_prefix,
            ban_duration,
        }
    }
}

impl RuleFeatureModule for AutoRangeBan {
    fn name(&self) -> &'static str {
        "AutoRangeBan"
    }
    fn config_name(&self) -> &'static str {
        "auto-range-ban"
    }
    fn should_ban(&self, _torrent: &Torrent, peer: &Peer) -> CheckResult {
        // 握手阶段不判定（与上游一致）。
        if peer.is_handshaking() {
            return CheckResult::pass(self.name());
        }
        let ip = peer.address.ip;
        let prefix = if ip.is_ipv4() {
            self.ipv4_prefix
        } else {
            self.ipv6_prefix
        };
        let Ok(block) = IpNetwork::new_truncate(ip, prefix) else {
            return CheckResult::pass(self.name());
        };
        match self.ban_list.any_active_ban_in(block) {
            // 段内有其它已封 IP（排除当前 peer 自己）。
            Some(src) if src != ip => CheckResult {
                module: self.name(),
                action: PeerAction::Ban,
                duration_ms: self.ban_duration,
                rule: format!("{block}"),
                reason: format!("同段 {block} 已封禁 {src}"),
            },
            _ => CheckResult::pass(self.name()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pbh_domain::{BanMetadata, PeerAddress};

    fn meta() -> BanMetadata {
        BanMetadata {
            context: "test".into(),
            random_id: "id".into(),
            peer: PeerAddress::new("1.2.3.4".parse().unwrap(), 6881),
            ban_at: 0,
            unban_at: i64::MAX,
            ban_for_disconnect: false,
            exclude_from_report: false,
            exclude_from_display: false,
            rule: String::new(),
            description: String::new(),
        }
    }

    fn peer(ip: &str) -> Peer {
        Peer {
            address: PeerAddress::new(ip.parse().unwrap(), 6881),
            peer_id: None,
            client_name: None,
            download_speed: 1,
            upload_speed: 1,
            downloaded: 0,
            uploaded: 0,
            progress: 0.0,
            flags: None,
        }
    }

    fn torrent() -> Torrent {
        Torrent {
            id: "h".into(),
            hash: "h".into(),
            name: "t".into(),
            progress: 0.0,
            size: 100,
            completed_size: -1,
            private_torrent: false,
        }
    }

    #[test]
    fn bans_neighbor_in_same_v4_block() {
        let bl = Arc::new(BanList::new());
        bl.ban("1.2.3.4", meta()); // 已封
        let m = AutoRangeBan::new(bl, 30, 48, 1000);
        // 1.2.3.5 与 1.2.3.4 同 /30（1.2.3.4/30 含 .4–.7）。
        assert_eq!(
            m.should_ban(&torrent(), &peer("1.2.3.5")).action,
            PeerAction::Ban
        );
        // 1.2.3.200 不在该 /30。
        assert_eq!(
            m.should_ban(&torrent(), &peer("1.2.3.200")).action,
            PeerAction::NoAction
        );
    }

    #[test]
    fn skips_ban_for_disconnect_entries() {
        let bl = Arc::new(BanList::new());
        let mut md = meta();
        md.ban_for_disconnect = true;
        bl.ban("1.2.3.4", md);
        let m = AutoRangeBan::new(bl, 30, 48, 1000);
        // 仅有 ban_for_disconnect 记录 → 不触发段封。
        assert_eq!(
            m.should_ban(&torrent(), &peer("1.2.3.5")).action,
            PeerAction::NoAction
        );
    }

    #[test]
    fn handshaking_peer_passes() {
        let bl = Arc::new(BanList::new());
        bl.ban("1.2.3.4", meta());
        let m = AutoRangeBan::new(bl, 30, 48, 1000);
        let mut p = peer("1.2.3.5");
        p.upload_speed = 0;
        p.download_speed = 0;
        assert_eq!(m.should_ban(&torrent(), &p).action, PeerAction::NoAction);
    }
}
