//! pbh-rules —— 共享规则匹配引擎 + 各封禁规则模块。
//!
//! 对应 Java：`util/rule/**`（匹配引擎）、`module/impl/rule/**` + `module/impl/monitor/**`（模块）。
//!
//! 骨架阶段仅落地匹配引擎的**精确优先级语义**（std-only、可离线单测）。
//! 各模块按里程碑补：
//! - M4：AntiVampire、ClientNameBlacklist、PeerIdBlacklist、AutoRangeBan、IdleConnectionDosProtection、MultiDialingBlocker、PTRBlacklist
//! - M5：ProgressCheatBlocker（依赖 pbh-storage）
//! - M6：IPBlackList / IPBlackRuleList（依赖 pbh-geoip）
//! - M8：BtnNetworkOnline（在 pbh-btn 内调用引擎）
//!
//! 注：上游的 ExpressionRule（Aviator 脚本引擎，JVM 限定）已**完全移除**，不保留 trait 边界。

pub mod matcher;

pub use matcher::{MatchOutcome, RuleMethod, RuleSet, StringRule};

/// 规则模块统一接口（对应 Java `RuleFeatureModule`）。
///
/// 依赖抽象、不依赖具体（见守则第 9 条）：引擎/模块通过该 trait 接入流水线。
pub trait RuleModule: Send + Sync {
    /// 模块配置名（`profile.yml` 的 `module.<name>`）。
    fn config_name(&self) -> &str;

    /// 对单个 peer 做检查。骨架阶段签名占位，M3 接入 `pbh-engine` 时定稿
    /// （加入 `&Torrent, &Peer, &dyn Downloader` 与 `&AppContext`）。
    fn check_stub(&self) -> pbh_domain::CheckResult;
}
