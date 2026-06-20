//! IPBlackRuleList —— IP 黑名单订阅。对应上游 `module/impl/rule/IPBlackRuleList.java`。
//!
//! 从 URL 下载社区封禁名单,解析为 CIDR 前缀 trie,命中 peer IP 即封。
//! 订阅来自 `profile.yml` 的 `module.ip-address-blocker-rules.rules` 配置;
//! 状态（最后更新/条数/日志）落 `rule_sub_info` / `rule_sub_log` 两表供 Web 展示。
//!
//! 支持格式:eMule DAT（`起始IP , 结束IP , 等级 , 名称`,等级 ≥128 丢弃）、CIDR、纯 IP，
//! 行内注释 `#` 或 `//`。下载失败时保留上一次的内存匹配器。

use std::net::Ipv4Addr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use parking_lot::RwLock;
use pbh_domain::{CheckResult, Peer, PeerAction, Torrent};
use pbh_rules::{IpMatcher, RuleFeatureModule};
use pbh_storage::{Db, RuleSubInfo};

/// 单条订阅配置（来自 profile.yml）。
#[derive(Debug, Clone)]
pub struct SubConfig {
    pub rule_id: String,
    pub rule_name: String,
    pub url: String,
    pub enabled: bool,
}

struct SubMatcher {
    rule_id: String,
    rule_name: String,
    matcher: IpMatcher<()>,
}

/// IP 黑名单订阅模块。
pub struct IpBlackRuleList {
    ban_duration: i64,
    matchers: Arc<RwLock<Vec<SubMatcher>>>,
    shutdown: Arc<AtomicBool>,
}

impl IpBlackRuleList {
    /// 构造并启动后台刷新任务（立即一次 + 每 `check_interval_ms`）。
    /// `proxy` 为空字符串时不使用代理，与 `pbh_net::build_client` 语义相同。
    pub fn new(
        ban_duration: i64,
        subs: Vec<SubConfig>,
        check_interval_ms: i64,
        db: Db,
        proxy: &str,
    ) -> Arc<Self> {
        let matchers: Arc<RwLock<Vec<SubMatcher>>> = Arc::new(RwLock::new(Vec::new()));
        let shutdown = Arc::new(AtomicBool::new(false));
        let me = Arc::new(IpBlackRuleList {
            ban_duration,
            matchers: matchers.clone(),
            shutdown: shutdown.clone(),
        });
        let http = pbh_net::build_client(proxy, Duration::from_secs(45));
        let interval_secs = (check_interval_ms / 1000).clamp(60, 86_400) as u64;
        tokio::spawn(async move {
            loop {
                if shutdown.load(Ordering::Relaxed) {
                    return;
                }
                refresh_all(&http, &subs, &matchers, &db, "AUTO").await;
                for _ in 0..interval_secs {
                    if shutdown.load(Ordering::Relaxed) {
                        return;
                    }
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
            }
        });
        me
    }
}

impl Drop for IpBlackRuleList {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
    }
}

impl RuleFeatureModule for IpBlackRuleList {
    fn name(&self) -> &'static str {
        "IPBlackRuleList"
    }
    fn config_name(&self) -> &'static str {
        "ip-address-blocker-rules"
    }
    fn should_ban(&self, _torrent: &Torrent, peer: &Peer) -> CheckResult {
        let ip = peer.address.ip;
        let guard = self.matchers.read();
        for sm in guard.iter() {
            if sm.matcher.contains(ip) {
                return CheckResult {
                    module: self.name(),
                    action: PeerAction::Ban,
                    duration_ms: self.ban_duration,
                    rule: format!("ipbl:{}", sm.rule_id),
                    reason: format!("命中 IP 黑名单订阅「{}」", sm.rule_name),
                };
            }
        }
        CheckResult::pass(self.name())
    }
}

/// 刷新所有启用订阅。下载失败时保留旧匹配器。
async fn refresh_all(
    http: &reqwest::Client,
    subs: &[SubConfig],
    matchers: &Arc<RwLock<Vec<SubMatcher>>>,
    db: &Db,
    update_type: &str,
) {
    // 取出旧匹配器以便失败时保留。
    let mut old: Vec<SubMatcher> = std::mem::take(&mut *matchers.write());
    let mut next: Vec<SubMatcher> = Vec::new();
    for sub in subs.iter().filter(|s| s.enabled) {
        match download_and_parse(http, &sub.url).await {
            Ok(cidrs) => {
                let mut m = IpMatcher::new();
                let mut cnt: i64 = 0;
                for c in &cidrs {
                    if m.insert(c, ()) {
                        cnt += 1;
                    }
                }
                let now = now_ms();
                let _ = db
                    .upsert_rule_sub(&RuleSubInfo {
                        rule_id: sub.rule_id.clone(),
                        enabled: true,
                        rule_name: sub.rule_name.clone(),
                        sub_url: sub.url.clone(),
                        last_update: Some(now),
                        ent_count: Some(cnt),
                    })
                    .await;
                let _ = db
                    .insert_rule_sub_log(&sub.rule_id, now, cnt, update_type)
                    .await;
                tracing::info!("IP 订阅「{}」更新: {cnt} 条", sub.rule_name);
                next.push(SubMatcher {
                    rule_id: sub.rule_id.clone(),
                    rule_name: sub.rule_name.clone(),
                    matcher: m,
                });
            }
            Err(e) => {
                tracing::warn!("IP 订阅「{}」下载失败: {e}", sub.rule_name);
                // 保留旧匹配器（若有）。
                if let Some(pos) = old.iter().position(|sm| sm.rule_id == sub.rule_id) {
                    next.push(old.remove(pos));
                }
            }
        }
    }
    *matchers.write() = next;
}

