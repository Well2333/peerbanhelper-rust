//! pbh-config —— 配置模型与加载。对应 Java `config/**`、`configuration/**`、`resources/{config,profile}.yml`。
//!
//! 两份 YAML：
//! - `config.yml`（基础设施：server/persist/btn/ip-database/proxy/performance/privacy）
//! - `profile.yml`（封禁行为：check-interval / ban-duration / ignore / module.<name>.*）
//!
//! M0 实现：serde 模型 + 加载 + 默认值 + `tokio::sync::watch` 热重载广播 + 版本迁移链
//! （有序 `Vec<fn(&mut Value)>`）。注释保留是难点（见 docs/01 风险 R4）。
//!
//! 骨架阶段先给出关键结构占位（std-only）。

/// 基础设施配置（`config.yml` 子集占位）。M0 补全并加 serde。
#[derive(Debug, Clone)]
pub struct AppConfig {
    pub server_http: u16,
    pub server_address: String,
    pub server_token: String,
    pub allow_cors: bool,
}

impl Default for AppConfig {
    fn default() -> Self {
        AppConfig {
            server_http: 9898,
            server_address: "0.0.0.0".into(),
            server_token: String::new(),
            allow_cors: false,
        }
    }
}

/// 封禁行为配置（`profile.yml` 子集占位）。M0 补全。
#[derive(Debug, Clone)]
pub struct ProfileConfig {
    /// ban wave 间隔（毫秒）。
    pub check_interval_ms: i64,
    /// 全局默认封禁时长（毫秒）。
    pub ban_duration_ms: i64,
    /// 旁路 CIDR 列表。
    pub ignore_peers_from_addresses: Vec<String>,
}

impl Default for ProfileConfig {
    fn default() -> Self {
        ProfileConfig {
            check_interval_ms: 5000,
            ban_duration_ms: 1_209_600_000, // 14 天
            ignore_peers_from_addresses: Vec::new(),
        }
    }
}
