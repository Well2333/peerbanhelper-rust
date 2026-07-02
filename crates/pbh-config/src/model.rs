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
    pub network: NetworkConfig,
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

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "kebab-case")]
pub struct NetworkConfig {
    /// 出站代理 URL(http/https/socks5);空字符串表示直连。
    pub proxy: String,
    /// 分类代理开关:仅对勾选的出站类别启用代理。默认只勾选易被墙/审查的境外目标。
    pub proxy_targets: ProxyTargets,
}

/// 出站请求的代理分类开关。默认:境外/易被墙目标(BTN/GeoIP/订阅/更新)走代理;
/// 境内目标(本机公网 IP 探测)直连。下载器连接恒不走代理(本地,不可配)。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct ProxyTargets {
    /// BTN 云端威胁情报(github/ghostchu 系,易被墙)。
    pub btn: bool,
    /// GeoIP 库镜像下载(含 github)。
    pub geoip: bool,
    /// IP 规则订阅下载(多为境外源)。
    pub rule_subscription: bool,
    /// 检查更新 / 一键自更新(api.github.com)。
    pub update: bool,
    /// 本机公网 IP 探测(默认用境内服务,通常直连更快更准)。
    pub public_ip: bool,
}

impl Default for ProxyTargets {
    fn default() -> Self {
        ProxyTargets {
            btn: true,
            geoip: true,
            rule_subscription: true,
            update: true,
            public_ip: false,
        }
    }
}

/// 出站类别标识,配合 [`NetworkConfig::proxy_for`] 决定该类别是否走代理。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProxyTarget {
    Btn,
    Geoip,
    RuleSubscription,
    Update,
    PublicIp,
}

impl NetworkConfig {
    /// 返回某类别应使用的代理字符串:该类别已勾选且 `proxy` 非空 → `proxy`;否则空串(直连)。
    /// 各联网点统一调用它取 proxy 再交给 `pbh_net::build_client`,即可实现"分类走代理"。
    pub fn proxy_for(&self, target: ProxyTarget) -> &str {
        let on = match target {
            ProxyTarget::Btn => self.proxy_targets.btn,
            ProxyTarget::Geoip => self.proxy_targets.geoip,
            ProxyTarget::RuleSubscription => self.proxy_targets.rule_subscription,
            ProxyTarget::Update => self.proxy_targets.update,
            ProxyTarget::PublicIp => self.proxy_targets.public_ip,
        };
        if on {
            &self.proxy
        } else {
            ""
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

    #[test]
    fn network_proxy_roundtrips() {
        let mut a = AppConfig::default();
        assert_eq!(a.network.proxy, "");
        a.network.proxy = "http://127.0.0.1:7890".into();
        let y = serde_yaml::to_string(&a).unwrap();
        assert!(y.contains("network:"));
        assert!(y.contains("proxy: http://127.0.0.1:7890"));
        let back: AppConfig = serde_yaml::from_str(&y).unwrap();
        assert_eq!(back.network.proxy, "http://127.0.0.1:7890");
    }

    #[test]
    fn proxy_targets_default_only_censored() {
        let t = ProxyTargets::default();
        // 默认:境外/易被墙目标走代理,境内公网 IP 探测直连。
        assert!(t.btn && t.geoip && t.rule_subscription && t.update);
        assert!(!t.public_ip);
    }

    #[test]
    fn proxy_for_gates_by_target() {
        let mut n = NetworkConfig {
            proxy: "http://127.0.0.1:7890".into(),
            ..Default::default()
        };
        // 默认勾选的类别拿到代理,未勾选的(public_ip)拿到空串(直连)。
        assert_eq!(n.proxy_for(ProxyTarget::Btn), "http://127.0.0.1:7890");
        assert_eq!(n.proxy_for(ProxyTarget::Update), "http://127.0.0.1:7890");
        assert_eq!(n.proxy_for(ProxyTarget::PublicIp), "");
        // 关掉 btn 后该类别直连,其它不受影响。
        n.proxy_targets.btn = false;
        assert_eq!(n.proxy_for(ProxyTarget::Btn), "");
        assert_eq!(n.proxy_for(ProxyTarget::Geoip), "http://127.0.0.1:7890");
        // proxy 为空时,任何类别都直连。
        n.proxy.clear();
        n.proxy_targets.btn = true;
        assert_eq!(n.proxy_for(ProxyTarget::Btn), "");
    }

    #[test]
    fn proxy_targets_roundtrip_kebab() {
        let mut a = AppConfig::default();
        a.network.proxy_targets.public_ip = true;
        a.network.proxy_targets.btn = false;
        let y = serde_yaml::to_string(&a).unwrap();
        assert!(y.contains("proxy-targets:"));
        assert!(y.contains("public-ip: true"));
        let back: AppConfig = serde_yaml::from_str(&y).unwrap();
        assert!(back.network.proxy_targets.public_ip);
        assert!(!back.network.proxy_targets.btn);
        assert!(back.network.proxy_targets.geoip); // 缺省字段回退默认(true)
    }
}
