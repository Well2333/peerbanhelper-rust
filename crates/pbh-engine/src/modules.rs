//! 从 `profile.yml` 构建启用的规则模块。
//!
//! 缺省策略：未配置 section 时，默认启用 PeerId/ClientName/AntiVampire 三个低成本规则（带内置默认名单），
//! 让开箱即能拦截常见坏客户端;用户可在 profile.yml 覆盖或关闭。

use std::sync::Arc;

use pbh_config::ProfileConfig;
use pbh_rules::{
    AntiVampire, ClientNameBlacklist, IdleConnectionDosProtection, MultiDialingBlocker,
    PeerIdBlacklist, ProtectMode, RuleFeatureModule, RuleSet,
};
use std::collections::HashSet;

use pbh_geoip::GeoIpProvider;
use pbh_storage::Db;

use crate::{
    AutoRangeBan, BanList, IpBlackList, IpBlackRuleList, PcbConfig, ProgressCheatBlocker,
    PtrBlacklist, SubConfig,
};

/// 内置默认 PeerID 黑名单（常见离线下载/吸血客户端）。
const DEFAULT_PEER_ID: &[&str] = &[
    r#"{"method":"STARTS_WITH","content":"-XL"}"#,
    r#"{"method":"STARTS_WITH","content":"-SD"}"#,
    r#"{"method":"STARTS_WITH","content":"-XF"}"#,
    r#"{"method":"STARTS_WITH","content":"-QD"}"#,
    r#"{"method":"STARTS_WITH","content":"-BN"}"#,
    r#"{"method":"STARTS_WITH","content":"-DL"}"#,
    r#"{"method":"STARTS_WITH","content":"-dt"}"#,
    r#"{"method":"CONTAINS","content":"cacao"}"#,
];

/// 内置默认客户端名黑名单。
const DEFAULT_CLIENT_NAME: &[&str] = &[
    r#"{"method":"CONTAINS","content":"Xunlei"}"#,
    r#"{"method":"CONTAINS","content":"XL0012"}"#,
    r#"{"method":"CONTAINS","content":"QQDownload"}"#,
    r#"{"method":"CONTAINS","content":"anacrolix"}"#,
    r#"{"method":"STARTS_WITH","content":"TaiPei-Torrent"}"#,
    r#"{"method":"CONTAINS","content":"github.com/anacrolix/torrent"}"#,
];

