# PeerBanHelper-Rust 施工指南

> 配套阅读：`01-research-report.md`（架构）、`03-api-contract.md`（端点）、`04-db-schema.md`（库表）。
> 本指南给出**分阶段里程碑、任务拆解、依赖顺序、验收标准**。每个阶段产出可独立验证。

## 0. 工作方式与总则

- **开发纪律：** 采用 TDD（先写测试再写实现），尤其 PCB、规则匹配引擎、IP 规范化、BTN 序列化这些「等价性关键」部分必须有对拍测试。
- **参照系：** Java 源码在 `./source/`，作为行为基准（**不要逆向二进制，一切信息在源码内**）。
- **等价性验证：** 关键路径准备一个 fixture 库：保存 Java 端真实 JSON 响应 / qB 封禁串 / BTN 报文，Rust 端做快照对拍。
- **里程碑出口准则：** 每阶段末必须 `cargo test` 全绿 + `cargo clippy` 无 error + 该阶段「验收标准」逐条勾选。
- **不做的事（本期）：** AutoSTUN、Aviator 脚本引擎实现、桌面 GUI、PBH Plus、历史数据迁移、qB/EE 以外的下载器。

## 1. 里程碑总览（建议顺序）

| 阶段 | 名称 | 目标产出 | 依赖 |
|---|---|---|---|
| **M0** | 地基 | workspace、配置加载、SQLite 层、日志、AppContext、错误类型 | — |
| **M1** | 领域模型 + 规则引擎 | Peer/Torrent/PeerFlag/CheckResult/PeerAction、共享规则匹配引擎、BanList、i18n | M0 |
| **M2** | 下载器 | Downloader trait + qBittorrent + qBittorrentEE，登录/拉取/封禁字节级一致 | M1 |
| **M3** | Ban 流水线 + 调度 | DigestionSession（channel 流水线）、Ban Wave 循环、BanManager、到期解封、封禁应用 | M2 |
| **M4** | 规则模块（离线） | 不依赖网络/GeoIP 的模块：AntiVampire/Client/PeerId/AutoRange/Idle/MultiDialing/PTR | M3 |
| **M5** | PCB（进度作弊） | ProgressCheatBlocker + 两表持久化 + 脏刷缓存 + 清理 + 解封钩子 | M3, M4 |
| **M6** | GeoIP + IP 黑名单族 | IPDB（MaxMind+GeoCN）、IPBlackList、IPBlackRuleList（订阅下载/解析/trie） | M4 |
| **M7** | Web 层 + 鉴权 + 静态 | axum、StdResp、鉴权中间件、fail2ban、静态/SPA、WS 日志流、OOBE、manifest | M1–M6（逐控制器接入） |
| **M8** | BTN 在线网络 | ability 系统、HTTP 协议、规则同步、上报、PoW、BtnNetworkOnline 模块 | M5（history 表）, M7 |
| **M9** | 支撑服务 | Alert、Push（8 通道）、metric、监视模块（保留部分）、UPnP（可选） | M3, M7 |
| **M10** | 收尾 | 打包（单文件/`rust-embed`）、配置迁移链、端到端验收、文档 | 全部 |

> **关键路径：** M0→M1→M2→M3 是主干，必须先行。M4/M5/M6 可在 M3 后部分并行。M7 可在 M3 后开始搭框架，随各模块就绪逐步接入控制器。

---

## 2. 各阶段任务拆解与验收

### M0 — 地基

**任务：**
- [ ] 初始化 Cargo workspace（见骨架），统一 `edition`、`rust-toolchain.toml`、`clippy`/`fmt` 配置。
- [ ] `pbh-domain`：错误类型（`thiserror`）、`AppContext` 占位、数据目录解析（`data/`、`config/`、`logs/`、`persist/`）。
- [ ] `pbh-config`：`config.yml` + `profile.yml` 的 `serde` 模型（先覆盖 server/persist/btn/ip-database/proxy/performance/privacy + profile 的 check-interval/ban-duration/ignore + module map）；加载、默认值、`tokio::sync::watch` 热重载广播；版本迁移链脚手架（有序 `Vec<fn(&mut Value)>`）。
- [ ] `pbh-storage`：`sqlx` SQLite 连接（pragma：WAL/synchronous=NORMAL/busy_timeout=60000/mmap_size），写池 `max_connections(1)`；`sqlx::migrate!` + 合并版 `V1__initial.sql`（见 `04-db-schema.md`）；KV `metadata` 读写。
- [ ] `tracing` 日志初始化（文件 + 控制台 + 环形缓冲供 WS 日志流）。

**验收：** 程序能启动、创建数据目录、建库建表、读写一个 metadata KV、加载并打印两份配置、热重载触发 watch 通知；`cargo test` 含配置加载与迁移测试。

