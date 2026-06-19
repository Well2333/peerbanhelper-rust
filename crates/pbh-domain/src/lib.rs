//! pbh-domain —— PeerBanHelper-Rust 的领域核心类型。
//!
//! 对应 Java：`module/{CheckResult,PeerAction}.java`、`bittorrent/{peer,torrent}/**`、
//! `wrapper/{BanMetadata,PeerAddress}.java`。
//!
//! 骨架阶段为 std-only，承载「契约 + 可隔离纯逻辑」（PeerAction 优先级合并、PeerFlag 解析等），
//! 以便在纯净宿主里离线编译与单测（见 `docs/最高优先级工作守则.md` 第 7/8 条）。
//! M1 起补充 serde/chrono 派生与持久化映射。

pub mod error;
pub mod peer;
pub mod torrent;
pub mod ban;

pub use ban::{BanMetadata, PeerAction, CheckResult};
pub use error::{PbhError, Result};
pub use peer::{Peer, PeerAddress, PeerFlag};
pub use torrent::Torrent;
