# 当前状态与待办（handoff）

> 最近更新：2026-06-19。清理上下文后从这份读起，再看 `design/roadmap.md`（里程碑详情）、
> `test-status/`（已测/待测）、`changelog/`（逐提交）。流程守则见 `memory/最高优先级工作守则.md`。

## 一句话现状

PeerBanHelper 的 Rust 重写（v2 极简重构）已是**可运行、已真机验证的成品**：
单文件二进制 + 内置 Web 单页，能登录 qBittorrent、拉 peer、按规则自动封禁、写回 qB、落库历史。
已交付里程碑：**M0 地基 / M1 规则引擎 / M2 下载器 / M3 ban-wave 引擎 / M4(部分:3 规则) / M7 Web 界面**。

## 怎么构建/运行（关键！）

- **必须用 rustup 的 cargo**（`~/.cargo/bin/cargo`，已装 stable 1.96）。系统自带 cargo 1.75 **无法编译**（现代依赖要 edition2024/≥1.85）。
- 构建脚本：`./build.sh`（`build`/`run`/`test`/`clippy`/`package`/`clean`）。
- 成品：`target/release/pbh`（11M 单文件）、`dist/pbh-0.1.0-linux-x86_64.tar.gz`（4.6M，不入库）。
- 运行：`PBH_DATA_DIR=./data ./target/release/pbh` → 日志打印一次 API token → 浏览器 `http://127.0.0.1:9898` 登录 → 加下载器。

## 已在真机验证（用户的 qB：your-qb.example.com:50443，v5.1.3.10/EE；密码在对话里，未存仓库）

✅ 登录、版本、get_torrents、**get_peers 解析**、**规则自动封禁**（µTorrent 6 peer）、apply_ban_list 全量(`banned_IPs` 字节级写入)、Web 手动封禁。每次测后都恢复了 qB 原 `banned_IPs`。
→ **M2+M3 核心闭环真机跑通**。

## 仓库/工程约定（务必遵守）

- 提交流程：类型分支(`feat/fix/chore/docs/test`) → squash 合并到 `main` → **两步提交 changelog**(`memory/changelog/YYYY-MM-DD-<hash>.md`)。
- **只本地提交，未 push**（远程操作需用户当次授权）。当前所有工作在本地 `main`。
- 提交前过 `./build.sh clippy`（clippy -D warnings + fmt）与 `cargo test --workspace`（用 rustup cargo）。
- 架构/范围权威在 `memory/guidelines/`；详细设计在 `memory/design/`；`docs/` 留给未来端用户文档（现仅空）。
- 已确认范围：仅 qB/qBEE；**完全移除** 脚本引擎/AutoSTUN/NAT/UPnP/i18n/图表/推送/Alert/PBH Plus；弃用上游 Vue 前端改自研极简 API + 内置单页；仅嵌入式 SQLite。

## 待办（按优先级）

1. **M5 — ProgressCheatBlocker（PCB）**：反吸血核心。两表 `pcb_address/pcb_range` 已在 schema/迁移里；需实现逐 peer/逐前缀上传跟踪、fastPcbTest(BAN_FOR_DISCONNECT)、excessive/difference(ban-delay 窗口)/rewind 子检查、脏刷缓存、8h 清理、解封钩子。**需补 BanMetadata 的 serde/chrono**（M1 时延后了）。最好建序列回放测试套件。
2. **M4 余下规则**：AutoRangeBan（依赖 BanList，放 pbh-engine）、IdleConnectionDosProtection、MultiDialingBlocker、PTRBlacklist（hickory-resolver）。
3. **M6 — GeoIP + IP 黑名单族**：`maxminddb`+GeoCN（可选注入，缺失降级）；IPBlackList（IP/CIDR/端口/ASN/地区/中国网络类型）；IPBlackRuleList（订阅下载/SHA-256/DAT-eMule-P2P 解析/前缀 trie/`rule_sub_log`/定时刷新）。
4. **M8 — BTN 云端网络（完整）**：ability 系统、gzip 上行、PoW、游标、下行 denylist/allowlist/rules、上行 submit bans/swarm/history、`BtnNetworkOnline` 模块。需轻量 PeerRecording/SwarmTracking 喂上行。
5. **真机补验**（可选，需用户 qB）：增量封禁(`increment-ban=true` → `/transfer/banPeers`)、EE shadowban（需服务端开启）。
6. **Web 完善**：WS 日志流(当前 3s 轮询)、`/blocklist` 导出、规则/订阅配置页(随 M4/M6 模块)、`profile.yml` 编辑端点。
7. **工程**：channel 并行流水线 + WatchDog + 每小时 banlist 快照(当前顺序执行/无 WatchDog)；配置注释保留式迁移(R4，当前 serde 重生成)。

## 已知注意点 / 坑

- pbh 对 qB 的 `banned_IPs` 是**权威接管**：启动首轮会用自己的 BanList 覆盖（空则清空）。真机测试务必先记录、后恢复用户原值。
- qB 报 `v5.1.3.10`（4 段=EE），版本解析取前 3 段→5.1.3，<5.3.0 故 RANGE_BAN 关（CIDR 段封禁会被丢弃，单 IP 不受影响）。
- 登录副作用：默认 set `enable_multi_connections_from_same_ip=false`（用户那台本就 false）。
- `completed_size` 暂为 -1（M5 要经 `/torrents/properties` 补，PCB 需要）。
- UA 用 `PeerBanHelper-Rust/<ver>`（非上游字节一致，仅 qB 日志可见）。

## 提交历史锚点（main）

`6700dc9` 引导 → `6b1efde` 报告+骨架 → `bf6c6bf` 转向 v2 → `5755e5e` 删脚本/AutoSTUN → `d6d0a65` 文档归位 →
`9c3246d` M0 → `b37d686` M1 → `51f1511` M2 → `8ea5415` M3 → `19059ed` M7 → `744e226` build.sh →
`263ab7b`/`305ba0b` 真机验证。
