//! 表访问助手（M3 起逐步补全）。当前：torrents upsert + 封禁历史写入/查询。

use crate::{Db, Result};

/// 一条新封禁历史（写入 `history`）。可空字段用 `Option`。
#[derive(Debug, Clone)]
pub struct NewBanHistory {
    pub ban_at: i64,
    pub unban_at: i64,
    pub ip: String,
    pub port: i64,
    pub peer_id: Option<String>,
    pub client_name: Option<String>,
    /// 我方累计上传给该 peer（Peer.uploaded；= BTN to_peer_traffic）。
    pub peer_uploaded: i64,
    /// 我方累计从该 peer 下载（Peer.downloaded；= BTN from_peer_traffic）。
    pub peer_downloaded: i64,
    pub peer_progress: f64,
    pub downloader_progress: f64,
    pub torrent_id: i64,
    pub module_name: String,
    pub rule_name: String,
    pub description: String,
    pub flags: Option<String>,
    pub downloader: String,
}

/// 封禁历史的展示行（供 `/api/bans/history`）。
#[derive(Debug, Clone, serde::Serialize)]
pub struct BanHistoryRow {
    pub id: i64,
    pub ban_at: i64,
    pub unban_at: i64,
    pub ip: String,
    pub port: i64,
    pub peer_id: Option<String>,
    pub client_name: Option<String>,
    pub module_name: String,
    pub rule_name: String,
    pub description: String,
    pub downloader: String,
}

impl Db {
    /// upsert 种子，返回其自增 id。`ON CONFLICT(info_hash)`：保留更大 size、非空 private。
    pub async fn upsert_torrent(
        &self,
        info_hash: &str,
        name: &str,
        size: i64,
        private: Option<bool>,
    ) -> Result<i64> {
        sqlx::query(
            "INSERT INTO torrents(info_hash, name, size, private_torrent) VALUES(?, ?, ?, ?)
             ON CONFLICT(info_hash) DO UPDATE SET
                name = excluded.name,
                size = MAX(torrents.size, excluded.size),
                private_torrent = COALESCE(excluded.private_torrent, torrents.private_torrent)",
        )
        .bind(info_hash)
        .bind(name)
        .bind(size)
        .bind(private.map(|b| b as i64))
        .execute(self.pool())
        .await?;
        let row: (i64,) = sqlx::query_as("SELECT id FROM torrents WHERE info_hash = ?")
            .bind(info_hash)
            .fetch_one(self.pool())
            .await?;
        Ok(row.0)
    }

