//! 当前 swarm 跟踪（`tracked_swarm`）。供 BTN 上行 submit_swarm。
//!
//! 每轮 ban wave 把看到的 peer 批量 upsert;带 offset 的单调累加处理 peer 重连归零
//! （新报告值 < 旧值 → 把旧值累进 offset，真实累计 = offset + 当前值）。
//! 表为「临时表」，进程启动时清空。

use crate::{Db, Result};

/// 一条 swarm 观测（写入 `tracked_swarm`）。
#[derive(Debug, Clone)]
pub struct SwarmRow {
    pub ip: String,
    pub port: i64,
    pub info_hash: String,
    pub torrent_is_private: bool,
    pub torrent_size: i64,
    pub downloader: String,
    pub downloader_progress: f64,
    pub peer_id: Option<String>,
    pub client_name: Option<String>,
    pub peer_progress: f64,
    pub uploaded: i64,
    pub upload_speed: i64,
    pub downloaded: i64,
    pub download_speed: i64,
    pub last_flags: Option<String>,
    pub now: i64,
}

/// 供 BTN submit_swarm 的行（含 id + offset + 时间）。
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct BtnSwarmRow {
    pub id: i64,
    pub ip: String,
    pub port: i64,
    pub info_hash: String,
    pub torrent_is_private: bool,
    pub torrent_size: i64,
    pub downloader: String,
    pub downloader_progress: f64,
    pub peer_id: Option<String>,
    pub client_name: Option<String>,
    pub peer_progress: f64,
    pub uploaded: i64,
    pub uploaded_offset: i64,
    pub upload_speed: i64,
    pub downloaded: i64,
    pub downloaded_offset: i64,
    pub download_speed: i64,
    pub last_flags: Option<String>,
    pub first_time_seen: i64,
    pub last_time_seen: i64,
    pub download_speed_max: i64,
    pub upload_speed_max: i64,
}

impl Db {
    /// 启动清空临时表。
    pub async fn clear_tracked_swarm(&self) -> Result<()> {
        sqlx::query("DELETE FROM tracked_swarm")
            .execute(self.pool())
            .await?;
        Ok(())
    }

