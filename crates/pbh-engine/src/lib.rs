//! pbh-engine —— Ban 流水线、调度循环、BanManager。
//!
//! 对应 Java `banpipeline/**`、`DownloaderServerImpl.java`、`BanList.java`、`event/banwave/**`。
//!
//! M3 实现：
//! - 用 bounded `mpsc`(64) 重写 organ 流水线（provider→login→torrents→peers→snapshot→monitor→check）
//! - 每 peer 并发检查（`buffer_unordered` + 非线程安全模块串行化），每阶段 `tokio::time::timeout`
//! - `BanList`（IPv4/IPv6 前缀 trie + RwLock，最长前缀匹配）
//! - `BanManager`：banPeer（解析时长：模块级>全局）/ unban / removeExpiredBans / 白名单解封 / 手动队列
//! - Ban Wave 循环：固定延迟 + try_lock 防重叠 + WatchDog + 每小时快照 + globalPaused
//!
//! AutoSTUN 本期不做，但保留 `NatAddressProvider` 抽象（可选注入，拿不到则恒等映射）。

/// NAT 地址映射抽象。AutoSTUN 提供具体实现；本期注入恒等实现。
/// （守则第 9 条：可选能力用可选注入，拿不到则降级照常工作。）
pub trait NatAddressProvider: Send + Sync {
    /// 把 peer 观察到的地址映射为对外可路由地址。恒等实现直接返回输入。
    fn translate(&self, raw: &str) -> String;
}

/// 恒等 NAT 映射（AutoSTUN 未启用时的默认）。
#[derive(Debug, Default, Clone, Copy)]
pub struct IdentityNatProvider;

impl NatAddressProvider for IdentityNatProvider {
    fn translate(&self, raw: &str) -> String {
        raw.to_string()
    }
}

/// 内存封禁表占位。M3 用 IPv4/IPv6 前缀 trie + RwLock 实现，支持 CIDR 范围与最长前缀匹配。
#[derive(Debug, Default)]
pub struct BanList {
    // TODO(M3): DualStack prefix trie<BanMetadata>。
    _private: (),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_nat_is_passthrough() {
        assert_eq!(IdentityNatProvider.translate("1.2.3.4:6881"), "1.2.3.4:6881");
    }
}
