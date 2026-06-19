//! pbh-i18n —— 翻译组件与多语言。对应 Java `text/**`、`util/MsgUtil.java`、`resources/lang/**`。
//!
//! **前端契约：** `TranslationComponent` 必须序列化为 `{ "key": ..., "params": [...] }`
//! （M1 加 serde）。语言文件为扁平 YAML（`KEY: "... {} ..."`），占位符是顺序 `{}`（**非** printf）。
//! 语言：en_us / zh_cn / zh_tw + `messages_fallback.yml` 兜底填充。

/// 翻译参数：可以是字面量，或嵌套的翻译组件（递归解析）。
#[derive(Debug, Clone)]
pub enum TransParam {
    Text(String),
    Component(Box<TranslationComponent>),
}

/// 一个可本地化的消息：i18n key + 顺序参数。对应 Java `TranslationComponent`。
#[derive(Debug, Clone)]
pub struct TranslationComponent {
    pub key: String,
    pub params: Vec<TransParam>,
}

impl TranslationComponent {
    pub fn new(key: impl Into<String>) -> Self {
        TranslationComponent {
            key: key.into(),
            params: Vec::new(),
        }
    }

    pub fn with_params(key: impl Into<String>, params: Vec<TransParam>) -> Self {
        TranslationComponent {
            key: key.into(),
            params,
        }
    }
}

/// 按顺序把 `args` 填入模板中的 `{}`。对应 Java `MsgUtil.fillArgs`。
///
/// 语义（必须精确）：从左到右逐个把 `{}` 替换为下一个参数；参数用尽后剩余 `{}` 原样保留；
/// 多余参数忽略。**不是** `format!` —— `{}` 是字面两字符标记。
pub fn fill_args(template: &str, args: &[String]) -> String {
    let mut out = String::with_capacity(template.len());
    let mut idx = 0;
    let bytes = template.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if i + 1 < bytes.len() && bytes[i] == b'{' && bytes[i + 1] == b'}' {
            if idx < args.len() {
                out.push_str(&args[idx]);
                idx += 1;
            } else {
                out.push_str("{}");
            }
            i += 2;
        } else {
            // 按字符推进以保证 UTF-8 正确。
            let ch_len = utf8_char_len(bytes[i]);
            out.push_str(std::str::from_utf8(&bytes[i..i + ch_len]).unwrap_or(""));
            i += ch_len;
        }
    }
    out
}

fn utf8_char_len(first: u8) -> usize {
    match first {
        0x00..=0x7F => 1,
        0xC0..=0xDF => 2,
        0xE0..=0xEF => 3,
        _ => 4,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fills_in_order() {
        assert_eq!(fill_args("封禁 {} 端口 {}", &["1.2.3.4".into(), "6881".into()]), "封禁 1.2.3.4 端口 6881");
    }

    #[test]
    fn leftover_placeholders_kept() {
        assert_eq!(fill_args("{} 和 {}", &["a".into()]), "a 和 {}");
    }

    #[test]
    fn extra_args_ignored() {
        assert_eq!(fill_args("{}", &["a".into(), "b".into()]), "a");
    }

    #[test]
    fn utf8_safe() {
        assert_eq!(fill_args("中文{}结尾", &["值".into()]), "中文值结尾");
    }
}