### M1 — 领域模型 + 规则引擎

**任务：**
- [ ] `pbh-domain`：`Peer`/`PeerAddress`/`Torrent`/`PeerFlag`（libtorrent 标志串解析，含 21 peer 位 + 6 source 位，对拍 `bittorrent/peer/PeerFlag.java`）。
- [ ] `CheckResult` + `PeerAction`（`enum` 派生 `Ord`：`NO_ACTION<BAN_FOR_DISCONNECT<BAN<SKIP`，合并取最高 + 同级更长 duration）+ `pass()`/`handshaking()` 哨兵。
- [ ] `BanMetadata` + `BanList`（IPv4/IPv6 前缀 trie + `RwLock`，`elementsContaining` 最长前缀查、`ban_for_disconnect` 元数据）。
- [ ] `pbh-rules`（引擎部分）：`RuleParser` —— `method ∈ {STARTS_WITH,ENDS_WITH,CONTAINS,EQUALS,REGEX,LENGTH}`，`matchRule` 优先级（`FALSE` 短路获胜，`TRUE` 可被后续 `FALSE` 覆盖）；`IPMatcher`（CIDR 集 trie）；`ModuleMatchCache`（`moka`，含「仅写 pass」变体）。
- [ ] `pbh-i18n`：`TranslationComponent{key, params[]}`（serde，前端契约）+ `TextManager`（加载 en_us/zh_cn/zh_tw + fallback 填充；`{}` 顺序占位符填充，**非** `format!`；递归解析 param）。

**验收：** 规则引擎对拍 `profile.yml` 默认规则用例；`PeerFlag` 解析往返一致；`PeerAction` 合并语义单测；`BanList` 最长前缀命中单测；i18n `{}` 填充与缺键回退单测。

### M2 — 下载器（qBittorrent + qBittorrentEE）

**任务：**
- [ ] `pbh-downloader`：`Downloader` trait（全部契约方法）+ `DownloaderManager` 工厂表（`type` 字符串 → 构造器）。
- [ ] `QBittorrentClient`：`reqwest` 客户端（cookies 自动 `SID`、basic-auth、`verify-ssl` 开关、UA、并发信号量 128）。
- [ ] 实现全部端点（见报告 2.2）：登录（密码 / api-key Bearer 双模式）、版本探测、torrents 分页+去重+properties 补全、`/sync/torrentPeers` peer 解析与过滤、统计、限速、listen_port、tracker。
- [ ] 封禁分发：`BanHandler` trait（`Normal`/`ShadowBan`），全量 `banned_IPs`(`\n`+compressed) / 增量 `banPeers`(`\|`+`ip:port`)；EE 的 `shadowbanPeers`/`shadow_banned_IPs`/`test()` gate/`shadowbanned` 过滤。
- [ ] 特性门控 `RANGE_BAN_IP`（版本阈值）→ 是否下发 CIDR；`remapBanListAddress` IP 规范化（对拍 `util/IPAddressUtil.java`）。
- [ ] 登录副作用 `enable_multi_connections_from_same_ip=false`。

**验收：** 对真实 qB / qB-EE 实例（或录制 fixture）：能登录、拉到 torrents+peers、写入封禁并在 qB 偏好中可见；封禁串格式与 Java 端逐字节对拍；EE shadowban 路径验证。

### M3 — Ban 流水线 + 调度

**任务：**
- [ ] `pbh-engine`：用 bounded `mpsc`(64) 重写流水线阶段（provider→login→torrents→peers→snapshot→monitor→check），每 peer 并发检查（`buffer_unordered` + per-module 串行化非线程安全模块），每阶段 `tokio::time::timeout`。
- [ ] `BanManager`：`banPeer`（解析实际时长：模块级>全局）、`unbanPeers`、`removeExpiredBans`、白名单解封、手动封禁队列（`/api/bans` 用）、`PeerBanEvent`/`PeerUnbanEvent`（broadcast）。
- [ ] Ban Wave 循环：固定延迟 `check-interval`、`try_lock` 防重叠、WatchDog（卡死重启）、每小时 banlist 快照、`globalPaused`。
- [ ] 封禁应用到下载器（增量/全量、`needReApplyBanList`）。

**验收：** 端到端跑一轮 wave：拉 peer → 走（占位）模块 → 命中 → 写 BanList → 下发下载器 → 到期解封；WatchDog 卡死恢复测试；并发不重叠测试。

### M4 — 规则模块（离线类）

> 每个模块 = 实现 `RuleFeatureModule`，注册进模块表，读 `profile.yml` 对应 section。

