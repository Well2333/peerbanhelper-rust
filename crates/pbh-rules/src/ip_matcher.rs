//! IP/CIDR 集合匹配（最长前缀）。对应上游 `util/rule/matcher/IPMatcher.java`
//! （上游用 `DualIPv4v6AssociativeTries`）。
//!
//! 用 `ip_network_table` 的双栈 LPM 表，每个 CIDR 关联一个值（如规则名/注释/分类）。

use std::net::IpAddr;
use std::str::FromStr;

use ip_network::IpNetwork;
use ip_network_table::IpNetworkTable;

/// CIDR → 关联值 `V` 的最长前缀匹配表。
pub struct IpMatcher<V> {
    table: IpNetworkTable<V>,
}

impl<V> Default for IpMatcher<V> {
    fn default() -> Self {
        Self::new()
    }
}

impl<V> IpMatcher<V> {
    pub fn new() -> Self {
        IpMatcher {
            table: IpNetworkTable::new(),
        }
    }

    /// 插入一个 CIDR 或裸 IP（裸 IP 视为 /32 或 /128）。返回是否解析成功。
    pub fn insert(&mut self, cidr_or_ip: &str, value: V) -> bool {
        match parse_network(cidr_or_ip) {
            Some(net) => {
                self.table.insert(net, value);
                true
            }
            None => false,
        }
    }

    /// 查询 IP 是否被某 CIDR 覆盖，返回最长匹配的关联值。
    pub fn longest_match(&self, ip: IpAddr) -> Option<&V> {
        self.table.longest_match(ip).map(|(_net, v)| v)
    }

    /// 仅判断是否命中。
    pub fn contains(&self, ip: IpAddr) -> bool {
        self.longest_match(ip).is_some()
    }

    pub fn len(&self) -> usize {
        let (v4, v6) = self.table.len();
        v4 + v6
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// 解析 CIDR（`1.2.3.0/24`）或裸 IP（`1.2.3.4` → /32，`::1` → /128）。
fn parse_network(s: &str) -> Option<IpNetwork> {
    if let Ok(net) = IpNetwork::from_str(s) {
        return Some(net);
    }
    // 裸 IP 回退。
    match IpAddr::from_str(s.trim()) {
        Ok(IpAddr::V4(v4)) => IpNetwork::new(v4, 32).ok(),
        Ok(IpAddr::V6(v6)) => IpNetwork::new(v6, 128).ok(),
        Err(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cidr_longest_prefix_match() {
        let mut m: IpMatcher<&str> = IpMatcher::new();
        assert!(m.insert("10.0.0.0/8", "wide"));
        assert!(m.insert("10.1.2.0/24", "narrow"));
        // 最长前缀：10.1.2.5 命中 /24。
        assert_eq!(
            m.longest_match("10.1.2.5".parse().unwrap()),
            Some(&"narrow")
        );
        // 10.9.9.9 仅命中 /8。
        assert_eq!(m.longest_match("10.9.9.9".parse().unwrap()), Some(&"wide"));
        // 外部 IP 不命中。
        assert!(!m.contains("8.8.8.8".parse().unwrap()));
    }

    #[test]
    fn bare_ip_and_ipv6() {
        let mut m: IpMatcher<u8> = IpMatcher::new();
        assert!(m.insert("1.2.3.4", 1));
        assert!(m.insert("2001:db8::/32", 2));
        assert!(m.contains("1.2.3.4".parse().unwrap()));
        assert!(!m.contains("1.2.3.5".parse().unwrap()));
        assert!(m.contains("2001:db8::dead".parse().unwrap()));
        assert_eq!(m.len(), 2);
    }

    #[test]
    fn rejects_garbage() {
        let mut m: IpMatcher<()> = IpMatcher::new();
        assert!(!m.insert("not-an-ip", ()));
        assert!(m.is_empty());
    }
}
