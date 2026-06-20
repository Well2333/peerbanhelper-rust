//! BTN HTTP 客户端：固定头注入、config 拉取、规则/名单下行、gzip 上行。
//! 对应上游 `btn/BtnNetwork` 的 HTTP 中间件。

use std::io::Write;

use flate2::write::GzEncoder;
use flate2::Compression;
use reqwest::header::{HeaderMap, HeaderValue};

use crate::model::{BtnConfigResponse, BtnRuleset};
use crate::{PROTOCOL_IMPL_VERSION, PROTOCOL_READABLE_VERSION};

const EXAMPLE_VALUES: &[&str] = &["", "example", "your_app_id", "your_app_secret"];

/// BTN 客户端。
pub struct BtnClient {
    http: reqwest::Client,
    app_id: String,
    app_secret: String,
    installation_id: String,
}

impl BtnClient {
    /// `proxy` 为空字符串时直连（当前所有调用均传 `""`，代理接通在后续任务中完成）。
    pub fn new(app_id: String, app_secret: String, installation_id: String, proxy: &str) -> Self {
        let http = pbh_net::build_client(proxy, std::time::Duration::from_secs(45));
        BtnClient {
            http,
            app_id,
            app_secret,
            installation_id,
        }
    }

    fn is_anonymous(&self) -> bool {
        EXAMPLE_VALUES.contains(&self.app_id.as_str())
            || EXAMPLE_VALUES.contains(&self.app_secret.as_str())
    }

    /// 构造每请求固定头。
    fn base_headers(&self) -> HeaderMap {
        let mut h = HeaderMap::new();
        let ua = format!(
            "PeerBanHelper-Rust/{}/BTN-Protocol/{PROTOCOL_READABLE_VERSION}/{PROTOCOL_IMPL_VERSION}",
            env!("CARGO_PKG_VERSION")
        );
        h.insert(reqwest::header::USER_AGENT, hv(&ua));
        h.insert(reqwest::header::CONTENT_TYPE, hv("application/json"));
        if self.is_anonymous() {
            h.insert("X-BTN-InstallationID", hv(&self.installation_id));
        } else {
            h.insert("X-BTN-AppID", hv(&self.app_id));
            h.insert("X-BTN-AppSecret", hv(&self.app_secret));
            h.insert("BTN-AppID", hv(&self.app_id));
            h.insert("BTN-AppSecret", hv(&self.app_secret));
            h.insert(
                "Authentication",
                hv(&format!("Bearer {}@{}", self.app_id, self.app_secret)),
            );
        }
        h
    }

    /// 拉取 config。
    pub async fn fetch_config(&self, url: &str) -> reqwest::Result<BtnConfigResponse> {
        self.http
            .get(url)
            .headers(self.base_headers())
            .send()
            .await?
            .error_for_status()?
            .json::<BtnConfigResponse>()
            .await
    }

    /// 拉取规则集（`?rev=`）。204 → None（未变）。
    pub async fn fetch_rules(&self, url: &str, rev: &str) -> reqwest::Result<Option<BtnRuleset>> {
        let resp = self
            .http
            .get(url)
            .query(&[("rev", rev)])
            .headers(self.base_headers())
            .send()
            .await?
            .error_for_status()?;
        if resp.status() == reqwest::StatusCode::NO_CONTENT {
            return Ok(None);
        }
        Ok(Some(resp.json::<BtnRuleset>().await?))
    }

    /// 拉取纯文本 IP 名单,返回 (文本, X-BTN-ContentVersion)。
    pub async fn fetch_ip_list(
        &self,
        url: &str,
        rev: &str,
    ) -> reqwest::Result<Option<(String, String)>> {
        let resp = self
            .http
            .get(url)
            .query(&[("rev", rev)])
            .headers(self.base_headers())
            .send()
            .await?
            .error_for_status()?;
        if resp.status() == reqwest::StatusCode::NO_CONTENT {
            return Ok(None);
        }
        let ver = resp
            .headers()
            .get("X-BTN-ContentVersion")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();
        let text = resp.text().await?;
        Ok(Some((text, ver)))
    }

    /// 心跳：上报 `{"ifaddr":"default"}`,返回服务端看到的外网 IP。
    pub async fn heartbeat(&self, url: &str) -> reqwest::Result<Option<String>> {
        let resp = self
            .http
            .post(url)
            .headers(self.base_headers())
            .json(&serde_json::json!({"ifaddr": "default"}))
            .send()
            .await?
            .error_for_status()?;
        let v: serde_json::Value = resp.json().await?;
        Ok(v.get("external_ip")
            .and_then(|x| x.as_str())
            .map(|s| s.to_string()))
    }

    /// gzip 上行 JSON。
    pub async fn submit_gzip(&self, url: &str, json_body: &str) -> reqwest::Result<()> {
        let gz = gzip(json_body.as_bytes());
        let mut headers = self.base_headers();
        headers.insert(reqwest::header::CONTENT_ENCODING, hv("gzip"));
        self.http
            .post(url)
            .headers(headers)
            .body(gz)
            .send()
            .await?
            .error_for_status()?;
        Ok(())
    }
}

fn hv(s: &str) -> HeaderValue {
    HeaderValue::from_str(s).unwrap_or_else(|_| HeaderValue::from_static(""))
}

/// gzip 压缩。
pub fn gzip(data: &[u8]) -> Vec<u8> {
    let mut enc = GzEncoder::new(Vec::new(), Compression::default());
    let _ = enc.write_all(data);
    enc.finish().unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    #[test]
    fn gzip_roundtrip() {
        let data = b"{\"bans\":[]}";
        let gz = gzip(data);
        let mut dec = flate2::read::GzDecoder::new(&gz[..]);
        let mut out = Vec::new();
        dec.read_to_end(&mut out).unwrap();
        assert_eq!(out, data);
    }

    #[test]
    fn anonymous_uses_installation_id() {
        let c = BtnClient::new(String::new(), String::new(), "install-123".into(), "");
        assert!(c.is_anonymous());
        let h = c.base_headers();
        assert!(h.contains_key("X-BTN-InstallationID"));
        assert!(!h.contains_key("X-BTN-AppID"));
    }

    #[test]
    fn authed_sets_app_headers() {
        let c = BtnClient::new("appid".into(), "secret".into(), "i".into(), "");
        assert!(!c.is_anonymous());
        let h = c.base_headers();
        assert_eq!(h.get("X-BTN-AppID").unwrap(), "appid");
        assert_eq!(h.get("Authentication").unwrap(), "Bearer appid@secret");
    }
}
