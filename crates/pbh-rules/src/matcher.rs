//! 共享字符串规则匹配引擎。对应上游 `util/rule/RuleParser.java` + `matcher/*`。
//!
//! 被 ClientNameBlacklist / PeerIdBlacklist / PTRBlacklist 复用。
//!
//! **优先级语义（精确复刻）：** 遍历规则 —— 任一规则判定「显式不匹配(False)」立即获胜并短路;
//! 「匹配(True)」暂被接受、可被后续 False 覆盖;全部走完后有 True 则 True，否则 False。
//!
//! 规则可由 JSON 字符串解析（`profile.yml` 的 `banned-peer-id` 等），形如
//! `{"method":"STARTS_WITH","content":"-XL"}`，可选 `hit` 控制命中判定。

use regex::Regex;

/// 单条规则的匹配方法。除 `Regex` 外大小写不敏感（与上游一致）。
#[derive(Debug)]
pub enum Matcher {
    StartsWith(String),
    EndsWith(String),
    Contains(String),
    Equals(String),
    /// 长度区间 [min, max]。
    Length {
        min: usize,
        max: usize,
    },
    /// 正则（大小写敏感）。
    Regex(Regex),
}

impl Matcher {
    /// 该方法是否命中输入。
    fn hits(&self, input: &str) -> bool {
        match self {
            Matcher::StartsWith(c) => input.to_lowercase().starts_with(&c.to_lowercase()),
            Matcher::EndsWith(c) => input.to_lowercase().ends_with(&c.to_lowercase()),
            Matcher::Contains(c) => input.to_lowercase().contains(&c.to_lowercase()),
            Matcher::Equals(c) => input.eq_ignore_ascii_case(c),
            Matcher::Length { min, max } => {
                let n = input.chars().count();
                n >= *min && n <= *max
            }
            Matcher::Regex(re) => re.is_match(input),
        }
    }
}

/// 单条规则：方法 + 命中后判定为 True 还是「显式放行 False」。
#[derive(Debug)]
pub struct StringRule {
    pub matcher: Matcher,
    /// 命中时判为 True（默认）;为 false 表示命中即「显式放行」。
    pub hit_is_true: bool,
}

impl StringRule {
    pub fn new(matcher: Matcher) -> Self {
        StringRule {
            matcher,
            hit_is_true: true,
        }
    }

    /// 单条判定：`Some(true)`=匹配，`Some(false)`=显式放行，`None`=未命中。
    fn evaluate(&self, input: &str) -> Option<bool> {
        if self.matcher.hits(input) {
            Some(self.hit_is_true)
        } else {
            None
        }
    }
}

/// 一组规则的匹配结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchOutcome {
    /// 匹配（通常 → BAN）。
    True,
    /// 不匹配 / 显式放行。
    False,
}

