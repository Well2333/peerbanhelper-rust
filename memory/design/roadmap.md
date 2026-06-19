# PeerBanHelper-Rust 路线图与施工指南

> 日期：2026-06-19 ・ v2（极简重构）
>
> **范围、保留/砍除、四项决策** → 见 `memory/guidelines/01-scope-and-decisions.md`（权威）。
> **架构约定**（crate 分层、数据流、流水线、可选注入） → 见 `memory/guidelines/02-architecture.md`（权威）。
> **上游子系统事实分析** → `memory/design/architecture-analysis.md`。**库表** → `memory/design/db-schema.md`。
> 本文聚焦：**新版 API 设计 + 里程碑 + 验收 + 对拍策略**。

一句话范围：保留完整封禁能力（全部高价值规则 + 完整 BTN + 封禁历史），自研极简 API + 内置单页;
**完全移除**脚本引擎、AutoSTUN、图表/分析/通知等与封禁 peer 无关或纯外围的内容。

---

## 1. 新版极简 REST API（自研，清晰为先）

- **鉴权：** Bearer token（配置文件设定;首启自动生成并打印一次）。简单字符串比对，无会话 cookie、无 fail2ban 复杂逻辑。
- **信封：** `{ "ok": bool, "data": <any>, "error": <string|null> }`。分页 data：`{ "page", "size", "total", "items" }`，请求 `?page=`(默认1)`&pageSize=`(默认20)。
- **端点（约 18 个）：**
  - `GET /api/status` — 运行状态、版本、各模块开关、BTN 概览、暂停标志
  - `GET/PUT /api/downloaders` ・ `PATCH/DELETE /api/downloaders/{id}` ・ `POST /api/downloaders/test`
  - `GET /api/downloaders/{id}/torrents` ・ `GET /api/downloaders/{id}/torrent/{hash}/peers`
  - `GET /api/bans` (当前封禁,分页) ・ `PUT /api/bans` (手动封) ・ `DELETE /api/bans` (解封)
  - `GET /api/bans/history` (封禁历史,分页/过滤)
  - `GET/PUT /api/config/profile` (规则与全局配置) ・ `POST /api/config/reload`
  - `GET/PUT/DELETE /api/sub/rules[/{id}]` ・ `POST /api/sub/rules/update` (IP 订阅规则)
  - `GET /api/btn/status`
  - `GET /api/logs` (历史) ・ `WS /api/logs/stream?token=&offset=` (实时)
  - `GET /blocklist/{ip,p2p-plain-format,dat-emule}` (纯文本,供下载器/外部消费)
- **内置单页 UI：** vanilla HTML/JS（无构建工具链），`rust-embed` 内嵌。覆盖：状态、下载器增删改、封禁列表/历史、实时日志、规则与订阅配置。后续可替换，不影响 API。

> 因弃用上游前端，API/UI **不要求与原版一致**，以清晰、好测为准。

---

## 2. 持久化

保留表：`pcb_address`、`pcb_range`、`banlist`、`history`、`rule_sub_info`、`rule_sub_log`、`metadata`、`peer_records`、`tracked_swarm`（后两者供 BTN 上行）。完整定义、pragma、关键 upsert 见 `memory/design/db-schema.md`。

---

## 3. 里程碑路线图（含任务拆解与验收）

> 开发纪律见守则;关键等价性逻辑（PCB、规则引擎、IP 规范化、下载器封禁串、BTN 序列化）**先写测试**。
> 关键路径 M0→M1→M2→M3;M4/M5/M6 在 M3 后可部分并行;M7 可早搭;M8 依赖 M3(history)+M7。