    /// 写入一条封禁历史。
    pub async fn insert_ban_history(&self, h: &NewBanHistory) -> Result<()> {
        sqlx::query(
            "INSERT INTO history(
                ban_at, unban_at, ip, port, peer_id, peer_client_name,
                peer_uploaded, peer_downloaded, peer_progress, downloader_progress,
                torrent_id, module_name, rule_name, description, flags, downloader)
             VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
        )
        .bind(h.ban_at)
        .bind(h.unban_at)
        .bind(&h.ip)
        .bind(h.port)
        .bind(&h.peer_id)
        .bind(&h.client_name)
        .bind(h.peer_uploaded)
        .bind(h.peer_downloaded)
        .bind(h.peer_progress)
        .bind(h.downloader_progress)
        .bind(h.torrent_id)
        .bind(&h.module_name)
        .bind(&h.rule_name)
        .bind(&h.description)
        .bind(&h.flags)
        .bind(&h.downloader)
        .execute(self.pool())
        .await?;
        Ok(())
    }

    /// 分页查询封禁历史（按 ban_at 倒序）。
    pub async fn query_ban_history(&self, limit: i64, offset: i64) -> Result<Vec<BanHistoryRow>> {
        let rows: Vec<BanHistoryRow> = sqlx::query_as::<_, (i64, i64, i64, String, i64, Option<String>, Option<String>, String, String, String, String)>(
            "SELECT id, ban_at, unban_at, ip, port, peer_id, peer_client_name, module_name, rule_name, description, downloader
             FROM history ORDER BY ban_at DESC LIMIT ? OFFSET ?",
        )
        .bind(limit)
        .bind(offset)
        .fetch_all(self.pool())
        .await?
        .into_iter()
        .map(|r| BanHistoryRow {
            id: r.0,
            ban_at: r.1,
            unban_at: r.2,
            ip: r.3,
            port: r.4,
            peer_id: r.5,
            client_name: r.6,
            module_name: r.7,
            rule_name: r.8,
            description: r.9,
            downloader: r.10,
        })
        .collect();
        Ok(rows)
    }

    /// 历史总数。
    pub async fn count_ban_history(&self) -> Result<i64> {
        let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM history")
            .fetch_one(self.pool())
            .await?;
        Ok(row.0)
    }

    /// 供 BTN SubmitBans：按 id 游标取封禁历史（join torrents 取 info_hash/size/private）。
    pub async fn query_btn_bans(&self, after_id: i64, limit: i64) -> Result<Vec<BtnBanRow>> {
        let rows = sqlx::query_as::<_, BtnBanRow>(
            "SELECT h.id, h.ban_at, h.ip, h.port, h.peer_id,
                    h.peer_client_name AS client_name,
                    COALESCE(h.peer_uploaded,0) AS peer_uploaded,
                    COALESCE(h.peer_downloaded,0) AS peer_downloaded,
                    h.peer_progress, h.downloader_progress,
                    COALESCE(t.info_hash,'') AS info_hash,
                    COALESCE(t.size,0) AS torrent_size,
                    COALESCE(t.private_torrent,0) AS torrent_is_private,
                    h.module_name, h.rule_name, h.description, h.flags
             FROM history h LEFT JOIN torrents t ON h.torrent_id = t.id
             WHERE h.id > ? ORDER BY h.id ASC LIMIT ?",
        )
        .bind(after_id)
        .bind(limit)
        .fetch_all(self.pool())
        .await?;
        Ok(rows)
    }
}

/// 供 BTN 上行的封禁行（join torrents）。
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct BtnBanRow {
    pub id: i64,
    pub ban_at: i64,
    pub ip: String,
    pub port: i64,
    pub peer_id: Option<String>,
    pub client_name: Option<String>,
    pub peer_uploaded: i64,
    pub peer_downloaded: i64,
    pub peer_progress: f64,
    pub downloader_progress: f64,
    pub info_hash: String,
    pub torrent_size: i64,
    pub torrent_is_private: bool,
    pub module_name: String,
    pub rule_name: String,
    pub description: String,
    pub flags: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn torrent_upsert_and_history() {
        let db = Db::open_in_memory().await.unwrap();
        let id1 = db
            .upsert_torrent("hashA", "name", 100, Some(false))
            .await
            .unwrap();
        let id2 = db.upsert_torrent("hashA", "name2", 50, None).await.unwrap();
        assert_eq!(id1, id2); // 同 info_hash → 同 id
        db.insert_ban_history(&NewBanHistory {
            ban_at: 1,
            unban_at: 2,
            ip: "1.2.3.4".into(),
            port: 6881,
            peer_id: Some("-XL-".into()),
            client_name: None,
            peer_uploaded: 5000,
            peer_downloaded: 100,
            peer_progress: 0.5,
            downloader_progress: 1.0,
            torrent_id: id1,
            module_name: "PeerIdBlacklist".into(),
            rule_name: "peer-id-blacklist".into(),
            description: "test".into(),
            flags: Some("D".into()),
            downloader: "d1".into(),
        })
        .await
        .unwrap();
        assert_eq!(db.count_ban_history().await.unwrap(), 1);
        let rows = db.query_ban_history(10, 0).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].ip, "1.2.3.4");
        // BTN 行查询含新字段。
        let btn = db.query_btn_bans(0, 10).await.unwrap();
        assert_eq!(btn.len(), 1);
        assert_eq!(btn[0].peer_uploaded, 5000);
        assert_eq!(btn[0].peer_downloaded, 100);
        assert_eq!(btn[0].flags.as_deref(), Some("D"));
    }
}