/// 按 profile 构建模块列表。
///
/// `ban_list` 供 AutoRangeBan 只读查询同段已封 IP。
///
/// 默认启用：peer-id / client-name / anti-vampire（精确、低误伤）。
/// 默认**关闭**：auto-range-ban、multi-dialing-blocker、idle-connection-dos-protection、ptr-blacklist
/// （会扩大封禁面或需联网，按需在 profile.yml 开启）。
pub fn build_modules(
    profile: &ProfileConfig,
    global_dur: i64,
    ban_list: &Arc<BanList>,
    db: &Db,
    geoip: &Option<Arc<dyn GeoIpProvider>>,
) -> Vec<Arc<dyn RuleFeatureModule>> {
    let mut out: Vec<Arc<dyn RuleFeatureModule>> = Vec::new();

    // peer-id-blacklist（默认启用）
    if enabled(profile, "peer-id-blacklist", true) {
        let raw = string_list(profile, "peer-id-blacklist", "banned-peer-id")
            .unwrap_or_else(|| DEFAULT_PEER_ID.iter().map(|s| s.to_string()).collect());
        match RuleSet::parse(&raw) {
            Ok(rs) => out.push(Arc::new(PeerIdBlacklist::new(
                rs,
                dur(profile, "peer-id-blacklist", global_dur),
            ))),
            Err(e) => tracing::warn!("peer-id-blacklist 规则解析失败: {e}"),
        }
    }

    // client-name-blacklist（默认启用）
    if enabled(profile, "client-name-blacklist", true) {
        let raw = string_list(profile, "client-name-blacklist", "banned-client-name")
            .unwrap_or_else(|| DEFAULT_CLIENT_NAME.iter().map(|s| s.to_string()).collect());
        match RuleSet::parse(&raw) {
            Ok(rs) => out.push(Arc::new(ClientNameBlacklist::new(
                rs,
                dur(profile, "client-name-blacklist", global_dur),
            ))),
            Err(e) => tracing::warn!("client-name-blacklist 规则解析失败: {e}"),
        }
    }

    // anti-vampire（默认启用，迅雷预设开）
    if enabled(profile, "anti-vampire", true) {
        let xunlei = profile
            .module_section("anti-vampire")
            .and_then(|s| s.get("presets"))
            .and_then(|s| s.get("xunlei"))
            .and_then(|s| s.get("enabled"))
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        out.push(Arc::new(AntiVampire::new(
            dur(profile, "anti-vampire", global_dur),
            xunlei,
        )));
    }

    // auto-range-ban（默认关闭）
    if enabled(profile, "auto-range-ban", false) {
        let ipv4 = field_i64(profile, "auto-range-ban", "ipv4", 30).clamp(0, 32) as u8;
        let ipv6 = field_i64(profile, "auto-range-ban", "ipv6", 48).clamp(0, 128) as u8;
        out.push(Arc::new(AutoRangeBan::new(
            ban_list.clone(),
            ipv4,
            ipv6,
            dur(profile, "auto-range-ban", global_dur),
        )));
    }

    // multi-dialing-blocker（默认关闭）
    if enabled(profile, "multi-dialing-blocker", false) {
        let m = "multi-dialing-blocker";
        out.push(Arc::new(MultiDialingBlocker::new(
            field_i64(profile, m, "subnet-mask-length", 24).clamp(0, 32) as u8,
            field_i64(profile, m, "subnet-mask-v6-length", 56).clamp(0, 128) as u8,
            field_i64(profile, m, "tolerate-num-ipv4", 2).max(1) as usize,
            field_i64(profile, m, "tolerate-num-ipv6", 5).max(1) as usize,
            field_i64(profile, m, "cache-lifespan", 86_400).max(1) as u64,
            field_bool(profile, m, "keep-hunting", false),
            field_i64(profile, m, "keep-hunting-time", 2_592_000).max(1) as u64,
            dur(profile, m, global_dur),
        )));
    }

    // idle-connection-dos-protection（默认关闭）
    if enabled(profile, "idle-connection-dos-protection", false) {
        let m = "idle-connection-dos-protection";
        out.push(Arc::new(IdleConnectionDosProtection::new(
            dur(profile, m, global_dur),
            field_i64(profile, m, "max-allowed-idle-time", 300_000).max(0),
            field_i64(profile, m, "idle-speed-threshold", 64).max(0),
            field_f64(profile, m, "min-status-change-percentage", 0.001),
            field_bool(profile, m, "reset-on-status-change", true),
            ProtectMode::from_u8(field_i64(profile, m, "protect-mode", 0).clamp(0, 2) as u8),
        )));
    }

    // progress-cheat-blocker（默认启用——反吸血核心）
    if enabled(profile, "progress-cheat-blocker", true) {
        let m = "progress-cheat-blocker";
        let pcfg = PcbConfig {
            minimum_size: field_i64(profile, m, "minimum-size", 50_000_000),
            maximum_difference: field_f64(profile, m, "maximum-difference", 0.1),
            rewind_maximum_difference: field_f64(profile, m, "rewind-maximum-difference", 0.07),
            block_excessive: field_bool(profile, m, "block-excessive-clients", true),
            excessive_threshold: field_f64(profile, m, "excessive-threshold", 1.5),
            ipv4_prefix: field_i64(profile, m, "ipv4-prefix-length", 32).clamp(0, 32) as u8,
            ipv6_prefix: field_i64(profile, m, "ipv6-prefix-length", 56).clamp(0, 128) as u8,
            ban_duration: field_i64(profile, m, "ban-duration", 2_592_000_000),
            max_wait_duration: field_i64(profile, m, "max-wait-duration", 30_000),
            fast_pcb_test_percentage: field_f64(profile, m, "fast-pcb-test-percentage", 0.1),
            fast_pcb_test_block_duration: field_i64(
                profile,
                m,
                "fast-pcb-test-block-duration",
                15_000,
            ),
            enable_persist: field_bool(profile, m, "enable-persist", true),
            persist_duration: field_i64(profile, m, "persist-duration", 1_209_600_000),
        };
        if pcfg.enable_persist {
            out.push(ProgressCheatBlocker::with_persistence(pcfg, db.clone()));
        } else {
            out.push(Arc::new(ProgressCheatBlocker::new(pcfg)));
        }
    }

    // ip-address-blocker（IP/端口/ASN/地区/城市/网络类型黑名单，默认关闭）
    if enabled(profile, "ip-address-blocker", false) {
        let m = "ip-address-blocker";
        let ips = string_list(profile, m, "ips").unwrap_or_default();
        let ports: HashSet<u16> = int_list(profile, m, "ports")
            .into_iter()
            .filter_map(|v| u16::try_from(v).ok())
            .collect();
        let asns: HashSet<u32> = int_list(profile, m, "asns")
            .into_iter()
            .filter_map(|v| u32::try_from(v).ok())
            .collect();
        let regions: HashSet<String> = str_list(profile, m, "regions").into_iter().collect();
        let cities: Vec<String> = str_list(profile, m, "cities");
        let net_types = enabled_net_types(profile, m);
        out.push(Arc::new(IpBlackList::new(
            dur(profile, m, global_dur),
            &ips,
            ports,
            asns,
            regions,
            cities,
            net_types,
            geoip.clone(),
        )));
    }

    // ip-address-blocker-rules（IP 黑名单订阅，默认关闭）
    if enabled(profile, "ip-address-blocker-rules", false) {
        let m = "ip-address-blocker-rules";
        let subs = parse_subs(profile, m);
        let check_interval = field_i64(profile, m, "check-interval", 1_800_000);
        out.push(IpBlackRuleList::new(
            dur(profile, m, global_dur),
            subs,
            check_interval,
            db.clone(),
        ));
    }

    // ptr-blacklist（默认关闭，需联网 DNS）
    if enabled(profile, "ptr-blacklist", false) {
        let m = "ptr-blacklist";
        let raw = string_list(profile, m, "ptr-rules").unwrap_or_default();
        match RuleSet::parse(&raw) {
            Ok(rs) => out.push(Arc::new(PtrBlacklist::new(
                rs,
                dur(profile, m, global_dur),
                field_i64(profile, m, "cache-ttl", 3600).max(60) as u64,
            ))),
            Err(e) => tracing::warn!("ptr-blacklist 规则解析失败: {e}"),
        }
    }

    out
}

