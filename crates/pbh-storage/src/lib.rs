//! pbh-storage —— 嵌入式 SQLite 持久化层。对应 Java `databasent/**`。
//!
//! **零额外部署依赖**：单文件 `<dataDir>/persist/peerbanhelper-nt.db`，WAL，单写者。
//! 表结构与关键 SQL 见 `memory/design/db-schema.md`。
//!
//! M0：连接 + pragma + `sqlx::migrate!`(合并版 V1) + KV(metadata)。
//! M5：pcb_address / pcb_range（脏刷缓存）。M6：rule_sub_log。M8：history/tracked_swarm 游标。
//! M9：peer_records / 各 metrics / traffic_journal / alert / torrents。
//!
//! 设计要点（守则第 9 条）：各表通过 `*Service` 抽象暴露，消费方注入抽象。
//! 清理走单线程后台 + 分块短事务（LIMIT 200），避免长写锁 / SQLITE_BUSY。
//!
//! 迁移 SQL 放在 `migrations/`（M0 落地）。骨架阶段仅占位常量。

/// 数据库文件相对数据目录的路径。
pub const DB_RELATIVE_PATH: &str = "persist/peerbanhelper-nt.db";

/// 连接时应用的 pragma（见 04-db-schema.md）。
pub const PRAGMAS: &[(&str, &str)] = &[
    ("journal_mode", "WAL"),
    ("synchronous", "NORMAL"),
    ("busy_timeout", "60000"),
    ("mmap_size", "134217728"),
    ("journal_size_limit", "67108864"),
];

/// 键值元数据存储抽象（对应 Java `MetadataService`）。M0 用 sqlx 实现。
pub trait MetadataStore: Send + Sync {
    fn get(&self, key: &str) -> Option<String>;
    fn set(&self, key: &str, value: &str);
}
