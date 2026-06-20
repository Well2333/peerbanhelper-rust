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
use pbh_geoip::{GeoIpHandle, MaxmindProvider};
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
    // 恢复上次的封禁快照（未过期的）。
    let restored = BanManager::restore_banlist(&ban_list, &db).await;
    if restored > 0 {
        tracing::info!("恢复 {restored} 条封禁快照");
    }
    // GeoIP 可选注入：从 <data>/geoip/ 加载 MaxMind mmdb；缺失则空句柄降级（ASN/地区检查跳过）。
    let geoip: GeoIpHandle =
        MaxmindProvider::load_from_dir(&paths.data_dir().join("geoip"))
            .map(|p| GeoIpHandle::from_provider(Arc::new(p)))
            .unwrap_or_else(GeoIpHandle::new_empty);
    // BTN 云端威胁情报（仅当 config.yml 启用 + 有凭证）：后台拉取规则/名单更新共享状态。
    let app_cfg = config.current().app.clone();
    let btn_mgr = std::sync::Arc::new(pbh_web::BtnManager::new(db.clone(), installation_id.clone()));
    btn_mgr.apply(&app_cfg, profile.ban_duration);
    let btn_state = btn_mgr.current_state(); // 供首次 build_modules
    let modules = build_modules(
        &profile,
        profile.ban_duration,
        &ban_list,
        &db,
        &geoip,
        &btn_state,
        &app_cfg.network.proxy,
    );
    let module_count = modules.len();
    // BTN 开启上报时跟踪 swarm（供 submit_swarm）。
    let track_swarm = app_cfg.btn.enabled && app_cfg.btn.submit;
    if track_swarm {
        let _ = db.clear_tracked_swarm().await; // 临时表,启动清空
    }
    let ban_manager = BanManager::new(
        ban_list,
        downloaders.clone(),
        modules,
        db.clone(),
        profile.ban_duration,
        &profile.ignore_peers_from_addresses,
        track_swarm,
        geoip.clone(),
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

    // GeoIP 自动下载(缺文件或过期);完成后热替换 provider。
    {
        let geoip = geoip.clone();
        let geoip_dir = ctx.paths.data_dir().join("geoip");
        let app_cfg = ctx.config.current().app.clone();
        tokio::spawn(async move {
            let client = pbh_net::build_client(&app_cfg.network.proxy, std::time::Duration::from_secs(60));
            let changed = pbh_geoip::download::ensure_databases(
                &client,
                &geoip_dir,
                app_cfg.ip_database.auto_update,
                &app_cfg.ip_database.account_id,
                &app_cfg.ip_database.license_key,
            ).await;
            if changed || !geoip.is_loaded() {
                if let Some(p) = pbh_geoip::MaxmindProvider::load_from_dir(&geoip_dir) {
                    geoip.install(std::sync::Arc::new(p) as std::sync::Arc<dyn pbh_geoip::GeoIpProvider>);
                    tracing::info!("GeoIP 库已就绪并热加载");
                }
            }
        });
    }

    // 9. 启动 Web 服务
    let web_state = pbh_web::WebState {
        config: ctx.config.clone(),
        paths: ctx.paths.clone(),
        downloaders: ctx.downloaders.clone(),
        ban_manager: ctx.ban_manager.clone(),
        db: ctx.db.clone(),
        logs: ctx.logs.clone(),
        geoip: geoip.clone(),
        btn: btn_mgr.clone(),
        geoip_lock: std::sync::Arc::new(tokio::sync::Mutex::new(())),
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

    // 等待退出信号（Ctrl-C / SIGTERM）。
    shutdown_signal().await;
    tracing::info!("收到退出信号，正在关闭…");
    ctx.ban_manager.snapshot_to_db().await; // 关闭前快照封禁
    ctx.db.close().await;
    Ok(())
}

/// 等待 Ctrl-C 或（unix）SIGTERM，用于优雅关闭（systemd/docker stop 发 SIGTERM）。
#[cfg(unix)]
async fn shutdown_signal() {
    use tokio::signal::unix::{signal, SignalKind};
    let mut term = match signal(SignalKind::terminate()) {
        Ok(s) => s,
        Err(_) => {
            tokio::signal::ctrl_c().await.ok();
            return;
        }
    };
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {}
        _ = term.recv() => {}
    }
}

#[cfg(not(unix))]
async fn shutdown_signal() {
    tokio::signal::ctrl_c().await.ok();
}

/// 生成 32 hex 字符（16 字节）随机串，用于 token / 安装 ID。
fn gen_hex_token() -> String {
    let mut bytes = [0u8; 16];
    getrandom::getrandom(&mut bytes).expect("系统随机源不可用");
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
