//! pbh-web —— 自研极简 HTTP API + 内置单页。对应方案 `memory/design/roadmap.md` §1。
//!
//! 不复刻上游 Vue/StdResp/Gson;Bearer token 鉴权;内置 vanilla 单页(`include_str!` 内嵌)。

mod btn_manager;
mod envelope;
mod routes;
mod selfupdate;

use std::net::SocketAddr;
use std::sync::Arc;

use pbh_config::{ConfigHandle, Paths};
use pbh_domain::LogBuffer;
use pbh_downloader::DownloaderManager;
use pbh_engine::BanManager;
use pbh_geoip::GeoIpHandle;
use pbh_storage::Db;

pub use btn_manager::BtnManager;
pub use envelope::{ApiResp, Page};

/// Web 层共享状态。
#[derive(Clone)]
pub struct WebState {
    pub config: ConfigHandle,
    pub paths: Paths,
    pub downloaders: Arc<DownloaderManager>,
    pub ban_manager: Arc<BanManager>,
    pub db: Db,
    pub logs: Arc<LogBuffer>,
    /// GeoIP 句柄（供 list_bans + profile 热重载时重建 IPBlackList）。
    pub geoip: GeoIpHandle,
    /// BTN 热启停管理器（供 profile 热重载时重建 BtnNetworkOnline）。
    pub btn: std::sync::Arc<BtnManager>,
    /// 防止 GeoIP 更新并发触发(重复下载/写竞争)。
    pub geoip_lock: std::sync::Arc<tokio::sync::Mutex<()>>,
}

/// 启动 HTTP 服务（阻塞直到出错/关闭）。
pub async fn serve(state: WebState, addr: SocketAddr) -> std::io::Result<()> {
    let app = routes::router(state);
    let listener = bind_with_retry(addr).await?;
    tracing::info!("Web 已监听 http://{addr}");
    axum::serve(listener, app).await
}

/// 绑定端口；地址被占用时短暂重试（自更新重启后旧进程可能尚未完全释放端口）。
async fn bind_with_retry(addr: SocketAddr) -> std::io::Result<tokio::net::TcpListener> {
    let mut last_err = None;
    for attempt in 0..12 {
        match tokio::net::TcpListener::bind(addr).await {
            Ok(l) => return Ok(l),
            Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => {
                if attempt == 0 {
                    tracing::warn!("端口 {addr} 暂被占用，重试绑定中…");
                }
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                last_err = Some(e);
            }
            Err(e) => return Err(e),
        }
    }
    Err(last_err.unwrap_or_else(|| std::io::Error::new(std::io::ErrorKind::AddrInUse, "绑定端口失败")))
}