### M0 — 地基 ✅ 已完成
- ✅ `pbh-config`：config/profile serde 模型 + 加载(缺失写默认) + `tokio::sync::watch` 热重载 + 目录解析(`Paths`)。
- ✅ `pbh-storage`：`sqlx` SQLite(WAL/NORMAL/busy_timeout=60000/mmap_size)，写池 `max_connections(1)`;`migrations/0001_initial.sql`(精简表集);KV `metadata`。
- ✅ `tracing` 日志（控制台 + 按日文件 + 环形缓冲 `pbh-domain::LogBuffer` 供 WS）;`AppContext` 组合根。
- ✅ 二进制：解析目录→日志→配置(首启生成 token 写回)→SQLite+迁移→安装ID→状态→干净退出。
- ✅ **验收达成**：24 单测全绿(config/storage/domain/...)，二进制两次启动验证 token/安装ID 持久化;clippy+fmt 零告警。
- 🔧 **工具链**：环境原 cargo 1.75 无法编译现代依赖(edition2024) → 已装 rustup `stable`(1.96)，构建用 `~/.cargo/bin/cargo`。
- ⏭ 留待后续：版本迁移链(注释保留 R4)目前仅 serde 重生成;file 日志轮转保留默认。

### M1 — 领域模型 + 规则引擎 ✅ 已完成
- ✅ 领域类型：Peer/PeerAddress/Torrent/PeerFlag、CheckResult/PeerAction(优先级合并)、BanMetadata。
- ✅ 共享匹配引擎 `pbh-rules`：`Matcher`(STARTS_WITH/ENDS_WITH/CONTAINS/EQUALS/LENGTH/REGEX)+ `RuleSet`(FALSE 短路优先级)+ JSON `RuleSet::parse`;`IpMatcher`(CIDR 最长前缀,ip_network_table);`ModuleMatchCache`(moka,含 pass-only 写)。
- ✅ `BanList`(`pbh-engine`)：双栈前缀 trie + RwLock,ban/unban/get/contains/remove_expired/snapshot,含 ban_for_disconnect 元数据。**纯字符串**承载 rule/description(无 i18n)。
- ✅ **验收达成**：37 单测全绿(matcher 8 / ip_matcher 3 / cache 2 / ban_list 4 + M0)。
- ⏭ 留待后续：PeerFlag 只解析模块实际用到的 interest 位(BTN 用原始串,无需全位重建);BanMetadata 的 serde/chrono 等到 M3(banlist 快照落库)再加。

### M2 — 下载器（qB + qBEE）✅ 已完成（HTTP 部分待真机验证）
- ✅ `Downloader` trait（v2 精简：login/get_torrents/get_peers/apply_ban_list/feature_flags/is_paused）+ `build_downloader` 工厂。
- ✅ `QBittorrentClient`（reqwest：cookie SID + basic-auth + api-key Bearer + verify-ssl + gzip + UA）;
  登录(密码/api-key 双模式 + EE shadowban test + 多连接副作用);分页 torrents;`/sync/torrentPeers` peer 解析与过滤;
  封禁全量(`banned_IPs`/`shadow_banned_IPs`)与增量(`banPeers`/`shadowbanPeers`);RANGE_BAN 版本门控;IP 规范化。
- ✅ **纯逻辑验收**：9 单测(封禁串全量/增量/CIDR 门控/IPv6 压缩、config 往返、版本解析、peer JSON、工厂)。
- ⏳ **待真机验证**（记入待测报告）：对真实 qB/EE 登录、拉 peer、封禁写入可见;封禁串与上游逐字节对拍。
- ⏭ completed_size 暂为 -1（M5 PCB 经 `/torrents/properties` 补）;DownloaderManager(持久化/列表)在 M3 装配。

### M3 — 流水线 + 调度 + BanManager ✅ 已完成（首版顺序流水线）
- ✅ `BanManager`：run_once(到期解封→每下载器:登录→拉 torrents→拉 peers→逐 peer 跑模块→命中即封→历史落库→下发) + `spawn_loop`(固定延迟 + AtomicBool 防重叠) + 旁路名单(IpMatcher)。
- ✅ `DownloaderManager`(pbh-downloader)：YAML 持久化 + 列表 + upsert/remove。
- ✅ `pbh-storage` 表助手：`upsert_torrent` / `insert_ban_history` / `query_ban_history` / `count_ban_history`。
- ✅ 装配到二进制：加载下载器 + 构建模块 + 启动 ban wave + Ctrl-C 干净退出。**二进制可运行**。
- ✅ **验收达成**：49 单测全绿;二进制实跑(0 下载器 3 模块,wave 启动/退出正常)。
- ⏭ 留待后续：channel 并行流水线 + WatchDog + 每小时快照(首版顺序执行,够用);真实 qB 封禁端到端见待测报告。

