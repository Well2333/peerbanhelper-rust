//! pbh-web —— HTTP/WS 层（前端契约）。对应 Java `web/**` + `module/impl/webapi/*Controller.java`。
//!
//! 前端零改动复用，故信封/鉴权/状态码必须精确（见 `docs/03-api-contract.md`）。
//! 骨架阶段仅定义响应信封与角色枚举（std-only）。M7 用 axum 实现：
//! - StdResp 信封、分页、异常→状态码映射（401/403/303→/init/400/429/405/500）
//! - 鉴权中间件（Bearer / ?token= / 会话三通道）+ fail2ban + UA 扫描器拦截
//! - 静态 + SPA fallback（/api、/blocklist 先路由），WS `/api/logs/stream`
//! - 全部控制器（**删除 PBH Plus 的 13 个端点**）
//!
//! 角色枚举去掉了 Java 的 `PBH_PLUS`（付费功能整体删除）。

pub mod envelope;

pub use envelope::{PbhPage, StdResp};

/// 路由角色。对应 Java `web/Role.java`，去除 `PBH_PLUS`。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Anyone,
    UserRead,
    UserWrite,
}
