# PeerBanHelper → Rust 重写：预研与架构报告

> 版本：v1.0 ・ 日期：2026-06-19
> 目标：将 [PeerBanHelper](https://github.com/PBH-BTN/PeerBanHelper)（Java/JVM，分析时版本 `9.4.0-dev`）重写为 Rust，**零额外部署依赖**（不需要独立数据库等服务），保留全部封禁相关在线功能，直接复用现有前端。

---

## 0. 执行摘要（TL;DR）

PeerBanHelper（下称 PBH）是一个 BitTorrent 反吸血 / 封禁工具：它周期性轮询下载器（qBittorrent 等）拉取每个种子的 peer 列表，让一组「封禁规则模块」逐个检查每个 peer，把命中的 peer 写入下载器的 IP 封禁表（ipfilter / banned_IPs），并可选地接入 BTN 云端威胁情报网络共享与下载封禁数据。前端是一个独立的 Vue3 SPA，通过 REST + 单个 WebSocket 与后端通信。

**本次重写的关键结论：**

| 维度 | 现状（Java） | Rust 方案 |
|---|---|---|
| 运行时 | JVM + Spring + Javalin | 单一原生二进制，`tokio` 异步运行时 |
| 数据库 | SQLite/H2/MySQL/PostgreSQL（MyBatis-Plus+Flyway） | **仅嵌入式 SQLite**（`sqlx` 或 `rusqlite`，文件内置 WAL），零外部依赖 |
| Web | Javalin（Jetty） | `axum` + `tower-http` |
| HTTP 客户端 | OkHttp | `reqwest` |
| 下载器 | 6 种（qB/qBEE/Transmission/BiglyBT/BitComet/Deluge） | **仅 qBittorrent + qBittorrentEE**，但保留 `trait` 抽象 + 工厂注册，可扩展 |
| 脚本引擎 | Aviator（JVM 限定） | **本期不做**，留 trait 边界，未来可挂 JVM sidecar |
| 桌面 GUI | Swing 托盘 | 丢弃，仅保留 headless / console |
| 付费许可 | PBH Plus（RSA 许可证） | **整体删除**（含被 gate 的 13 个端点） |
| AutoSTUN | NAT 穿透+TCP 转发 | **本期不做**，留接口与配置位 |
| 前端 | Vue3 SPA → `dist` 拷入 `resources/static` | **原样复用**，Rust 端用静态目录 + SPA fallback 提供服务 |

**代码体量参考：** Java 主源码约 673 个 `.java` 文件。Rust 版预计核心逻辑显著小于此（去掉 5 种下载器、Spring/MyBatis 样板、GUI、许可证、Aviator）。

**最高风险/难点（按难度）：** ① `ProgressCheatBlocker`（进度作弊检测，核心价值、需持久化状态机）② BTN 在线协议（多 ability、gzip、PoW、游标）③ IP 订阅规则解析 + 前缀树 ④ GeoIP（MaxMind + GeoCN 叠加）。

---

## 1. 系统总览与数据流

```
                        ┌─────────────────────────────────────────────┐
                        │            Ban Wave 调度循环                  │
                        │   (默认每 check-interval=5000ms 触发一次)     │
                        └─────────────────────────────────────────────┘
                                          │
   ┌──────────────┐   登录   ┌────────────▼────────────┐  拉取 torrents/peers
   │ qBittorrent  │◀────────│   Downloader 抽象层      │◀───────────────────┐
   │ qBittorrentEE│────────▶│  (trait + 工厂注册表)    │                    │
   └──────────────┘ 写封禁表 └────────────┬────────────┘                    │
        ▲                                  │ FetchedPeersBatch              │
        │ banned_IPs /                     ▼                                │
        │ shadowban /          ┌───────────────────────┐                   │
        │ banPeers             │   Ban Pipeline         │   每个 (torrent,  │
        │                      │  (organ/stage 流水线)  │    peer) 并发检查  │
        │                      └───────────┬───────────┘                   │
        │                                  │ 对每个 peer                    │
        │                      ┌───────────▼───────────┐                   │
        │                      │   规则模块集合         │                   │
        │                      │  PCB / IPBlackList /   │                   │
        │   BanList            │  AutoRangeBan / BTN /  │                   │
        │  (IP 前缀 trie +     │  AntiVampire / ...     │                   │
        │   元数据)            └───────────┬───────────┘                   │
        └──────────────────────────────────┘ CheckResult                   │
                  │ 合并(取最高优先级)                                       │
                  ▼                                                         │
        ┌───────────────────┐   持久化(快照)  ┌──────────────┐             │
        │  BanManager        │───────────────▶│ SQLite (嵌入) │─────────────┘
        │  (banPeer/unban,   │                │  history/ban  │  解封到期/分析
        │   到期解封)        │                │  pcb/peer等   │
        └───────────────────┘                └──────────────┘
                  │                                  ▲
                  ▼                                  │
        ┌───────────────────┐    REST + WS    ┌──────┴───────┐
        │  axum Web Server   │◀──────────────▶│  Vue3 SPA     │
        │  /api/* + 静态文件 │                │ (原样复用)    │
        └───────────────────┘                └──────────────┘
                  ▲
                  │ gzip JSON / 规则同步 / PoW
        ┌─────────┴─────────┐
        │  BTN 云端网络      │  (上报 bans/swarm/history，下载 rules/denylist/allowlist)
        └───────────────────┘
```

**一次 Ban Wave 的生命周期（核心循环）：**

1. 调度器（Java 中是单线程 `ScheduledExecutorService` "Ban Wave"，固定延迟 `check-interval`）触发 `banWave()`，用 `tryLock(3s)` 防止重叠。
2. 移除到期封禁（`removeExpiredBans`：`now > unbanAt`）。
3. 构造一次「消化会话」(`DigestionSession`)，按流水线阶段执行：登录下载器 → 拉取 torrents → 拉取 peers → 快照（供 BTN/统计）→ 监视模块（被动观测）→ 检查模块（实际封禁判定）。
4. 每个 `(torrent, peer)` 对被并发送入所有 `RuleFeatureModule`，每个返回 `CheckResult{action, duration, rule, reason}`；同一 peer 多模块结果按 `PeerAction` 优先级合并（`SKIP > BAN > BAN_FOR_DISCONNECT > NO_ACTION`，同级取更长时长）。
5. 命中 BAN 的构造 `BanMetadata`（解析实际时长：模块级 > 全局），写入内存 `BanList`（IP 前缀 trie）。
6. 把封禁列表应用到各下载器：增量（`banPeers`）或全量（`banned_IPs` 偏好）。
7. 喂狗（WatchDog）、记录指标、解锁。

---

## 2. 子系统逐项分析

### 2.1 核心生命周期与 Ban 流水线

**关键 Java 文件：** `Main.java`、`PeerBanHelper.java`、`DownloaderServerImpl.java`、`banpipeline/*`、`module/*`、`wrapper/BanMetadata.java`、`BanList.java`。

- **启动顺序（严格有序、串行）：** 解析数据目录 → 加载 `config.yml`/`profile.yml`（含版本迁移）→ 日志/Sentry → 启动 Web 容器 → Spring 容器 refresh → `server.start()` → 注册 ~38 个模块（**串行，有依赖顺序**）→ 加载下载器 → 注册 Ban Wave 定时器。
- **模块系统：** `FeatureModule` 基接口 + 三个行为子接口：`RuleFeatureModule`（`shouldBanPeer(torrent,peer,downloader) -> CheckResult`，热路径）、`BatchMonitorFeatureModule`（`onPeersRetrieved`，被动观测不封禁）、Web 控制器模块。非线程安全模块在检查时用每模块 `ReentrantLock` 串行化。
- **Ban 数据模型：** `BanMetadata{context(模块名), randomId, banAt, unbanAt, banForDisconnect, excludeFromReport/Display, rule, description(TranslationComponent), structuredData}`。内存 `BanList` 用 `inet.ipaddr` 的 **IPv4/IPv6 关联前缀 trie**（支持 CIDR 范围封禁与最长前缀匹配），读写锁保护。**运行时内存权威，数据库仅做周期快照（每小时 + 关闭时），不是实时镜像。**
- **配置模型：** 两个 YAML —— `config.yml`（基础设施：端口/token/btn/ip-database/persist 等，`config-version: 46`）与 `profile.yml`（封禁行为：`check-interval`、`ban-duration`、`ignore-peers-from-addresses`、`module.<name>.*`，`config-version: 40`）。带版本化迁移脚本（`@UpdateScript`）+ 注释保留（Bukkit YAML 特性）。
- **事件总线：** Guava `EventBus`，**同步** post。事件含 `PeerBanEvent`/`PeerUnbanEvent`/`LivePeersUpdatedEvent`/模块注册（可取消）/生命周期/BTN 规则更新等。
- **并发：** 多个线程池 —— 通用调度器（8 线程）、单线程后台清理（避免 SQLITE_BUSY）、单线程 Ban Wave、work-stealing 池（封禁应用 + 每会话两个池）、WatchDog。

**Rust 映射要点：**
- `tokio` 多线程运行时即工作窃取执行器。**不要**像 Java 那样每个 wave 新建线程池（Java 这里有泄漏隐患）——用共享运行时 + `Semaphore`/`buffer_unordered` 限并发。
- Ban 流水线的 "organ" 状态机用 **bounded `mpsc` channel**（容量 64 对应 `ArrayBlockingQueue(64)` 背压）重写，"DONE" 用 channel 关闭语义自然表达，丢弃手写状态轮询。
- `BanList` → `ip_network_table` / `treebitmap` / `iptrie`（IPv4+IPv6 最长前缀匹配）+ `parking_lot::RwLock`。
- 全局 service-locator（`Main.getX()`）→ 显式 `Arc<AppContext>` 注入（**结构上最大的改动，触及所有模块**）。
- 事件总线 → `tokio::sync::broadcast`；可取消事件（模块注册）保留为直接函数返回 `Result`/否决，不走 broadcast。
- `PeerAction` 的序数优先级 → Rust `enum` 派生 `Ord` + `max_by`（同级取更长 `duration`）。

### 2.2 下载器子系统（仅 qBittorrent + qBittorrentEE）

**关键 Java 文件：** `downloader/{Downloader,AbstractDownloader,DownloaderManager*}.java`、`downloader/impl/qbittorrent/**`。

**`Downloader` trait 契约（必须保留以维持可扩展性）：** `login`、`getTorrents`/`getAllTorrents`、`getPeers`、`getTrackers`/`setTrackers`、`setBanList(full, added, removed, applyFullList)`、`getStatistics`、`getSpeedLimiter`/`setSpeedLimiter`、`getBTProtocolPort`/`set...`、`getFeatureFlags`、`getMaxConcurrentPeerRequestSlots`（qB=128）、状态/分页/并发信号量。

**qBittorrent Web API（base = `endpoint + /api/v2`）—— 必须字节级一致：**

| 方法 | 路径 | 用途 |
|---|---|---|
| POST | `/auth/login` | form `username`/`password`，捕获 `SID` cookie |
| GET | `/app/buildInfo` | 登录探测（`libtorrent` 字段非空即已登录） |
| GET | `/app/version` | 版本（Semver LOOSE 解析） |
| GET/POST | `/app/preferences` `/app/setPreferences` | 读/写偏好（`banned_IPs`/限速/`listen_port`/多连接开关） |
| GET | `/torrents/info?filter=active&limit=100&offset=N` | 分页种子列表（按 hash 去重） |
| GET | `/torrents/properties?hash=` | 补全 `is_private`/`piece_size`/`pieces_have` |
| GET | `/torrents/trackers` ・ POST `/torrents/{add,remove}Trackers` | tracker 读写 |
| GET | `/sync/torrentPeers?hash=` | **peer 列表**（map 键 `ip:port` 即 `rawIp`，封禁时回传此键） |
| GET | `/sync/maindata` | 统计（`server_state.alltime_ul/dl`） |
| POST | `/transfer/banPeers` | **增量封禁**（form `peers`，`\|` 分隔 `ip:port`） |

- **封禁分发逻辑：** `removed.isEmpty() && !added.isEmpty() && config.incrementBan && !applyFullList` → 增量；否则全量（`POST /app/setPreferences`，body `json={"banned_IPs":"<\n 分隔的 compressed IP>"}`）。
- **登录副作用（必须复刻）：** 默认 `enable_multi_connections_from_same_ip=false`。
- **Peer 过滤：** 跳过 `connection ∈ {HTTP,HTTPS,Web}`、空 ip、`.onion`/`.i2p`。
- **qBittorrentEE 差异：** type=`qBittorrentEE`；可选 shadowban —— 增量走 `/transfer/shadowbanPeers`，全量用 `shadow_banned_IPs` 键，探测 `shadow_ban_enabled`，登录时 `test()` gate；peer 多 `files`+`shadowbanned` 字段，过滤掉 `shadowbanned==true` 的 peer。
- **RANGE_BAN_IP 特性门控**（决定是否能下发 CIDR 段封禁）：qB 版本 `>=5.3.0` 等阈值；老版本只支持单 IP。
- **配置（YAML kebab-case / JSON camelCase 双写）：** `type`、`name`、`endpoint`、`username`、`password`、`api-key`（≥5.2.0 Bearer）、`basic-auth.{user,pass}`、`increment-ban`、`use-shadow-ban`、`verify-ssl`、`ignore-private`、`paused`。

**Rust 映射：** `reqwest::Client`（启用 `cookies` 自动管理 `SID`；`danger_accept_invalid_certs/hostnames` 对应 `verify-ssl=false`）；`serde` `#[serde(rename)]` 对齐字段；封禁策略做成 `trait BanHandler { test / set_increment / set_full }`（`Normal`/`ShadowBan` 两实现，正好对应 EE 设计）；`DownloaderManager` 是 `type`→构造器的工厂表。**易错点：** `banned_IPs` 的 `\n` 分隔 compressed 形、增量的 `\|` 分隔 `ip:port`（取自 `/sync/torrentPeers` map 键）、`semver` LOOSE 解析、登录副作用、并发信号量（128）。

### 2.3 封禁规则模块（产品核心）

12 个规则模块 + 4 个监视模块。按移植难度排序：

| 难度 | 模块 | 说明 | 持久化 |
|---|---|---|---|
| ⚫ 极高 | **ProgressCheatBlocker (PCB)** | 进度作弊检测。逐 peer/逐前缀跟踪上传量与进度，识别谎报进度/计数器重置/超量下载的吸血客户端。含 `fastPcbTest`（短暂 `BAN_FOR_DISCONNECT` 强制重握手）、ban-delay 窗口状态机、excessive/difference/rewind 三类子检查 | **必须 DB**（`pcb_address` + `pcb_range` 两表，脏标志 LRU 刷盘，8h 清理，订阅解封事件） |
| ⚫ 极高 | **ExpressionRule** | 用户脚本规则（Aviator `.av`）。**本期 stub** | 脚本在磁盘 |
| 🔴 高 | **IPBlackRuleList** | 下载远端 IP 黑名单订阅（DAT/eMule/P2P/纯文本格式），SHA-256 缓存，前缀 trie，DB 记录更新日志，定时刷新 | 磁盘缓存 + `rule_sub_log` 表 |
| 🟠 中高 | **IPBlackList** | 静态黑名单：IP/CIDR/端口/ASN/国家/城市/中国网络类型；依赖 GeoIP | 配置文件 |
| 🟠 中 | **MultiDialingBlocker** | 多拨检测（同子网大量 IP 下同种子，PCDN 特征），可选 hunting 模式 | 内存（进程级静态缓存） |
| 🟡 中 | **IdleConnectionDosProtection** | 空闲连接 DoS（占用连接但几乎不传输） | 内存 TTL 缓存 |
| 🟡 中低 | **PTRBlacklist** | 反向 DNS + 规则匹配，默认关 | 仅结果缓存 |
| 🟢 低 | **AutoRangeBan** | 某 peer 被封后，连带封同 CIDR 段已连接 peer | 依赖 BanList |
| 🟢 低 | **ClientNameBlacklist / PeerIdBlacklist** | 按客户端名 / PeerID 规则匹配（共享匹配引擎） | 无 |
| 🟢 极低 | **AntiVampire** | 硬编码迅雷（Xunlei）预设 | 无 |
| ⚪ 跳过 | **PeerNameBlackRuleList** | 整文件被注释，已禁用 | — |
| ⚪ 延后 | **4 个监视模块** | ActiveMonitoring/PeerRecording/SessionAnalyse/SwarmTracking，纯 DB 观测，不参与封禁判定 | DB |

**跨模块前置依赖（先建好再写模块）：**
1. `Peer`/`Torrent`/`PeerFlag`/`PeerAddress` 数据模型 + `PeerFlag` libtorrent 标志串解析。
2. `CheckResult` + `PeerAction`（含 `BAN_FOR_DISCONNECT`）+ `pass()`/`handshaking()` 哨兵。
3. **共享规则匹配引擎**（`RuleParser`：`method ∈ {STARTS_WITH,ENDS_WITH,CONTAINS,EQUALS,REGEX,LENGTH}`；优先级 `FALSE` 短路获胜）——被 3 个模块复用。
4. `BanList` 抽象（含 `ban_for_disconnect` 元数据）。
5. `ModuleMatchCache`（按模块记忆化，含「仅写 pass」变体）。
6. GeoIP 基础设施（`maxminddb` + GeoCN）。
7. IP trie / 最长前缀匹配 crate。
8. SQLite 层。
9. `MonitorFeatureModule` 钩子。
10. 嵌入式脚本运行时决策（ExpressionRule，本期 stub）。

**推荐 crate：** `ipnet`/`ip_network`/`iprange`/`treebitmap`、`maxminddb`、`moka`/`dashmap`/`quick_cache`、`regex`、`serde_json`、`reqwest`、`sha2`、`hickory-resolver`、`sqlx`/`rusqlite`。

### 2.4 BTN 在线网络（用户明确要求全部保留）

BTN = BitTorrent Threat Network，云端威胁情报，跨 PBH 实例共享封禁/peer/swarm 与规则。

- **配置（`config.yml` 的 `btn:`）：** `enabled`（默认 false）、`config-url`、`submit`、`app-id`、`app-secret`、`allow-script-execute`。协议版本常量 `PBH_BTN_PROTOCOL_IMPL_VERSION=20`、可读版本 `2.0.1`。
- **Ability 系统：** 服务端 config 端点返回启用哪些 ability，客户端据此构建并自调度（每 ability 带 `interval`/`endpoint`/`random_initial_delay`/可选 `pow_captcha`）。
  - **下行：** `HeartBeat`（探测外网 IP）、`Rules`（`?rev=` 拉规则集，204=未变）、`IPDenyList`/`IPAllowList`（纯文本，`X-BTN-ContentVersion` 头版本；AllowList 命中即 SKIP 并解封已封的白名单 IP）、`IpQuery`（按需查 IP 风险）、`Reconfigure`（轮询版本变化触发重建）。
  - **上行（需 `submit`）：** `SubmitBans`（DB 游标分页 `history` 表）、`SubmitSwarm`（游标分页 `tracked_swarm`）、`SubmitHistory`（游标分页 peer 记录）。
- **HTTP 协议：** 无共享 base URL，每 ability 给完整 URL。每请求注入头：`User-Agent`（含 `BTN-Protocol/2.0.1`）、`Content-Type: application/json`、`X-BTN-AppID`/`X-BTN-AppSecret`（+ 旧 `BTN-*`）、`Authentication: Bearer <appId>@<appSecret>`；匿名时加 `X-BTN-InstallationID`。上行一律 **gzip JSON**（`Content-Encoding: gzip`）。
- **PoW captcha：** `GET powEndpoint?type=<ability>` 拿挑战，`PoWClient.solve` 求解，结果放 `X-BTN-PowID`/`X-BTN-PowSolution` 头。
- **规则应用（`BtnNetworkOnline.shouldBanPeer`）顺序：** AllowList→SKIP；脚本（若开启）；DenyList→BAN；Rules 规则集（peer_id/client_name/ip/port 分类）。
- **隐私：** 种子标识用双重哈希（`getHashedIdentifier`），从不上报原始 infohash/名称。
- **新旧协议：** `min_protocol_version < 20` 走 legacy（`submit_peers` 全量快照、旧 `submit_bans`、规则键 `rules`、无 allow/deny list、无 PoW）。

**Rust 映射：** `reqwest`（gzip）+ `reqwest-middleware` 注入固定头；`serde` 对齐每个 `@SerializedName`；`flate2` 压缩上行；`tokio` 任务调度（初始随机延迟 + 固定间隔）；IP 名单用前缀 trie；PoW 需照搬 `util/pow/PoWClient.java` 算法。**易错序列化：** 时间字段（`Timestamp` → epoch millis 数字 vs `OffsetDateTime` → ISO 串，需对照真实服务端）、`InetAddress` 的 Gson 串形、`BtnBan.structured_data`（字符串化 JSON）vs legacy（嵌套对象）、名单纯文本行解析、游标 KV 键的复刻（重启续传）。

### 2.5 Web API 层（前端契约，必须精确复刻）

**框架：** Javalin（Jetty）；GZIP；虚拟线程；Gson JSON。**端口默认 9898**，bind `0.0.0.0`，无全局前缀（REST 在 `/api/*`，另有 `/blocklist/*` 与少数单例）。

- **鉴权模型（单一共享 token）：** `POST /api/auth/login` body `{token}` → 设置会话属性；每请求也接受 `Authorization: Bearer <token>` 或 `?token=`（WS 用 query）。角色 `ANYONE/USER_READ/USER_WRITE/PBH_PLUS`。无 token 配置时受保护路由返回 **303 → `/init`**；token 错 **401**；未登录 **403**；demo 模式写操作 **400**；fail2ban **429**（IP 前缀 /24 或 /50，10 次失败，15 分钟）。
- **响应信封（`StdResp`）：** `{ "success": bool, "message": string|null, "data": any|null }`。分页 data：`{ page, size, total, results }`（请求 `?page=`(默认1)`&pageSize=`(默认10)）。
- **端点目录：** 21 个控制器，约 90 个端点。完整清单见 `docs/03-api-contract.md`。关键路由顺序敏感：`/api/general/global` 先于 `/{configName}`、`/api/torrent/query` 先于 `/{infoHash}`、`/api/sub/rules` 先于 `/api/sub/rule/{ruleId}`。
- **WebSocket（唯一实时通道）：** `WS /api/logs/stream`，`?token=` + `?offset=` 回放，15s ping，帧为 `StdResp{data: WebSocketLogEntryDTO{time,thread,level,content,seq}}`。
- **非 JSON 端点：** `/blocklist/*`（纯文本，供下载器消费）、`/api/egg`（302）、`/api/peer/{ip}/btnQueryIframe`（HTML）。

**Rust 映射：** `axum` + `tower-http`（compression/cors/ServeDir）。`StdResp<T>` 泛型 + `serde_json::Value` 处理动态 data。鉴权用 `axum` 中间件复刻（含三种 token 通道 + 异常→状态码精确映射）。静态：`ServeDir` + SPA fallback 到 `index.html`，但 `/api/*` 与 `/blocklist/*` 必须先路由，未匹配返回 JSON 405/404 而非 index.html。fail2ban 用 `dashmap`/`moka`。WS 用 `axum::extract::ws` + `tokio::sync::broadcast`。**注意 Gson 序列化细节（null、long vs double、epoch millis）需对齐，否则前端解析异常。**

### 2.6 数据库层（标准化为嵌入式 SQLite）

现状支持 4 种后端；**Rust 仅做 SQLite**，绝大部分方言复杂度消失。

- **文件：** `<dataDir>/persist/peerbanhelper-nt.db`，WAL，`synchronous=NORMAL`，`busy_timeout=60000`，`mmap_size=128MB`，**连接池 `maxActive=1`（单写者）**。
- **表（14 张）：** `history`（封禁历史）、`banlist`（封禁快照 KV）、`pcb_address`/`pcb_range`（PCB 状态）、`peer_records`、`peer_connection_metrics`/`_track`、`traffic_journal_v3`、`rule_sub_info`/`rule_sub_log`、`alert`、`torrents`、`metadata`（KV 游标/缓存）、`tracked_swarm`（临时表）。完整 schema 见 `docs/04-db-schema.md`。
- **类型存储（SQLite，统一应用层编码，无原生 inet/jsonb/timestamptz）：** 时间戳 → INTEGER epoch millis；IP → TEXT（规范串）；JSON / TranslationComponent → TEXT（serde_json）；bool → INTEGER 0/1；枚举 → TEXT。
- **时间分桶：** **不用任何 DB 日期函数**——分桶在应用层（`TimeUtil.getStartOfHour/Today`，系统时区）完成后存为整数，再 `GROUP BY timestamp`。极大简化移植。
- **关键自定义 SQL（驱动仪表盘，需手工移植）：** History 的 `sumField`/`countField`（CTE + 百分比 + `HAVING`，`${field}` 白名单替换）、`getBannedIps`（top-N）；PeerRecord 的 `upsert`（带偏移量单调累加的冲突解决，**最难单条语句**）、`queryClientAnalyse`；TrafficJournal 的分桶聚合。**注意：所有 `${field}`/`${orderBy}` 必须做枚举白名单，绝不拼接用户输入。**
- **清理调度：** 单独后台单线程，`splitBatchDelete` 按 LIMIT 200 分块删除（避免长写锁）。

**Rust 映射：** 推荐 **`sqlx`（sqlite + runtime-tokio）**（异步、编译期校验、内置迁移）或 `rusqlite`+`r2d2`（同步、最简）。**避免 `sea-orm`/`diesel`**（难表达手写分析 SQL 与复杂 upsert，CRUD 又很简单）。连接：写用 `max_connections(1)`，WAL 下读可并发；连接时设 pragma。迁移用 `sqlx::migrate!` 单个合并的 `V1__initial.sql`（反映 V1_5 后状态）。**丢弃 legacy ORMLite 导入器**（用户不需要历史迁移）。修两个潜在 bug：`@Select sessionBetween` 用了 camelCase 列名；`tracked_swarm.peer_progress` DDL 是 TEXT 但实体是 double。

### 2.7 支撑服务

| 区域 | 处置 | Rust crate | 难度 |
|---|---|---|---|
| **GeoIP**（MaxMind City/ASN + GeoCN 叠加，含中国省市/ISP 与 TW/HK/MO 命名特例） | 保留 | `maxminddb`、`xz2`、`csv`、`reqwest`、`moka`、trie | 中 |
| **反向 DNS (PTR)** | 保留 | `hickory-resolver`（含 DoH、系统 DNS、PTR） | 低 |
| **Alert**（DB 持久化 + 去重 + 30 天清理 + 推送） | 保留 | DB + `tokio` | 低 |
| **Push**（8 通道：pushplus/serverchan/smtp/telegram/bark/pushdeer/gotify/ntfy） | 保留 | `reqwest`、`lettre`（SMTP）、`pulldown-cmark` | 低-中 |
| **i18n / TranslationComponent**（`{}` 顺序占位符；en_us/zh_cn/zh_tw；前端契约 `{key,params[]}`） | 保留 | `serde_yaml` + 自定义 `{}` 填充 | 低-中 |
| **脚本引擎（Aviator）** | **stub/延后** | 留 trait 边界给 JVM sidecar | stub 低 |
| **PBH Plus 许可证** | **删除** | 无 | trivial |
| **metric**（内部计数器，非 Prometheus） | 保留 | atomics + DB | 低 |
| **rule 匹配引擎** | 保留 | `serde_json`、`regex`、IP trie、`moka` | 中 |
| **UPnP 端口映射** | 保留（可延后） | `igd-next`、`if-addrs` | 低-中 |
| **AutoSTUN / NAT 穿透 / TCP 转发** | **本期不做，留接口** | `tokio`、`stun_codec`/`webrtc-stun` | 高 |
| **platform**（EcoQoS/AMSI/working-set 等 OS 原生） | **丢弃**（headless） | — | 低 |
| **GUI Swing 托盘** | **丢弃**；保留 console | — | 低 |

**跨切面替换：** `Sentry`→`tracing`；Spring `@Component`→显式组合根；`simplereloadlib.Reloadable`→`tokio::sync::watch` 配置广播；Guava `EventBus`→channel；Guava cache→`moka`；OkHttp→`reqwest`。**前端要原样消费的数据契约：** `TranslationComponent{key,params[]}` 与 `IPGeoData`。

### 2.8 前端（原样复用）

- **技术栈：** Vue3 + TypeScript + Vite 8（rolldown）+ Arco Design Vue + ECharts 6 + Pinia + vue-router 5 + vue-i18n（**i18n 完全前端打包，不从后端拉**）。**包管理器必须 pnpm**（用了 pnpm patch）。
- **构建：** `pnpm run build` → `webui/dist/`（`base: './'` 相对路径，可挂任意前缀）。现状 CI 把 `dist/*` 拷进 `src/main/resources/static`，Javalin 从 classpath `/static` 提供 + SPA fallback。
- **API 契约（客户端视角）：** 同源相对（生产构建 `VITE_APP_BASE_URL` 为空 → 回落同源）；**每请求带 `Authorization: Bearer <authToken>`**（不依赖 cookie）+ `Content-Type: application/json` + `Accept-Language` + `X-TimeZone`。信封 `{data, message, success}`。状态码语义：**401/403→重新登录，303(+`Location:/init`)→OOBE**，必须精确。
- **实时：** 唯一 WS `/api/logs/stream`，`?token=` 鉴权（浏览器 WS 不能设头）。少数弹窗 REST 轮询。
- **manifest：** `GET /api/metadata/manifest` 返回 `{version:{version,os,branch,commit,abbrev}, analytics, modules:[{className,configName}]}`，SPA 据 `modules[].configName` 与 `version.version`（`<4.0.0` 跳过登录，故须报 `>=4.0.0`）控制菜单/路由。

**集成方案：** 前端零改动。Rust 端：构建 `webui` → `dist/*` 放静态目录（可 `rust-embed` 内嵌进二进制以实现真正单文件部署）→ `axum` `ServeDir` + SPA fallback，`/api`/`/blocklist` 先路由。鉴权纯靠 `Authorization` 头即可。

---

## 3. 三项关键决策（已与用户确认）

1. **AutoSTUN/NAT 穿透 —— 本期不做，预留接口。** 保留 `NatAddressProvider` trait 与 `auto-stun` 配置位；前端 AutoSTUN 页面暂返回「不可用」状态。后续单独里程碑实现。
2. **PBH Plus 付费功能 —— 整体删除（含被 gate 的 13 个端点）。** 删除 `pbhplus/` 包、`PBHPlusController`、`Role.PBH_PLUS`、`RequirePBHPlusLicenseException`，以及以下 13 个端点：
   - `PBHChartController`：`/api/chart/{geoIpInfo,trend,traffic,sessionBetween,sessionDayBucket,sessionAnalyse,clientAnalyse}`（7）
   - `PBHPeerController`：`/api/peer/{ip}/{accessHistory,banHistory}`（2）
   - `PBHTorrentController`：`/api/torrent/{infoHash}/{accessHistory,banHistory}`（2）
   - `PBHPlusController`：`/api/pbhplus/*` 整组（许可证管理）
   - **前端影响（用户已接受）：** 对应的图表与「访问/封禁历史」详情页将失效/空白。manifest 不再宣告这些模块，SPA 在可隐藏处隐藏。
   - **数据层保留：** 仍保留底层表与基础统计（`/api/statistic/*` 保留），未来如需可重新启用。仅为这些被删图表服务的监视模块（SessionAnalyse/ClientAnalyse）随之延后。
3. **交付物 —— 报告 + 施工指南 + Rust 项目骨架。** 见 `docs/` 与工作目录下的 Cargo workspace 骨架。

---

## 4. 技术选型总表

| 关注点 | 选型 | 备注 |
|---|---|---|
| 异步运行时 | `tokio`（多线程） | 工作窃取执行器 |
| Web 框架 | `axum` + `tower-http` | 静态/压缩/CORS/中间件 |
| HTTP 客户端 | `reqwest`（+ `reqwest-middleware`、gzip、cookies） | 下载器 + BTN |
| 数据库 | **`sqlx`（sqlite, runtime-tokio）** | 嵌入式，零部署依赖；备选 `rusqlite` |
| 序列化 | `serde` / `serde_json` / `serde_yaml` | 对齐 Gson 字段名 |
| IP / CIDR | `ipnet` + `ip_network_table`/`treebitmap`/`iprange` | 最长前缀匹配 trie |
| GeoIP | `maxminddb` + `xz2` + `csv` + `patricia_tree` | MaxMind + GeoCN |
| DNS | `hickory-resolver` | PTR / DoH / 系统 DNS |
| 缓存 | `moka` / `dashmap` / `quick_cache` | 替换 Guava cache |
| 正则 | `regex` | 规则匹配 |
| 哈希 | `sha2` | 订阅缓存/BTN id |
| 时间 | `chrono`（Local，保持分桶边界一致） | epoch millis 存储 |
| SMTP | `lettre` | 邮件推送 |
| Markdown | `pulldown-cmark` | 推送正文 |
| 压缩 | `flate2` / `xz2` | BTN gzip / mmdb xz |
| 日志/telemetry | `tracing` + `tracing-subscriber`（+ 可选 `sentry`） | 替换 Logback/Sentry |
| 配置 | `serde_yaml` + `config`/手写 | 注释保留是难点，见下 |
| 静态资源内嵌 | `rust-embed`（可选） | 真正单文件部署 |
| 脚本引擎（未来） | trait 边界，预留 `rhai`/JVM sidecar | 本期 stub |
| STUN（未来） | `stun_codec`/`webrtc-stun` | 本期不做 |
| UPnP | `igd-next` | 可延后 |

---

## 5. 风险登记册

| # | 风险 | 等级 | 缓解 |
|---|---|---|---|
| R1 | **PCB 算法移植不等价** —— 阈值/状态机微妙，是产品核心价值 | 高 | 建专门测试套件，回放上传/进度序列；逐子检查（excessive/difference/rewind/fastPcb）对拍 Java 行为 |
| R2 | **BTN 线上协议序列化不匹配** —— 时间/InetAddress/structured_data 表示差异 | 高 | 对照真实 BTN 服务端抓包；先实现下行（rules/denylist）再上行；保留游标 KV 复刻重启续传 |
| R3 | **qB 封禁字节级不一致** —— `banned_IPs` 分隔/compressed 形、增量 `ip:port` 取值、版本门控 | 中高 | 针对 IP 规范化与封禁串写单测；对照真实 qB/EE 实例验证 |
| R4 | **YAML 注释保留迁移** —— 主流 Rust YAML crate 丢注释 | 中 | 接受重生成配置文件 / 或 `yaml-rust2` 编辑 AST；迁移链用有序 `Vec<fn(&mut Value)>` |
| R5 | **Aviator 无对等物** —— 用户已有 `.av` 脚本不兼容 | 中（本期 stub 规避） | trait 边界 + 未来 JVM sidecar 或新 `rhai` DSL（破坏性） |
| R6 | **Gson 序列化怪癖** —— 前端解析依赖精确 JSON 形 | 中 | 用 `serde` 对齐；对关键端点做 JSON 快照对拍 |
| R7 | **SQLite 单写者并发** —— 高峰 SQLITE_BUSY | 中 | `busy_timeout=60000`、写串行化、清理分块短事务、写后缓存批刷 |
| R8 | **路由顺序/特化** —— 字面路由须先于路径参数 | 低 | `axum` matchit 特化优先；补路由测试 |
| R9 | **删除 PBH Plus 端点导致前端报错** | 低（已确认接受） | manifest 不宣告；前端在可隐藏处隐藏；返回干净 404/JSON |
| R10 | 全局 service-locator → `Arc<AppContext>` 重构触及全部模块 | 中 | 早期定好 `AppContext` 结构与依赖注入约定 |

---

## 6. 参考：源码关键路径索引

- 核心：`Main.java`、`PeerBanHelper.java`、`DownloaderServerImpl.java`、`banpipeline/**`、`module/{FeatureModule,RuleFeatureModule,CheckResult,PeerAction}.java`、`BanList.java`、`wrapper/BanMetadata.java`
- 下载器：`downloader/{Downloader,AbstractDownloader}.java`、`downloader/impl/qbittorrent/**`、`util/{HTTPUtil,IPAddressUtil}.java`、`bittorrent/{peer,torrent}/**`
- 规则模块：`module/impl/rule/**`、`module/impl/monitor/**`、`util/rule/**`
- BTN：`btn/**`、`module/impl/rule/BtnNetworkOnline.java`、`util/pow/**`
- Web：`web/**`、`module/impl/webapi/*Controller.java`、`util/query/**`
- DB：`databasent/**`、`resources/db/migration/sqlite/V1_*.sql`、`resources/mapper/sqlite/*.xml`、`util/TimeUtil.java`
- 支撑：`util/ipdb/**`、`util/dns/**`、`alert/**`、`util/push/**`、`text/**`、`util/scriptengine/**`、`pbhplus/**`（删）、`metric/**`、`util/portmapper/**`、`util/traversal/**`（延后）、`platform/**`（丢）
- 前端：`webui/**`、`webui/src/{service,stores,api,locale}/**`、`webui/vite.config.ts`
