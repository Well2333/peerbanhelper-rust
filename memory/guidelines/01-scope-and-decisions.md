# 项目范围与已确认决策（v2 · 极简重构）

> 本文件是**范围/决策的权威单一来源**。路线图与里程碑见 `memory/design/roadmap.md`;架构约定见 `02-architecture.md`。

## 目标（不可漂移）

把 [PeerBanHelper](https://github.com/PBH-BTN/PeerBanHelper)（Java）**完全重构、最精简**地重写为 Rust:
**单文件原生二进制、零额外部署依赖**(内置嵌入式 SQLite)、达到与原版**一致的封禁效果**。
保留完整封禁能力(全部高价值规则 + 完整 BTN + 封禁历史),弃用现有 Vue 前端,自研极简 API + 内置单页。

上游 Java 源码克隆于 `./source/`,是行为基准。**一切信息以源码为准,禁止逆向二进制。**
**不照搬上游结构**(Spring DI / 全局 service-locator / "organ" 隐喻 / 每 wave 新建线程池 / 多余抽象),只在有明确收益处分层。

## 四项已确认决策（用户拍板）

1. **效果 = 封禁判定基本一致** → 保留全部高价值检测规则(PCB、IPBlackList、IPBlackRuleList、
   ClientNameBlacklist、PeerIdBlacklist、AntiVampire、AutoRangeBan、MultiDialingBlocker、
   IdleConnectionDosProtection、PTRBlacklist)。
2. **BTN = 完整保留**(下行 denylist/allowlist/rules + 上行 submit bans/swarm/history + PoW + 游标)。
   ⇒ 为喂上行,轻量保留 PeerRecording / SwarmTracking 采集。
3. **前端 = 弃用现有 Vue** → 自研极简 REST/JSON API + 内置轻量单页(vanilla,rust-embed,无构建链)。
   ⇒ 不复刻 Java 的 StdResp/Gson/SPA/OOBE/fail2ban/~90 端点;**砍除 i18n,后端单语言纯字符串**。
4. **持久化 = 保留封禁历史 + 日志**(+ PCB 状态 + 封禁快照 + BTN 所需 peer_records/tracked_swarm)。

## 完全移除（与封禁 peer 无关 / 纯外围 / 已死 / 付费）

- **脚本引擎（Aviator / ExpressionRule，JVM 限定）：完全删除**，不保留 trait 边界、不挂 sidecar。
- **AutoSTUN / NAT 穿透 / TCP 转发 / UPnP 端口映射：完全删除**，不留接口;peer 地址直接用下载器返回的原始 `ip:port`，不做 NAT 改写(去掉 `natTranslate`)。
- 整套 Vue 前端及其专属 API 契约;i18n/TranslationComponent;图表/会话分析/客户端分析;
  SessionAnalyse/ActiveMonitoring(图表/限速);表 `traffic_journal_v3`、`peer_connection_metrics(+track)`、`alert`(降级为日志);
  推送通知(8 通道);PF4J 插件、Laboratory、桌面 GUI、MTR、平台原生、多数据库后端、PBH Plus。

## 明确不做

qB/qBEE 以外的下载器(保留 trait+工厂可扩展);历史数据迁移;MySQL/PostgreSQL/H2(仅嵌入式 SQLite)。

## 下载器 trait（v2 精简）

只保留与封禁相关的能力：login / getTorrents / getPeers / setBanList(Normal+ShadowBan) / featureFlags / 并发槽 / 基础状态。
**去掉** speed-limiter、BTProtocolPort(listen_port)、natTranslate、traffic statistics 等（原服务于已砍的限速/AutoSTUN/图表）。

## 精简后表集

保留:`pcb_address`、`pcb_range`、`banlist`、`history`、`rule_sub_info`、`rule_sub_log`、`metadata`、
`peer_records`、`tracked_swarm`。砍除:`traffic_journal_v3`、`peer_connection_metrics(+track)`、`alert`。

## 新版极简 API（要点）

Bearer token 鉴权;自有信封 `{ ok, data, error }`;约 18 个端点;内置 vanilla 单页。详见 `memory/design/roadmap.md` §1。

## 技术选型

`tokio` / `axum`+`tower-http` / `reqwest` / `sqlx`(sqlite) / `serde`(json,yaml) / IP trie `ip_network_table` /
GeoIP `maxminddb`+`xz2`+`csv`(可选注入) / DNS `hickory-resolver` / `moka`/`dashmap` / `regex` / `chrono` /
`flate2`+`sha2`(BTN) / `tracing` / `rust-embed`(内嵌单页)。**不含** i18n、lettre、pulldown-cmark、STUN、脚本引擎相关依赖。
