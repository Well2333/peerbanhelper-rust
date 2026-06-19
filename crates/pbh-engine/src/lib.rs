//! pbh-engine —— Ban 流水线、调度循环、BanManager、BanList。
//!
//! 对应上游 `banpipeline/**`、`DownloaderServerImpl.java`、`BanList.java`、`event/banwave/**`，
//! 但**不照搬**其 "organ" 隐喻与每 wave 新建线程池等结构（见 guidelines/02-architecture）。
//!
//! M1：`BanList`（内存权威，IPv4/IPv6 前缀 trie + RwLock，最长前缀匹配）。
//! M3：`BanManager` + Ban Wave 循环（登录→拉 peer→跑模块→封禁→下发→到期解封）;`build_modules` 从 profile 构建规则。
//!
//! 注：AutoSTUN/NAT 已完全移除，peer 地址直接用下载器返回的原始 `ip:port`，不做 NAT 改写。

pub mod auto_range_ban;
pub mod ban_list;
pub mod ban_manager;
pub mod ip_blacklist;
pub mod ip_rule_list;
pub mod modules;
pub mod pcb;
pub mod ptr_blacklist;

pub use auto_range_ban::AutoRangeBan;
pub use ban_list::BanList;
pub use ban_manager::{BanManager, StatsSnapshot};
pub use ip_blacklist::IpBlackList;
pub use ip_rule_list::{IpBlackRuleList, SubConfig};
pub use modules::build_modules;
pub use pcb::{PcbConfig, ProgressCheatBlocker};
pub use ptr_blacklist::PtrBlacklist;
