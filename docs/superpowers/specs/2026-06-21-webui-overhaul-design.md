# 设计:WebUI 重构 + 网络代理 + GeoIP 自动下载 + 检查更新

日期:2026-06-21
状态:待实现

## 背景

当前 WebUI(`crates/pbh-web/assets/index.html`,单文件 vanilla SPA)缺少一批面向用户的能力,
且部分联网行为无代理支持、GeoIP 需手动放文件。本设计一次性补齐 11 项需求,并以"网络代理"为公共基础。

## 目标(对应用户提出的 11 点)

1. WebUI 增加 BTN 设置(当前完全没有)。
2. 修复缺失的"退出"按钮图标。
3. 标题中 `Rust` 与 `PeerBanHelper` 统一样式(不再分离)。
4. 下载器增删改并入仪表盘"下载器状态"区(去掉独立下载器页)。
5. 规则配置改为**全部模块图形化 + 中文解释**,保留 YAML 高级回退。
6. 新增"检查更新"。
7. 标题处显示版本号。
8. 右上角新增 GitHub 跳转按钮。
9. GeoIP 自动下载(遵循 PBH 本体设计)。
10. 联网请求支持代理设置;代理不通或未配置则直连;下载器不走代理。
11. GitHub Release 构建包名 `pbh` → `pbh-rust`。

## 决策记录(已与用户确认)

- GeoIP 数据源/更新:**完全遵循 PBH 本体设计**(见 §B)。
- 规则配置:**全部 9 个模块图形化 + YAML 高级回退**。
- 代理范围:**所有外网请求,排除 qBittorrent 下载器**;代理不通/未配置则直连。
- 下载器 UX:**直接在仪表盘状态区**增删改,去掉独立"下载器"页。
- BTN/代理改动:**热加载/卸载,不重启进程**。

---

## A. 网络代理基础(其余功能的公共底座)

### 配置模型
`config.yml` 新增 `network` 段:

```yaml
network:
  proxy: ""   # 形如 http://127.0.0.1:7890 或 socks5://127.0.0.1:1080;空=直连
```

在 `pbh-config::model` 新增 `NetworkConfig { proxy: String }`,挂到 `AppConfig.network`(`#[serde(default)]`)。

### 共享 HTTP 客户端构造器
新增轻量 crate `pbh-net`(仅依赖 `reqwest`、`tracing`),导出:

```rust
/// 按代理配置构造 reqwest::Client。
/// proxy 为空 → 直连;非空但 host:port 不可达(短超时 TCP 探测)→ 直连 + warn;
/// 非空且可达 → reqwest::Proxy::all(proxy)。
pub fn build_client(proxy: &str, timeout: Duration) -> reqwest::Client;

/// 仅做一次 TCP 可达性探测(用于 build_client 内部,亦可单独调用)。
pub fn proxy_reachable(proxy: &str) -> bool;
```

实现要点:解析 proxy URL 取 host:port,`TcpStream::connect_timeout`(~1s)探测;失败则 fallback 直连。
这满足"代理连不通或未配置则不使用代理"。客户端在每个使用点按需构造,因此读取的是**当前** proxy 配置。

### 接入点(全部改用 `pbh_net::build_client(current_proxy, ..)`)
- `pbh-btn`(`client.rs`):BTN 拉规则/名单/心跳/上报。
- `pbh-engine::ip_rule_list`:IP 订阅下载。
- GeoIP 下载器(§B,新增)。
- 检查更新(§F,新增)。
- **不改** `pbh-downloader::qbittorrent`:下载器保持直连。

---

## B. GeoIP 自动下载(遵循 PBH 本体)

来源:上游 `util/ipdb/IPDB.java`。

### 数据库与文件名
- `GeoIP-City.mmdb`(City)
- `GeoIP-ASN.mmdb`(ASN)
- `GeoCN.mmdb`(中国网络类型/省市)

落地目录沿用 `<data>/geoip/`。`pbh-geoip` 的加载搜索名补充 `GeoIP-City.mmdb`/`GeoIP-ASN.mmdb`
(保留原有 GeoLite2-*/City.mmdb 兼容)。

### 镜像源(按序自动回退)
1. `https://github.com/PBH-BTN/GeoLite.mmdb/releases/latest/download/`
2. `https://pbh-static.paulzzh.com/ipdb/`
3. `https://pbh-static.ghostchu.com/ipdb/`

文件名直接拼接到 base(均为直链 `.mmdb`,无需解压)。逐源尝试,失败换下一个。

### 凭证与更新
- `ip-database.account-id` / `license-key`:**仅当镜像返回 401** 时作为 `Authorization: Basic` 回退(对齐上游)。
- 过期周期:`45 天`(上游 `updateInterval = 3888000000L`)。
- 触发重下:文件缺失 **或**(`ip-database.auto-update == true` 且 `now - mtime > 45天`)。

