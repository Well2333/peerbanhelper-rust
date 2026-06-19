# 架构约定

## Crate 分层（workspace）

`pbh-domain`（领域类型）← `pbh-config` / `pbh-storage` / `pbh-i18n` / `pbh-rules` / `pbh-downloader` /
`pbh-geoip` / `pbh-engine` / `pbh-btn` / `pbh-web` / `pbh-notify` ← `pbh-server`（组合根 + 二进制 `pbh`）。

- **依赖方向单一**：领域类型在底层，业务 crate 横向不互相依赖（必要协作走 `pbh-domain` 的抽象或在 `pbh-server` 装配）。
- **组合根集中**：`pbh-server` 负责显式装配（构造各组件并 `Arc` 共享），取代 Java 的 Spring DI + 全局 service-locator(`Main.getX()`)。**不引入 DI 框架。**

## 依赖抽象、不依赖具体（守则第 9 条，硬约束）

- 模块间通过 trait 协作；消费方注入 `Arc<dyn Trait>`，对具体实现无编译期依赖。
- **可选能力用可选注入**，拿不到则降级照常工作：
  - `NatAddressProvider`（AutoSTUN 未启用 → `IdentityNatProvider` 恒等映射）
  - `GeoIpProvider`（mmdb 缺失 → IPBlackList 的 ASN/region 检查跳过）
  - `ScriptEngine`（Aviator 未实现 → ExpressionRule 禁用并记日志）

## 关键运行时约定

- **Ban 流水线** 用 bounded `mpsc`(容量 64) 重写 Java 的 "organ" 状态机；"DONE" 用 channel 关闭表达，不照搬手写轮询状态机。每阶段 `tokio::time::timeout`。
- **不要每个 ban wave 新建线程池**（Java 此处有泄漏隐患）；用共享 `tokio` 运行时 + `Semaphore`/`buffer_unordered` 限并发。
- **BanList 内存权威**：IPv4/IPv6 前缀 trie + `RwLock`，支持 CIDR 范围与最长前缀匹配；数据库仅周期快照（每小时 + 关闭时），非实时镜像。
- **调度循环**：固定延迟 `check-interval`，`try_lock` 防重叠，WatchDog 卡死重启。
- **事件**：用 `tokio::sync::broadcast`；**可取消事件**（如模块注册否决）不走 broadcast，保留为直接函数返回 `Result`/否决。
- **热重载**：`tokio::sync::watch` 广播配置变更，组件订阅。
- **SQLite 单写者**：写池 `max_connections(1)`，WAL 下读可并发；清理走单线程后台 + 分块短事务（LIMIT 200）避免长写锁。

## 等价性边界（必须字节级一致的产出）

下载器封禁串（`banned_IPs`/`peers`/`shadow_banned_IPs`）、规则引擎判定、BTN 上行报文、关键 API JSON、PCB 决策。这些处建 golden fixture 对拍（见 03 规范）。
