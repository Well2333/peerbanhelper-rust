# 当前状态与待办（handoff）

> 最近更新：2026-07-01（**v0.1.1 发布前审核修复 + 推送发布**）。清理上下文后从这份读起，再看 `design/roadmap.md`（里程碑详情）、
> `test-status/`（已测/待测）、`changelog/`（逐提交）。流程守则见 `memory/最高优先级工作守则.md`。

## 一句话现状

PeerBanHelper 的 Rust 重写（v2 极简重构）**M0–M8 全部里程碑代码完成 + v0.1.1 WebUI 重构**，是**可运行、已真机验证的成品**：
单文件二进制 + 内置单页 Web UI，能登录 qBittorrent、拉 peer、按全部规则自动封禁、写回 qB、落库历史、
持久化 PCB 续算、下载社区 IP 订阅、GeoIP 自动下载、BTN 云端威胁情报（默认关）。

## v0.1.1（WebUI 重构 + 发布前审核修复，已合并 main、标签 v0.1.1 移至最终提交并推送）

设置页(BTN/代理/GeoIP,热生效) · 规则配置分组折叠图形化(+YAML 回退) · 下载器并入仪表盘(模态) ·
头部品牌/版本/GitHub/SVG 图标 · 检查更新 + **一键自更新**(`/api/update/apply` 下载 `pbh-rust-<target>.gz`→替换→重启) ·
GeoIP 三镜像后台重试下载(City/ASN/GeoCN,45天) · **全局代理**(`pbh-net`,BTN/订阅/GeoIP/更新走代理,下载器直连) ·
本机局域网/公网 IP 展示(配 qB 白名单) · **登录退避**(鉴权错误即暂停,避免硬刷封 IP) · 包名 `pbh`→`pbh-rust`。
详见 `changelog/2026-06-21-v0.1.1.md` 与 `docs/superpowers/specs|plans/2026-06-21-webui-overhaul*`。

## 已交付（全部里程碑）

- **M0 地基** ✅ config/储存/日志/二进制装配
- **M1 规则引擎** ✅ 匹配引擎 / IpMatcher / BanList
- **M2 下载器** ✅ qB + qBEE（真机验证）
- **M3 ban-wave 引擎** ✅ BanManager + 调度（真机验证）
- **M4 规则模块（7/7）** ✅ PeerId/ClientName/AntiVampire + AutoRangeBan/MultiDialing/IdleDos/PTR
- **M5 ProgressCheatBlocker** ✅ 反吸血核心 + DB 持久化（真机验证：追踪 222MB 上传、12 条落库、重启载入）
- **M6 GeoIP + IP 黑名单族** ✅ IP 订阅（真机验证：628 条/24 命中）+ IPBlackList + GeoIP 可选注入
- **M7 极简 Web** ✅ 单页 UI（深浅主题/仪表盘/订阅状态等）→ **v0.1.1 重构**（设置页/规则图形化/下载器模态/检查与自更新/本机IP）
- **M8 BTN 云端** ✅ PoW/哈希/序列化/BtnNetworkOnline/调度（冒烟验证；真实服务端需用户账号）

## 怎么构建/运行（关键！）

- **必须用 rustup 的 cargo**（`~/.cargo/bin/cargo`，stable 1.96）。系统自带 cargo 1.75 **无法编译**。
- 构建脚本：`./build.sh`（`build`/`run`/`test`/`clippy`/`package`/`clean`）。
- 成品：`target/release/pbh`（cargo bin 名仍为 `pbh`）、打包为 `dist/pbh-rust-0.1.1-<os>-<arch>.tar.gz`（内含可执行 `pbh-rust`）+ 自更新资产 `pbh-rust-<target>.gz`（不入库）。
- 运行：`PBH_DATA_DIR=./data ./target/release/pbh` → 日志打印一次 API token → 浏览器 `http://127.0.0.1:9898` 登录 → 加下载器。
- 全工作区单元测试全绿（21 套件）；clippy `-D warnings` 零告警；fmt 干净。
- **开发会话沙箱网络封 GitHub**（直连/经代理 TLS 均被重置）→ `git push`/拉 GitHub 须在用户本机终端跑；git 全局代理已设 `http://10.0.0.180:7890`。

## 已在真机验证（用户 qB：your-qb.example.com:50443，v5.1.3.10/EE；密码在对话里，未存仓库）

- ✅ 登录/版本/get_torrents/**get_peers 解析**/规则自动封禁/apply_ban_list 字节级写入/Web 手动封禁（M2+M3）。
- ✅ **PCB**：追踪真实上传（222MB / 自报 6%）、未误封正常 peer、批刷 12 条落库、**重启载入 29 条续算**。
- ✅ **IP 订阅**：下载 all-in-one（628 条 CIDR）、trie 命中 24 个真实 peer、rule_sub 落库、web 端点。
- ✅ **BTN**：启用后调度启动、模块计入（modules=5）、config 失败优雅 600s 重试、无崩溃。
- 每次激进/订阅测试后均恢复 qB `banned_IPs`（自动过期 + 手动清理）。

## 仓库/工程约定（务必遵守）

