//! 组合根上下文。各子系统通过 `Arc<AppContext>`（或其中字段）协作，取代上游的 Spring DI / 全局 service-locator。

use std::sync::Arc;

use pbh_config::{ConfigHandle, Paths};
use pbh_domain::LogBuffer;
use pbh_storage::Db;

/// 全局运行期上下文。后续里程碑在此追加 BanManager / DownloaderManager / 模块表 / BTN 等。
#[derive(Clone)]
pub struct AppContext {
    pub paths: Paths,
    pub config: ConfigHandle,
    pub db: Db,
    pub logs: Arc<LogBuffer>,
}
