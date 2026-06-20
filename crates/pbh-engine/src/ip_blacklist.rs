//! IPBlackList —— IP 黑名单（IP/CIDR、端口、ASN、地区、城市、中国网络类型）。
//! 对应上游 `module/impl/rule/IPBlackList.java`。
//!
//! 短路顺序:port → ip/CIDR → （GeoIP 可用时）asn → region → city → net-type。
//! GeoIP 不可用（无 mmdb）时,asn/region/city/net-type 检查全部跳过,仅 ip/port 生效。

use std::collections::HashSet;

use pbh_domain::{CheckResult, Peer, PeerAction, Torrent};
use pbh_geoip::GeoIpHandle;
use pbh_rules::{IpMatcher, RuleFeatureModule};

/// IP 黑名单模块配置。
pub struct IpBlackList {
    ban_duration: i64,
    ips: IpMatcher<()>,
    ports: HashSet<u16>,
    asns: HashSet<u32>,
    regions: HashSet<String>,
    cities: Vec<String>,
    /// 启用的中国网络类型名（需 GeoCN 数据库）。
    net_types: HashSet<String>,
    geoip: GeoIpHandle,
}

impl IpBlackList {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        ban_duration: i64,
        ip_list: &[String],
        ports: HashSet<u16>,
        asns: HashSet<u32>,
        regions: HashSet<String>,
        cities: Vec<String>,
        net_types: HashSet<String>,
        geoip: GeoIpHandle,
    ) -> Self {
        let mut ips = IpMatcher::new();
        for c in ip_list {
            ips.insert(c, ());
        }
        IpBlackList {
            ban_duration,
            ips,
            ports,
            asns,
            regions,
            cities,
            net_types,
            geoip,
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

    fn needs_geoip(&self) -> bool {
        !self.asns.is_empty()
            || !self.regions.is_empty()
            || !self.cities.is_empty()
            || !self.net_types.is_empty()
    }
}

impl RuleFeatureModule for IpBlackList {
    fn name(&self) -> &'static str {
        "IPBlackList"
    }
    fn config_name(&self) -> &'static str {
        "ip-address-blocker"
    }
    fn should_ban(&self, _torrent: &Torrent, peer: &Peer) -> CheckResult {
        let ip = peer.address.ip;
        // 1) 端口。
        if self.ports.contains(&peer.address.port) {
            return self.ban(
                "ipbl:port",
                format!("端口 {} 命中黑名单", peer.address.port),
            );
        }
        // 2) IP / CIDR。
        if self.ips.contains(ip) {
            return self.ban("ipbl:ip", format!("IP {ip} 命中黑名单"));
        }
        // 3) GeoIP 依赖项（不可用则跳过）。
        if !self.needs_geoip() {
            return CheckResult::pass(self.name());
        }
        let Some(geo) = self.geoip.query(ip) else {
            return CheckResult::pass(self.name());
        };
        if let Some(asn) = geo.asn {
            if self.asns.contains(&asn) {
                return self.ban("ipbl:asn", format!("ASN {asn} 命中黑名单"));
            }
        }
        if let Some(iso) = &geo.country_iso {
            if self.regions.contains(iso) {
                return self.ban("ipbl:region", format!("地区 {iso} 命中黑名单"));
            }
        }
        if let Some(city) = &geo.city_name {
            for c in &self.cities {
                if city.contains(c) {
                    return self.ban("ipbl:city", format!("城市 {city} 命中黑名单"));
                }
            }
        }
        if let Some(nt) = &geo.net_type {
            if self.net_types.contains(nt) {
                return self.ban("ipbl:net-type", format!("网络类型 {nt} 命中黑名单"));
            }
        }
        CheckResult::pass(self.name())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pbh_domain::PeerAddress;

    fn peer(ip: &str, port: u16) -> Peer {
        Peer {
            address: PeerAddress::new(ip.parse().unwrap(), port),
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
    fn torrent() -> Torrent {
        Torrent {
            id: "t".into(),
            hash: "t".into(),
            name: "t".into(),
            progress: 0.0,
            size: 1,
            completed_size: -1,
            private_torrent: false,
        }
    }

    #[test]
    fn bans_by_ip_and_port_without_geoip() {
        let m = IpBlackList::new(
            1000,
            &["1.2.3.0/24".to_string()],
            HashSet::from([2003u16]),
            HashSet::new(),
            HashSet::new(),
            Vec::new(),
            HashSet::new(),
            GeoIpHandle::new_empty(),
        );
        // 端口命中。
        assert_eq!(
            m.should_ban(&torrent(), &peer("9.9.9.9", 2003)).action,
            PeerAction::Ban
        );
        // IP 命中（CIDR）。
        assert_eq!(
            m.should_ban(&torrent(), &peer("1.2.3.55", 6881)).action,
            PeerAction::Ban
        );
        // 都不命中。
        assert_eq!(
            m.should_ban(&torrent(), &peer("8.8.8.8", 6881)).action,
            PeerAction::NoAction
        );
    }

    #[test]
    fn geoip_checks_skip_when_provider_absent() {
        // 配了 region 但无 provider → 跳过,不封。
        let m = IpBlackList::new(
            1000,
            &[],
            HashSet::new(),
            HashSet::new(),
            HashSet::from(["CN".to_string()]),
            Vec::new(),
            HashSet::new(),
            GeoIpHandle::new_empty(),
        );
        assert_eq!(
            m.should_ban(&torrent(), &peer("1.2.3.4", 6881)).action,
            PeerAction::NoAction
        );
    }
}
