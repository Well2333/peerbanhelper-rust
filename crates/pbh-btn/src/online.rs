//! BtnNetworkOnline 规则模块 + 共享威胁情报状态。
//! 对应上游 `module/impl/rule/BtnNetworkOnline.java`。
//!
//! 判定短路：AllowList → SKIP；DenyList → BAN；Rules（peer_id / client_name / ip / port 分类）→ BAN。
//! （上游的 script 分支已随脚本引擎移除。）
//!
//! 共享状态 `BtnState` 由调度器（下行 ability 拉取）更新,模块只读应用。

use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::RwLock;
use pbh_domain::{CheckResult, Peer, PeerAction, Torrent};
use pbh_rules::{IpMatcher, MatchOutcome, Matcher, RuleFeatureModule, RuleSet, StringRule};

use crate::model::BtnRuleset;

/// BTN 下行威胁情报编译后的匹配状态。
#[derive(Default)]
pub struct BtnState {
    pub allowlist: IpMatcher<()>,
    pub denylist: IpMatcher<()>,
    /// (分类名, 规则集)。
    pub peer_id_rules: Vec<(String, RuleSet)>,
    pub client_name_rules: Vec<(String, RuleSet)>,
    /// CIDR → 分类名。
    pub ip_rules: IpMatcher<String>,
    /// 端口 → 分类名。
    pub port_rules: HashMap<u16, String>,
}

/// BTN 在线规则模块。
pub struct BtnNetworkOnline {
    ban_duration: i64,
    state: Arc<RwLock<BtnState>>,
}

impl BtnNetworkOnline {
    pub fn new(ban_duration: i64, state: Arc<RwLock<BtnState>>) -> Self {
        BtnNetworkOnline {
            ban_duration,
            state,
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

impl RuleFeatureModule for BtnNetworkOnline {
    fn name(&self) -> &'static str {
        "BtnNetworkOnline"
    }
    fn config_name(&self) -> &'static str {
        "btn"
    }
    fn should_ban(&self, _torrent: &Torrent, peer: &Peer) -> CheckResult {
        let ip = peer.address.ip;
        let s = self.state.read();
        // 1) 白名单 → SKIP（覆盖一切封禁）。
        if s.allowlist.contains(ip) {
            return CheckResult {
                module: self.name(),
                action: PeerAction::Skip,
                duration_ms: 0,
                rule: "btn:allowlist".into(),
                reason: "BTN 白名单放行".into(),
            };
        }
        // 2) 黑名单 → BAN。
        if s.denylist.contains(ip) {
            return self.ban("btn:denylist", format!("IP {ip} 命中 BTN 黑名单"));
        }
        // 3) 规则集:peer_id。
        if let Some(pid) = peer.peer_id.as_deref() {
            for (cat, rs) in &s.peer_id_rules {
                if rs.match_input(pid) == MatchOutcome::True {
                    return self.ban(
                        &format!("btn:peer_id:{cat}"),
                        format!("PeerID 命中 BTN 规则 {cat}"),
                    );
                }
            }
        }
        // 4) 规则集:client_name。
        if let Some(name) = peer.client_name.as_deref() {
            for (cat, rs) in &s.client_name_rules {
                if rs.match_input(name) == MatchOutcome::True {
                    return self.ban(
                        &format!("btn:client_name:{cat}"),
                        format!("客户端名命中 BTN 规则 {cat}"),
                    );
                }
            }
        }
        // 5) 规则集:ip。
        if let Some(cat) = s.ip_rules.longest_match(ip) {
            return self.ban(
                &format!("btn:ip:{cat}"),
                format!("IP {ip} 命中 BTN 规则 {cat}"),
            );
        }
        // 6) 规则集:port。
        if let Some(cat) = s.port_rules.get(&peer.address.port) {
            return self.ban(
                &format!("btn:port:{cat}"),
                format!("端口 {} 命中 BTN 规则 {cat}", peer.address.port),
            );
        }
        CheckResult::pass(self.name())
    }
}

// ---------------- 状态应用（调度器拉取后调用）----------------

/// 把规则集编译进共享状态。BTN 模式串按 substring（CONTAINS）近似（上游 RuleParser 语义）。
pub fn apply_ruleset(state: &Arc<RwLock<BtnState>>, rs: &BtnRuleset) {
    let to_ruleset = |pats: &Vec<String>| -> RuleSet {
        RuleSet::new(
            pats.iter()
                .map(|p| StringRule::new(Matcher::Contains(p.clone())))
                .collect(),
        )
    };
    let peer_id_rules = rs
        .peer_id
        .iter()
        .map(|(cat, pats)| (cat.clone(), to_ruleset(pats)))
        .collect();
    let client_name_rules = rs
        .client_name
        .iter()
        .map(|(cat, pats)| (cat.clone(), to_ruleset(pats)))
        .collect();
    let mut ip_rules = IpMatcher::new();
    for (cat, cidrs) in &rs.ip {
        for c in cidrs {
            ip_rules.insert(c, cat.clone());
        }
    }
    let mut port_rules = HashMap::new();
    for (cat, ports) in &rs.port {
        for p in ports {
            if let Ok(p) = u16::try_from(*p) {
                port_rules.insert(p, cat.clone());
            }
        }
    }
    let mut g = state.write();
    g.peer_id_rules = peer_id_rules;
    g.client_name_rules = client_name_rules;
    g.ip_rules = ip_rules;
    g.port_rules = port_rules;
}

/// 解析纯文本 IP 名单（CIDR / 纯 IP；行内 `#` 注释）。BTN 名单极少用 DAT 范围,此处不处理。
fn parse_ip_lines(text: &str) -> IpMatcher<()> {
    let mut m = IpMatcher::new();
    for raw in text.lines() {
        let line = raw.split('#').next().unwrap_or("").trim();
        if line.is_empty() || line.contains(',') {
            continue;
        }
        m.insert(line, ());
    }
    m
}

pub fn apply_denylist(state: &Arc<RwLock<BtnState>>, text: &str) {
    state.write().denylist = parse_ip_lines(text);
}

pub fn apply_allowlist(state: &Arc<RwLock<BtnState>>, text: &str) {
    state.write().allowlist = parse_ip_lines(text);
}

#[cfg(test)]
mod tests {
    use super::*;
    use pbh_domain::PeerAddress;

