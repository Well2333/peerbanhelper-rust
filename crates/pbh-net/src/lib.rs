//! pbh-net —— 代理感知的 reqwest 客户端构造。
//!
//! 守则:所有出站请求(BTN/订阅/GeoIP/检查更新)统一经此构造;qBittorrent 下载器除外。
//! 代理为空 → 直连;非空但不可达 → 直连 + warn;非空且可达 → 走代理。

use std::time::Duration;

/// 探测代理 host:port 是否可 TCP 连接(~1s 超时)。proxy 为空返回 false。
///
/// # 阻塞说明
///
/// 当传入非空代理字符串时,本函数会执行阻塞式 DNS 解析 + TCP 连接探测(超时约 1 秒)。
/// 设计上仅供客户端构造(启动/配置变更)时调用,请勿在每次请求的热路径中调用。
pub fn proxy_reachable(proxy: &str) -> bool {
    if proxy.trim().is_empty() {
        return false;
    }
    let Ok(u) = reqwest::Url::parse(proxy) else {
        return false;
    };
    let Some(host) = u.host_str() else {
        return false;
    };
    let port = u.port_or_known_default().unwrap_or(1080);
    use std::net::ToSocketAddrs;
    let Ok(mut addrs) = (host, port).to_socket_addrs() else {
        return false;
    };
    // TOCTOU:探测时可达、构造后不可达(或反之)均属正常,因客户端构造频率极低,此权衡可接受。
    addrs.any(|addr| std::net::TcpStream::connect_timeout(&addr, Duration::from_millis(1000)).is_ok())
}

/// 构造 reqwest 客户端。proxy 为空或不可达 → 直连;否则走代理。
///
/// # 阻塞说明
///
/// 若 proxy 非空,本函数内部会调用 [`proxy_reachable`] 执行阻塞式 DNS + TCP 探测
/// (超时约 1 秒)。本函数仅供启动或配置变更时调用,请勿在每次请求的热路径中调用。
pub fn build_client(proxy: &str, timeout: Duration) -> reqwest::Client {
    let mut b = reqwest::Client::builder().timeout(timeout);
    if !proxy.trim().is_empty() {
        if proxy_reachable(proxy) {
            match reqwest::Proxy::all(proxy) {
                Ok(p) => {
                    tracing::info!("出站代理已启用: {proxy}");
                    b = b.proxy(p);
                }
                Err(e) => tracing::warn!("代理 URL 无效({proxy}),改直连: {e}"),
            }
        } else {
            tracing::warn!("代理不可达({proxy}),本次改直连");
        }
    }
    b.build().unwrap_or_else(|e| {
        tracing::error!("构造 reqwest 客户端失败,回退默认客户端: {e}");
        reqwest::Client::new()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_proxy_not_reachable() {
        assert!(!proxy_reachable(""));
        assert!(!proxy_reachable("   "));
    }

    #[test]
    fn garbage_proxy_not_reachable() {
        assert!(!proxy_reachable("not a url"));
    }

    #[test]
    fn build_client_empty_is_direct() {
        let _c = build_client("", Duration::from_secs(10));
    }

    #[test]
    fn build_client_unreachable_falls_back() {
        // 无恐慌冒烟测试:reqwest 未公开 API 来内省代理配置,仅验证函数不崩溃。
        let _c = build_client("http://127.0.0.1:1", Duration::from_secs(5));
    }
}
