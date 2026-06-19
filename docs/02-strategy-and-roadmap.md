# PeerBanHelper-Rust 战略与路线图（权威）

> 版本：v2 ・ 日期：2026-06-19 ・ **本文件是当前唯一权威的方案与施工依据。**
> 配套：`01-architecture-analysis.md`（上游子系统事实分析）、`03-db-schema.md`（库表）、`最高优先级工作守则.md`（流程）。

---

## 1. 定位

把 [PeerBanHelper](https://github.com/PBH-BTN/PeerBanHelper)（Java）**完全重构、最精简**地重写为 Rust：
**单文件原生二进制、零额外部署依赖**（内置嵌入式 SQLite），达到与原版**一致的封禁效果**。

**一句话范围：** 保留完整封禁能力（全部高价值规则 + 完整 BTN + 封禁历史），**弃用沉重的 Vue 前端及其专属契约**，改自研极简 API + 内置单页;砍掉一切只为图表/分析/桌面服务的外围。

上游 Java 源码克隆于 `./source/`，是行为基准。**一切信息以源码为准，禁止逆向二进制。**

---

## 2. 四项决策（用户已确认）

1. **效果 = 封禁判定基本一致** → 保留全部高价值检测规则。
2. **BTN = 完整保留**（下行 denylist/allowlist/rules + 上行 submit bans/swarm/history + PoW + 游标）。为喂上行，轻量保留 PeerRecording / SwarmTracking 采集。
3. **前端 = 弃用现有 Vue** → 自研极简 REST/JSON API + 内置轻量单页（vanilla，`rust-embed` 内嵌，无构建工具链）。不复刻 Java 的 StdResp/Gson/SPA/OOBE/fail2ban/~90 端点;**砍 i18n，后端单语言纯字符串**。
4. **持久化 = 保留封禁历史 + 日志**（+ PCB 状态 + 封禁快照 + BTN 所需 peer_records/tracked_swarm）。

---

## 3. 保留 / 砍除 总表

### ✅ 保留（完整封禁能力）
| 部分 | 说明 |
|---|---|
| 主循环引擎 | 登录→拉 torrents→拉 peers→规则检查→下发封禁→到期解封 |
| qBittorrent + qBittorrentEE | 封禁下发字节级一致（banned_IPs / banPeers / shadowban）;保留 trait+工厂可扩展 |
| 全部高价值规则 | PCB、IPBlackList、IPBlackRuleList(订阅)、ClientNameBlacklist、PeerIdBlacklist、AntiVampire、AutoRangeBan、MultiDialingBlocker、IdleConnectionDosProtection、PTRBlacklist |
| BTN（完整） | 全部 ability、PoW、gzip、游标;下行 + 上行 |
| GeoIP（可选注入） | MaxMind + GeoCN;供 IPBlackList 的 ASN/地区/网络类型封禁;mmdb 缺失则降级 |
| 持久化（精简表集） | pcb_address/pcb_range、banlist、history、rule_sub_info/log、metadata、peer_records、tracked_swarm |
| 极简 Web | 自研 REST API + Bearer 鉴权 + WS 日志流 + 内置单页 + /blocklist 导出 |

### ❌ 砍除（不影响封禁效果）
整套 Vue 前端及其 API 契约;i18n/TranslationComponent;图表/会话分析/客户端分析;监视模块 SessionAnalyse / ActiveMonitoring(图表/限速);表 `traffic_journal_v3`、`peer_connection_metrics(+track)`、`alert`(降级为日志);推送通知(8 通道,后续可作可选 YAML 项);AutoSTUN、UPnP、Aviator 脚本、PF4J 插件、Laboratory、桌面 GUI、MTR、平台原生、多数据库后端、PBH Plus。

### ⏸ 留接口不实现
- **AutoSTUN/NAT 穿透**：保留 `NatAddressProvider` 抽象 + `auto-stun` 配置位（默认恒等映射）。
- **Aviator 脚本引擎**：保留 `ScriptEngine` trait 边界，ExpressionRule 禁用并记日志;未来可挂 JVM sidecar。

---

## 4. 新版极简 REST API（自研，清晰为先）

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

> 设计自由：因弃用原前端，API/UI **不要求与原版一致**，以清晰、好测为准。

---

## 5. 持久化（精简表集）

保留：`pcb_address`、`pcb_range`、`banlist`、`history`、`rule_sub_info`、`rule_sub_log`、`metadata`、`peer_records`、`tracked_swarm`。
砍除：`traffic_journal_v3`、`peer_connection_metrics(+track)`、`alert`。
连接/类型策略（WAL、单写者、epoch millis、TEXT IP/JSON）与完整表定义见 `03-db-schema.md`。

---

## 6. 里程碑路线图（含任务拆解与验收）

> 开发纪律见守则;关键等价性逻辑（PCB、规则引擎、IP 规范化、下载器封禁串、BTN 序列化）**先写测试**。
> 关键路径 M0→M1→M2→M3;M4/M5/M6 在 M3 后可部分并行;M7 可早搭;M8 依赖 M3(history)+M7。

### M0 — 地基
- workspace、`config.yml`/`profile.yml` 的 serde 模型 + 加载 + 默认值 + `tokio::sync::watch` 热重载 + 版本迁移链脚手架。
- `sqlx` SQLite 连接（pragma：WAL/synchronous=NORMAL/busy_timeout=60000/mmap_size），写池 `max_connections(1)`;`sqlx::migrate!` + 合并版 `V1__initial.sql`（**精简表集**，见 03）;KV `metadata`。
- `tracing` 日志（文件 + 控制台 + 环形缓冲供 WS）;`AppContext` 组合根骨架。
- **验收：** 启动建目录/建库/读写 KV/加载两份配置/热重载通知;迁移与配置加载有单测。
- ⚠️ **首次引入外部依赖**（serde/tokio/sqlx 等），需联网验证版本与 rustc 兼容性。

### M1 — 领域模型 + 规则引擎
- Peer/PeerAddress/Torrent/PeerFlag(libtorrent 标志串往返)、CheckResult/PeerAction(优先级合并)、BanMetadata、BanList(IPv4/IPv6 前缀 trie + RwLock)。
- 共享匹配引擎 `RuleParser`（method 枚举 + `FALSE` 短路优先级）、IPMatcher(CIDR trie)、ModuleMatchCache。
- **纯字符串**承载 rule/description（无 i18n）。
- **验收：** 规则引擎对 `profile.yml` 默认规则对拍;PeerFlag 往返;PeerAction 合并;BanList 最长前缀命中。

### M2 — 下载器（qB + qBEE）
- `Downloader` trait + `DownloaderManager` 工厂表。`QBittorrentClient`（reqwest + cookie SID + basic-auth + verify-ssl + UA + 并发信号量 128）。
- 全端点（登录双模式/版本/分页 torrents+properties/peers 解析过滤/统计/限速/listen_port/tracker）。
- `BanHandler`：Normal（banned_IPs/banPeers）与 ShadowBan（shadow_banned_IPs/shadowbanPeers，EE）;RANGE_BAN_IP 门控;IP 规范化;登录副作用 `enable_multi_connections_from_same_ip=false`。
- **验收：** 对真实/录制 qB/EE：登录、拉 peer、封禁可见;封禁串与 Java 版逐字节对拍;EE shadowban 验证。

### M3 — 流水线 + 调度 + BanManager
- bounded `mpsc`(64) channel 流水线（provider→login→torrents→peers→snapshot→monitor→check），每 peer 并发检查 + 非线程安全模块串行化，每阶段 timeout。
- BanManager：banPeer(时长：模块级>全局)/unban/removeExpiredBans/白名单解封/手动队列;`PeerBanEvent`/`PeerUnbanEvent`(broadcast);**封禁历史落库**。
- Ban Wave 循环：固定延迟 + try_lock 防重叠 + WatchDog + 每小时 banlist 快照 + globalPaused;封禁下发(增量/全量)。
- **验收：** 端到端一轮 wave（拉 peer→模块→命中→BanList→下发→到期解封）;WatchDog 卡死恢复;不重叠。

### M4 — 规则模块（离线）
AntiVampire、ClientNameBlacklist、PeerIdBlacklist、AutoRangeBan、IdleConnectionDosProtection、MultiDialingBlocker、PTRBlacklist;`MonitorFeatureModule`/`BatchMonitorFeatureModule` 钩子。
- **验收：** 每模块单测覆盖阈值与配置，与 `profile.yml` 默认一致。

### M5 — ProgressCheatBlocker
- `pcb_address`+`pcb_range` 两表、脏标志 + `moka` LRU(1024/180s) + 驱逐批刷;`shouldBanPeer` 精确短路顺序（上传增量跟踪→computedUploaded=max→fastPcbTest(BAN_FOR_DISCONNECT)→excessive→difference(ban-delay 窗口)→rewind）;8h 清理;订阅解封事件。`Option<Instant>` 取代零时间哨兵。
- **验收：** 专项序列回放套件逐子检查对拍;持久化重启续算;ban-delay 状态机;解封重置。

### M6 — GeoIP + IP 黑名单族
- `maxminddb` 读 City/ASN/GeoCN + 下载(三镜像/45 天)/xz 解压/原子替换 + GeoCN2/1 + 行政区划 CSV trie + `IpGeoData` + 叠加 + TW/HK/MO 命名 + moka;**可选注入**(缺失则降级)。
- IPBlackList（IP/CIDR/端口/ASN/国家/城市/中国网络类型）、IPBlackRuleList（下载/SHA-256 缓存/DAT-eMule-P2P-纯文本解析/前缀 trie/`rule_sub_log`/定时刷新/磁盘回退）。
- **验收：** GeoIP 对已知 IP(含中国省市/ISP)对拍;订阅格式解析单测;trie 命中;更新日志入库。

### M7 — 极简 Web（自研）
- `axum`：`ApiResp{ok,data,error}` 信封、分页、Bearer 鉴权（首启生成 token）、约 18 端点（见 §4）。
- WS `/api/logs/stream`（`?token=`+`?offset=`、ping、环形缓冲回放、broadcast）。
- 内置 vanilla 单页（`rust-embed` 内嵌），`/blocklist` 导出（纯文本）。
- **验收：** 单页能登录、看状态、增删下载器、查看/手动封禁、查历史、实时日志、改规则/订阅;接口常规测试（无需与原版对拍）。

### M8 — BTN（完整）
- HTTP 中间件（固定头 + Bearer + gzip 上行）、config 拉取、new/legacy 分支。
- 下行：HeartBeat/Rules(`?rev=`)/IPDenyList/IPAllowList(+解封白名单)/IpQuery/Reconfigure。
- 上行：SubmitBans/SubmitSwarm/SubmitHistory（DB 游标 + KV 续传）;轻量 PeerRecording/SwarmTracking 喂数据。
- PoW（移植 `PoWClient`）;`BtnNetworkOnline` 规则应用（Allow→SKIP / 脚本 stub / Deny→BAN / Rules 分类）;每 ability tokio 任务（随机初始延迟 + 固定间隔）;600s config 重试。
- **验收：** 对真实/录制 BTN：config/规则下行/心跳/IP 名单;上行 gzip 报文字段对拍;游标续传;PoW 通过。

### M9 — 收尾
- 单文件打包（`rust-embed` 内嵌单页）、配置随包、端到端验收、性能基线、README/部署文档。

---

## 7. 等价性对拍策略

为达成「与原版一致的封禁效果」，对以下产出建 golden fixture 并在 CI 对拍：
1. **下载器封禁写入串**：`banned_IPs`/`peers`/`shadow_banned_IPs`（含 IPv6 规范化、CIDR、shadowban）逐字节一致。
2. **规则引擎判定**：`profile.yml` 默认规则 + 一批 peer，命中一致。
3. **BTN 上下行报文**：解 gzip 后 JSON 字段/类型一致（时间戳 millis、双哈希 id 等）。
4. **PCB 序列回放**：固定输入序列 → 相同决策与 DB 状态。

> **自有 API/UI 不在对拍范围**（已弃用前端），走常规接口/集成测试。

---

## 8. 端到端验收（最终）

- [ ] 单文件二进制启动，无需任何外部数据库/服务。
- [ ] 首启生成 token，内置单页可登录并添加 qBittorrent。
- [ ] qB + qBEE 均能登录、拉 peer、下发封禁并在 qB 端可见（含 EE shadowban）。
- [ ] 全部高价值规则 + PCB + BTN（下行+上行）按 `profile.yml` 默认工作。
- [ ] 封禁串/规则判定与 Java 版抽样对拍一致。
- [ ] 内置单页可用：状态/下载器/封禁列表+历史/实时日志/规则+订阅配置。
- [ ] AutoSTUN/脚本引擎相关返回「不可用」占位，不崩。
- [ ] 24h 连续运行无内存泄漏、无致命 SQLITE_BUSY、到期解封正常。