### GeoCN 解析(填上现有空字段)
`pbh-geoip` 增加 GeoCN reader,把记录映射到 `IpGeoData.net_type` / `cn_province` / `cn_city`
(GeoCN 记录含 `isp`/`net`/`province`/`city`/`districts` 等;用 maxminddb 自定义 struct 反序列化)。
启用后 `ip-address-blocker` 的 net-type / 地区检查即可生效。

### 热加载(provider 可替换)
现状:`Option<Arc<dyn GeoIpProvider>>` 在启动时构造一次,克隆进 `BanManager` 与 `WebState`。
改为可热替换的 `GeoIpHandle`(`arc-swap` 包 `ArcSwapOption<dyn GeoIpProvider>`),`query()` 委托当前值。
- `BanManager` 与 `WebState` 改持 `GeoIpHandle`。
- `build_modules` 的 geoip 入参随之改为 `&GeoIpHandle`。
- 启动时:先尝试用现有文件构造 provider 装入 handle;若文件缺失,后台任务下载完成后 `handle.install(provider)` 热替换。
- 设置页提供"立即更新 GeoIP"按钮(`POST /api/geoip/update`),手动触发下载 + 热替换。

---

## C. config.yml 读写 API + 新增「设置」页

### 后端
新增受保护端点:
- `GET /api/config/app`:返回可编辑子集 —— `btn`(enabled/config-url/submit/app-id/app-secret)、
  `ip-database`(account-id/license-key/auto-update)、`network.proxy`、`persist.ban-logs-keep-days`。
  (端点在鉴权后,敏感字段原样返回供编辑。)
- `PUT /api/config/app`:接收同结构,合并写回 `config.yml` 并热重载;随后执行 §H 的热加载副作用。

### 前端
新增导航页 **设置**,分区:
- **BTN 云端情报**:enabled 开关、config-url、submit 开关、app-id、app-secret,带说明。
- **GeoIP / IP 库**:account-id、license-key、auto-update 开关、"立即更新 GeoIP"按钮、当前库状态(是否已加载)。
- **网络代理**:proxy URL 输入、说明(留空=直连;不通自动直连)。
- 每区独立"保存"。保存即生效(见 §H),不需重启。

---

## D. 规则配置:全部模块图形化 + YAML 回退

### 覆盖模块(全部 9 个)
`progress-cheat-blocker`、`peer-id-blacklist`、`client-name-blacklist`、`ip-address-blocker`、
`ip-address-blocker-rules`(并入 IP 订阅管理)、`multi-dialing-blocker`、
`idle-connection-dos-protection`、`ptr-blacklist`、`auto-range-ban`。

### 顶部全局项
`check-interval`、`ban-duration`、`ignore-peers-from-addresses`。

### 交互与数据流
- 进入页面 `GET /api/config/profile` 取当前 YAML,前端解析为对象。
- 每个模块一张卡片:启用开关 + 该模块字段表单 + 中文解释。
- 保存时:前端在**原始 profile 对象**上只覆写已知字段(未知键保留,避免破坏高级用户的自定义),
  序列化为 YAML 走现有 `PUT /api/config/profile`(后端不变,仍校验 + 热重建模块)。
- 保留**「高级(YAML)」可折叠编辑器**(即现有 textarea)作为回退,与表单同源。
- 前端需要一个 YAML 序列化能力:内置极简 JS YAML 库(`include_str!` 嵌入,或手写有限子集)。
  倾向嵌入一个小型 MIT 许可的 js-yaml 单文件,避免外网 CDN 依赖(单文件零依赖原则)。

> 注:`check-interval` / 全局 `ban-duration` 改动仍需重启才完全生效(ban-wave 循环按启动值);
> 模块级改动即时生效。UI 对前者标注"需重启"。

---

## E. 下载器并入仪表盘

- 删除独立"下载器"导航页与 `page-downloaders`。
- 仪表盘"下载器状态"区:
  - 顶部 `+ 添加下载器` 按钮 → 弹出模态表单(复用原表单字段)。
  - 每张下载器卡片显示在线/离线/暂停状态 + "编辑"/"删除"按钮;编辑打开同一模态。
- 复用现有端点:`GET /api/downloaders`、`PUT /api/downloaders`、`DELETE /api/downloaders/:id`、
  `POST /api/downloaders/test`。
- 导航与路由表去掉 `downloaders`。

---

## F. 头部:品牌 / 版本 / 检查更新 / GitHub