- [ ] AntiVampire（迅雷预设，含 0019 变体 seeding 判定）
- [ ] ClientNameBlacklist / PeerIdBlacklist（复用规则引擎）
- [ ] AutoRangeBan（遍历 BanList，CIDR 含；跳过 `ban_for_disconnect`）
- [ ] IdleConnectionDosProtection（TTL 缓存 + 平均速率 + protect-mode + peer-flag 门控 + `onPeersRetrieved` 驱逐钩子）
- [ ] MultiDialingBlocker（三 TTL 缓存：subnet 计数 / hunting；用 `DashMap<Subnet, HashSet<IP>>` 直接 `.len()`，替代 Guava 嵌套计数 hack）
- [ ] PTRBlacklist（`hickory-resolver` reverse_lookup + 3s timeout + 缓存 + 规则引擎；默认关）
- [ ] `MonitorFeatureModule`/`BatchMonitorFeatureModule` 钩子（供 Idle 驱逐与后续监视模块）

**验收：** 每模块单测覆盖其阈值与配置；与 `profile.yml` 默认值一致；Idle 的批量驱逐计数正确。

### M5 — ProgressCheatBlocker（最高难度）

**任务：**
- [ ] `pcb_address` + `pcb_range` 两表（见 schema），`sqlx` 服务层。
- [ ] 脏标志 + `moka` LRU（size 1024, TTL 180s）+ 驱逐时批量刷盘。
- [ ] `shouldBanPeer` 按精确短路顺序移植：上传增量跟踪（防计数器重置）→ `computedUploaded=max(peer, addr, range)` → fastPcbTest（`BAN_FOR_DISCONNECT` 15s）→ excessiveClient（超量）→ differenceTest（带 ban-delay 窗口）→ progressRewind。用 `Option<Instant>` 取代「零时间」哨兵。
- [ ] 8h 清理（按 `persist-duration` 删旧行）；订阅 `PeerUnbanEvent` 删对应 address 行。

**验收：** **专项测试套件**：构造上传/进度序列回放，逐子检查（excessive/difference/rewind/fastPcb）对拍 Java 行为；持久化重启续算；ban-delay 窗口状态机正确；解封后记录重置。

### M6 — GeoIP + IP 黑名单族

**任务：**
- [ ] `pbh-geoip`：`maxminddb` 读 City/ASN/GeoCN；下载（`reqwest` basic-auth、三镜像、45 天更新判定）、`xz2` 解压、原子替换；GeoCN2/GeoCN1 解析 + 行政区划 CSV(`ok_data_level3.csv`) trie；`IPGeoData`（前端契约）+ GeoCN 叠加 + TW/HK/MO 命名特例；`moka` 缓存。
- [ ] IPBlackList（IP/CIDR/端口/ASN/国家/城市/中国网络类型；net-type 中文串→枚举）。
- [ ] IPBlackRuleList（`reqwest` 下载、`sha2` 缓存比对、DAT/eMule/P2P/纯文本行解析、前缀 trie、`rule_sub_log` 记录、定时刷新、磁盘回退）。

**验收：** GeoIP 查询对拍若干已知 IP（含中国 IP 的省市/ISP）；订阅解析对 DAT/P2P 样例行单测；trie 最长前缀命中；更新日志入库。

### M7 — Web 层 + 鉴权 + 静态

**任务：**
- [ ] `pbh-web`：`axum` 应用、`StdResp<T>` 信封、分页 `{page,size,total,results}`、异常→状态码精确映射（401/403/303→`/init`/400/402删/429/405/500）。
- [ ] 鉴权中间件：三通道 token（Bearer / `?token=` / 会话）、角色（去掉 `PBH_PLUS`）、fail2ban（`dashmap` /24 或 /50、10 次、15min）、UA 扫描器拦截。
- [ ] 静态 + SPA：`ServeDir` + fallback `index.html`，`/api`/`/blocklist` 先路由；未匹配 `/api/*` 返回 JSON 405。
- [ ] WS `/api/logs/stream`（`?token=`+`?offset=`、15s ping、环形缓冲回放、broadcast 推送）。
- [ ] 控制器（**逐个随模块就绪接入**，删掉 PBH_PLUS 的 13 个）：auth、oobe、metadata/manifest、general、bans、metrics(`/api/statistic/*` 保留)、downloaders、peer(去2)、torrent(去2)、alerts、btn、lab、plugins、push、utilities、sub、logs、egg、blocklist。完整清单见 `03-api-contract.md`。
- [ ] OOBE 流程（首次无 token → `/init`）。

**验收：** 前端 `dist` 放入静态目录后能登录、看到 dashboard 基础数据、实时日志流、增删下载器、查看/手动封禁；状态码语义与前端期望一致；删除的端点返回干净 404 且 manifest 不宣告。

### M8 — BTN 在线网络

