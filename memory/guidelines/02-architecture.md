# 架构约定（权威）

> 本文件是**架构约定的权威单一来源**。范围/决策见 `01-scope-and-decisions.md`;路线图见 `memory/design/roadmap.md`。

## 不照搬上游（硬原则）

重构而非移植：**不照搬**上游的 Spring DI、全局 service-locator(`Main.getX()`)、"organ" 隐喻、每 wave 新建线程池、为多后端/多下载器/付费/脚本预留的层层抽象。只在有明确收益处分层;凡与封禁 peer 无关的结构一律不引入。

## Crate 分层（workspace · 10 crate）

`pbh-domain`（领域类型）← `pbh-config` / `pbh-storage` / `pbh-rules` / `pbh-downloader` /
`pbh-geoip` / `pbh-engine` / `pbh-btn` / `pbh-web` ← `pbh-server`（组合根 + 二进制 `pbh`）。

- **依赖方向单一**：领域类型在底层，业务 crate 横向不互相依赖（必要协作走 `pbh-domain` 的抽象或在 `pbh-server` 装配）。
- **组合根集中**：`pbh-server` 显式装配（构造各组件并 `Arc` 共享）。**不引入 DI 框架。**
- `pbh-geoip` 单独成 crate 仅为隔离 maxminddb/xz 等重依赖;其余按职责切分,不为「对称」而拆。

## 依赖抽象、不依赖具体（守则第 9 条，硬约束）

- 模块间通过 trait 协作；消费方注入 `Arc<dyn Trait>`，对具体实现无编译期依赖。
- **可选能力用可选注入**，拿不到则降级照常工作。当前仅一处：
  - `GeoIpProvider`（mmdb 缺失 → IPBlackList 的 ASN/region/net-type 检查跳过，其余照常）。
- （AutoSTUN 的 `NatAddressProvider`、Aviator 的 `ScriptEngine` 已**完全移除**，不再作为可选注入点。）

## 关键运行时约定

- **Ban 流水线** 用 bounded `mpsc`(容量 64) 实现；"DONE" 用 channel 关闭表达，不写轮询状态机。每阶段 `tokio::time::timeout`。
- **不每个 ban wave 新建线程池**；用共享 `tokio` 运行时 + `Semaphore`/`buffer_unordered` 限并发。
- **BanList 内存权威**：IPv4/IPv6 前缀 trie + `RwLock`，支持 CIDR 范围与最长前缀匹配；数据库仅周期快照（每小时 + 关闭时），非实时镜像。
- **调度循环**：固定延迟 `check-interval`，`try_lock` 防重叠，WatchDog 卡死重启。
- **事件**：`tokio::sync::broadcast`；可取消事件（如模块注册否决）保留为直接函数返回 `Result`/否决，不走 broadcast。
- **热重载**：`tokio::sync::watch` 广播配置变更，组件订阅。
- **peer 地址**：直接用下载器返回的原始 `ip:port`，不做 NAT 改写（AutoSTUN 已删）。
- **SQLite 单写者**：写池 `max_connections(1)`，WAL 下读可并发；清理走单线程后台 + 分块短事务（LIMIT 200）避免长写锁。

## 等价性边界（必须与上游一致的产出）

下载器封禁串（`banned_IPs`/`peers`/`shadow_banned_IPs`）、规则引擎判定、BTN 上下行报文、PCB 决策。这些处建 golden fixture 对拍（见 `03-coding-conventions.md`）。

> 自研 API/UI 不要求与原版一致（已弃用前端），不在对拍范围;后端单语言纯字符串（无 i18n）。
