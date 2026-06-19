//! 统一错误类型。M1 起改用 `thiserror` 派生（见 workspace.dependencies）。

use std::fmt;

/// 全局结果别名。
pub type Result<T> = std::result::Result<T, PbhError>;

/// PeerBanHelper 顶层错误。骨架阶段为手写枚举；后续用 `thiserror`。
#[derive(Debug)]
pub enum PbhError {
    /// 配置加载 / 校验错误。
    Config(String),
    /// 下载器交互错误（登录 / 拉取 / 封禁）。
    Downloader(String),
    /// 存储层（SQLite）错误。
    Storage(String),
    /// 网络 / HTTP 错误（含 BTN）。
    Network(String),
    /// 其它。
    Other(String),
}

impl fmt::Display for PbhError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PbhError::Config(m) => write!(f, "config error: {m}"),
            PbhError::Downloader(m) => write!(f, "downloader error: {m}"),
            PbhError::Storage(m) => write!(f, "storage error: {m}"),
            PbhError::Network(m) => write!(f, "network error: {m}"),
            PbhError::Other(m) => write!(f, "{m}"),
        }
    }
}

impl std::error::Error for PbhError {}