    fn peer(ip: &str, port: u16, pid: Option<&str>) -> Peer {
        Peer {
            address: PeerAddress::new(ip.parse().unwrap(), port),
            peer_id: pid.map(String::from),
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
    fn allowlist_skips_over_denylist() {
        let state = Arc::new(RwLock::new(BtnState::default()));
        apply_denylist(&state, "1.2.3.0/24\n");
        apply_allowlist(&state, "1.2.3.4\n");
        let m = BtnNetworkOnline::new(1000, state);
        // 1.2.3.4 同时在 deny(/24) 和 allow → allow 优先 SKIP。
        assert_eq!(
            m.should_ban(&torrent(), &peer("1.2.3.4", 6881, None))
                .action,
            PeerAction::Skip
        );
        // 1.2.3.5 仅在 deny → BAN。
        assert_eq!(
            m.should_ban(&torrent(), &peer("1.2.3.5", 6881, None))
                .action,
            PeerAction::Ban
        );
    }

    #[test]
    fn ruleset_peer_id_and_port() {
        let state = Arc::new(RwLock::new(BtnState::default()));
        let rs: BtnRuleset =
            serde_json::from_str(r#"{"peer_id":{"bad":["-XL"]},"port":{"weird":[2003]}}"#).unwrap();
        apply_ruleset(&state, &rs);
        let m = BtnNetworkOnline::new(1000, state);
        assert_eq!(
            m.should_ban(&torrent(), &peer("8.8.8.8", 6881, Some("-XL0019-")))
                .action,
            PeerAction::Ban
        );
        assert_eq!(
            m.should_ban(&torrent(), &peer("8.8.8.8", 2003, None))
                .action,
            PeerAction::Ban
        );
        assert_eq!(
            m.should_ban(&torrent(), &peer("8.8.8.8", 6881, Some("-qB4250-")))
                .action,
            PeerAction::NoAction
        );
    }
}
