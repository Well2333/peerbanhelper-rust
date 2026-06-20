# PeerBanHelper-Rust

> BitTorrent 反吸血 / 自动封禁工具 —— [PeerBanHelper](https://github.com/PBH-BTN/PeerBanHelper) 的 Rust 重写：**单文件、零依赖、内置网页界面**。

[![License: GPL v3](https://img.shields.io/badge/License-GPLv3-blue.svg)](LICENSE)

自动识别并封禁连接到你 qBittorrent 的**吸血 / 作弊 / 恶意 peer**（迅雷、离线下载、谎报进度、IP 黑名单等），把它们写进下载器的封禁表,保护你的上传带宽。

- 🦀 **单文件原生二进制**,无需 JVM / 数据库 / 任何外部依赖(内置 SQLite)。
- 🌐 **内置网页界面** —— 浏览器打开即用,看封禁、改规则、管订阅。
- 🛡️ **完整封禁能力** —— 全部高价值检测规则 + BTN 云端威胁情报 + 封禁历史持久化。
- ⚡ 默认开箱即用,封禁判定对齐原版效果。

---

## 快速开始

### 1. 获取程序

- **下载**:从 [Releases](../../releases) 下载对应平台的压缩包(Linux / Windows / macOS),解压得到 `pbh-rust` 可执行文件。
- **或自行构建**:见文末「从源码构建」。

### 2. 运行

```bash
# Linux / macOS
PBH_DATA_DIR=./data ./pbh-rust

# Windows (PowerShell)
$env:PBH_DATA_DIR="./data"; ./pbh-rust.exe
```

首次启动会在 `./data/` 生成配置和数据库,并在**日志里打印一次 API token**(形如 `→ d2dd7431...`),请记下。

### 3. 打开网页界面

浏览器访问 **http://127.0.0.1:9898**,用上面的 token 登录,然后:

1. 在「**下载器**」页添加你的 qBittorrent / qBittorrentEE(端点、用户名、密码),点「测试连接」确认;
2. 程序每 5 秒自动跑一轮:登录 → 拉取 peer → 规则判定 → 把命中的坏 peer 写入 qB 封禁表;
3. 在「**仪表盘 / 封禁列表 / 封禁历史 / 实时日志**」实时查看,也能手动封禁/解封 IP。

> 不想开界面?直接编辑 `./data/config/downloaders.yml` 后重启也可。

---

## 网页界面

| 页面 | 功能 |
|---|---|
| **仪表盘** | 运行统计(检查/封禁/解封/Wave)、下载器在线状态 |
| **封禁列表** | 当前封禁的 peer(搜索、手动封禁/解封、地理信息) |
| **封禁历史** | 已落库的封禁记录(分页) |
| **下载器** | 增删改 qBittorrent(含 EE 影子封禁) |
| **规则配置** | 编辑 `profile.yml`(保存即生效)+ IP 黑名单订阅增删改 |
| **实时日志** | WebSocket 实时日志流 |

支持深色 / 浅色主题。

---

## 封禁规则

**默认启用**(精确、低误伤):

- **进度作弊检测(PCB)** —— 反吸血核心:追踪你给每个 peer 的实际上传量 vs 它自报的进度,识破谎报进度、过量下载、进度回退、计数器重置的吸血客户端。状态持久化,重启续算。
- **PeerID / 客户端名黑名单** —— 封禁已知的离线下载 / 吸血客户端(迅雷、QQ旋风等)。
- **反吸血(迅雷预设)**。

**默认关闭**(按需在「规则配置」页开启,可能扩大封禁面):

- **IP 黑名单订阅** —— 订阅社区维护的封禁名单(如 all-in-one),自动下载并封禁。
- **IP 黑名单** —— 按 IP / 端口 / ASN / 地区 / 城市封禁。
- **自动段封禁、多拨号封禁、空闲连接 DoS 防护、PTR 反向 DNS 黑名单**。

### 可选:BTN 云端威胁情报

加入 [BTN](https://github.com/PBH-BTN/PeerBanHelper)(BitTorrent Threat Network),与全网共享封禁情报:下载社区规则/黑白名单封禁恶意 peer,可选上报你的封禁/swarm 数据贡献网络。需在 `config.yml` 填入 `btn.app-id` / `app-secret` 并设 `enabled: true`。

### 可选:GeoIP

把 MaxMind 的 `GeoLite2-City.mmdb` / `GeoLite2-ASN.mmdb` 放进 `<数据目录>/geoip/`,即启用按地区 / ASN / 城市封禁与封禁列表的地理信息显示。无文件时自动降级(不影响其它功能)。

---

## 配置

数据目录(默认 `./data`,可用环境变量 `PBH_DATA_DIR` 指定)结构:

```
data/
├── config/
│   ├── config.yml         # 基础设施:端口、token、BTN、IP 库
│   ├── profile.yml        # 封禁行为:检查间隔、封禁时长、各规则模块
│   └── downloaders.yml    # 下载器列表
├── persist/peerbanhelper-nt.db   # SQLite(封禁历史/PCB/订阅等)
├── logs/                  # 按日志文件
└── geoip/                 # (可选)放 MaxMind mmdb
```

绝大多数设置可在网页「规则配置」页直接改(保存即生效,规则模块无需重启)。`/blocklist/ip` 端点以纯文本导出当前封禁列表,供外部消费。

---

## 从源码构建

需 Rust **≥ 1.85**(系统旧版 1.75 无法编译)。未装 rustup:

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable -c clippy -c rustfmt
```

构建:

```bash
./build.sh            # 发布构建 → target/release/pbh
./build.sh run        # 构建并运行(调试版)
./build.sh test       # 全部单元测试
./build.sh package    # 打包 → dist/pbh-rust-<ver>-<os>-<arch>.tar.gz
```

或手动 `cargo build --release -p pbh-server`。

发版:推送 `v*` 标签触发 GitHub Actions 自动构建 Linux / Windows / macOS 可执行并附加到 Release。

---

## 许可

本项目遵循上游 PeerBanHelper 的许可,采用 **[GPL-3.0-or-later](LICENSE)**。

## 致谢

本项目是 [**PeerBanHelper**](https://github.com/PBH-BTN/PeerBanHelper)(by PBH-BTN / Ghost_chu,GPL-3.0)的 Rust 重写,封禁逻辑、BTN 协议、规则设计均参考并致敬原项目。感谢原作者与 BTN 社区。
