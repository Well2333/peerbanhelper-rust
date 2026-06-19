//! 数据目录解析。所有运行期文件都在一个数据根目录下，便于单文件部署。

use std::path::{Path, PathBuf};

/// 运行期目录布局：
/// ```text
/// <data>/
///   config/   config.yml, profile.yml
///   persist/  peerbanhelper-nt.db
///   logs/
/// ```
#[derive(Debug, Clone)]
pub struct Paths {
    data: PathBuf,
}

impl Paths {
    /// 由环境解析：`PBH_DATA_DIR` 优先，否则当前目录下的 `./data`。
    pub fn from_env() -> Self {
        let data = std::env::var_os("PBH_DATA_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("data"));
        Paths { data }
    }

    /// 指定数据根（测试/嵌入用）。
    pub fn with_data_dir(data: PathBuf) -> Self {
        Paths { data }
    }

    pub fn data_dir(&self) -> &Path {
        &self.data
    }

    pub fn config_dir(&self) -> PathBuf {
        self.data.join("config")
    }

    pub fn persist_dir(&self) -> PathBuf {
        self.data.join("persist")
    }

    pub fn logs_dir(&self) -> PathBuf {
        self.data.join("logs")
    }

    /// config 目录下的某个文件路径。
    pub fn config_file(&self, name: &str) -> PathBuf {
        self.config_dir().join(name)
    }

    /// SQLite 数据库文件绝对路径。
    pub fn db_file(&self) -> PathBuf {
        self.persist_dir().join("peerbanhelper-nt.db")
    }

    /// 创建全部运行期子目录（幂等）。
    pub fn ensure_dirs(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(self.config_dir())?;
        std::fs::create_dir_all(self.persist_dir())?;
        std::fs::create_dir_all(self.logs_dir())?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_under_data_root() {
        let p = Paths::with_data_dir(PathBuf::from("/x/data"));
        assert_eq!(
            p.config_file("config.yml"),
            PathBuf::from("/x/data/config/config.yml")
        );
        assert_eq!(
            p.db_file(),
            PathBuf::from("/x/data/persist/peerbanhelper-nt.db")
        );
    }
}
