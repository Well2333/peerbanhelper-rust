//! pbh-config —— 配置模型、目录解析、加载与热重载。
//!
//! 对应上游 `config/**`、`configuration/**`、`resources/{config,profile}.yml`，但**只保留 v2 需要的子集**
//! （见 `memory/guidelines/01-scope-and-decisions.md`）。
//!
//! - `config.yml`：基础设施（server / persist / btn / ip-database）
//! - `profile.yml`：封禁行为（check-interval / ban-duration / ignore / module.<name>）
//!
//! 注：v2 暂不做注释保留式迁移（R4）——缺失即写默认、版本不符即按 serde 重生成。

pub mod model;
pub mod paths;

use std::sync::Arc;
use tokio::sync::watch;

pub use model::{AppConfig, ProfileConfig};
pub use paths::Paths;

/// 配置加载/IO 错误。
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("配置 IO 错误 ({path}): {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("配置解析错误 ({path}): {source}")]
    Parse {
        path: String,
        #[source]
        source: serde_yaml::Error,
    },
}

/// 两份配置的合并视图（运行期只读快照，热重载时整体替换）。
#[derive(Debug, Clone)]
pub struct Config {
    pub app: AppConfig,
    pub profile: ProfileConfig,
}

impl Config {
    /// 从 config 目录加载;文件缺失则写入默认值后再读。
    pub fn load(paths: &Paths) -> Result<Self, ConfigError> {
        let app: AppConfig = load_or_create(&paths.config_file("config.yml"))?;
        let profile: ProfileConfig = load_or_create(&paths.config_file("profile.yml"))?;
        Ok(Config { app, profile })
    }
}

/// 把 `AppConfig` 写回 `config.yml`（用于首启生成 token 后持久化）。
pub fn save_app(paths: &Paths, app: &AppConfig) -> Result<(), ConfigError> {
    let path = paths.config_file("config.yml");
    let path_str = path.display().to_string();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|source| ConfigError::Io {
            path: path_str.clone(),
            source,
        })?;
    }
    let yaml = serde_yaml::to_string(app).map_err(|source| ConfigError::Parse {
        path: path_str.clone(),
        source,
    })?;
    std::fs::write(&path, yaml).map_err(|source| ConfigError::Io {
        path: path_str,
        source,
    })
}

/// 可热重载的配置句柄：持有 `watch` 通道，组件订阅 `subscribe()` 获取变更。
#[derive(Debug, Clone)]
pub struct ConfigHandle {
    paths: Paths,
    tx: watch::Sender<Arc<Config>>,
}

impl ConfigHandle {
    /// 首次加载并建立 watch 通道。
    pub fn init(paths: Paths) -> Result<Self, ConfigError> {
        let cfg = Arc::new(Config::load(&paths)?);
        let (tx, _rx) = watch::channel(cfg);
        Ok(ConfigHandle { paths, tx })
    }

    /// 当前配置快照。
    pub fn current(&self) -> Arc<Config> {
        self.tx.borrow().clone()
    }

    /// 订阅配置变更（每次 `reload()` 成功后 receiver 会收到通知）。
    pub fn subscribe(&self) -> watch::Receiver<Arc<Config>> {
        self.tx.subscribe()
    }

    /// 重新从磁盘加载并广播。失败则保持旧配置不变。
    pub fn reload(&self) -> Result<(), ConfigError> {
        let cfg = Arc::new(Config::load(&self.paths)?);
        // send_replace 不要求有订阅者存在。
        self.tx.send_replace(cfg);
        tracing::info!("配置已热重载");
        Ok(())
    }
}

/// 读取 YAML;若文件不存在则把 `T::default()` 序列化写入再返回默认值。
fn load_or_create<T>(path: &std::path::Path) -> Result<T, ConfigError>
where
    T: serde::de::DeserializeOwned + serde::Serialize + Default,
{
    let path_str = path.display().to_string();
    if !path.exists() {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|source| ConfigError::Io {
                path: path_str.clone(),
                source,
            })?;
        }
        let default = T::default();
        let yaml = serde_yaml::to_string(&default).map_err(|source| ConfigError::Parse {
            path: path_str.clone(),
            source,
        })?;
        std::fs::write(path, yaml).map_err(|source| ConfigError::Io {
            path: path_str.clone(),
            source,
        })?;
        return Ok(default);
    }
    let raw = std::fs::read_to_string(path).map_err(|source| ConfigError::Io {
        path: path_str.clone(),
        source,
    })?;
    serde_yaml::from_str(&raw).map_err(|source| ConfigError::Parse {
        path: path_str,
        source,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_creates_defaults_then_reloads() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::with_data_dir(dir.path().to_path_buf());
        // 首次加载：文件不存在 → 写默认。
        let cfg = Config::load(&paths).unwrap();
        assert_eq!(cfg.app.server.http, 9898);
        assert_eq!(cfg.profile.check_interval, 5000);
        // 文件已生成。
        assert!(paths.config_file("config.yml").exists());
        assert!(paths.config_file("profile.yml").exists());
        // 句柄 + 重载。
        let handle = ConfigHandle::init(paths).unwrap();
        assert_eq!(handle.current().app.server.http, 9898);
        handle.reload().unwrap();
    }

    #[test]
    fn parses_overridden_values() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::with_data_dir(dir.path().to_path_buf());
        std::fs::create_dir_all(paths.config_dir()).unwrap();
        std::fs::write(
            paths.config_file("config.yml"),
            "server:\n  http: 7777\n  token: \"abc\"\n",
        )
        .unwrap();
        std::fs::write(paths.config_file("profile.yml"), "check-interval: 3000\n").unwrap();
        let cfg = Config::load(&paths).unwrap();
        assert_eq!(cfg.app.server.http, 7777);
        assert_eq!(cfg.app.server.token, "abc");
        assert_eq!(cfg.profile.check_interval, 3000);
        // 未给的字段走默认。
        assert_eq!(cfg.profile.ban_duration, 1_209_600_000);
    }
}
