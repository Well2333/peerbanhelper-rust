//! Torrent。对应 Java `bittorrent/torrent/Torrent.java`。

/// 一个种子的快照（仅模块/统计实际用到的字段）。
#[derive(Debug, Clone)]
pub struct Torrent {
    pub id: String,
    pub hash: String,
    pub name: String,
    pub progress: f64,
    pub size: i64,
    /// 已完成字节数；`-1` 表示未知（对应 Java `getCompletedSize()`）。
    pub completed_size: i64,
    pub private_torrent: bool,
}

impl Torrent {
    /// 是否做种中（进度 ≥ 1.0）。对应 Java `Torrent.isSeeding()`。
    pub fn is_seeding(&self) -> bool {
        self.progress >= 1.0
    }
}
