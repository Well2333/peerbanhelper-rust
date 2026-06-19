//! pbh-server —— 组合根 + 二进制入口（`pbh`）。
//!
//! 对应 Java `Main.java` + `PeerBanHelper.java` 的启动装配。
//!
//! 启动顺序（M0 起逐步落地，保持 Java 的有序串行初始化）：
//! 1. 解析数据目录（data/config/logs/persist）
//! 2. 加载 config.yml / profile.yml（含版本迁移）
//! 3. 初始化 tracing 日志（文件 + 控制台 + 环形缓冲供 WS）
//! 4. 打开 SQLite + 迁移
//! 5. 装配各子系统（显式注入抽象：DownloaderManager / BanManager / 模块表 / BTN / Web）
//! 6. 启动 Web 服务 + Ban Wave 调度循环
//!
//! 骨架阶段：仅打印横幅并触达各 crate 的公共类型，验证 workspace 可链接（离线可跑）。

use pbh_config::{AppConfig, ProfileConfig};

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn main() {
    let app = AppConfig::default();
    let profile = ProfileConfig::default();

    println!("PeerBanHelper-Rust v{VERSION} (骨架阶段 / scaffold)");
    println!(
        "  默认监听: {}:{}  | ban-wave 间隔: {}ms | 默认封禁: {}ms",
        app.server_address, app.server_http, profile.check_interval_ms, profile.ban_duration_ms
    );
    println!("  下一步: 按 docs/05-revised-strategy.md（v2 极简重构）从 M0 开始实现。");

    // 触达各子系统公共符号，确保骨架可链接（不构成运行逻辑）。
    let _ = pbh_btn::PROTOCOL_READABLE_VERSION;
    let _ = pbh_web::Role::Anyone;
    let _ = pbh_storage::DB_RELATIVE_PATH;
    let _ = pbh_engine::IdentityNatProvider;
    let _ = pbh_geoip::IpGeoData::default();
    let _ = pbh_rules::matcher::RuleSet::default();
    let _ = pbh_downloader::create_downloader_stub("qbittorrent");
    let _ = pbh_domain::PeerAction::default();
}