    /// 批量 upsert 当前 swarm（一个事务）。
    pub async fn upsert_tracked_swarm(&self, rows: &[SwarmRow]) -> Result<()> {
        if rows.is_empty() {
            return Ok(());
        }
        let mut tx = self.pool().begin().await?;
        for r in rows {
            sqlx::query(
                "INSERT INTO tracked_swarm(
                    ip, port, info_hash, torrent_is_private, torrent_size, downloader,
                    downloader_progress, peer_id, client_name, peer_progress,
                    uploaded, uploaded_offset, upload_speed, downloaded, downloaded_offset,
                    download_speed, last_flags, first_time_seen, last_time_seen,
                    download_speed_max, upload_speed_max)
                 VALUES(?,?,?,?,?,?,?,?,?,?,?,0,?,?,0,?,?,?,?,?,?)
                 ON CONFLICT(ip, port, info_hash, downloader) DO UPDATE SET
                    torrent_is_private = excluded.torrent_is_private,
                    torrent_size = excluded.torrent_size,
                    downloader_progress = excluded.downloader_progress,
                    peer_id = excluded.peer_id,
                    client_name = excluded.client_name,
                    peer_progress = excluded.peer_progress,
                    uploaded_offset = tracked_swarm.uploaded_offset
                        + CASE WHEN excluded.uploaded < tracked_swarm.uploaded THEN tracked_swarm.uploaded ELSE 0 END,
                    uploaded = excluded.uploaded,
                    upload_speed = excluded.upload_speed,
                    downloaded_offset = tracked_swarm.downloaded_offset
                        + CASE WHEN excluded.downloaded < tracked_swarm.downloaded THEN tracked_swarm.downloaded ELSE 0 END,
                    downloaded = excluded.downloaded,
                    download_speed = excluded.download_speed,
                    last_flags = excluded.last_flags,
                    last_time_seen = excluded.last_time_seen,
                    download_speed_max = MAX(tracked_swarm.download_speed_max, excluded.download_speed),
                    upload_speed_max = MAX(tracked_swarm.upload_speed_max, excluded.upload_speed)",
            )
            .bind(&r.ip)
            .bind(r.port)
            .bind(&r.info_hash)
            .bind(r.torrent_is_private as i64)
            .bind(r.torrent_size)
            .bind(&r.downloader)
            .bind(r.downloader_progress)
            .bind(&r.peer_id)
            .bind(&r.client_name)
            .bind(r.peer_progress)
            .bind(r.uploaded)
            .bind(r.upload_speed)
            .bind(r.downloaded)
            .bind(r.download_speed)
            .bind(&r.last_flags)
            .bind(r.now) // first_time_seen
            .bind(r.now) // last_time_seen
            .bind(r.download_speed) // download_speed_max 初值
            .bind(r.upload_speed) // upload_speed_max 初值
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// 供 submit_swarm：游标 `(last_time_seen, id)` 分页。
    pub async fn query_btn_swarm(
        &self,
        cursor_time: i64,
        cursor_id: i64,
        limit: i64,
    ) -> Result<Vec<BtnSwarmRow>> {
        let rows = sqlx::query_as::<_, BtnSwarmRow>(
            "SELECT id, ip, port, info_hash, torrent_is_private, torrent_size, downloader,
                    downloader_progress, peer_id, client_name, peer_progress,
                    uploaded, uploaded_offset, upload_speed, downloaded, downloaded_offset,
                    download_speed, last_flags, first_time_seen, last_time_seen,
                    download_speed_max, upload_speed_max
             FROM tracked_swarm
             WHERE last_time_seen > ? OR (last_time_seen = ? AND id > ?)
             ORDER BY last_time_seen ASC, id ASC LIMIT ?",
        )
        .bind(cursor_time)
        .bind(cursor_time)
        .bind(cursor_id)
        .bind(limit)
        .fetch_all(self.pool())
        .await?;
        Ok(rows)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(ip: &str, up: i64, now: i64) -> SwarmRow {
        SwarmRow {
            ip: ip.into(),
            port: 6881,
            info_hash: "hash".into(),
            torrent_is_private: false,
            torrent_size: 1000,
            downloader: "d1".into(),
            downloader_progress: 1.0,
            peer_id: Some("-qB-".into()),
            client_name: Some("qB".into()),
            peer_progress: 0.5,
            uploaded: up,
            upload_speed: 10,
            downloaded: 0,
            download_speed: 0,
            last_flags: Some("u".into()),
            now,
        }
    }

    #[tokio::test]
    async fn swarm_upsert_offset_on_reset() {
        let db = Db::open_in_memory().await.unwrap();
        db.upsert_tracked_swarm(&[row("1.2.3.4", 1000, 100)])
            .await
            .unwrap();
        // 重连归零：上报 200 < 1000 → offset 累进 1000。
        db.upsert_tracked_swarm(&[row("1.2.3.4", 200, 200)])
            .await
            .unwrap();
        let rows = db.query_btn_swarm(0, 0, 10).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].uploaded, 200);
        assert_eq!(rows[0].uploaded_offset, 1000); // 真实累计 = 1200
        assert_eq!(rows[0].first_time_seen, 100); // 保留首见
        assert_eq!(rows[0].last_time_seen, 200);
    }

    #[tokio::test]
    async fn swarm_clear_and_cursor() {
        let db = Db::open_in_memory().await.unwrap();
        db.upsert_tracked_swarm(&[row("1.1.1.1", 10, 50), row("2.2.2.2", 20, 60)])
            .await
            .unwrap();
        assert_eq!(db.query_btn_swarm(0, 0, 10).await.unwrap().len(), 2);
        // 游标过 50 → 只剩 60。
        assert_eq!(db.query_btn_swarm(50, 999, 10).await.unwrap().len(), 1);
        db.clear_tracked_swarm().await.unwrap();
        assert_eq!(db.query_btn_swarm(0, 0, 10).await.unwrap().len(), 0);
    }
}