- **标题**:统一为 `PeerBanHelper-Rust`(去掉分离的灰色小号 `Rust`)。
- **版本号**:标题旁显示 `v{version}`,取自 `/api/status` 的 `version` 字段(已有)。
- **GitHub 按钮**:右上角小图标按钮,新窗口打开 `https://github.com/Well2333/peerbanhelper-rust`。
- **退出图标**:把 `⏻`(渲染缺失)换成内嵌 SVG(power/exit);主题切换也改为内嵌 SVG,避免字体缺字。
- **检查更新**:
  - 后端 `GET /api/update/check`:用代理客户端请求
    `https://api.github.com/repos/Well2333/peerbanhelper-rust/releases/latest`(带 User-Agent),
    返回 `{ current, latest, newer, html_url }`。`newer` 由语义化版本比较得出。
  - 前端:头部在 `newer` 时显示一个小"有新版本"徽标(点击去 release 页);设置页放"检查更新"按钮显示详情。

---

## G. Release 包名 pbh → pbh-rust

- `.github/workflows/release.yml`:
  - 压缩包前缀 `pbh-${ver}` → `pbh-rust-${ver}`。
  - 拷入压缩包的可执行从 `pbh`/`pbh.exe` 重命名为 `pbh-rust`/`pbh-rust.exe`。
- `build.sh` 的 `package` 步骤同步改名。
- Cargo `[[bin]]` 名仍保持 `pbh`(产物路径不变,打包时重命名),避免牵动源码内 `env!`/路径假设。
- README 中相关下载/运行示例同步更新为 `pbh-rust`。

---

## H. 热加载副作用(BTN / 代理,保存即生效)

引入 `BtnManager`(放 `pbh-server` 或 `pbh-btn`),持有:当前 `SharedBtnState` + 后台任务中止句柄
(`tokio` 任务的 `AbortHandle` 或 `CancellationToken`)+ 启动所需依赖(db、installation-id)。

`PUT /api/config/app` 保存并 reload 后,依据新旧 `AppConfig` 差异执行:
- **BTN 关闭**:中止任务,清空/置空共享状态。
- **BTN 启用 / 凭证变 / 代理变**:中止旧任务,用**新代理客户端**重启 BTN 调度。
- **重建规则模块**:`BtnNetworkOnline` 模块按 BTN 是否启用动态增减(`btn_state` 为 None 时不构建该模块);
  代理变更也重建模块,让 `ip-address-blocker-rules` 的订阅下载客户端取到新代理。

`BtnManager::current_state() -> Option<SharedBtnState>`:仅在启用且运行时返回 `Some`,供 `build_modules` 使用。

`WebState` 改持 `Arc<BtnManager>` 取代 `Option<SharedBtnState>`;`main.rs` 启动时用 `BtnManager` 完成初始装配。

GeoIP/订阅/检查更新客户端按需新建、读当前代理,代理改动天然生效,无需重启。

---

## 受影响文件(概览)

- `crates/pbh-config/src/model.rs`:`NetworkConfig` + 挂载。
- `crates/pbh-net`(新):`build_client` / `proxy_reachable`。
- `crates/pbh-geoip/src/`:`GeoIpHandle`(arc-swap)、`download.rs`(镜像回退 + 45 天)、GeoCN 解析、加载搜索名。
- `crates/pbh-btn/`:`client.rs` 用 `pbh-net`;`spawn` 返回可中止句柄;`BtnManager`。
- `crates/pbh-engine/`:`ip_rule_list.rs` 用 `pbh-net`;`build_modules` geoip 入参改 `&GeoIpHandle`;`ban_manager` 字段类型。
- `crates/pbh-web/src/routes.rs`:`/api/config/app`(GET/PUT)、`/api/geoip/update`、`/api/update/check`;BTN 热加载副作用。
- `crates/pbh-web/src/lib.rs`:`WebState`(geoip→handle、btn→BtnManager)。
- `crates/pbh-web/assets/index.html`:头部、设置页、规则图形化、下载器并入仪表盘、SVG 图标、嵌入 yaml 库。
- `crates/pbh-server/src/main.rs` / `context.rs`:装配 GeoIpHandle + BtnManager + 启动 GeoIP 后台下载。
- `.github/workflows/release.yml`、`build.sh`、`README.md`:包名 pbh-rust。

## 测试策略

- 单元:`pbh-net` 代理探测回退;版本比较(newer 判定);GeoCN 记录反序列化;profile 表单→YAML 只覆写已知键的往返。
- 手动:设置页改 BTN/代理后无需重启即生效;GeoIP 缺文件时后台下载并热加载;规则图形化保存生效;下载器模态增删改;检查更新徽标;退出/GitHub 图标显示;release 产物名 `pbh-rust`。

## 非目标 / YAGNI

- 不做带注释保留的 YAML 迁移(沿用现状)。
- 不做 MaxMind 官方 tar.gz 解压路径(镜像直链已覆盖;401 Basic 回退保留)。
- 代理可达性为构造时探测,不做每请求级实时探测/切换。
