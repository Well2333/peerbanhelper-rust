//! BTN 协议序列化模型。对应上游 `btn/**` 的 `@SerializedName` 字段。

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// config 端点响应。
#[derive(Debug, Clone, Deserialize)]
pub struct BtnConfigResponse {
    pub min_protocol_version: Option<u32>,
    pub max_protocol_version: Option<u32>,
    #[serde(default)]
    pub ability: HashMap<String, AbilityConfig>,
    pub proof_of_work_captcha: Option<PowEndpoint>,
}

impl BtnConfigResponse {
    /// 是否走 legacy 分支（`min_protocol_version < 20`）。
    pub fn is_legacy(&self) -> bool {
        self.min_protocol_version.unwrap_or(20) < 20
    }
}

/// 单个 ability 的配置。
#[derive(Debug, Clone, Deserialize)]
pub struct AbilityConfig {
    pub endpoint: Option<String>,
    pub interval: Option<i64>,
    pub random_initial_delay: Option<i64>,
    #[serde(default)]
    pub pow_captcha: bool,
    pub version: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PowEndpoint {
    pub endpoint: String,
}

/// 规则集（`rule_peer_identity` ability 响应）。各分类名 → 模式列表。
#[derive(Debug, Clone, Default, Deserialize)]
pub struct BtnRuleset {
    pub version: Option<String>,
    #[serde(default)]
    pub peer_id: HashMap<String, Vec<String>>,
    #[serde(default)]
    pub client_name: HashMap<String, Vec<String>>,
    #[serde(default)]
    pub ip: HashMap<String, Vec<String>>,
    #[serde(default)]
    pub port: HashMap<String, Vec<u32>>,
}

// ---------------- 上行 SubmitBans ----------------

/// 上报的单条封禁（对应 `BtnBan`）。
#[derive(Debug, Clone, Serialize)]
pub struct BtnBan {
    /// epoch millis。
    pub ban_at: i64,
    pub peer_ip: String,
    pub peer_port: i64,
    pub peer_id: Option<String>,
    pub peer_client_name: Option<String>,
    pub peer_progress: f64,
    pub torrent_identifier: String,
    pub torrent_size: i64,
    pub module: String,
    pub rule: String,
    pub description: String,
}

/// SubmitBans 请求体。
#[derive(Debug, Clone, Serialize)]
pub struct SubmitBansBody {
    pub bans: Vec<BtnBan>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_config_with_abilities() {
        let json = r#"{
            "min_protocol_version": 20,
            "max_protocol_version": 20,
            "ability": {
                "rule_peer_identity": {"endpoint":"https://x/rules","interval":3600000,"pow_captcha":false},
                "ip_denylist": {"endpoint":"https://x/deny","interval":3600000}
            },
            "proof_of_work_captcha": {"endpoint":"https://x/pow"}
        }"#;
        let cfg: BtnConfigResponse = serde_json::from_str(json).unwrap();
        assert!(!cfg.is_legacy());
        assert_eq!(cfg.ability.len(), 2);
        assert_eq!(
            cfg.ability
                .get("rule_peer_identity")
                .unwrap()
                .endpoint
                .as_deref(),
            Some("https://x/rules")
        );
        assert_eq!(cfg.proof_of_work_captcha.unwrap().endpoint, "https://x/pow");
    }

    #[test]
    fn legacy_detection() {
        let json = r#"{"min_protocol_version": 12, "ability": {}}"#;
        let cfg: BtnConfigResponse = serde_json::from_str(json).unwrap();
        assert!(cfg.is_legacy());
    }

    #[test]
    fn parse_ruleset() {
        let json = r#"{
            "version": "v20250101",
            "peer_id": {"bad_clients": ["-XL", "cacao"]},
            "ip": {"datacenter": ["1.2.3.0/24"]},
            "port": {"weird": [2003, 6889]}
        }"#;
        let rs: BtnRuleset = serde_json::from_str(json).unwrap();
        assert_eq!(rs.version.as_deref(), Some("v20250101"));
        assert_eq!(rs.peer_id.get("bad_clients").unwrap().len(), 2);
        assert_eq!(rs.port.get("weird").unwrap(), &vec![2003, 6889]);
    }

    #[test]
    fn submit_bans_serializes() {
        let body = SubmitBansBody {
            bans: vec![BtnBan {
                ban_at: 1_640_000_000_000,
                peer_ip: "1.2.3.4".into(),
                peer_port: 6881,
                peer_id: Some("-XL0019-".into()),
                peer_client_name: None,
                peer_progress: 0.5,
                torrent_identifier: "deadbeef".into(),
                torrent_size: 1024,
                module: "BtnNetworkOnline".into(),
                rule: "bad_clients".into(),
                description: "命中 BTN 规则".into(),
            }],
        };
        let json = serde_json::to_string(&body).unwrap();
        assert!(json.contains("\"ban_at\":1640000000000"));
        assert!(json.contains("\"peer_ip\":\"1.2.3.4\""));
        assert!(json.contains("\"torrent_identifier\":\"deadbeef\""));
    }
}
