//! MultiDialingBlocker —— 多拨号封禁。对应上游 `module/impl/rule/MultiDialingBlocker.java`。
//!
//! 同一种子下，同一子网段（IPv4 默认 /24、IPv6 默认 /56）内出现的不同 IP 超过容忍数，
//! 判定为多拨（一个用户用多个拨号 IP 连同一种子吸血），封禁后续 IP。可选「追猎」模式：
//! 段一旦被标记，在追猎期内对该段所有新 peer 持续封禁。
//!
//! 内部状态用 `moka` 缓存承载 TTL，无需手动清理。

use std::collections::HashSet;
use std::net::IpAddr;
use std::sync::Arc;
use std::time::Duration;

use ip_network::IpNetwork;
use moka::sync::Cache;
use parking_lot::Mutex;
use pbh_domain::{CheckResult, Peer, PeerAction, Torrent};

use crate::module::RuleFeatureModule;

/// 多拨号封禁模块。
pub struct MultiDialingBlocker {
    ipv4_prefix: u8,
    ipv6_prefix: u8,
    tolerate_v4: usize,
    tolerate_v6: usize,
    keep_hunting: bool,
    ban_duration: i64,
    /// `torrentId@subnet` → 该段内出现过的 IP 集合。
    subnet_ips: Cache<String, Arc<Mutex<HashSet<IpAddr>>>>,
    /// `torrentId@subnet` → 追猎标记（仅 keep_hunting 时使用）。
    hunting: Cache<String, ()>,
}

impl MultiDialingBlocker {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        ipv4_prefix: u8,
        ipv6_prefix: u8,
        tolerate_v4: usize,
        tolerate_v6: usize,
        cache_lifespan_secs: u64,
        keep_hunting: bool,
        keep_hunting_secs: u64,
        ban_duration: i64,
    ) -> Self {
        MultiDialingBlocker {
            ipv4_prefix,
            ipv6_prefix,
            tolerate_v4,
            tolerate_v6,
            keep_hunting,
            ban_duration,
            subnet_ips: Cache::builder()
                .max_capacity(4096)
                .time_to_live(Duration::from_secs(cache_lifespan_secs.max(1)))
                .build(),
            hunting: Cache::builder()
                .max_capacity(4096)
                .time_to_live(Duration::from_secs(keep_hunting_secs.max(1)))
                .build(),
        }
    }

    fn ban(&self, rule: &str, reason: String) -> CheckResult {
        CheckResult {
            module: self.name(),
            action: PeerAction::Ban,
            duration_ms: self.ban_duration,
            rule: rule.into(),
            reason,
        }
    }
}

impl RuleFeatureModule for MultiDialingBlocker {
    fn name(&self) -> &'static str {
        "MultiDialingBlocker"
    }
    fn config_name(&self) -> &'static str {
        "multi-dialing-blocker"
    }
    fn should_ban(&self, torrent: &Torrent, peer: &Peer) -> CheckResult {
        if peer.is_handshaking() {
            return CheckResult::pass(self.name());
        }
        let ip = peer.address.ip;
        let (prefix, tolerate) = if ip.is_ipv4() {
            (self.ipv4_prefix, self.tolerate_v4)
        } else {
            (self.ipv6_prefix, self.tolerate_v6)
        };
        let Ok(block) = IpNetwork::new_truncate(ip, prefix) else {
            return CheckResult::pass(self.name());
        };
        let key = format!("{}@{block}", torrent.id);

        // 追猎模式：段已被标记 → 持续封禁该段新 peer。
        if self.keep_hunting && self.hunting.get(&key).is_some() {
            self.hunting.insert(key, ()); // 刷新追猎期
            return self.ban("mdb:hunting", format!("追猎段 {block}"));
        }

        // 登记本 IP，统计段内去重 IP 数。
        let set = self
            .subnet_ips
            .get_with(key.clone(), || Arc::new(Mutex::new(HashSet::new())));
        let count = {
            let mut g = set.lock();
            g.insert(ip);
            g.len()
        };
        if count > tolerate {
            if self.keep_hunting {
                self.hunting.insert(key, ());
            }
            return self.ban(
                "mdb:multi-dialing",
                format!("同段 {block} 多拨 {count} 个 IP（容忍 {tolerate}）"),
            );
        }
        CheckResult::pass(self.name())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pbh_domain::PeerAddress;

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
    fn torrent(id: &str) -> Torrent {
        Torrent {
            id: id.into(),
            hash: id.into(),
            name: "t".into(),
            progress: 0.0,
            size: 100,
            completed_size: -1,
            private_torrent: false,
        }
    }

    #[test]
    fn bans_after_exceeding_tolerance() {
        // /24 段，容忍 2 → 第 3 个不同 IP 触发。
        let m = MultiDialingBlocker::new(24, 56, 2, 5, 86400, false, 100, 1000);
        let t = torrent("T1");
        assert_eq!(
            m.should_ban(&t, &peer("1.2.3.10")).action,
            PeerAction::NoAction
        );
        assert_eq!(
            m.should_ban(&t, &peer("1.2.3.11")).action,
            PeerAction::NoAction
        );
        assert_eq!(m.should_ban(&t, &peer("1.2.3.12")).action, PeerAction::Ban);
    }

    #[test]
    fn different_subnet_or_torrent_independent() {
        let m = MultiDialingBlocker::new(24, 56, 2, 5, 86400, false, 100, 1000);
        // 不同段不累加。
        assert_eq!(
            m.should_ban(&torrent("T1"), &peer("1.2.3.10")).action,
            PeerAction::NoAction
        );
        assert_eq!(
            m.should_ban(&torrent("T1"), &peer("9.9.9.10")).action,
            PeerAction::NoAction
        );
        assert_eq!(
            m.should_ban(&torrent("T1"), &peer("9.9.9.11")).action,
            PeerAction::NoAction
        );
    }

    #[test]
    fn same_ip_repeated_does_not_accumulate() {
        let m = MultiDialingBlocker::new(24, 56, 2, 5, 86400, false, 100, 1000);
        let t = torrent("T1");
        for _ in 0..5 {
            assert_eq!(
                m.should_ban(&t, &peer("1.2.3.10")).action,
                PeerAction::NoAction
            );
        }
    }
}