async fn download_and_parse(
    http: &reqwest::Client,
    url: &str,
) -> std::result::Result<Vec<String>, reqwest::Error> {
    let text = http
        .get(url)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    Ok(parse_rule_list(&text))
}

fn now_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// 去掉行内注释（`#` 或 `//`，取最先出现者）。
fn strip_inline_comment(line: &str) -> &str {
    let hash = line.find('#');
    let slash = line.find("//");
    let cut = match (hash, slash) {
        (Some(h), Some(s)) => Some(h.min(s)),
        (Some(h), None) => Some(h),
        (None, Some(s)) => Some(s),
        (None, None) => None,
    };
    match cut {
        Some(i) => &line[..i],
        None => line,
    }
}

/// 解析 eMule DAT 风格的零填充 IPv4（`016.000.000.000`）。std 解析器拒绝前导零,故手工解析。
fn parse_padded_ipv4(s: &str) -> Option<Ipv4Addr> {
    let parts: Vec<&str> = s.trim().split('.').collect();
    if parts.len() != 4 {
        return None;
    }
    let mut octets = [0u8; 4];
    for (i, p) in parts.iter().enumerate() {
        octets[i] = p.parse().ok()?;
    }
    Some(Ipv4Addr::new(octets[0], octets[1], octets[2], octets[3]))
}

/// 把一段订阅文本解析成 CIDR/IP 字符串列表。
pub fn parse_rule_list(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with("//") {
            continue;
        }
        let line = strip_inline_comment(line).trim();
        if line.is_empty() {
            continue;
        }
        if line.contains(',') {
            // eMule DAT: 起始 , 结束 , 等级 , 名称
            let parts: Vec<&str> = line.split(',').map(|s| s.trim()).collect();
            if parts.len() < 3 {
                continue;
            }
            let level: i64 = parts[2].parse().unwrap_or(0);
            if level >= 128 {
                continue; // eMule 等级 ≥128 视为低可信,丢弃
            }
            if let (Some(start), Some(end)) =
                (parse_padded_ipv4(parts[0]), parse_padded_ipv4(parts[1]))
            {
                if start <= end {
                    for net in ipnet::Ipv4Subnets::new(start, end, 0) {
                        out.push(net.to_string());
                    }
                }
            }
        } else {
            // CIDR 或纯 IP（IpMatcher::insert 负责校验）。
            out.push(line.to_string());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_plain_and_cidr_with_comments() {
        let text = "# 头部注释\n1.2.3.4\n10.0.0.0/8 # 内网\n203.0.113.0/24 // CDN\n\n// 空注释行\n";
        let out = parse_rule_list(text);
        assert_eq!(out, vec!["1.2.3.4", "10.0.0.0/8", "203.0.113.0/24"]);
    }

    #[test]
    fn parse_emule_dat_range_to_cidr() {
        // 192.168.1.0 - 192.168.1.255 → /24;等级 200 ≥128 的行丢弃。
        let text = "192.168.001.000 , 192.168.001.255 , 100 , Some Org\n\
                    016.000.000.000 , 016.255.255.255 , 200 , Dropped (level>=128)\n";
        let out = parse_rule_list(text);
        assert_eq!(out, vec!["192.168.1.0/24"]);
    }

    #[test]
    fn matcher_built_from_parsed() {
        let out = parse_rule_list("1.2.3.0/24\n9.9.9.9\n");
        let mut m: IpMatcher<()> = IpMatcher::new();
        let cnt = out.iter().filter(|c| m.insert(c, ())).count();
        assert_eq!(cnt, 2);
        assert!(m.contains("1.2.3.55".parse().unwrap()));
        assert!(m.contains("9.9.9.9".parse().unwrap()));
        assert!(!m.contains("8.8.8.8".parse().unwrap()));
    }

    #[test]
    fn emule_multi_block_range() {
        // 不对齐的范围会拆成多个 CIDR。
        let out = parse_rule_list("1.0.0.0 , 1.0.0.3 , 50 , x\n");
        // 1.0.0.0-1.0.0.3 → 1.0.0.0/30
        assert_eq!(out, vec!["1.0.0.0/30"]);
    }
}
