# PeerBanHelper-Rust API 契约（前端复用基准）

> 来源：分析 `web/JavalinWebContainer.java` + `module/impl/webapi/*Controller.java` + 前端 `webui/src/service|stores|api`。
> Rust 端用 `axum` 复刻。**删除标记 `[删除]` 的端点本期不实现**（PBH Plus，用户已确认）。

## 通用约定

- **基址：** 同源，REST 在 `/api/*`，另有 `/blocklist/*` 与 `/api/egg`。无全局前缀。
- **鉴权：** 单一共享 token。客户端每请求带 `Authorization: Bearer <token>`；WS 用 `?token=`；也接受 `?token=` query 与会话 cookie。
- **请求头（前端发）：** `Authorization`、`Content-Type: application/json`、`Accept-Language`、`X-TimeZone`。
- **响应信封 `StdResp`：** `{ "success": bool, "message": string|null, "data": any|null }`。
- **分页 data：** `{ "page": i64, "size": i64, "total": i64, "results": [...] }`；请求 `?page=`(默认1)`&pageSize=`(默认10)`&search=`(URI 编码)。亦存在 legacy 游标分页 `?limit=&lastBanTime=&search=`。
- **状态码语义（前端依赖，必须精确）：** `200` 成功；`401`/`403` token 无效/未登录 → 前端重登录；`303` + `Location: /init` → OOBE；`400` 参数错；`429` fail2ban；`405` 未知 `/api/*` 路由；`500` 兜底。
- **角色：** `ANYONE` / `USER_READ` / `USER_WRITE`（demo 模式写→400）。~~`PBH_PLUS`~~ 删除。
- **路由顺序敏感：** `/api/general/global` 先于 `/{configName}`；`/api/torrent/query` 先于 `/{infoHash}`；`/api/sub/rules` 先于 `/api/sub/rule/{ruleId}`。

## 端点目录

### 鉴权 PBHAuthenticateController
- `POST /api/auth/login` `{token}` → 设会话/校验 [ANYONE]
- `POST /api/auth/logout` [ANYONE]

### OOBE PBHOOBEController（仅未初始化时注册）
- `POST /api/oobe/init` 设 token + 建首个下载器 [ANYONE]
- `POST /api/oobe/scanDownloader` 扫描本地下载器 [ANYONE]
- `POST /api/oobe/testDownloader` 校验下载器草稿 [ANYONE]
- `POST /api/oobe/testDatabaseConfig` 测试外部 DB（Rust 仅 SQLite，可简化/恒成功） [ANYONE]

### 元数据 PBHMetadataController
- `GET /api/metadata/manifest` → `{version:{version,os,branch,commit,abbrev}, analytics, modules:[{className,configName}]}` [ANYONE]（version 须 ≥4.0.0）

### 通用 PBHGeneralController
- `GET /api/general/status` → `{globalPaused, analytics, ...}` [USER_READ]
- `POST /api/general/refreshNatStatus` [USER_WRITE]
- `GET /api/general/checkModuleAvailable` [USER_READ]
- `GET /api/general/stacktrace` [USER_READ]
- `GET /api/general/heapdump` [USER_WRITE]（Rust 可返回不支持/占位）
- `POST /api/general/reload` 重载配置/模块 [USER_WRITE]
- `GET /api/general/global` ・ `PATCH /api/general/global` 全局运行配置 [READ/WRITE]
- `GET /api/general/{configName}` ・ `PUT /api/general/{configName}` 命名配置读写 [READ/WRITE]

### 封禁 PBHBanController
- `GET /api/bans` 当前封禁（分页/过滤） [USER_READ]
- `GET /api/bans/logs` 封禁历史 [USER_READ]
- `GET /api/bans/ranks` 封禁排行 [USER_READ]
- `DELETE /api/bans` 解封 [USER_WRITE]
- `PUT /api/bans` 手动封禁 [USER_WRITE]

### 统计 PBHMetricsController（**保留**）
- `GET /api/statistic/counter` 基础计数 [USER_READ]
- `GET /api/statistic/analysis/field` `?type=&field=&filter=&downloader=` [USER_READ]
- `GET /api/statistic/analysis/banTrends` [USER_READ]
- `GET /api/statistic/analysis/date` [USER_READ]

### 图表 PBHChartController — **[删除] 全部 7 个（PBH_PLUS）**
- ~~`/api/chart/{geoIpInfo,trend,traffic,sessionBetween,sessionDayBucket,sessionAnalyse,clientAnalyse}`~~

### 下载器 PBHDownloaderController
- `GET /api/downloaders` [USER_READ]
- `POST /api/downloaders/scan` [USER_WRITE]
- `PUT /api/downloaders` 新建 [USER_WRITE]
- `PATCH /api/downloaders/{id}` 更新 [USER_WRITE]
- `POST /api/downloaders/test` 测试 [USER_WRITE]
- `DELETE /api/downloaders/{id}` [USER_WRITE]
- `GET /api/downloaders/{id}/status` [USER_READ]
- `GET /api/downloaders/{id}/torrents` [USER_READ]
- `GET /api/downloaders/{id}/torrent/{torrentId}/peers` [USER_READ]

