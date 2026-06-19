//! pbh-rules —— 共享规则匹配引擎 + 各封禁规则模块。
//!
//! 对应上游：`util/rule/**`（匹配引擎）、`module/impl/rule/**` + `module/impl/monitor/**`（模块）。
//!
//! M1 落地引擎：字符串规则匹配（`matcher`）、CIDR 最长前缀匹配（`ip_matcher`）、记忆化缓存（`cache`）。
//! 各模块按里程碑补：
//! - M4：AntiVampire、ClientNameBlacklist、PeerIdBlacklist、AutoRangeBan、IdleConnectionDosProtection、MultiDialingBlocker、PTRBlacklist
//! - M5：ProgressCheatBlocker（依赖 pbh-storage）
//! - M6：IPBlackList / IPBlackRuleList（依赖 pbh-geoip）
//! - M8：BtnNetworkOnline（在 pbh-btn 内调用引擎）
//!
//! 注：上游的 ExpressionRule（Aviator 脚本引擎，JVM 限定）已**完全移除**，不保留 trait 边界。

pub mod cache;
pub mod ip_matcher;
pub mod matcher;
pub mod module;

pub use cache::ModuleMatchCache;
pub use ip_matcher::IpMatcher;
pub use matcher::{MatchOutcome, Matcher, RuleParseError, RuleSet, StringRule};
pub use module::{AntiVampire, ClientNameBlacklist, PeerIdBlacklist, RuleFeatureModule};
