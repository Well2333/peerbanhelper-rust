//! pbh-downloader —— 下载器抽象 + qBittorrent / qBittorrentEE。
//!
//! 对应 Java `downloader/**`。本期仅 qB + qBEE，但 **保留 trait + 工厂注册以维持可扩展性**
//! （守则第 9 条：依赖抽象）。协议细节见 `memory/design/architecture-analysis.md` §2.2，必须字节级一致。
//!
//! M2 实现：
//! - `Downloader` trait（登录/拉 torrents/拉 peers/封禁/统计/限速/端口/特性标志）
//! - `QBittorrentClient`（reqwest + cookie SID + basic-auth + verify-ssl 开关 + UA + 并发信号量 128）
//! - `BanHandler`：Normal（banned_IPs / banPeers）与 ShadowBan（shadow_banned_IPs / shadowbanPeers，EE）
//! - `DownloaderManager` 工厂：`type` 字符串 → 构造器（qbittorrent / qbittorrentee）

use pbh_domain::{Peer, Result, Torrent};

/// 下载器特性标志。对应 Java `DownloaderFeatureFlag`。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeatureFlag {
    ReadPeerProtocols,
    UnbanIp,
    TrafficStats,
    LiveUpdateBtProtocolPort,
    /// 是否支持下发 CIDR 段封禁（qB ≥ 5.3.0 等阈值）。
    RangeBanIp,
}

/// 下载器抽象。对应 Java `Downloader` 接口（裁剪到本期所需，签名将在 M2 随异步运行时定稿）。
pub trait Downloader: Send + Sync {
    fn id(&self) -> &str;
    fn name(&self) -> &str;
    /// 类型标签，如 `"qBittorrent"` / `"qBittorrentEE"`。
    fn type_label(&self) -> &str;
    fn feature_flags(&self) -> &[FeatureFlag];

    // 以下为占位签名；M2 接入 tokio 后改为 async fn / 返回 Future。
    fn login_stub(&self) -> Result<()>;
    fn get_torrents_stub(&self) -> Result<Vec<Torrent>>;
    fn get_peers_stub(&self, _torrent: &Torrent) -> Result<Vec<Peer>>;
}

/// 封禁策略（对应 Java EE 的 `BanHandler`）。Normal vs ShadowBan。
pub trait BanHandler: Send + Sync {
    /// shadowban 模式下探测下载器是否启用了该能力。Normal 恒 true。
    fn test_stub(&self) -> Result<bool>;
}

/// 工厂：按配置 `type` 创建下载器。保留为表驱动以便扩展。
pub fn create_downloader_stub(downloader_type: &str) -> Result<&'static str> {
    match downloader_type.to_lowercase().as_str() {
        "qbittorrent" => Ok("qBittorrent"),
        "qbittorrentee" => Ok("qBittorrentEE"),
        other => Err(pbh_domain::PbhError::Downloader(format!(
            "unsupported downloader type: {other}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn factory_dispatch() {
        assert_eq!(
            create_downloader_stub("qBittorrent").unwrap(),
            "qBittorrent"
        );
        assert_eq!(
            create_downloader_stub("qbittorrentee").unwrap(),
            "qBittorrentEE"
        );
        assert!(create_downloader_stub("transmission").is_err());
    }
}
