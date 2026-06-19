//! pbh-btn —— BTN 云端威胁情报网络（用户要求全部保留）。
//!
//! 对应 Java `btn/**`、`module/impl/rule/BtnNetworkOnline.java`、`util/pow/**`。
//! 协议细节见 `docs/01-architecture-analysis.md` §2.4。
//!
//! M8 实现：
//! - HTTP 中间件（固定头 + Bearer + gzip 上行）、config 端点拉取、new/legacy 分支
//! - 下行 ability：HeartBeat / Rules(`?rev=`) / IPDenyList / IPAllowList(+解封白名单) / IpQuery / Reconfigure
//! - 上行 ability：SubmitBans / SubmitSwarm / SubmitHistory（DB 游标 + KV 续传）
//! - PoW（移植 `PoWClient`）、`BtnRulesetParsed` + `BtnNetworkOnline` 规则应用
//! - 每 ability tokio 任务（初始随机延迟 + 固定间隔）、600s config 重试

/// BTN 协议实现版本（对应 Java `PBH_BTN_PROTOCOL_IMPL_VERSION`）。
pub const PROTOCOL_IMPL_VERSION: u32 = 20;
/// 可读协议版本（对应 `PBH_BTN_PROTOCOL_READABLE_VERSION`）。
pub const PROTOCOL_READABLE_VERSION: &str = "2.0.1";

/// 服务端 config 声明启用的 ability。`min_protocol_version < 20` 走 legacy 分支。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ability {
    HeartBeat,
    Rules,
    IpDenyList,
    IpAllowList,
    IpQuery,
    Reconfigure,
    SubmitBans,
    SubmitSwarm,
    SubmitHistory,
    // legacy:
    SubmitPeers,
}
