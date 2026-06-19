//! pbh-engine —— Ban 流水线、调度循环、BanManager、BanList。
//!
//! 对应上游 `banpipeline/**`、`DownloaderServerImpl.java`、`BanList.java`、`event/banwave/**`，
//! 但**不照搬**其 "organ" 隐喻与每 wave 新建线程池等结构（见 guidelines/02-architecture）。
//!
//! M1 落地：`BanList`（内存权威，IPv4/IPv6 前缀 trie + RwLock，最长前缀匹配）。
//! 后续里程碑：
//! - M3：channel 流水线、Ban Wave 循环、BanManager（banPeer/unban/到期解封/手动队列）、封禁下发。
//!
//! 注：AutoSTUN/NAT 已完全移除，peer 地址直接用下载器返回的原始 `ip:port`，不做 NAT 改写。

pub mod ban_list;

pub use ban_list::BanList;
