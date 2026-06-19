//! 响应信封。对应 Java `web/wrapper/StdResp.java` 与 `util/query/PBHPage.java`。
//!
//! 序列化形状（M7 加 serde 派生，字段名必须与下保持一致）：
//! - `StdResp`：`{ "success": bool, "message": string|null, "data": any|null }`
//! - 分页 `data`：`{ "page", "size", "total", "results" }`

/// 标准响应信封。`T` 为 `data` 的类型。
#[derive(Debug, Clone)]
pub struct StdResp<T> {
    pub success: bool,
    pub message: Option<String>,
    pub data: Option<T>,
}

impl<T> StdResp<T> {
    pub fn ok(data: T) -> Self {
        StdResp {
            success: true,
            message: None,
            data: Some(data),
        }
    }

    pub fn ok_msg(message: impl Into<String>, data: Option<T>) -> Self {
        StdResp {
            success: true,
            message: Some(message.into()),
            data,
        }
    }

    pub fn err(message: impl Into<String>) -> Self {
        StdResp {
            success: false,
            message: Some(message.into()),
            data: None,
        }
    }
}

/// 分页结果。对应 Java `PBHPage`。请求参数 `?page=`(默认1)`&pageSize=`(默认10)。
#[derive(Debug, Clone)]
pub struct PbhPage<T> {
    pub page: i64,
    pub size: i64,
    pub total: i64,
    pub results: Vec<T>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ok_sets_success_true() {
        let r = StdResp::ok(42);
        assert!(r.success);
        assert_eq!(r.data, Some(42));
        assert!(r.message.is_none());
    }

    #[test]
    fn err_sets_success_false() {
        let r: StdResp<()> = StdResp::err("boom");
        assert!(!r.success);
        assert_eq!(r.message.as_deref(), Some("boom"));
    }
}
