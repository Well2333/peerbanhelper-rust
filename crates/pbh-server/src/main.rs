//! pbh-server —— 组合根 + 二进制入口（`pbh`）。
//!
//! 对应上游 `Main.java` + `PeerBanHelper.java` 的启动装配（但不照搬其全局静态/Spring）。
//!
//! M0 启动顺序：解析数据目录 → 初始化日志 → 加载配置(首启生成 token) → 打开 SQLite+迁移 →
//! 装配 AppContext → 打印状态 → 干净退出。后续里程碑接 Web 服务与 Ban Wave 调度。

mod context;
mod logging;

use context::AppContext;
use pbh_config::{ConfigHandle, Paths};
use pbh_domain::LogBuffer;
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

    // 5. 装配上下文
    let ctx = AppContext {
        paths,
        config,
        db,
        logs,
    };

    // 6. 状态
    let cfg = ctx.config.current();
    tracing::info!(
        "地基就绪：监听 {}:{} | ban-wave {}ms | 默认封禁 {}ms | 安装ID {}",
        cfg.app.server.address,
        cfg.app.server.http,
        cfg.profile.check_interval,
        cfg.profile.ban_duration,
        installation_id
    );
    tracing::info!(
        "数据库 {} | 日志缓冲 {} 条",
        ctx.paths.db_file().display(),
        ctx.logs.last_seq()
    );
    tracing::info!("M0 完成。后续里程碑：M1 领域模型+规则引擎 → M2 下载器 → M3 流水线/调度 …");

    // M0 到此为止（尚未启动 Web / Ban Wave）。干净退出。
    ctx.db.close().await;
    Ok(())
}

/// 生成 32 hex 字符（16 字节）随机串，用于 token / 安装 ID。
fn gen_hex_token() -> String {
    let mut bytes = [0u8; 16];
    getrandom::getrandom(&mut bytes).expect("系统随机源不可用");
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
