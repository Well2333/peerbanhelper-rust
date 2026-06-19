# 编码规范

## Rust 风格

- edition 2021（当前环境 rustc 1.75）；升级 edition 2024 须同步 `rust-toolchain.toml` 到 ≥1.85。
- 过 `cargo fmt`；尽量过 `cargo clippy`（当前环境未装 clippy，CI/开发机须装）。
- 错误处理：库 crate 用 `thiserror` 定义错误枚举（骨架阶段 `pbh-domain` 先手写 `PbhError`，M1 换 `thiserror`）；应用层可用 `anyhow`。不 `unwrap()`/`expect()` 于可恢复路径。
- 命名：保持与上游领域词汇可对照（`BanMetadata`/`CheckResult`/`PeerAction`/`ProgressCheatBlocker` 等），便于查源码基准。
- 注释里标注对应 Java 文件/方法，降低后续对拍成本。

## 序列化对齐（硬约束）

面向前端 / BTN / 下载器的 JSON 字段名与类型**必须与上游 Gson 输出一致**：
- 用 `#[serde(rename = "...")]` 对齐 `@SerializedName`；时间戳多为 epoch millis（`i64` 数字）。
- 前端契约对象：`TranslationComponent { key, params[] }`、`IpGeoData`、`StdResp { success, message, data }`、分页 `{ page, size, total, results }`。

## 等价性对拍（关键路径强制）

对「等价性边界」（见架构规范）建 golden fixture，CI 对拍：
1. qB 封禁写入串（含 IPv6 规范化、CIDR、shadowban）。
2. 规则引擎对 `profile.yml` 默认规则的判定。
3. BTN 上行报文（解 gzip 后字段/类型）。
4. 关键 API 响应 JSON（manifest / bans / statistic / general.status）。
5. PCB 序列回放 → 相同决策与 DB 状态。

## TDD / 测试政策（守则第 8 条）

- 关键等价性逻辑（PCB、规则引擎、IP 规范化、BTN 序列化、`{}` 填充）**先写测试**。
- 单元测试只覆盖能在纯净宿主实例化的**纯逻辑**；运行时/外部环境强耦合部分靠编译期 + 真实环境手测，并记入 `memory/test-status/`。

## 骨架阶段 std-only 政策

骨架阶段各 crate 尽量 **std-only**（仅 path 依赖），保证离线 `cargo test` 可跑；外部依赖按里程碑在需要时引入。`workspace.dependencies` 已声明的依赖**未被引用时不会被拉取**。
