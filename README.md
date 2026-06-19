# PeerBanHelper-Rust

[PeerBanHelper](https://github.com/PBH-BTN/PeerBanHelper)（Java）的 Rust 重写：**单文件原生二进制、零额外部署依赖**（内置嵌入式 SQLite）、保留全部封禁相关在线功能（含 BTN 云端网络）、**直接复用现有 Vue3 前端**。

> 状态：**骨架阶段（M0 前）**。本仓库目前包含预研报告、施工指南与 Cargo workspace 骨架，业务逻辑待按 `docs/02-construction-guide.md` 实现。

## 文档

| 文件 | 内容 |
|---|---|
| `docs/01-research-report.md` | 预研与架构报告（系统总览、子系统分析、技术选型、风险） |
| `docs/02-construction-guide.md` | 施工指南（M0–M10 里程碑、任务拆解、验收标准、对拍策略） |
| `docs/03-api-contract.md` | 前端复用基准：完整 REST/WS 端点契约 |
| `docs/04-db-schema.md` | 嵌入式 SQLite 表结构与关键 SQL |

## Crate 分层

| Crate | 职责 | 里程碑 |
|---|---|---|
| `pbh-domain` | 领域类型：Peer/Torrent/PeerFlag/CheckResult/PeerAction/BanMetadata/错误 | M1 |
| `pbh-config` | `config.yml` / `profile.yml` 模型、加载、热重载、迁移链 | M0 |
| `pbh-storage` | 嵌入式 SQLite（sqlx）、迁移、各表服务、KV | M0/M5 |
| `pbh-i18n` | `TranslationComponent`、多语言加载、`{}` 占位符 | M1 |
| `pbh-rules` | 共享规则匹配引擎 + 各封禁规则模块（含 PCB） | M1/M4/M5/M6 |
| `pbh-downloader` | `Downloader` trait + qBittorrent + qBittorrentEE | M2 |
| `pbh-geoip` | MaxMind + GeoCN 查询/下载 | M6 |
| `pbh-engine` | Ban 流水线、调度循环、BanManager、解封 | M3 |
| `pbh-btn` | BTN 在线网络（ability/协议/规则同步/上报/PoW） | M8 |
| `pbh-web` | axum、StdResp、鉴权、静态/SPA、WS 日志流、控制器 | M7 |
| `pbh-notify` | Alert + Push（8 通道）+ metric | M9 |
| `pbh-server` | 组合根 + 二进制入口 | 全程 |

## 不在本期范围

AutoSTUN/NAT 穿透（留接口）、Aviator 脚本引擎（留 trait 边界，可挂 JVM sidecar）、桌面 GUI、PBH Plus 付费功能（已删，含 13 个被 gate 端点）、历史数据迁移、qB/EE 以外的下载器。

## 前端

前端 **零改动** 复用。构建：在 `webui/`（取自上游）执行 `pnpm run build`，把 `dist/*` 放入 `crates/pbh-web/static/`（或用 `rust-embed` 内嵌进二进制）。Rust 端以 axum `ServeDir` + SPA fallback 提供，`/api`、`/blocklist` 先路由。

## 开发

```bash
cargo check        # 编译检查
cargo test         # 单元/对拍测试
cargo clippy       # lint
cargo run -p pbh-server
```

上游 Java 源码克隆在 `./source/`（不入库），作为行为基准——**一切信息以源码为准，不逆向**。