**任务：**
- [ ] `pbh-btn`：HTTP 客户端中间件（固定头 + Bearer + gzip 上行）、config 端点拉取与 ability 构建（new/legacy 分支）。
- [ ] 下行 ability：HeartBeat、Rules（`?rev=`/204/缓存/`BtnRuleUpdateEvent`）、IPDenyList、IPAllowList（+解封白名单）、IpQuery、Reconfigure。
- [ ] 上行 ability：SubmitBans/SubmitSwarm/SubmitHistory（DB 游标 + KV 续传）。
- [ ] PoW（移植 `util/pow/PoWClient.java` 算法）。
- [ ] `BtnRulesetParsed` + `BtnNetworkOnline` 模块（Allow→SKIP / 脚本(stub) / Deny→BAN / Rules 分类）。
- [ ] 每 ability `tokio` 任务调度（初始随机延迟 + 固定间隔）；600s config 重试。

**验收：** 对真实/录制 BTN 端点：config 拉取、规则下行解析、IP 名单解析、心跳拿外网 IP；上行报文 gzip+字段对拍；游标重启续传；PoW 求解通过。

### M9 — 支撑服务

- [ ] Alert（DB + 去重 + 30 天清理 + 推送 + console 通知）
- [ ] Push 8 通道（`reqwest` 7 个 + `lettre` SMTP；`pulldown-cmark` 正文）
- [ ] metric（atomics + `PersistMetrics` 写 history/torrent + GeoIP 富化）
- [ ] 保留的监视模块（ActiveMonitoring/PeerRecording/SwarmTracking；SessionAnalyse/ClientAnalyse 因服务于被删图表而延后）
- [ ] UPnP 端口映射（`igd-next`，可选/延后）

**验收：** 各推送通道发测试消息成功；alert 去重与清理；统计计数器与 DB 写入一致。

### M10 — 收尾

- [ ] 打包：`rust-embed` 内嵌前端 `dist` → 单文件二进制；或外置 static 目录（`server.external-webui` 等价）。
- [ ] 配置迁移链补全 + 默认配置随包。
- [ ] 端到端验收（见 §3）；性能基线（一轮 wave 耗时、内存占用 vs JVM 版）。
- [ ] README / 部署文档（单文件运行、数据目录、配置说明）。

---

## 3. 端到端验收清单（最终）

> 「在现有下载器上基础使用体验应完全一致」是硬指标。

- [ ] 单文件二进制启动，无需任何外部数据库/服务。
- [ ] 首次启动走 OOBE，设置 token + 添加 qBittorrent，前端正常。
- [ ] qBittorrent + qBittorrentEE 都能登录、拉 peer、下发封禁并在 qB 端可见（含 EE shadowban）。
- [ ] 全部离线规则模块 + PCB + BTN 在线封禁按 `profile.yml` 默认配置工作。
- [ ] 封禁串/偏好写入与 Java 版逐字节一致（抽样对拍）。
- [ ] 前端**零改动**直接复用：登录、dashboard、封禁列表、实时日志、下载器管理、订阅规则、推送配置、BTN 状态均可用。
- [ ] 被删的 PBH Plus 端点/页面干净失效，不报脏错误。
- [ ] AutoSTUN/脚本引擎页面显示「不可用」占位，不崩。
- [ ] 24h 连续运行无内存泄漏、无 SQLITE_BUSY 致命错误、到期解封正常。

## 4. 建议的并行化与人力分配（若多人/多 agent）

- **主干串行（1 人/agent 先行）：** M0→M1→M2→M3。
- **M3 后可并行三路：**
  - 路 A：M4（离线模块）→ M5（PCB）
  - 路 B：M6（GeoIP + IP 名单）
  - 路 C：M7（Web 框架与鉴权/静态/WS，先搭骨架）
- **汇合：** M7 控制器随 A/B 模块就绪逐步接入 → M8（BTN，依赖 history 表与 web）→ M9（支撑）→ M10。
- 若用工作流编排（需用户显式开启 ultracode/workflow），适合：①各下载器端点的 fixture 对拍 ②各推送通道并行实现 ③各规则模块并行实现后统一接入。

## 5. 「完全等价」对拍策略（重点）

为达成「体验完全一致」，对以下产出物建立 golden fixture 并在 CI 对拍：
1. **qB 封禁写入串**：给定一组 IP/CIDR/peer，Java 与 Rust 生成的 `banned_IPs`/`peers`/`shadow_banned_IPs` 必须逐字节相同。
2. **规则引擎判定**：`profile.yml` 默认规则 + 一批 peer，命中结果一致。
3. **BTN 上行报文**：解 gzip 后 JSON 字段/类型一致（时间戳为 millis 数、双哈希 id 等）。
4. **关键 API 响应 JSON**：manifest、bans 列表、statistic、general/status 的 JSON 形与前端期望一致。
5. **PCB 序列回放**：固定输入序列 → 相同封禁决策与 DB 状态。
