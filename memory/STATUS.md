# 当前状态与待办（handoff）

> 最近更新：2026-06-20。清理上下文后从这份读起，再看 `design/roadmap.md`（里程碑详情）、
> `test-status/`（已测/待测）、`changelog/`（逐提交）。流程守则见 `memory/最高优先级工作守则.md`。

## 一句话现状

PeerBanHelper 的 Rust 重写（v2 极简重构）**M0–M8 全部里程碑代码完成**，是**可运行、已真机验证的成品**：
单文件二进制 + 内置多页 Web UI，能登录 qBittorrent、拉 peer、按全部规则自动封禁、写回 qB、落库历史、
持久化 PCB 续算、下载社区 IP 订阅、可选 GeoIP、BTN 云端威胁情报（默认关）。

## 已交付（全部里程碑）

- **M0 地基** ✅ config/储存/日志/二进制装配
- **M1 规则引擎** ✅ 匹配引擎 / IpMatcher / BanList
- **M2 下载器** ✅ qB + qBEE（真机验证）
- **M3 ban-wave 引擎** ✅ BanManager + 调度（真机验证）
- **M4 规则模块（7/7）** ✅ PeerId/ClientName/AntiVampire + AutoRangeBan/MultiDialing/IdleDos/PTR
- **M5 ProgressCheatBlocker** ✅ 反吸血核心 + DB 持久化（真机验证：追踪 222MB 上传、12 条落库、重启载入）
- **M6 GeoIP + IP 黑名单族** ✅ IP 订阅（真机验证：628 条/24 命中）+ IPBlackList + GeoIP 可选注入
- **M7 极简 Web** ✅ 阶段 A 已重做为多页 UI（深浅主题/仪表盘/订阅状态等）
- **M8 BTN 云端** ✅ PoW/哈希/序列化/BtnNetworkOnline/调度（冒烟验证；真实服务端需用户账号）

## 怎么构建/运行（关键！）

- **必须用 rustup 的 cargo**（`~/.cargo/bin/cargo`，stable 1.96）。系统自带 cargo 1.75 **无法编译**。
- 构建脚本：`./build.sh`（`build`/`run`/`test`/`clippy`/`package`/`clean`）。
- 成品：`target/release/pbh`（单文件）、`dist/pbh-0.1.0-linux-x86_64.tar.gz`（5.7M，不入库）。
- 运行：`PBH_DATA_DIR=./data ./target/release/pbh` → 日志打印一次 API token → 浏览器 `http://127.0.0.1:9898` 登录 → 加下载器。
- **85 单元测试**全绿；clippy `-D warnings` 零告警；fmt 干净。

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

## 待办（剩余打磨，非阻塞）

1. **BTN 真机对拍**：需用户 BTN `app-id`/`app-secret`。验证序列化字节级一致（时间格式/structured_data/InetAddress 串）、PoW、游标续传。submit_swarm/history 上行未实现（需 peer_records/tracked_swarm 采集模块）。
2. **GeoIP 数据**：放 `<data>/geoip/GeoLite2-City.mmdb` + `GeoLite2-ASN.mmdb` 即启用 asn/region/city 检查；GeoCN（net-type/行政区划）未移植。封禁历史 `peer_geoip` 回填 + WebUI 地理展示未做。
3. **下载器增强**：`completed_size` 仍 -1（PCB excessive Case2 需 `/torrents/properties` 补）；增量封禁/EE shadowban 真机补验。
4. **WebUI**：WS 日志流（当前 3s 轮询）、`/blocklist` 导出、订阅的增删改 UI（当前经 YAML 编辑 + 状态展示）、图表。
5. **工程**：channel 并行流水线 + WatchDog + 每小时 banlist 快照（当前顺序）；配置注释保留迁移（R4）；rust-embed（当前 include_str! 单文件已够）。

## 已知注意点 / 坑

- pbh 对 qB `banned_IPs` **权威接管**：启动首轮用自己的 BanList 覆盖（空则清空）。真机测试务必先记录后恢复。
- qB 报 `v5.1.3.10`（4 段=EE），版本解析取前 3 段→5.1.3，<5.3.0 故 RANGE_BAN 关。
- 激进模块（auto-range-ban/multi-dialing/订阅/BTN deny）默认关：在公网 swarm 上可能误伤真实用户,按需开。
- BTN 默认关，需 config.yml `btn.enabled=true` + app-id/secret。

## 提交历史锚点（main，截至 M8）

`54be292` WebUI改版 → `4e554c1` M4余下 → `c585194`+`4fad1ff` M5(核心+持久化) →
`151a035`+`2b6fde9` M6(订阅+IPBlackList/GeoIP) → `553be67` M8 BTN。（每个 feat 后跟 `docs: changelog` 提交。）
