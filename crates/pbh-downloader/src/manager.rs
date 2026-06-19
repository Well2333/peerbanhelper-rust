//! 下载器管理器：持久化配置（YAML）+ 维护运行期下载器列表。对应上游 `DownloaderManagerImpl`。

use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use crate::{build_downloader, Downloader, DownloaderConfig, DownloaderError, Result};

/// 下载器管理器。配置存于 `<data>/config/downloaders.yml`（YAML 列表）。
pub struct DownloaderManager {
    path: PathBuf,
    items: RwLock<Vec<Arc<dyn Downloader>>>,
    configs: RwLock<Vec<DownloaderConfig>>,
}

impl DownloaderManager {
    /// 从文件加载（缺失则空）。逐条构建，单条失败仅记日志、不影响其余。
    pub fn load(path: PathBuf) -> Self {
        let configs: Vec<DownloaderConfig> = std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_yaml::from_str(&s).ok())
            .unwrap_or_default();
        let mut items = Vec::new();
        for cfg in &configs {
            match build_downloader(cfg.clone()) {
                Ok(d) => items.push(d),
                Err(e) => tracing::warn!(id = %cfg.id, "构建下载器失败: {e}"),
            }
        }
        DownloaderManager {
            path,
            items: RwLock::new(items),
            configs: RwLock::new(configs),
        }
    }

    fn persist(&self) -> Result<()> {
        let configs = self.configs.read().unwrap();
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| DownloaderError::Config(format!("建目录失败: {e}")))?;
        }
        let yaml = serde_yaml::to_string(&*configs)
            .map_err(|e| DownloaderError::Config(format!("序列化失败: {e}")))?;
        std::fs::write(&self.path, yaml)
            .map_err(|e| DownloaderError::Config(format!("写入失败: {e}")))?;
        Ok(())
    }

    /// 当前下载器（运行期对象）。
    pub fn list(&self) -> Vec<Arc<dyn Downloader>> {
        self.items.read().unwrap().clone()
    }

    /// 当前配置副本（供 Web 展示;调用方负责脱敏密码）。
    pub fn configs(&self) -> Vec<DownloaderConfig> {
        self.configs.read().unwrap().clone()
    }

    pub fn count(&self) -> usize {
        self.items.read().unwrap().len()
    }

    /// 新增或更新（按 id）。构建成功后持久化。
    pub fn upsert(&self, config: DownloaderConfig) -> Result<()> {
        let downloader = build_downloader(config.clone())?;
        {
            let mut configs = self.configs.write().unwrap();
            let mut items = self.items.write().unwrap();
            if let Some(pos) = configs.iter().position(|c| c.id == config.id) {
                configs[pos] = config;
                items[pos] = downloader;
            } else {
                configs.push(config);
                items.push(downloader);
            }
        }
        self.persist()
    }

    /// 删除（按 id）。
    pub fn remove(&self, id: &str) -> Result<bool> {
        let removed = {
            let mut configs = self.configs.write().unwrap();
            let mut items = self.items.write().unwrap();
            if let Some(pos) = configs.iter().position(|c| c.id == id) {
                configs.remove(pos);
                items.remove(pos);
                true
            } else {
                false
            }
        };
        if removed {
            self.persist()?;
        }
        Ok(removed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_upsert_remove_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("downloaders.yml");
        let mgr = DownloaderManager::load(path.clone());
        assert_eq!(mgr.count(), 0);

        let cfg = DownloaderConfig {
            id: "d1".into(),
            kind: "qbittorrent".into(),
            name: "qb".into(),
            endpoint: "http://127.0.0.1:8080".into(),
            ..Default::default()
        };
        mgr.upsert(cfg).unwrap();
        assert_eq!(mgr.count(), 1);
        assert!(path.exists());

        // 重新加载持久化的配置。
        let mgr2 = DownloaderManager::load(path.clone());
        assert_eq!(mgr2.count(), 1);
        assert_eq!(mgr2.configs()[0].name, "qb");

        assert!(mgr2.remove("d1").unwrap());
        assert_eq!(mgr2.count(), 0);
        assert!(!mgr2.remove("nope").unwrap());
    }
}
