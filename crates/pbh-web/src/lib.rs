//! pbh-web —— 自研极简 HTTP API + 内置单页。对应方案 `memory/design/roadmap.md` §1。
//!
//! 不复刻上游 Vue/StdResp/Gson;Bearer token 鉴权;内置 vanilla 单页(`include_str!` 内嵌)。

mod envelope;
mod routes;

use std::net::SocketAddr;
use std::sync::Arc;

use pbh_config::ConfigHandle;
use pbh_domain::LogBuffer;
use pbh_downloader::DownloaderManager;
use pbh_engine::BanManager;
use pbh_geoip::GeoIpProvider;
use pbh_storage::Db;

pub use envelope::{ApiResp, Page};

/// Web 层共享状态。
#[derive(Clone)]
pub struct WebState {
    pub config: ConfigHandle,
    pub downloaders: Arc<DownloaderManager>,
    pub ban_manager: Arc<BanManager>,
    pub db: Db,
    pub logs: Arc<LogBuffer>,
    /// GeoIP 可选注入（供 profile 热重载时重建 IPBlackList）。
    pub geoip: Option<Arc<dyn GeoIpProvider>>,
}

/// 启动 HTTP 服务（阻塞直到出错/关闭）。
pub async fn serve(state: WebState, addr: SocketAddr) -> std::io::Result<()> {
    let app = routes::router(state);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!("Web 已监听 http://{addr}");
    axum::serve(listener, app).await
}
