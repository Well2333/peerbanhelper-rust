//! pbh-server —— 组合根 + 二进制入口（`pbh`）。
//!
//! 对应上游 `Main.java` + `PeerBanHelper.java` 的启动装配（但不照搬其全局静态/Spring）。
//!
//! M0 启动顺序：解析数据目录 → 初始化日志 → 加载配置(首启生成 token) → 打开 SQLite+迁移 →
//! 装配 AppContext → 打印状态 → 干净退出。后续里程碑接 Web 服务与 Ban Wave 调度。

mod context;
mod logging;

use std::sync::Arc;

use context::AppContext;
use pbh_config::{ConfigHandle, Paths};
use pbh_domain::LogBuffer;
use pbh_downloader::DownloaderManager;
use pbh_engine::{build_modules, BanList, BanManager};
use pbh_geoip::{GeoIpProvider, MaxmindProvider};
use pbh_storage::Db;

const VERSION: &str = env!("CARGO_PKG_VERSION");

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 1. 数据目录
    let paths = Paths::from_env();
    paths.ensure_dirs()?;

    // 2. 日志（控制台 + 文件 + 环形缓冲）
    let logs = LogBuffer::new(5000);
    let _log_guard = logging::init(logs.clone(), &paths.logs_dir());
    tracing::info!(
        "PeerBanHelper-Rust v{VERSION} 启动（数据目录 {}）",
        paths.data_dir().display()
    );

    // 3. 配置（缺失则写默认）
    let config = ConfigHandle::init(paths.clone())?;

    // 3a. 首启生成 API token 并写回 config.yml
    {
        let app = config.current().app.clone();
        if app.server.token.is_empty() {
            let token = gen_hex_token();
            let mut app = app;
            app.server.token = token.clone();
            pbh_config::save_app(&paths, &app)?;
            config.reload()?;
            tracing::warn!("首次启动：已生成 API token 并写入 config.yml → {token}");
        }
    }

    // 4. SQLite（建库 + 迁移）
    let db = Db::open(&paths.db_file()).await?;

    // 4a. 安装 ID（KV 演示 + 后续遥测/标识用）
    let installation_id = match db.meta_get("installation-id").await? {
        Some(v) => v,
        None => {
            let id = gen_hex_token();
            db.meta_set("installation-id", &id).await?;
            id
        }
    };

    // 5. 下载器管理器 + 规则模块 + BanManager
    let profile = config.current().profile.clone();
    let downloaders = Arc::new(DownloaderManager::load(
        paths.config_file("downloaders.yml"),
    ));
    let ban_list = Arc::new(BanList::new());
    // GeoIP 可选注入：从 <data>/geoip/ 加载 MaxMind mmdb；缺失则降级（ASN/地区检查跳过）。
    let geoip: Option<Arc<dyn GeoIpProvider>> =
        MaxmindProvider::load_from_dir(&paths.data_dir().join("geoip"))
            .map(|p| Arc::new(p) as Arc<dyn GeoIpProvider>);
    // BTN 云端威胁情报（仅当 config.yml 启用 + 有凭证）：后台拉取规则/名单更新共享状态。
    let app_cfg = config.current().app.clone();
    let btn_state: Option<pbh_btn::SharedBtnState> = if app_cfg.btn.enabled {
        let state = pbh_btn::new_state();
        pbh_btn::spawn(
            pbh_btn::BtnRuntimeConfig {
                config_url: app_cfg.btn.config_url.clone(),
                app_id: app_cfg.btn.app_id.clone(),
                app_secret: app_cfg.btn.app_secret.clone(),
                installation_id: installation_id.clone(),
                submit: app_cfg.btn.submit,
                ban_duration: profile.ban_duration,
            },
            db.clone(),
            state.clone(),
        );
        tracing::info!("BTN 已启用,后台调度启动");
        Some(state)
    } else {
        None
    };
    let modules = build_modules(
        &profile,
        profile.ban_duration,
        &ban_list,
        &db,
        &geoip,
        &btn_state,
    );
    let module_count = modules.len();
    let ban_manager = BanManager::new(
        ban_list,
        downloaders.clone(),
        modules,
        db.clone(),
        profile.ban_duration,
        &profile.ignore_peers_from_addresses,
    );

    // 6. 装配上下文
    let ctx = AppContext {
        paths,
        config,
        db,
        logs,
        downloaders,
        ban_manager,
    };

    // 7. 状态
    let cfg = ctx.config.current();
    tracing::info!(
        "就绪：监听 {}:{} | ban-wave {}ms | 默认封禁 {}ms | 安装ID {}",
        cfg.app.server.address,
        cfg.app.server.http,
        cfg.profile.check_interval,
        cfg.profile.ban_duration,
        installation_id
    );
    tracing::info!(
        "下载器 {} 个 | 启用规则模块 {} 个 | 数据库 {} | 日志缓冲 {} 条",
        ctx.downloaders.count(),
        module_count,
        ctx.paths.db_file().display(),
        ctx.logs.last_seq()
    );

    // 8. 启动 Ban Wave 循环
    let _wave = ctx
        .ban_manager
        .clone()
        .spawn_loop(cfg.profile.check_interval as u64);

    // 9. 启动 Web 服务
    let web_state = pbh_web::WebState {
        config: ctx.config.clone(),
        downloaders: ctx.downloaders.clone(),
        ban_manager: ctx.ban_manager.clone(),
        db: ctx.db.clone(),
        logs: ctx.logs.clone(),
        geoip: geoip.clone(),
        btn_state: btn_state.clone(),
    };
    let bind = format!("{}:{}", cfg.app.server.address, cfg.app.server.http);
    match bind.parse::<std::net::SocketAddr>() {
        Ok(addr) => {
            tokio::spawn(async move {
                if let Err(e) = pbh_web::serve(web_state, addr).await {
                    tracing::error!("Web 服务错误: {e}");
                }
            });
            tracing::info!(
                "界面: http://{} （token 见上方日志）。按 Ctrl-C 退出。",
                bind
            );
        }
        Err(e) => tracing::error!("监听地址无效 {bind}: {e}"),
    }

    // 等待退出信号。
    tokio::signal::ctrl_c().await.ok();
    tracing::info!("收到退出信号，正在关闭…");
    ctx.db.close().await;
    Ok(())
}

/// 生成 32 hex 字符（16 字节）随机串，用于 token / 安装 ID。
fn gen_hex_token() -> String {
    let mut bytes = [0u8; 16];
    getrandom::getrandom(&mut bytes).expect("系统随机源不可用");
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
