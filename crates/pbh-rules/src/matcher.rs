//! 共享字符串规则匹配引擎。对应 Java `util/rule/RuleParser.java` + `matcher/*`。
//!
//! 被 ClientNameBlacklist / PeerIdBlacklist / PTRBlacklist 复用。
//!
//! **优先级语义（必须精确复刻）：** 遍历规则列表 ——
//! - 任一规则判定为「显式不匹配(False)」立即获胜并短路（最高优先级 = 显式放行）；
//! - 「匹配(True)」暂被接受，但可被后续的 False 覆盖；
//! - 全部走完后：有 True 则匹配，否则默认不匹配。

/// 单条规则的匹配方法。对应 Java method 枚举。
/// 注：除 `Regex` 外均大小写不敏感（与 Java 一致）。`Regex` 在 M4 接入 `regex` crate。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuleMethod {
    StartsWith,
    EndsWith,
    Contains,
    Equals,
    /// 长度区间 [min, max]。
    Length { min: usize, max: usize },
    /// 正则。骨架阶段不求值（返回 false 并标记 TODO）。M4 用 `regex`。
    Regex,
}

/// 单条字符串规则。
#[derive(Debug, Clone)]
pub struct StringRule {
    pub method: RuleMethod,
    /// 匹配内容（Regex/Length 时可为模式/忽略）。
    pub content: String,
    /// 命中后判定为「匹配(hit=True)」还是「显式放行(hit=False)」。
    /// 对应 Java 规则里的 `hit`/`miss` 控制（默认命中即 True）。
    pub hit_is_true: bool,
}

impl StringRule {
    /// 该规则对给定输入的单条判定：`Some(true)`=匹配，`Some(false)`=显式放行，`None`=不适用。
    fn evaluate(&self, input: &str) -> Option<bool> {
        let hit = match &self.method {
            RuleMethod::StartsWith => {
                input.to_lowercase().starts_with(&self.content.to_lowercase())
            }
            RuleMethod::EndsWith => input.to_lowercase().ends_with(&self.content.to_lowercase()),
            RuleMethod::Contains => input.to_lowercase().contains(&self.content.to_lowercase()),
            RuleMethod::Equals => input.eq_ignore_ascii_case(&self.content),
            RuleMethod::Length { min, max } => {
                let n = input.chars().count();
                n >= *min && n <= *max
            }
            // TODO(M4): 接入 `regex` crate，大小写敏感匹配。
            RuleMethod::Regex => false,
        };
        if hit {
            Some(self.hit_is_true)
        } else {
            None
        }
    }
}

/// 一组规则的匹配结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchOutcome {
    /// 匹配（应被规则模块处置，通常 → BAN）。
    True,
    /// 不匹配 / 显式放行。
    False,
}

/// 一组字符串规则。
#[derive(Debug, Clone, Default)]
pub struct RuleSet {
    pub rules: Vec<StringRule>,
}

impl RuleSet {
    pub fn new(rules: Vec<StringRule>) -> Self {
        RuleSet { rules }
    }

    /// 按 Java 优先级语义匹配：False 短路获胜，否则有 True 即 True。
    pub fn match_input(&self, input: &str) -> MatchOutcome {
        let mut saw_true = false;
        for rule in &self.rules {
            match rule.evaluate(input) {
                Some(false) => return MatchOutcome::False, // 显式放行，最高优先级，短路
                Some(true) => saw_true = true,
                None => {}
            }
        }
        if saw_true {
            MatchOutcome::True
        } else {
            MatchOutcome::False
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(method: RuleMethod, content: &str, hit_is_true: bool) -> StringRule {
        StringRule {
            method,
            content: content.into(),
            hit_is_true,
        }
    }

    #[test]
    fn starts_with_case_insensitive() {
        let rs = RuleSet::new(vec![r(RuleMethod::StartsWith, "-XL", true)]);
        assert_eq!(rs.match_input("-xl0019-blah"), MatchOutcome::True);
        assert_eq!(rs.match_input("-qB4250-"), MatchOutcome::False);
    }

    #[test]
    fn false_rule_short_circuits_over_true() {
        // 一条 True（contains "xl"）+ 一条 False（equals 白名单），False 必须获胜。
        let rs = RuleSet::new(vec![
            r(RuleMethod::Contains, "xl", true),
            r(RuleMethod::Equals, "-XL-trusted", false),
        ]);
        assert_eq!(rs.match_input("-XL-trusted"), MatchOutcome::False);
    }

    #[test]
    fn length_rule() {
        let rs = RuleSet::new(vec![r(RuleMethod::Length { min: 1, max: 3 }, "", true)]);
        assert_eq!(rs.match_input("ab"), MatchOutcome::True);
        assert_eq!(rs.match_input("abcd"), MatchOutcome::False);
    }

    #[test]
    fn empty_ruleset_is_false() {
        assert_eq!(RuleSet::default().match_input("anything"), MatchOutcome::False);
    }
}
