//! pbh-engine —— Ban 流水线、调度循环、BanManager、BanList。
//!
//! 对应上游 `banpipeline/**`、`DownloaderServerImpl.java`、`BanList.java`、`event/banwave/**`，
//! 但**不照搬**其 "organ" 隐喻与每 wave 新建线程池等结构（见 guidelines/02-architecture）。
//!
//! M3 实现：
//! - bounded `mpsc`(64) channel 流水线（provider→login→torrents→peers→snapshot→check）
//! - 每 peer 并发检查（`buffer_unordered` + 非线程安全模块串行化），每阶段 `tokio::time::timeout`
//! - `BanList`（IPv4/IPv6 前缀 trie + RwLock，最长前缀匹配，含 ban_for_disconnect 元数据）
//! - `BanManager`：banPeer（时长：模块级>全局）/ unban / removeExpiredBans / 白名单解封 / 手动队列
//! - Ban Wave 循环：固定延迟 + try_lock 防重叠 + WatchDog + 每小时快照 + globalPaused
//!
//! 注：AutoSTUN/NAT 穿透已**完全移除**（与封禁 peer 无关），peer 地址直接用下载器返回的原始 `ip:port`，
//! 不做 NAT 改写。

/// 内存封禁表占位。M3 用 IPv4/IPv6 前缀 trie + RwLock 实现，支持 CIDR 范围与最长前缀匹配。
#[derive(Debug, Default)]
pub struct BanList {
    // TODO(M3): DualStack prefix trie<BanMetadata>。
    _private: (),
}
