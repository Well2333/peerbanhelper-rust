//! pbh-web —— 自研极简 HTTP/WS 层（v2:弃用原 Vue 前端,自由设计的小型 API）。
//!
//! 见 `docs/02-roadmap.md` §4。**不复刻** Java 的 StdResp/Gson/SPA/OOBE/fail2ban/~90 端点。
//!
//! 骨架阶段仅定义信封与角色枚举（std-only）。M7 用 axum 实现:
//! - 自有信封 `{ ok, data, error }`、分页
//! - Bearer token 鉴权（配置文件设定;首启自动生成并打印一次）
//! - 约 18 个端点:status / downloaders / bans / bans.history / config.profile / sub.rules /
//!   btn.status / logs(+WS `/api/logs/stream`) / blocklist 导出
//! - 内置 vanilla 单页(`rust-embed` 内嵌),覆盖状态/下载器/封禁/日志/规则

pub mod envelope;

pub use envelope::{ApiResp, Page};

/// 路由角色(精简:只读 / 读写;无会话、无付费角色)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// 公开(如 /blocklist 导出、登录)。
    Anyone,
    /// 需 token。
    Authed,
}
