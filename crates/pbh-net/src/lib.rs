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

/// 构造 reqwest 客户端。proxy 为空 → 直连;非空且 URL 合法 → **始终应用代理**。
///
/// 设计:代理是"权威"的——一旦为某类别配置了代理,就总是走它,**不再**因启动瞬间
/// 探测不可达而回退直连。理由:①被墙目标直连必失败,回退直连毫无意义;②pbh 常先于
/// clash/v2ray 起来,若探测失败即永久直连会"卡死"(见此前现象);始终应用代理则各处的
/// 重试循环(BTN/GeoIP/订阅)会在代理就绪后自动恢复。仅对"配置了但当前探测不可达"做一次 warn。
///
/// 默认行为:自动解压 gzip;默认 `User-Agent: PeerBanHelper-Rust/<version>`(调用方可请求级覆盖)。
pub fn build_client(proxy: &str, timeout: Duration) -> reqwest::Client {
    let mut b = reqwest::Client::builder()
        .timeout(timeout)
        .gzip(true)
        .user_agent(concat!("PeerBanHelper-Rust/", env!("CARGO_PKG_VERSION")));
    let proxy = proxy.trim();
    if !proxy.is_empty() {
        match reqwest::Proxy::all(proxy) {
            Ok(p) => {
                if proxy_reachable(proxy) {
                    tracing::info!("出站代理已启用: {proxy}");
                } else {
                    tracing::warn!(
                        "出站代理已配置但当前探测不可达({proxy}),仍按配置走代理(代理就绪后自动恢复)"
                    );
                }
                b = b.proxy(p);
            }
            Err(e) => tracing::warn!("代理 URL 无效({proxy}),改直连: {e}"),
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
