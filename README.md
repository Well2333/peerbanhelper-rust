# PeerBanHelper-Rust

[PeerBanHelper](https://github.com/PBH-BTN/PeerBanHelper)（Java）的 **完全重构、最精简** Rust 重写：**单文件原生二进制、零额外部署依赖**（内置嵌入式 SQLite）、达到与原版**一致的封禁效果**。保留完整封禁能力（全部高价值规则 + 完整 BTN 云端网络 + 封禁历史），**弃用原 Vue 前端**，改为自研极简 REST API + 内置轻量单页。

> 状态：**骨架阶段（M0 前）**。当前包含分析、战略与 Cargo workspace 骨架，业务逻辑待按 `docs/02-roadmap.md` 实现。

## 文档（按阅读顺序）

| 文件 | 内容 |
|---|---|
| `docs/最高优先级工作守则.md` | **流程总纲（最高优先级）**：仓库记忆、分支/提交、验证 |
| `docs/02-roadmap.md` | **权威方案**：v2 战略、范围(保留/砍除)、自研 API、M0–M9 里程碑与验收、对拍策略 |
| `docs/01-architecture-analysis.md` | 上游系统与子系统的事实分析（建库时查源码基准用） |
| `docs/03-db-schema.md` | 嵌入式 SQLite 表结构（v2 精简表集）与关键 SQL |

> 长期规范沉淀在 `memory/guidelines/`，变更记录在 `memory/changelog/`。

## Crate 分层

| Crate | 职责 | 里程碑 |
|---|---|---|
| `pbh-domain` | 领域类型：Peer/Torrent/PeerFlag/CheckResult/PeerAction/BanMetadata/错误 | M1 |
| `pbh-config` | `config.yml` / `profile.yml` 模型、加载、热重载、迁移链 | M0 |
| `pbh-storage` | 嵌入式 SQLite（sqlx）、迁移、各表服务、KV | M0/M5 |
| `pbh-rules` | 共享规则匹配引擎 + 各封禁规则模块（含 PCB） | M1/M4/M5/M6 |
| `pbh-downloader` | `Downloader` trait + qBittorrent + qBittorrentEE | M2 |
| `pbh-geoip` | MaxMind + GeoCN 查询/下载 | M6 |
| `pbh-engine` | Ban 流水线、调度循环、BanManager、解封 | M3 |
| `pbh-btn` | BTN 在线网络（ability/协议/规则同步/上报/PoW） | M8 |
| `pbh-web` | axum、自研极简 API + 信封、Bearer 鉴权、WS 日志流、内置单页、blocklist 导出 | M7 |
| `pbh-notify` | Alert + Push（8 通道）+ metric | M9 |
| `pbh-server` | 组合根 + 二进制入口 | 全程 |

## 不在本期范围（v2 精简）

原 Vue 前端及其专属 API 契约、i18n、图表/分析、推送通知、Alert 独立系统、**AutoSTUN/NAT 穿透/UPnP（完全移除）**、**Aviator 脚本引擎/ExpressionRule（完全移除）**、桌面 GUI、PBH Plus、历史数据迁移、qB/EE 以外的下载器、多数据库后端。

## 前端（自研极简）

弃用原 Vue 前端。改为 **自研极简 REST/JSON API + 内置轻量单页**（vanilla HTML/JS，无构建工具链，`rust-embed` 内嵌进二进制，单文件部署）。覆盖：状态、下载器管理、封禁列表/历史、实时日志、规则与订阅配置。API 设计见 `docs/02-roadmap.md` §4。

## 开发

```bash
cargo check        # 编译检查
cargo test         # 单元/对拍测试
cargo clippy       # lint
cargo run -p pbh-server
```

上游 Java 源码克隆在 `./source/`（不入库），作为行为基准——**一切信息以源码为准，不逆向**。
