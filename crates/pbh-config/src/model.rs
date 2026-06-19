//! 配置数据模型（v2 子集）。YAML 用 kebab-case 键。
//!
//! 自定义默认值：每个结构实现 `Default`，并在容器上加 `#[serde(default)]`，
//! 这样任何缺失字段都回退到该类型 `Default` 的对应字段。

use serde::{Deserialize, Serialize};

// ---------------- config.yml ----------------

/// 基础设施配置（`config.yml`）。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "kebab-case")]
pub struct AppConfig {
    pub server: ServerConfig,
    pub persist: PersistConfig,
    pub btn: BtnConfig,
    pub ip_database: IpDatabaseConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct ServerConfig {
    /// HTTP 端口。
    pub http: u16,
    /// 监听地址。
    pub address: String,
    /// API token;空字符串表示首启自动生成。
    pub token: String,
}

impl Default for ServerConfig {
    fn default() -> Self {
        ServerConfig {
            http: 9898,
            address: "0.0.0.0".into(),
            token: String::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct PersistConfig {
    /// 封禁历史保留天数。
    pub ban_logs_keep_days: i64,
}

impl Default for PersistConfig {
    fn default() -> Self {
        PersistConfig {
            ban_logs_keep_days: 180,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct BtnConfig {
    pub enabled: bool,
    pub config_url: String,
    pub submit: bool,
    pub app_id: String,
    pub app_secret: String,
}

impl Default for BtnConfig {
    fn default() -> Self {
        BtnConfig {
            enabled: false,
            config_url: "https://sparkle.ghostchu.com/ping/config".into(),
            submit: true,
            app_id: String::new(),
            app_secret: String::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct IpDatabaseConfig {
    pub account_id: String,
    pub license_key: String,
    pub auto_update: bool,
}

impl Default for IpDatabaseConfig {
    fn default() -> Self {
        IpDatabaseConfig {
            account_id: String::new(),
            license_key: String::new(),
            auto_update: true,
        }
    }
}

// ---------------- profile.yml ----------------

/// 封禁行为配置（`profile.yml`）。
///
/// `module` 暂以原始 YAML 映射保存，各模块在自身里程碑解析对应 section（避免在此耦合所有模块字段）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct ProfileConfig {
    /// ban wave 间隔（毫秒）。
    pub check_interval: i64,
    /// 全局默认封禁时长（毫秒）。
    pub ban_duration: i64,
    /// 旁路 CIDR 列表（这些地址来的 peer 不检查）。
    pub ignore_peers_from_addresses: Vec<String>,
    /// 各模块配置原样保存：`module.<configName>.*`。
    pub module: serde_yaml::Mapping,
}

impl Default for ProfileConfig {
    fn default() -> Self {
        ProfileConfig {
            check_interval: 5000,
            ban_duration: 1_209_600_000, // 14 天
            ignore_peers_from_addresses: Vec::new(),
            module: serde_yaml::Mapping::new(),
        }
    }
}

impl ProfileConfig {
    /// 取某模块的配置 section（若存在）。
    pub fn module_section(&self, config_name: &str) -> Option<&serde_yaml::Value> {
        self.module.get(serde_yaml::Value::from(config_name))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_sane() {
        let a = AppConfig::default();
        assert_eq!(a.server.http, 9898);
        assert_eq!(a.server.address, "0.0.0.0");
        assert!(a.btn.submit);
        assert!(!a.btn.enabled);
        assert!(a.ip_database.auto_update);

        let p = ProfileConfig::default();
        assert_eq!(p.check_interval, 5000);
        assert_eq!(p.ban_duration, 1_209_600_000);
    }

    #[test]
    fn yaml_roundtrip_kebab_case() {
        let a = AppConfig::default();
        let y = serde_yaml::to_string(&a).unwrap();
        assert!(y.contains("ip-database"));
        assert!(y.contains("ban-logs-keep-days"));
        let back: AppConfig = serde_yaml::from_str(&y).unwrap();
        assert_eq!(back.server.http, a.server.http);
    }

    #[test]
    fn module_section_lookup() {
        let y =
            "module:\n  progress-cheat-blocker:\n    enabled: true\n    ban-duration: 2592000000\n";
        let p: ProfileConfig = serde_yaml::from_str(y).unwrap();
        let sec = p.module_section("progress-cheat-blocker").unwrap();
        assert_eq!(sec.get("enabled").unwrap().as_bool(), Some(true));
        assert!(p.module_section("nonexistent").is_none());
    }
}
