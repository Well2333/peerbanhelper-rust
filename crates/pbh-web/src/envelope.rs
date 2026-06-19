//! 自研响应信封。形状:
//! - 成功:`{ "ok": true,  "data": <any>, "error": null }`
//! - 失败:`{ "ok": false, "data": null,  "error": "<msg>" }`
//! - 分页 `data`:`{ "page", "size", "total", "items" }`

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Serialize;

/// 标准响应信封。
#[derive(Debug, Clone, Serialize)]
pub struct ApiResp<T> {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<T>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl<T: Serialize> ApiResp<T> {
    pub fn ok(data: T) -> Self {
        ApiResp {
            ok: true,
            data: Some(data),
            error: None,
        }
    }
}

impl ApiResp<()> {
    pub fn ok_empty() -> Self {
        ApiResp {
            ok: true,
            data: None,
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

impl<T: Serialize> IntoResponse for ApiResp<T> {
    fn into_response(self) -> Response {
        (StatusCode::OK, Json(self)).into_response()
    }
}

/// 分页结果。
#[derive(Debug, Clone, Serialize)]
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
    fn ok_serializes_with_data_no_error() {
        let j = serde_json::to_string(&ApiResp::ok(42)).unwrap();
        assert_eq!(j, r#"{"ok":true,"data":42}"#);
    }

    #[test]
    fn ok_empty_omits_data_and_error() {
        let j = serde_json::to_string(&ApiResp::ok_empty()).unwrap();
        assert_eq!(j, r#"{"ok":true}"#);
    }

    #[test]
    fn err_serializes_with_error() {
        let j = serde_json::to_string(&ApiResp::<()>::err("未授权")).unwrap();
        assert_eq!(j, r#"{"ok":false,"error":"未授权"}"#);
    }

    #[test]
    fn page_shape() {
        let p = Page {
            page: 1,
            size: 20,
            total: 100,
            items: vec!["a", "b"],
        };
        let j = serde_json::to_string(&ApiResp::ok(p)).unwrap();
        assert!(j.contains(r#""page":1"#));
        assert!(j.contains(r#""total":100"#));
        assert!(j.contains(r#""items":["a","b"]"#));
    }
}
