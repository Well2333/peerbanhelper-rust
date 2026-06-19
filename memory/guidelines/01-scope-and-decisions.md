# 项目范围与已确认决策

## 目标（不可漂移）

把 [PeerBanHelper](https://github.com/PBH-BTN/PeerBanHelper)（Java）重写为 Rust：
**单文件原生二进制、零额外部署依赖**（内置嵌入式 SQLite）、保留全部封禁相关在线功能、**直接复用现有 Vue3 前端**、在现有下载器上的基础使用体验**完全一致**。

上游 Java 源码克隆于 `./source/`，是行为基准。**一切信息以源码为准，禁止逆向二进制。**

## 三项已确认决策（用户拍板，不再反复确认）

1. **AutoSTUN / NAT 穿透：本期不做，预留接口。** 保留 `NatAddressProvider` 抽象与 `auto-stun` 配置位；前端 AutoSTUN 页面返回「不可用」占位，不删端点以免前端崩。
2. **PBH Plus 付费功能：整体删除，含被 gate 的 13 个端点。** 删除 `pbhplus/` 包、`PBHPlusController`、`Role::PbhPlus`、`RequirePBHPlusLicenseException`，以及 `PBHChartController`(7) + `PBHPeerController` 的 access/banHistory(2) + `PBHTorrentController` 的 access/banHistory(2)。底层数据表与 `/api/statistic/*` **保留**，未来可重启用。manifest 不宣告这些模块，前端据此隐藏。
3. **交付：报告 + 施工指南 + Rust 项目骨架**（已完成于 `docs/` 与 `crates/`）。

## 明确不在本期范围

- AutoSTUN / NAT 穿透（留接口）
- Aviator 脚本引擎实现（ExpressionRule **stub**，留 trait 边界，可挂未来 JVM sidecar）
- 桌面 GUI（仅保留 headless / console）
- PBH Plus 付费功能（已删）
- 历史数据迁移（无需兼容旧 PBH 数据）
- qBittorrent / qBittorrentEE 以外的下载器（Transmission/BiglyBT/BitComet/Deluge 不做，但保留 trait + 工厂以便扩展）
- MySQL/PostgreSQL/H2（仅嵌入式 SQLite）

## 技术选型（稳定决策，见 docs/01 §4 全表）

异步 `tokio`；Web `axum`+`tower-http`；HTTP 客户端 `reqwest`；DB `sqlx`(sqlite)（备选 `rusqlite`）；序列化 `serde`/`serde_json`/`serde_yaml`（**字段名须对齐上游 Gson**）；IP trie `ip_network_table`/`treebitmap`；GeoIP `maxminddb`+`xz2`+`csv`；DNS `hickory-resolver`；缓存 `moka`/`dashmap`；正则 `regex`；时间 `chrono`(Local，保持分桶边界一致)；SMTP `lettre`；Markdown `pulldown-cmark`；日志 `tracing`；前端内嵌 `rust-embed`(可选)。
