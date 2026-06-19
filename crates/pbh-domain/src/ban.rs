//! 封禁判定结果与封禁元数据。
//!
//! 对应 Java：`module/CheckResult.java`、`module/PeerAction.java`、`wrapper/BanMetadata.java`。

use crate::peer::PeerAddress;

/// 单个模块对单个 peer 的处置动作。
///
/// **序数即优先级**（来自 Java `PeerAction` 的 ordinal）：合并多个模块结果时取最高优先级，
/// 同级取更长封禁时长。顺序：`NoAction < BanForDisconnect < Ban < Skip`。
/// `Skip` 优先级最高（显式放行/短路），`Ban` 高于 `BanForDisconnect`（后者只为强制断开短暂封禁）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub enum PeerAction {
    /// 不处理。
    #[default]
    NoAction,
    /// 短暂封禁以强制断开连接（PCB fastTest 用），随后解封。
    BanForDisconnect,
    /// 封禁。
    Ban,
    /// 显式放行（短路其余规则）。
    Skip,
}

/// 一次规则检查的结果。对应 Java `CheckResult` record。
///
/// `rule`/`reason` 用纯 `String`（v2 无 i18n）。
#[derive(Debug, Clone)]
pub struct CheckResult {
    /// 产生该结果的模块标识（Java 中是模块 Class）。
    pub module: &'static str,
    /// 处置动作。
    pub action: PeerAction,
    /// 封禁时长（毫秒）；`0` 表示沿用全局默认。
    pub duration_ms: i64,
    /// 命中的规则（v2 纯字符串）。
    pub rule: String,
    /// 原因（v2 纯字符串）。
    pub reason: String,
}

impl CheckResult {
    /// 放行哨兵（对应 Java `pass()` → NO_ACTION）。
    pub fn pass(module: &'static str) -> Self {
        CheckResult {
            module,
            action: PeerAction::NoAction,
            duration_ms: 0,
            rule: String::new(),
            reason: String::new(),
        }
    }

    /// 合并两个结果：取更高优先级动作；同级取更长时长。
    ///
    /// 对应 Java `DigestionSession.extractFromLastOrgan` 的优先级合并逻辑。
    pub fn merge(self, other: CheckResult) -> CheckResult {
        match self.action.cmp(&other.action) {
            std::cmp::Ordering::Less => other,
            std::cmp::Ordering::Greater => self,
            std::cmp::Ordering::Equal => {
                if other.duration_ms > self.duration_ms {
                    other
                } else {
                    self
                }
            }
        }
    }
}

/// 一条封禁记录的元数据。对应 Java `BanMetadata`。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BanMetadata {
    /// 产生封禁的模块标识。
    pub context: String,
    /// 随机 id（UUID hex）。
    pub random_id: String,
    /// 被封 peer 地址。
    pub peer: PeerAddress,
    /// 封禁时刻（epoch millis）。
    pub ban_at: i64,
    /// 解封时刻（epoch millis）。
    pub unban_at: i64,
    /// 是否仅为强制断开的短暂封禁。
    pub ban_for_disconnect: bool,
    /// 是否排除出上报 / 展示。
    pub exclude_from_report: bool,
    pub exclude_from_display: bool,
    /// 规则与描述（i18n key 占位）。
    pub rule: String,
    pub description: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peer_action_priority_order() {
        assert!(PeerAction::Skip > PeerAction::Ban);
        assert!(PeerAction::Ban > PeerAction::BanForDisconnect);
        assert!(PeerAction::BanForDisconnect > PeerAction::NoAction);
    }

    #[test]
    fn merge_takes_higher_priority() {
        let ban = CheckResult {
            module: "A",
            action: PeerAction::Ban,
            duration_ms: 1000,
            rule: "r".into(),
            reason: "x".into(),
        };
        let skip = CheckResult {
            module: "B",
            action: PeerAction::Skip,
            duration_ms: 0,
            rule: String::new(),
            reason: String::new(),
        };
        // Skip 优先级最高，胜出。
        assert_eq!(ban.clone().merge(skip.clone()).action, PeerAction::Skip);
        assert_eq!(skip.merge(ban).action, PeerAction::Skip);
    }

    #[test]
    fn merge_same_action_takes_longer_duration() {
        let short = CheckResult {
            module: "A",
            action: PeerAction::Ban,
            duration_ms: 1000,
            rule: "a".into(),
            reason: String::new(),
        };
        let long = CheckResult {
            module: "B",
            action: PeerAction::Ban,
            duration_ms: 5000,
            rule: "b".into(),
            reason: String::new(),
        };
        assert_eq!(short.merge(long).duration_ms, 5000);
    }
}
