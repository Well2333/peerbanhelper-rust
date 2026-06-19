# 修订战略:极简重构版（权威）

> 版本：v2.0 ・ 日期：2026-06-19 ・ **本文件为当前权威方案,取代 docs/02、docs/03 的规划部分。**
> docs/01（子系统事实分析）仍然有效;docs/04 已按精简表集更新。

## 1. 战略转向

| | 原方案 (v1) | 现方案 (v2,本文件) |
|---|---|---|
| 定位 | 忠实移植:复刻 Java 行为 + **原样复用 Vue 前端** | **完全重构、最精简**,目标=与原版**一致的封禁效果** |
| 前端 | 复用现有 Vue SPA,后端高保真复刻 ~90 端点 | **弃用现有 Vue 前端**;自研极简 REST API + 内置轻量单页 |
| Web 契约 | 必须字节级匹配 StdResp/Gson/SPA/OOBE/fail2ban | **自由设计**的小型清晰 API,自有简洁信封 |
| i18n | 保留 TranslationComponent(前端契约) | **砍除**,后端单语言纯字符串 |
| 外围(图表/分析/通知/桌面) | 多数保留 | **砍除** |
| 封禁引擎与规则 | 全保留 | **全保留**(用户选「封禁判定基本一致」) |
| BTN 在线 | 全保留 | **全保留**(下行+上行) |
| 封禁历史/日志 | 保留 | **保留** |

**一句话:保留完整封禁能力(规则 + BTN + 历史),砍掉沉重前端及其专属契约,以及一切只为图表/分析/桌面服务的外围。**

## 2. 四项新决策(用户已确认)

1. **效果定义 = 封禁判定基本一致** → 保留全部高价值检测规则。
2. **BTN = 完整保留**(下行拉名单/规则 + 上行上报 bans/swarm/history)。
3. **前端 = 弃用现有 Vue**;改自研极简(REST API + 内置单页)。
4. **持久化 = 保留封禁历史 + 日志**(+ PCB 状态 + 封禁快照 + BTN 所需数据)。

## 3. 保留 / 砍除 总表

### ✅ 保留(完整封禁能力)
| 部分 | 说明 |
|---|---|
| 主循环引擎 | 登录→拉 torrents→拉 peers→规则检查→下发封禁→到期解封 |
| qBittorrent + qBittorrentEE | 封禁下发字节级一致(banned_IPs / banPeers / shadowban) |
| **全部高价值规则** | PCB、IPBlackList、IPBlackRuleList(订阅)、ClientNameBlacklist、PeerIdBlacklist、AntiVampire、AutoRangeBan、MultiDialingBlocker、IdleConnectionDosProtection、PTRBlacklist |
| **BTN(完整)** | 全部 ability、PoW、gzip、游标;下行 denylist/allowlist/rules + 上行 submit bans/swarm/history |
| GeoIP(可选注入) | MaxMind + GeoCN;供 IPBlackList 的 ASN/地区/网络类型封禁 + history 富化;mmdb 缺失则降级 |
| 持久化(精简表集) | pcb_address/pcb_range、banlist、history、rule_sub_info/log、metadata、peer_records、tracked_swarm |
| BTN 上行所需采集 | PeerRecording(喂 submit_history)、SwarmTracking(喂 submit_swarm)——**轻量保留** |
| 极简 Web | 自研 REST API + 鉴权 + WS 日志流 + 内置单页 + /blocklist 导出 |

### ❌ 砍除(不影响封禁效果)
| 部分 | 原因 |
|---|---|
| **整套 Vue 前端 + 其专属 API 契约** | 弃用;不复刻 21 控制器/~90 端点/Gson/SPA/OOBE/fail2ban 细节 |
| i18n / TranslationComponent | 无前端契约;后端单语言纯字符串 |
| 图表 / 会话分析 / 客户端分析 | 纯前端图表(原已随 PBH Plus 删一部分) |
| 监视模块 SessionAnalyse / ActiveMonitoring(图表/限速部分) | 仅喂图表/小众限速 |
| traffic_journal_v3、peer_connection_metrics(+track) 表 | 纯图表数据 |
| 推送通知(8 通道) | 外围便利;无前端配置面;后续可作可选 YAML 配置项再加 |
| Alert(独立系统) | 降级为日志条目(WARN/ERROR 进日志流即可) |
| AutoSTUN、UPnP、Aviator 脚本、PF4J 插件、Laboratory、桌面 GUI、MTR、平台原生、多数据库后端、PBH Plus | 同 v1:外围/已死/付费/已定仅 SQLite |