### Peer PBHPeerController
- `GET /api/peer/{ip}` [USER_READ]
- ~~`GET /api/peer/{ip}/accessHistory`~~ **[删除]**
- ~~`GET /api/peer/{ip}/banHistory`~~ **[删除]**
- `GET /api/peer/{ip}/btnQuery` BTN 情报查询 [USER_READ]
- `GET /api/peer/{ip}/btnQueryIframe` HTML iframe [USER_READ]

### 种子 PBHTorrentController
- `GET /api/torrent/query` 分页 [USER_READ]
- `GET /api/torrent/{infoHash}` [USER_READ]
- ~~`GET /api/torrent/{infoHash}/accessHistory`~~ **[删除]**
- ~~`GET /api/torrent/{infoHash}/banHistory`~~ **[删除]**

### 告警 PBHAlertController
- `GET /api/alerts` 分页 [USER_READ]
- `PATCH /api/alert/{id}/dismiss` [USER_WRITE]
- `POST /api/alert/dismissAll` [USER_WRITE]
- `DELETE /api/alert/{id}` [USER_WRITE]

### AutoSTUN PBHAutoStunController — **本期占位**（返回 disabled/不可用，不删端点以免前端崩）
- `GET /api/autostun/status` → `{enabled:false, ...}` [USER_READ]
- 其余（refreshNatType/restart/tunnels/tunnel/.../config）返回「不可用」占位 [READ/WRITE]

### BTN PBHBtnController
- `GET /api/modules/btn` BTN 状态与 abilities [USER_READ]

### PBH Plus PBHPlusController — **[删除] 整组**
- ~~`/api/pbhplus/*`~~

### 实验室 PBHLabController
- `POST/GET /api/laboratory/config` [USER_WRITE]
- `GET /api/laboratory/experiments` [USER_READ]
- `GET /api/laboratory/isExperimentActivated` [USER_READ]
- `PUT /api/laboratory/experiment/{id}` [USER_WRITE]

### 插件 PBHPluginController
- `GET /api/plugins` [USER_READ]（Rust 无 PF4J，返回空/占位）
- `POST /api/plugins/operate` [USER_WRITE]

### 推送 PBHPushController
- `GET /api/push` [USER_READ]
- `PUT /api/push` 新建 [USER_WRITE]
- `PATCH /api/push/{name}` 更新 [USER_WRITE]
- `POST /api/push/test` 测试 [USER_WRITE]
- `DELETE /api/push/{name}` [USER_WRITE]

### 工具 PBHUtilitiesController
- `POST /api/utilities/replaceTracker` 批量替换 tracker [USER_WRITE]

### 订阅规则 RuleSubController
- `GET/PATCH /api/sub/interval` [READ/WRITE]
- `PUT /api/sub/rule` 新增 [USER_WRITE]
- `POST /api/sub/rule/{ruleId}/update` 更新内容 [USER_WRITE]
- `GET /api/sub/rule/{ruleId}` [USER_READ]
- `POST /api/sub/rule/{ruleId}` 保存 [USER_WRITE]
- `DELETE /api/sub/rule/{ruleId}` [USER_WRITE]
- `PATCH /api/sub/rule/{ruleId}` 启停 [USER_WRITE]
- `GET /api/sub/rules` 列表 [USER_READ]
- `POST /api/sub/rules/update` 全部更新 [USER_WRITE]
- `GET /api/sub/logs` ・ `GET /api/sub/logs/{ruleId}` 更新日志 [USER_READ]

### 日志 PBHLogsController
- `GET /api/logs/history` 环形缓冲全量 [USER_WRITE]
- `WS /api/logs/stream` `?token=&offset=` 实时（15s ping，帧 `StdResp{data:{time,thread,level,content,seq}}`） [USER_WRITE]

### 彩蛋 PBHEasterEggController
- `GET /api/egg` 302 随机跳转 [public]

### 黑名单导出 BlockListController（**非 /api**，纯文本，供下载器消费）
- `GET /blocklist/ip` [public]
- `GET /blocklist/p2p-plain-format` [ANYONE]
- `GET /blocklist/dat-emule` [ANYONE]

## Rust 实现备注
- `StdResp<T>` 泛型 + 动态处用 `serde_json::Value`；对齐 Gson 字段名（camelCase）与数字类型（epoch millis 为 long）。
- 鉴权中间件统一处理三通道 token + 角色 + fail2ban + UA 扫描器拦截 + 异常→状态码映射。
- 静态/SPA fallback 必须让 `/api/*`、`/blocklist/*` 先路由，未匹配 `/api/*` 返回 JSON 405（而非 index.html）。
- WS 浏览器不能设头，必须支持 `?token=`。
- 删除端点：直接不注册路由；manifest 的 `modules` 不宣告对应模块，前端据此隐藏菜单/路由。