### M4 — 规则模块（离线）⏳ 进行中（3/7 已落地）
- ✅ PeerIdBlacklist、ClientNameBlacklist、AntiVampire（含内置默认名单,开箱即用）。
- ⏭ 待补：AutoRangeBan（依赖 BanList,放 pbh-engine）、IdleConnectionDosProtection、MultiDialingBlocker、PTRBlacklist。
- **验收：** 每模块单测覆盖阈值与配置。

### M5 — ProgressCheatBlocker
- `pcb_address`+`pcb_range` 两表、脏标志 + `moka` LRU(1024/180s) + 驱逐批刷;`shouldBanPeer` 精确短路顺序（上传增量→computedUploaded=max→fastPcbTest→excessive→difference(ban-delay 窗口)→rewind）;8h 清理;订阅解封事件。
- **验收：** 专项序列回放套件逐子检查对拍;持久化重启续算;ban-delay 状态机;解封重置。

### M6 — GeoIP + IP 黑名单族
- `maxminddb` 读 City/ASN/GeoCN + 下载/解压/原子替换 + GeoCN 解析 + 行政区划 trie + `IpGeoData` + 叠加 + TW/HK/MO 命名 + moka;**可选注入**(缺失则降级)。
- IPBlackList、IPBlackRuleList（下载/SHA-256/格式解析/前缀 trie/`rule_sub_log`/定时刷新/磁盘回退）。
- **验收：** GeoIP 对已知 IP 对拍;订阅格式解析单测;trie 命中;更新日志入库。

### M7 — 极简 Web（自研）
- `axum`：`ApiResp{ok,data,error}` 信封、分页、Bearer 鉴权、约 18 端点;WS `/api/logs/stream`;内置 vanilla 单页（`rust-embed`）;`/blocklist` 导出。
- **验收：** 单页能登录、看状态、增删下载器、查看/手动封禁、查历史、实时日志、改规则/订阅;接口常规测试。

### M8 — BTN（完整）
- HTTP 中间件（固定头 + Bearer + gzip 上行）、config 拉取、new/legacy 分支;下行 HeartBeat/Rules/IPDenyList/IPAllowList(+解封)/IpQuery/Reconfigure;上行 SubmitBans/SubmitSwarm/SubmitHistory（DB 游标 + KV 续传）;轻量 PeerRecording/SwarmTracking 喂数据;PoW;`BtnNetworkOnline`（Allow→SKIP / Deny→BAN / Rules 分类，**无脚本分支**）;每 ability tokio 任务;600s 重试。
- **验收：** config/规则下行/心跳/IP 名单;上行 gzip 报文字段对拍;游标续传;PoW 通过。

### M9 — 收尾
- 单文件打包（`rust-embed` 内嵌单页）、配置随包、端到端验收、性能基线、文档。

---

## 4. 等价性对拍策略

对以下产出建 golden fixture 并对拍：
1. **下载器封禁写入串**（`banned_IPs`/`peers`/`shadow_banned_IPs`，含 IPv6 规范化、CIDR、shadowban）逐字节一致。
2. **规则引擎判定**（`profile.yml` 默认规则 + 一批 peer）命中一致。
3. **BTN 上下行报文**（解 gzip 后字段/类型）一致。
4. **PCB 序列回放** → 相同决策与 DB 状态。

> 自有 API/UI 不在对拍范围，走常规接口/集成测试。

---

## 5. 端到端验收（最终）

- [ ] 单文件二进制启动，无需任何外部数据库/服务。
- [ ] 首启生成 token，内置单页可登录并添加 qBittorrent。
- [ ] qB + qBEE 均能登录、拉 peer、下发封禁并在 qB 端可见（含 EE shadowban）。
- [ ] 全部高价值规则 + PCB + BTN（下行+上行）按 `profile.yml` 默认工作。
- [ ] 封禁串/规则判定与上游抽样对拍一致。
- [ ] 内置单页可用：状态/下载器/封禁列表+历史/实时日志/规则+订阅配置。
- [ ] 24h 连续运行无内存泄漏、无致命 SQLITE_BUSY、到期解封正常。