/// 从 `module.<name>.rules`（id → {enabled,name,url}）解析订阅列表。
fn parse_subs(profile: &ProfileConfig, module: &str) -> Vec<SubConfig> {
    let mut out = Vec::new();
    let Some(rules) = profile
        .module_section(module)
        .and_then(|s| s.get("rules"))
        .and_then(|v| v.as_mapping())
    else {
        return out;
    };
    for (k, v) in rules {
        let Some(rule_id) = k.as_str() else { continue };
        let url = v.get("url").and_then(|u| u.as_str()).unwrap_or("");
        if url.is_empty() {
            continue;
        }
        out.push(SubConfig {
            rule_id: rule_id.to_string(),
            rule_name: v
                .get("name")
                .and_then(|n| n.as_str())
                .unwrap_or(rule_id)
                .to_string(),
            url: url.to_string(),
            enabled: v.get("enabled").and_then(|e| e.as_bool()).unwrap_or(true),
        });
    }
    out
}

/// 读模块字段下的整数序列。
fn int_list(profile: &ProfileConfig, module: &str, field: &str) -> Vec<i64> {
    profile
        .module_section(module)
        .and_then(|s| s.get(field))
        .and_then(|v| v.as_sequence())
        .map(|seq| seq.iter().filter_map(|v| v.as_i64()).collect())
        .unwrap_or_default()
}

/// 读模块字段下的字符串序列。
fn str_list(profile: &ProfileConfig, module: &str, field: &str) -> Vec<String> {
    profile
        .module_section(module)
        .and_then(|s| s.get(field))
        .and_then(|v| v.as_sequence())
        .map(|seq| {
            seq.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default()
}

/// 读 `net-type` 子映射中值为 true 的键集合（中国网络类型名）。
fn enabled_net_types(profile: &ProfileConfig, module: &str) -> HashSet<String> {
    profile
        .module_section(module)
        .and_then(|s| s.get("net-type"))
        .and_then(|v| v.as_mapping())
        .map(|map| {
            map.iter()
                .filter(|(_, v)| v.as_bool().unwrap_or(false))
                .filter_map(|(k, _)| k.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default()
}

/// 读模块 section 下的整数字段（缺失/类型不符则用默认）。
fn field_i64(profile: &ProfileConfig, module: &str, field: &str, default: i64) -> i64 {
    profile
        .module_section(module)
        .and_then(|s| s.get(field))
        .and_then(|v| v.as_i64())
        .unwrap_or(default)
}

/// 读模块 section 下的浮点字段。
fn field_f64(profile: &ProfileConfig, module: &str, field: &str, default: f64) -> f64 {
    profile
        .module_section(module)
        .and_then(|s| s.get(field))
        .and_then(|v| v.as_f64())
        .unwrap_or(default)
}

/// 读模块 section 下的布尔字段。
fn field_bool(profile: &ProfileConfig, module: &str, field: &str, default: bool) -> bool {
    profile
        .module_section(module)
        .and_then(|s| s.get(field))
        .and_then(|v| v.as_bool())
        .unwrap_or(default)
}

fn enabled(profile: &ProfileConfig, name: &str, default: bool) -> bool {
    match profile.module_section(name) {
        Some(s) => s
            .get("enabled")
            .and_then(|v| v.as_bool())
            .unwrap_or(default),
        None => default,
    }
}

fn dur(profile: &ProfileConfig, name: &str, global: i64) -> i64 {
    let d = profile
        .module_section(name)
        .and_then(|s| s.get("ban-duration"))
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    if d > 0 {
        d
    } else {
        global
    }
}

/// 读取某模块 section 下的字符串列表字段。
fn string_list(profile: &ProfileConfig, module: &str, field: &str) -> Option<Vec<String>> {
    let seq = profile.module_section(module)?.get(field)?.as_sequence()?;
    Some(
        seq.iter()
            .filter_map(|v| match v {
                serde_yaml::Value::String(s) => Some(s.clone()),
                // 允许内联对象规则（序列化回 JSON 串供 RuleSet::parse）。
                other => serde_json::to_string(other).ok(),
            })
            .collect(),
    )
}