## 4. 新版极简 REST API(自研,清晰为先)

- **鉴权:** Bearer token(配置文件设定;首启自动生成并打印一次)。简单字符串比对,不做会话 cookie。
- **信封:** 自有简洁形 `{ "ok": bool, "data": <any>, "error": <string|null> }`(不强求 Java StdResp/Gson 兼容)。
- **端点(约 18 个):**
  - `GET /api/status` — 运行状态、版本、各模块开关、BTN 概览、暂停标志
  - `GET/PUT /api/downloaders` ・ `PATCH/DELETE /api/downloaders/{id}` ・ `POST /api/downloaders/test`
  - `GET /api/downloaders/{id}/torrents` ・ `GET /api/downloaders/{id}/torrent/{hash}/peers`
  - `GET /api/bans` (当前封禁,分页) ・ `PUT /api/bans` (手动封) ・ `DELETE /api/bans` (解封)
  - `GET /api/bans/history` (封禁历史,分页/过滤)
  - `GET/PUT /api/config/profile` (规则与全局配置) ・ `POST /api/config/reload`
  - `GET/PUT/DELETE /api/sub/rules[/{id}]` ・ `POST /api/sub/rules/update` (IP 订阅规则)
  - `GET /api/btn/status`
  - `GET /api/logs` (历史) ・ `WS /api/logs/stream?token=&offset=` (实时)
  - `GET /blocklist/{ip,p2p-plain-format,dat-emule}` (纯文本,供下载器/外部消费,保留)
- **内置单页 UI:** vanilla HTML/JS(无构建工具链),`rust-embed` 内嵌。覆盖:状态、下载器增删改、封禁列表/历史、实时日志、规则与订阅配置。后续可替换,不影响 API。

## 5. 持久化(精简后表集)

保留:`pcb_address`、`pcb_range`、`banlist`、`history`、`rule_sub_info`、`rule_sub_log`、`metadata`、`peer_records`、`tracked_swarm`。
砍除:`traffic_journal_v3`、`peer_connection_metrics`、`peer_connection_metrics_track`、`alert`(降级为日志)。
连接/类型策略不变(WAL、单写者、epoch millis、TEXT IP/JSON)。详见 docs/04。

## 6. 修订里程碑

| 阶段 | 名称 | 目标 |
|---|---|---|
| **M0** | 地基 | workspace、配置加载、SQLite(精简表集)、tracing 日志+环形缓冲、AppContext |
| **M1** | 领域模型 + 规则引擎 | Peer/Torrent/PeerFlag/CheckResult/PeerAction、共享匹配引擎、BanList(**纯字符串,无 i18n**) |
| **M2** | 下载器 | Downloader trait + qBittorrent + qBittorrentEE(字节级一致) |
| **M3** | 流水线 + 调度 + BanManager | channel 流水线、Ban Wave 循环、到期解封、封禁下发、**历史落库** |
| **M4** | 规则模块(离线) | Anti/Client/PeerId/AutoRange/Idle/MultiDial/PTR |
| **M5** | PCB | ProgressCheatBlocker + 两表持久化 + 脏刷缓存 + 清理 + 解封钩子 |
| **M6** | GeoIP + IP 黑名单族 | IPDB(MaxMind+GeoCN,可选注入)、IPBlackList、IPBlackRuleList |
| **M7** | 极简 Web | 自研 REST API + Bearer 鉴权 + WS 日志流 + 内置单页 + /blocklist 导出 |
| **M8** | BTN(完整) | ability/协议/规则同步/上报 + PoW + 游标;轻量 PeerRecording/SwarmTracking 喂上行 |
| **M9** | 收尾 | 单文件打包(rust-embed 内嵌页面)、配置随包、端到端验收、文档 |

关键路径 M0→M1→M2→M3 不变;M4/M5/M6 可在 M3 后部分并行;M7 可早搭;M8 依赖 M3(history)+M7。

## 7. 与原文档的关系

- `docs/01-research-report.md`:子系统**事实分析**,仍有效(查源码基准用)。
- `docs/02-construction-guide.md`、`docs/03-api-contract.md`:**忠实移植方案(v1),已被本文件取代**,保留作参考。
- `docs/04-db-schema.md`:已更新,标注精简后保留/砍除的表。

## 8. 等价性边界(本版仍需对拍)

下载器封禁串、规则引擎判定、BTN 上下行报文、PCB 决策。**API/UI 不再要求与原版一致**(已弃用前端),故 web 层对拍要求消失,改为常规接口测试。
