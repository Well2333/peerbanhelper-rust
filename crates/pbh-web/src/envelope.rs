//! 自研响应信封（v2,见 docs/05 §4）。形状:
//! - 成功:`{ "ok": true,  "data": <any>, "error": null }`
//! - 失败:`{ "ok": false, "data": null,  "error": "<msg>" }`
//! - 分页 `data`:`{ "page", "size", "total", "items" }`
//!
//! M7 加 serde 派生;字段名以此为准(不与 Java StdResp 兼容)。

/// 标准响应信封。`T` 为 `data` 的类型。
#[derive(Debug, Clone)]
pub struct ApiResp<T> {
    pub ok: bool,
    pub data: Option<T>,
    pub error: Option<String>,
}

impl<T> ApiResp<T> {
    pub fn ok(data: T) -> Self {
        ApiResp {
            ok: true,
            data: Some(data),
            error: None,
        }
    }

    pub fn err(error: impl Into<String>) -> Self {
        ApiResp {
            ok: false,
            data: None,
            error: Some(error.into()),
        }
    }
}

/// 分页结果。请求参数 `?page=`(默认1)`&pageSize=`(默认20)。
#[derive(Debug, Clone)]
pub struct Page<T> {
    pub page: i64,
    pub size: i64,
    pub total: i64,
    pub items: Vec<T>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ok_sets_ok_true() {
        let r = ApiResp::ok(42);
        assert!(r.ok);
        assert_eq!(r.data, Some(42));
        assert!(r.error.is_none());
    }

    #[test]
    fn err_sets_ok_false() {
        let r: ApiResp<()> = ApiResp::err("boom");
        assert!(!r.ok);
        assert_eq!(r.error.as_deref(), Some("boom"));
        assert!(r.data.is_none());
    }
}
