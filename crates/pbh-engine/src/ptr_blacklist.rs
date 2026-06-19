//! PTRBlacklist —— PTR 反向 DNS 黑名单。对应上游 `module/impl/rule/PTRBlacklist.java`。
//!
//! 对 peer IP 做反向 DNS（PTR）查询，命中规则（STARTS_WITH/ENDS_WITH/CONTAINS/EQUALS/REGEX/LENGTH）即封禁。
//!
//! DNS 是网络 I/O，无法在同步 `should_ban` 里直接阻塞。采用「后台预解析 + 同步查缓存」：
//! 首次见到某 IP → 派发后台异步解析、本轮放行；解析结果入 moka 缓存，下一轮 wave 命中缓存再判定。
//! 由于封禁是持续的 ban wave，延后一轮判定可接受。

use std::collections::HashSet;
use std::net::IpAddr;
use std::sync::Arc;
use std::time::Duration;

use hickory_resolver::config::{ResolverConfig, ResolverOpts};
use hickory_resolver::TokioAsyncResolver;
use moka::sync::Cache;
use parking_lot::Mutex;
use pbh_domain::{CheckResult, Peer, PeerAction, Torrent};
use pbh_rules::{MatchOutcome, RuleFeatureModule, RuleSet};

/// PTR 反向 DNS 黑名单模块。
pub struct PtrBlacklist {
    rules: Arc<RuleSet>,
    ban_duration: i64,
    resolver: Arc<TokioAsyncResolver>,
    /// IP → PTR 解析结果（`Some(name)` 有 PTR；`None` 已解析但无 PTR）。
    cache: Cache<IpAddr, Option<Arc<str>>>,
    /// 正在解析中的 IP，去重避免重复派发。
    inflight: Arc<Mutex<HashSet<IpAddr>>>,
}

impl PtrBlacklist {
    pub fn new(rules: RuleSet, ban_duration: i64, ttl_secs: u64) -> Self {
        let resolver = match TokioAsyncResolver::tokio_from_system_conf() {
            Ok(r) => r,
            Err(_) => TokioAsyncResolver::tokio(ResolverConfig::default(), ResolverOpts::default()),
        };
        PtrBlacklist {
            rules: Arc::new(rules),
            ban_duration,
            resolver: Arc::new(resolver),
            cache: Cache::builder()
                .max_capacity(16_384)
                .time_to_live(Duration::from_secs(ttl_secs.max(60)))
                .build(),
            inflight: Arc::new(Mutex::new(HashSet::new())),
        }
    }

    /// 对已解析的 PTR 做规则判定（纯逻辑，便于测试）。
    fn judge(&self, ptr: Option<&str>) -> CheckResult {
        match ptr {
            Some(name) if self.rules.match_input(name) == MatchOutcome::True => CheckResult {
                module: self.name(),
                action: PeerAction::Ban,
                duration_ms: self.ban_duration,
                rule: "ptr-blacklist".into(),
                reason: format!("PTR {name} 命中黑名单"),
            },
            _ => CheckResult::pass(self.name()),
        }
    }

    /// 派发后台解析（去重）。结果入缓存供下一轮判定。
    fn spawn_resolve(&self, ip: IpAddr) {
        {
            let mut g = self.inflight.lock();
            if !g.insert(ip) {
                return; // 已在解析中
            }
        }
        let resolver = self.resolver.clone();
        let cache = self.cache.clone();
        let inflight = self.inflight.clone();
        tokio::spawn(async move {
            let ptr: Option<Arc<str>> = match resolver.reverse_lookup(ip).await {
                Ok(lookup) => lookup
                    .iter()
                    .next()
                    .map(|name| Arc::from(name.to_string().trim_end_matches('.'))),
                Err(_) => None,
            };
            cache.insert(ip, ptr);
            inflight.lock().remove(&ip);
        });
    }
}

impl RuleFeatureModule for PtrBlacklist {
    fn name(&self) -> &'static str {
        "PTRBlacklist"
    }
    fn config_name(&self) -> &'static str {
        "ptr-blacklist"
    }
    fn should_ban(&self, _torrent: &Torrent, peer: &Peer) -> CheckResult {
        if peer.is_handshaking() {
            return CheckResult::pass(self.name());
        }
        let ip = peer.address.ip;
        match self.cache.get(&ip) {
            Some(resolved) => self.judge(resolved.as_deref()),
            None => {
                // 尚未解析 → 后台解析，本轮放行。
                self.spawn_resolve(ip);
                CheckResult::pass(self.name())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pbh_domain::PeerAddress;
    use pbh_rules::{Matcher, StringRule};

    fn module() -> PtrBlacklist {
        // 规则:PTR 含 "datacenter" 或以 ".cn" 结尾。
        let rs = RuleSet::new(vec![
            StringRule::new(Matcher::Contains("datacenter".into())),
            StringRule::new(Matcher::EndsWith(".cn".into())),
        ]);
        // new() 会建 resolver——在 tokio 运行时内构造。
        PtrBlacklist::new(rs, 1000, 3600)
    }

    #[tokio::test]
    async fn judge_matches_and_passes() {
        let m = module();
        assert_eq!(
            m.judge(Some("host.datacenter.example")).action,
            PeerAction::Ban
        );
        assert_eq!(m.judge(Some("foo.bar.cn")).action, PeerAction::Ban);
        assert_eq!(
            m.judge(Some("clean.example.com")).action,
            PeerAction::NoAction
        );
        assert_eq!(m.judge(None).action, PeerAction::NoAction);
    }

    #[tokio::test]
    async fn first_sight_passes_and_schedules() {
        let m = module();
        let peer = Peer {
            address: PeerAddress::new("1.2.3.4".parse().unwrap(), 6881),
            peer_id: None,
            client_name: None,
            download_speed: 1,
            upload_speed: 1,
            downloaded: 0,
            uploaded: 0,
            progress: 0.0,
            flags: None,
        };
        let t = Torrent {
            id: "h".into(),
            hash: "h".into(),
            name: "t".into(),
            progress: 0.0,
            size: 1,
            completed_size: -1,
            private_torrent: false,
        };
        // 首次未解析 → 放行（并在后台派发解析,不阻塞）。
        assert_eq!(m.should_ban(&t, &peer).action, PeerAction::NoAction);
        // 命中后预解析,直接写缓存验证 judge 路径。
        m.cache.insert(
            "1.2.3.4".parse().unwrap(),
            Some(Arc::from("x.datacenter.net")),
        );
        assert_eq!(m.should_ban(&t, &peer).action, PeerAction::Ban);
    }
}
