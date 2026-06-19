//! pbh-storage —— 嵌入式 SQLite 持久化层（v2 精简）。对应上游 `databasent/**`。
//!
//! **零额外部署依赖**：单文件 `<data>/persist/peerbanhelper-nt.db`，WAL，单写者。
//! 表结构见 `memory/design/db-schema.md` 与 `migrations/0001_initial.sql`。
//!
//! M0：连接 + pragma + 迁移 + KV(metadata)。后续里程碑在此之上加各表服务。

pub mod repo;

pub use repo::{BanHistoryRow, NewBanHistory};

use std::path::Path;
use std::str::FromStr;
use std::time::Duration;

use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use sqlx::SqlitePool;

/// 存储层错误。
#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("数据库错误: {0}")]
    Sqlx(#[from] sqlx::Error),
    #[error("迁移错误: {0}")]
    Migrate(#[from] sqlx::migrate::MigrateError),
}

type Result<T> = std::result::Result<T, StorageError>;

/// 数据库句柄。内部为**单写者**连接池（WAL 下足够;后续如需可加独立只读池）。
#[derive(Debug, Clone)]
pub struct Db {
    pool: SqlitePool,
}

impl Db {
    /// 打开（必要时创建）数据库文件，设置 pragma 并运行迁移。
    pub async fn open(db_path: &Path) -> Result<Self> {
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        let opts = SqliteConnectOptions::new()
            .filename(db_path)
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Normal)
            .busy_timeout(Duration::from_secs(60))
            .pragma("mmap_size", "134217728")
            .pragma("journal_size_limit", "67108864");

        let pool = SqlitePoolOptions::new()
            .max_connections(1) // 单写者，避免 SQLITE_BUSY
            .connect_with(opts)
            .await?;

        sqlx::migrate!("./migrations").run(&pool).await?;
        tracing::info!(db = %db_path.display(), "SQLite 已就绪（WAL，单写者）");
        Ok(Db { pool })
    }

    /// 仅供测试：内存库。
    pub async fn open_in_memory() -> Result<Self> {
        let opts = SqliteConnectOptions::from_str("sqlite::memory:").unwrap();
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await?;
        sqlx::migrate!("./migrations").run(&pool).await?;
        Ok(Db { pool })
    }

    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    /// 读 KV。
    pub async fn meta_get(&self, key: &str) -> Result<Option<String>> {
        let row: Option<(String,)> = sqlx::query_as("SELECT v FROM metadata WHERE k = ?")
            .bind(key)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(|r| r.0))
    }

    /// 写 KV（upsert）。
    pub async fn meta_set(&self, key: &str, value: &str) -> Result<()> {
        sqlx::query(
            "INSERT INTO metadata(k, v) VALUES(?, ?) ON CONFLICT(k) DO UPDATE SET v = excluded.v",
        )
        .bind(key)
        .bind(value)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// 优雅关闭。
    pub async fn close(&self) {
        self.pool.close().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn migrate_and_kv_roundtrip() {
        let db = Db::open_in_memory().await.unwrap();
        assert!(db.meta_get("installation-id").await.unwrap().is_none());
        db.meta_set("installation-id", "abc-123").await.unwrap();
        assert_eq!(
            db.meta_get("installation-id").await.unwrap().as_deref(),
            Some("abc-123")
        );
        db.meta_set("installation-id", "xyz").await.unwrap();
        assert_eq!(
            db.meta_get("installation-id").await.unwrap().as_deref(),
            Some("xyz")
        );
    }

    #[tokio::test]
    async fn core_tables_exist() {
        let db = Db::open_in_memory().await.unwrap();
        for t in [
            "torrents",
            "history",
            "banlist",
            "pcb_address",
            "pcb_range",
            "peer_records",
            "rule_sub_info",
            "rule_sub_log",
            "metadata",
            "tracked_swarm",
        ] {
            let n: (i64,) = sqlx::query_as(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name = ?",
            )
            .bind(t)
            .fetch_one(db.pool())
            .await
            .unwrap();
            assert_eq!(n.0, 1, "表 {t} 应存在");
        }
    }

    #[tokio::test]
    async fn open_file_creates_db() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("persist").join("test.db");
        let db = Db::open(&path).await.unwrap();
        db.meta_set("k", "v").await.unwrap();
        assert!(path.exists());
        db.close().await;
    }
}
