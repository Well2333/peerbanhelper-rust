//! pbh-notify —— 告警与推送。对应 Java `alert/**`、`util/push/**`、`metric/**`。
//!
//! M9 实现：
//! - Alert：DB 持久化 + 按 identifier 去重 + 30 天清理 + 推送 + console 通知
//! - Push 8 通道：pushplus / serverchan / smtp / telegram / bark / pushdeer / gotify / ntfy
//! - metric：内部 atomics 计数器 + PersistMetrics 写 history/torrent + GeoIP 富化
//!
//! 设计（守则第 9 条）：每个推送通道实现 `PushProvider`，PushManager 注入抽象集合并扇出。

/// 推送通道类型。对应 Java `util/push/impl/*` 的 `type`。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PushChannel {
    PushPlus,
    ServerChan,
    Smtp,
    Telegram,
    Bark,
    PushDeer,
    Gotify,
    Ntfy,
}

/// 单个推送通道抽象。对应 Java `PushProvider`。
pub trait PushProvider: Send + Sync {
    fn name(&self) -> &str;
    fn channel(&self) -> PushChannel;
    /// 发送一条消息（title + markdown 正文）。M9 改为 async。
    fn push_stub(&self, title: &str, content: &str) -> pbh_domain::Result<()>;
}
