//! BTN 热启停管理:保存 config.yml 后按 enabled/凭证/代理变化中止并重启后台调度。

use std::sync::Mutex;

use pbh_btn::{BtnRuntimeConfig, SharedBtnState};
use pbh_storage::Db;

pub struct BtnManager {
    db: Db,
    installation_id: String,
    inner: Mutex<Option<(tokio::task::AbortHandle, SharedBtnState)>>,
}

impl BtnManager {
    pub fn new(db: Db, installation_id: String) -> Self {
        BtnManager { db, installation_id, inner: Mutex::new(None) }
    }

    /// 当前共享状态(仅启用且运行时为 Some),供 build_modules 决定是否构建 BtnNetworkOnline。
    pub fn current_state(&self) -> Option<SharedBtnState> {
        self.inner.lock().unwrap().as_ref().map(|(_, s)| s.clone())
    }

    /// 停止现有调度。
    pub fn stop(&self) {
        if let Some((handle, _)) = self.inner.lock().unwrap().take() {
            handle.abort();
            tracing::info!("BTN 调度已停止");
        }
    }

    /// 按新配置应用:enabled=false → 停;否则停旧再以新 proxy/凭证起新。
    pub fn apply(&self, app: &pbh_config::AppConfig, ban_duration: i64) {
        self.stop();
        if !app.btn.enabled {
            return;
        }
        let state = pbh_btn::new_state();
        let handle = pbh_btn::spawn(
            BtnRuntimeConfig {
                config_url: app.btn.config_url.clone(),
                app_id: app.btn.app_id.clone(),
                app_secret: app.btn.app_secret.clone(),
                installation_id: self.installation_id.clone(),
                submit: app.btn.submit,
                ban_duration,
                proxy: app.network.proxy.clone(),
            },
            self.db.clone(),
            state.clone(),
        );
        *self.inner.lock().unwrap() = Some((handle, state));
        tracing::info!("BTN 调度已启动");
    }
}

impl Drop for BtnManager {
    fn drop(&mut self) {
        self.stop();
    }
}
