//! 内存封禁表（运行期权威）。对应上游 `BanList.java`（`DualIPv4v6AssociativeTries`）。
//!
//! 双栈最长前缀匹配 + 读写锁。键为 IP/CIDR，值为 `BanMetadata`。数据库仅周期快照，非实时镜像。

use std::net::IpAddr;
use std::str::FromStr;

use ip_network::IpNetwork;
use ip_network_table::IpNetworkTable;
use parking_lot::RwLock;
use pbh_domain::BanMetadata;

/// 内存封禁表。
pub struct BanList {
    inner: RwLock<IpNetworkTable<BanMetadata>>,
}

impl Default for BanList {
    fn default() -> Self {
        Self::new()
    }
}

impl BanList {
    pub fn new() -> Self {
        BanList {
            inner: RwLock::new(IpNetworkTable::new()),
        }
    }

    /// 封禁一个 IP/CIDR。解析失败返回 false。
    pub fn ban(&self, ip_or_cidr: &str, meta: BanMetadata) -> bool {
        match parse_network(ip_or_cidr) {
            Some(net) => {
                self.inner.write().insert(net, meta);
                true
            }
            None => false,
        }
    }

    /// 解封指定 IP/CIDR（按精确网络键）。返回被移除的元数据。
    pub fn unban(&self, ip_or_cidr: &str) -> Option<BanMetadata> {
        let net = parse_network(ip_or_cidr)?;
        self.inner.write().remove(net)
    }

    /// 查询某 IP 是否被封（最长前缀匹配），返回封禁元数据副本。
    pub fn get(&self, ip: IpAddr) -> Option<BanMetadata> {
        self.inner
            .read()
            .longest_match(ip)
            .map(|(_net, meta)| meta.clone())
    }

    pub fn contains(&self, ip: IpAddr) -> bool {
        self.inner.read().longest_match(ip).is_some()
    }

    pub fn len(&self) -> usize {
        let (v4, v6) = self.inner.read().len();
        v4 + v6
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// 移除所有 `unban_at <= now_ms` 的封禁，返回被移除的元数据（供解封事件/下发）。
    pub fn remove_expired(&self, now_ms: i64) -> Vec<BanMetadata> {
        let mut guard = self.inner.write();
        let expired: Vec<IpNetwork> = guard
            .iter()
            .filter(|(_net, meta)| meta.unban_at <= now_ms)
            .map(|(net, _)| net)
            .collect();
        let mut removed = Vec::with_capacity(expired.len());
        for net in expired {
            if let Some(meta) = guard.remove(net) {
                removed.push(meta);
            }
        }
        removed
    }

    /// 给定网络块，返回块内任意一条「有效封禁」的网络地址（排除 `ban_for_disconnect`）。供 AutoRangeBan。
    ///
    /// 语义：当前 peer 所在的 /N 段内，是否已有别的 IP 被规则封禁。
    pub fn any_active_ban_in(&self, block: IpNetwork) -> Option<IpAddr> {
        let guard = self.inner.read();
        for (net, meta) in guard.iter() {
            if meta.ban_for_disconnect {
                continue;
            }
            let addr = net.network_address();
            if block.contains(addr) {
                return Some(addr);
            }
        }
        None
    }

    /// 全量快照 `(网络字符串, 元数据)`（供持久化与 AutoRangeBan 遍历）。
    pub fn snapshot(&self) -> Vec<(String, BanMetadata)> {
        self.inner
            .read()
            .iter()
            .map(|(net, meta)| (net.to_string(), meta.clone()))
            .collect()
    }
}

/// 解析 CIDR 或裸 IP（裸 IP → /32 或 /128）。
fn parse_network(s: &str) -> Option<IpNetwork> {
    if let Ok(net) = IpNetwork::from_str(s) {
        return Some(net);
    }
    match IpAddr::from_str(s.trim()) {
        Ok(IpAddr::V4(v4)) => IpNetwork::new(v4, 32).ok(),
        Ok(IpAddr::V6(v6)) => IpNetwork::new(v6, 128).ok(),
        Err(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pbh_domain::PeerAddress;

    fn meta(unban_at: i64, for_disconnect: bool) -> BanMetadata {
        BanMetadata {
            context: "test".into(),
            random_id: "id".into(),
            peer: PeerAddress::new("1.2.3.4".parse().unwrap(), 6881),
            ban_at: 0,
            unban_at,
            ban_for_disconnect: for_disconnect,
            exclude_from_report: false,
            exclude_from_display: false,
            rule: String::new(),
            description: String::new(),
        }
    }

    #[test]
    fn ban_and_lookup_single_ip() {
        let bl = BanList::new();
        assert!(bl.ban("1.2.3.4", meta(i64::MAX, false)));
        assert!(bl.contains("1.2.3.4".parse().unwrap()));
        assert!(!bl.contains("1.2.3.5".parse().unwrap()));
        assert_eq!(bl.len(), 1);
    }

    #[test]
    fn cidr_covers_range_longest_prefix() {
        let bl = BanList::new();
        bl.ban("10.0.0.0/8", meta(i64::MAX, false));
        bl.ban("10.1.2.0/24", meta(i64::MAX, false));
        assert!(bl.contains("10.1.2.200".parse().unwrap()));
        assert!(bl.contains("10.9.9.9".parse().unwrap()));
        assert!(!bl.contains("11.0.0.1".parse().unwrap()));
    }

    #[test]
    fn remove_expired_only() {
        let bl = BanList::new();
        bl.ban("1.1.1.1", meta(1000, false)); // 过期
        bl.ban("2.2.2.2", meta(i64::MAX, false)); // 不过期
        let removed = bl.remove_expired(5000);
        assert_eq!(removed.len(), 1);
        assert!(!bl.contains("1.1.1.1".parse().unwrap()));
        assert!(bl.contains("2.2.2.2".parse().unwrap()));
    }

    #[test]
    fn unban_and_snapshot() {
        let bl = BanList::new();
        bl.ban("3.3.3.0/24", meta(i64::MAX, true));
        let snap = bl.snapshot();
        assert_eq!(snap.len(), 1);
        assert_eq!(snap[0].0, "3.3.3.0/24");
        assert!(snap[0].1.ban_for_disconnect);
        assert!(bl.unban("3.3.3.0/24").is_some());
        assert!(bl.is_empty());
    }
}