/// 规则解析错误。
#[derive(Debug, thiserror::Error)]
pub enum RuleParseError {
    #[error("规则 JSON 解析失败: {0}")]
    Json(#[from] serde_json::Error),
    #[error("未知 method: {0}")]
    UnknownMethod(String),
    #[error("正则编译失败: {0}")]
    Regex(#[from] regex::Error),
    #[error("规则缺少字段: {0}")]
    MissingField(&'static str),
}

/// 一组字符串规则。
#[derive(Debug, Default)]
pub struct RuleSet {
    pub rules: Vec<StringRule>,
}

impl RuleSet {
    pub fn new(rules: Vec<StringRule>) -> Self {
        RuleSet { rules }
    }

    /// 从 JSON 规则字符串列表解析（逐条;遇错返回该条错误）。
    pub fn parse(rules: &[String]) -> Result<Self, RuleParseError> {
        let mut out = Vec::with_capacity(rules.len());
        for raw in rules {
            out.push(parse_one(raw)?);
        }
        Ok(RuleSet { rules: out })
    }

    /// 按优先级语义匹配：False 短路获胜，否则有 True 即 True。
    pub fn match_input(&self, input: &str) -> MatchOutcome {
        let mut saw_true = false;
        for rule in &self.rules {
            match rule.evaluate(input) {
                Some(false) => return MatchOutcome::False,
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

fn parse_one(raw: &str) -> Result<StringRule, RuleParseError> {
    let v: serde_json::Value = serde_json::from_str(raw)?;
    let method = v
        .get("method")
        .and_then(|m| m.as_str())
        .ok_or(RuleParseError::MissingField("method"))?;
    let content = v.get("content").and_then(|c| c.as_str()).unwrap_or("");
    let matcher = match method.to_ascii_uppercase().as_str() {
        "STARTS_WITH" => Matcher::StartsWith(content.to_string()),
        "ENDS_WITH" => Matcher::EndsWith(content.to_string()),
        "CONTAINS" => Matcher::Contains(content.to_string()),
        "EQUALS" => Matcher::Equals(content.to_string()),
        "REGEX" => Matcher::Regex(Regex::new(content)?),
        "LENGTH" => {
            let min = v.get("min").and_then(|x| x.as_u64()).unwrap_or(0) as usize;
            let max = v
                .get("max")
                .and_then(|x| x.as_u64())
                .map(|m| m as usize)
                .unwrap_or(usize::MAX);
            Matcher::Length { min, max }
        }
        other => return Err(RuleParseError::UnknownMethod(other.to_string())),
    };
    // hit: 命中判定。默认 true;显式 "hit":"FALSE"/false → 命中即放行。
    let hit_is_true = match v.get("hit") {
        Some(serde_json::Value::Bool(b)) => *b,
        Some(serde_json::Value::String(s)) => !s.eq_ignore_ascii_case("false"),
        _ => true,
    };
    Ok(StringRule {
        matcher,
        hit_is_true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_with_case_insensitive() {
        let rs = RuleSet::new(vec![StringRule::new(Matcher::StartsWith("-XL".into()))]);
        assert_eq!(rs.match_input("-xl0019-blah"), MatchOutcome::True);
        assert_eq!(rs.match_input("-qB4250-"), MatchOutcome::False);
    }

    #[test]
    fn false_rule_short_circuits_over_true() {
        let rs = RuleSet::new(vec![
            StringRule::new(Matcher::Contains("xl".into())),
            StringRule {
                matcher: Matcher::Equals("-XL-trusted".into()),
                hit_is_true: false,
            },
        ]);
        assert_eq!(rs.match_input("-XL-trusted"), MatchOutcome::False);
    }

    #[test]
    fn length_rule() {
        let rs = RuleSet::new(vec![StringRule::new(Matcher::Length { min: 1, max: 3 })]);
        assert_eq!(rs.match_input("ab"), MatchOutcome::True);
        assert_eq!(rs.match_input("abcd"), MatchOutcome::False);
    }

    #[test]
    fn regex_case_sensitive() {
        let rs = RuleSet::new(vec![StringRule::new(Matcher::Regex(
            Regex::new(r"^-XL\d{4}-").unwrap(),
        ))]);
        assert_eq!(rs.match_input("-XL0019-x"), MatchOutcome::True);
        assert_eq!(rs.match_input("-xl0019-x"), MatchOutcome::False);
    }

    #[test]
    fn parse_json_rules() {
        let rules = vec![
            r#"{"method":"STARTS_WITH","content":"-XL"}"#.to_string(),
            r#"{"method":"REGEX","content":"^cacao"}"#.to_string(),
            r#"{"method":"LENGTH","min":1,"max":3}"#.to_string(),
        ];
        let rs = RuleSet::parse(&rules).unwrap();
        assert_eq!(rs.match_input("-XL0019"), MatchOutcome::True);
        assert_eq!(rs.match_input("cacaoclient"), MatchOutcome::True);
        assert_eq!(rs.match_input("ab"), MatchOutcome::True);
        assert_eq!(rs.match_input("normalclient"), MatchOutcome::False);
    }

    #[test]
    fn parse_hit_false_is_allowlist() {
        let rules = vec![r#"{"method":"EQUALS","content":"good","hit":"FALSE"}"#.to_string()];
        let rs = RuleSet::parse(&rules).unwrap();
        assert_eq!(rs.match_input("good"), MatchOutcome::False);
    }

    #[test]
    fn parse_unknown_method_errors() {
        let rules = vec![r#"{"method":"FOO","content":"x"}"#.to_string()];
        assert!(RuleSet::parse(&rules).is_err());
    }

    #[test]
    fn empty_ruleset_is_false() {
        assert_eq!(
            RuleSet::default().match_input("anything"),
            MatchOutcome::False
        );
    }
}