- 提交流程：类型分支 → squash 合并到 `main` → **两步提交 changelog**（`memory/changelog/YYYY-MM-DD-<hash>.md`）。
- **只本地提交，未 push**（远程操作需用户当次授权）。
- 提交前过 `./build.sh clippy` 与 `cargo test --workspace`（用 rustup cargo）。
- 默认启用模块：peer-id / client-name / anti-vampire / **PCB**。**默认关闭**（opt-in）：auto-range-ban、multi-dialing、idle-dos、ptr-blacklist、ip-address-blocker(IP黑名单)、ip-address-blocker-rules(订阅)、BTN。理由见各 changelog（避免开箱误伤 / 需联网 / 需凭证）。

## 已补全（2026-06-20 第二轮，全部真机验证）

- ✅ **BTN 全 ability 真机验证**（真实凭证连 sparkle.ghostchu.com）：下行 rules/denylist/allowlist;上行 **submit_bans**(64条)+ **submit_swarm**(10条);**heartbeat**(外网IP)。submit_bans 400 已修(补全 torrent_is_private/from·to_peer_traffic/downloader_progress 等必填字段)。submit_histories 服务端未 offer。
- ✅ **completed_size**：qB `/torrents/info` 的 `completed` 字段,启用 PCB excessive Case2。
- ✅ **`/blocklist/ip`**：公开纯文本封禁导出。
- ✅ **WS 实时日志流**：`/api/logs/stream`(token query),前端改 WebSocket(断线重连)。
- ✅ **SwarmTracking**：每轮记 tracked_swarm(offset 重连累加),启动清空。
- ✅ **GeoIP peer_geoip 回填** + 封禁列表 geo 富化(无 mmdb 降级)。
- ✅ **GitHub Actions**：`ci.yml`(fmt/clippy/test)+ `release.yml`(v* 标签矩阵构建 linux/windows/macos×4 → Release)。
- ✅ **测试补全**：web envelope + geoip 等,**94 单测全绿**。
- 🐛 **重大修复**：① WebUI `.hidden` CSS 缺失(登录框遮罩);② **参数路由** axum 0.7 用 `:param` 但代码写 `{param}` → `/api/bans/{ip}` 等真实 ID 404(手动解封/删下载器实际坏的),改 `:param`。CDP(headless chrome)验证前端各页正常。

## 已补全（续）

- ✅ **banlist 持久化**：save/load + 每小时快照 + 关闭快照 + 启动恢复;SIGTERM 优雅关闭。修复重启丢失封禁。真机验证快照→恢复闭环。
- ✅ **订阅增删改 WebUI**：PUT/DELETE `/api/sub/rules` 操作 profile.yml + 重建模块;前端订阅 CRUD 表单。真机验证下载 626 条/增删。

## 已补全（2026-07-01 发布前审核修复，折入 v0.1.1）

- 🐛 **GeoIP 并发下载写竞争**：后台自动下载循环不持 `geoip_lock`、与持锁的手动更新端点并发时争用固定临时名 `{file}.tmp` → 目标 `.mmdb` 可能被写成截断/损坏。改为临时名带 `pid + 原子序号`(`download.rs` 的 `TMP_SEQ`),各写各的完整文件再各自原子 rename,目标永远完整。
- ✅ **手动「更新」= 强制刷新**：`ensure_databases` 加 `force` 参数;手动端点传 `force=true`,无视 45 天/存在性门槛必定重下(此前库较新时点击为空操作)。后台循环 `force=false`。
- 🔒 **token 常数时间比较**：`auth`/`login` 从 `==` 改 `ct_eq`,消除计时侧信道。
- 🧹 **clippy 清零**：`selfupdate.rs` 的 `spawn_restart` 上移到测试模块前,消除 `items after a test module`。
- 全工作区 `clippy --all-targets -D warnings` 零告警、`cargo test --workspace` 21 套件全绿。详见 `changelog/2026-07-01-9b16695.md`。

## 待办（剩余，均为可选/低价值或需外部条件）

1. **可选低价值**：BTN legacy(<20)分支、PoW 自动获取(当前 ability 全 pow=false 不需要)、reconfigure/ip_query(非必需)、WatchDog、channel 并行流水线、R4 配置注释保留迁移、LogBuffer broadcast 真推送(当前 WS 700ms 轮询)。
2. **需外部条件**：GeoIP mmdb 文件(放 `<data>/geoip/` 即生效)、GeoCN 未移植、EE shadowban/增量封禁真机补验(需 qB 配置)、GitHub Actions 真实运行(需 push)。
3. **axum 版本**：当前 0.7（路由用 `:param`）;若升级 0.8 需把 `:param` 改回 `{param}`。

## 已知注意点 / 坑

- pbh 对 qB `banned_IPs` **权威接管**：启动首轮用自己的 BanList 覆盖（空则清空）。真机测试务必先记录后恢复。
- qB 报 `v5.1.3.10`（4 段=EE），版本解析取前 3 段→5.1.3，<5.3.0 故 RANGE_BAN 关。
- 激进模块（auto-range-ban/multi-dialing/订阅/BTN deny）默认关：在公网 swarm 上可能误伤真实用户,按需开。
- BTN 默认关，需 config.yml `btn.enabled=true` + app-id/secret。

## 提交历史锚点（main，截至 M8）

`54be292` WebUI改版 → `4e554c1` M4余下 → `c585194`+`4fad1ff` M5(核心+持久化) →
`151a035`+`2b6fde9` M6(订阅+IPBlackList/GeoIP) → `553be67` M8 BTN。（每个 feat 后跟 `docs: changelog` 提交。）
