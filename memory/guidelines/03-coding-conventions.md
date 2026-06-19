# 编码规范

## Rust 工具链与风格

- **toolchain：rustup `stable`（实测 1.96）**。`rust-toolchain.toml` 固定 channel=stable + clippy/rustfmt 组件。
  现代依赖内部用 edition2024,故需 rustc ≥ 1.85;本仓 edition 仍为 2021（`rust-version = "1.85"`）。
- 构建一律用 `~/.cargo/bin/cargo`（rustup 版,尊重 `rust-toolchain.toml`）;勿用系统 1.75。
- **提交前必须过 `cargo fmt --check` 与 `cargo clippy --workspace --all-targets`（零告警）**。
- 错误处理：库 crate 用 `thiserror` 定义错误枚举（`pbh-config`/`pbh-storage` 已用）；应用层可用 `anyhow`。不在可恢复路径 `unwrap()`/`expect()`。
- 命名：保持与上游领域词汇可对照（`BanMetadata`/`CheckResult`/`PeerAction`/`ProgressCheatBlocker` 等），便于查源码基准。
- 注释里标注对应 Java 文件/方法，降低后续对拍成本。

## 序列化对齐（硬约束）

面向 **BTN / 下载器** 的 JSON 字段名与类型**必须与上游 Gson 输出一致**：
- 用 `#[serde(rename = "...")]` 对齐 `@SerializedName`；时间戳多为 epoch millis（`i64` 数字）。
- v2：面向**自有 API** 的 JSON 由本项目自定义（信封 `ApiResp { ok, data, error }`、分页 `{ page, size, total, items }`），**不**与上游 Gson 兼容，无需对齐。
- 已弃用 `TranslationComponent`（无 i18n）。`IpGeoData` 仍为内部结构（GeoIP 输出）。

## 等价性对拍（关键路径强制）

对「等价性边界」（见架构规范）建 golden fixture，CI 对拍：
1. qB 封禁写入串（含 IPv6 规范化、CIDR、shadowban）。
2. 规则引擎对 `profile.yml` 默认规则的判定。
3. BTN 上下行报文（解 gzip 后字段/类型）。
4. PCB 序列回放 → 相同决策与 DB 状态。

（自有 API 不在对拍范围，走常规接口测试。）

## TDD / 测试政策（守则第 8 条）

- 关键等价性逻辑（PCB、规则引擎、IP 规范化、BTN 序列化、`{}` 填充）**先写测试**。
- 单元测试只覆盖能在纯净宿主实例化的**纯逻辑**；运行时/外部环境强耦合部分靠编译期 + 真实环境手测，并记入 `memory/test-status/`。

## 依赖引入政策

- 各 crate **在自身里程碑落地时**才引入外部依赖;尚未开工的骨架 crate（rules/downloader/geoip/engine/btn/web 等）暂保持 std-only，以便快速编译。
- `workspace.dependencies` 已声明的依赖**未被引用时不会被拉取**;用 `.workspace = true` 引入，crate 级可加 `features`。
- 引入新顶层依赖前先看是否已在 workspace 声明;新增重依赖在变更记录里说明理由。
