//! 从 `profile.yml` 构建启用的规则模块。
//!
//! 缺省策略：未配置 section 时，默认启用 PeerId/ClientName/AntiVampire 三个低成本规则（带内置默认名单），
//! 让开箱即能拦截常见坏客户端;用户可在 profile.yml 覆盖或关闭。

use std::sync::Arc;

use pbh_config::ProfileConfig;
use pbh_rules::{AntiVampire, ClientNameBlacklist, PeerIdBlacklist, RuleFeatureModule, RuleSet};

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
pub fn build_modules(profile: &ProfileConfig, global_dur: i64) -> Vec<Arc<dyn RuleFeatureModule>> {
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

    out
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
