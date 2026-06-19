//! PCB 状态持久化（`pcb_address` / `pcb_range`）。供 M5 ProgressCheatBlocker 重启续算。
//!
//! `downloader` 字段在 v2 单下载器简化下统一存空串（cache key 未含 downloader）。

use crate::{Db, Result};

/// PCB 分析字段（address 与 range 共用）。
#[derive(Debug, Clone, Default)]
pub struct PcbAnalysis {
    pub last_report_progress: f64,
    pub last_report_uploaded: i64,
    pub tracking_uploaded_increase_total: i64,
    pub rewind_counter: i64,
    pub progress_difference_counter: i64,
    pub first_time_seen: i64,
    pub last_time_seen: i64,
    pub ban_delay_window_end_at: i64,
    pub fast_pcb_test_execute_at: i64,
    pub last_torrent_completed_size: i64,
}

/// 单 IP 状态行。
#[derive(Debug, Clone)]
pub struct PcbAddressRow {
    pub ip: String,
    pub port: i64,
    pub torrent_id: String,
    pub a: PcbAnalysis,
}

/// 前缀段状态行。
#[derive(Debug, Clone)]
pub struct PcbRangeRow {
    pub ip_range: String,
    pub torrent_id: String,
    pub a: PcbAnalysis,
}

impl Db {
    /// 批量 upsert 单 IP 状态（一个事务）。`downloader` 统一空串。
    pub async fn upsert_pcb_addresses(&self, rows: &[PcbAddressRow]) -> Result<()> {
        if rows.is_empty() {
            return Ok(());
        }
        let mut tx = self.pool().begin().await?;
        for r in rows {
            sqlx::query(
                "INSERT INTO pcb_address(
                    ip, port, torrent_id, last_report_progress, last_report_uploaded,
                    tracking_uploaded_increase_total, rewind_counter, progress_difference_counter,
                    first_time_seen, last_time_seen, downloader, ban_delay_window_end_at,
                    fast_pcb_test_execute_at, last_torrent_completed_size)
                 VALUES(?,?,?,?,?,?,?,?,?,?,'',?,?,?)
                 ON CONFLICT(ip, port, torrent_id, downloader) DO UPDATE SET
                    last_report_progress = excluded.last_report_progress,
                    last_report_uploaded = excluded.last_report_uploaded,
                    tracking_uploaded_increase_total = excluded.tracking_uploaded_increase_total,
                    rewind_counter = excluded.rewind_counter,
                    progress_difference_counter = excluded.progress_difference_counter,
                    last_time_seen = excluded.last_time_seen,
                    ban_delay_window_end_at = excluded.ban_delay_window_end_at,
                    fast_pcb_test_execute_at = excluded.fast_pcb_test_execute_at,
                    last_torrent_completed_size = excluded.last_torrent_completed_size",
            )
            .bind(&r.ip)
            .bind(r.port)
            .bind(&r.torrent_id)
            .bind(r.a.last_report_progress)
            .bind(r.a.last_report_uploaded)
            .bind(r.a.tracking_uploaded_increase_total)
            .bind(r.a.rewind_counter)
            .bind(r.a.progress_difference_counter)
            .bind(r.a.first_time_seen)
            .bind(r.a.last_time_seen)
            .bind(r.a.ban_delay_window_end_at)
            .bind(r.a.fast_pcb_test_execute_at)
            .bind(r.a.last_torrent_completed_size)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// 批量 upsert 前缀段状态。
    pub async fn upsert_pcb_ranges(&self, rows: &[PcbRangeRow]) -> Result<()> {
        if rows.is_empty() {
            return Ok(());
        }
        let mut tx = self.pool().begin().await?;
        for r in rows {
            sqlx::query(
                "INSERT INTO pcb_range(
                    ip_range, torrent_id, last_report_progress, last_report_uploaded,
                    tracking_uploaded_increase_total, rewind_counter, progress_difference_counter,
                    first_time_seen, last_time_seen, downloader, ban_delay_window_end_at,
                    fast_pcb_test_execute_at, last_torrent_completed_size)
                 VALUES(?,?,?,?,?,?,?,?,?,'',?,?,?)
                 ON CONFLICT(ip_range, torrent_id, downloader) DO UPDATE SET
                    last_report_progress = excluded.last_report_progress,
                    last_report_uploaded = excluded.last_report_uploaded,
                    tracking_uploaded_increase_total = excluded.tracking_uploaded_increase_total,
                    rewind_counter = excluded.rewind_counter,
                    progress_difference_counter = excluded.progress_difference_counter,
                    last_time_seen = excluded.last_time_seen,
                    ban_delay_window_end_at = excluded.ban_delay_window_end_at,
                    fast_pcb_test_execute_at = excluded.fast_pcb_test_execute_at,
                    last_torrent_completed_size = excluded.last_torrent_completed_size",
            )
            .bind(&r.ip_range)
            .bind(&r.torrent_id)
            .bind(r.a.last_report_progress)
            .bind(r.a.last_report_uploaded)
            .bind(r.a.tracking_uploaded_increase_total)
            .bind(r.a.rewind_counter)
            .bind(r.a.progress_difference_counter)
            .bind(r.a.first_time_seen)
            .bind(r.a.last_time_seen)
            .bind(r.a.ban_delay_window_end_at)
            .bind(r.a.fast_pcb_test_execute_at)
            .bind(r.a.last_torrent_completed_size)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// 载入 `last_time_seen >= since` 的单 IP 状态（重启续算）。
    pub async fn load_pcb_addresses(&self, since: i64) -> Result<Vec<PcbAddressRow>> {
        let rows = sqlx::query_as::<_, (String, i64, String, f64, i64, i64, i64, i64, i64, i64, i64, i64, i64)>(
            "SELECT ip, port, torrent_id, last_report_progress,
                    COALESCE(last_report_uploaded,0), COALESCE(tracking_uploaded_increase_total,0),
                    rewind_counter, progress_difference_counter, first_time_seen, last_time_seen,
                    COALESCE(ban_delay_window_end_at,0), COALESCE(fast_pcb_test_execute_at,0),
                    COALESCE(last_torrent_completed_size,0)
             FROM pcb_address WHERE last_time_seen >= ?",
        )
        .bind(since)
        .fetch_all(self.pool())
        .await?
        .into_iter()
        .map(|r| PcbAddressRow {
            ip: r.0,
            port: r.1,
            torrent_id: r.2,
            a: PcbAnalysis {
                last_report_progress: r.3,
                last_report_uploaded: r.4,
                tracking_uploaded_increase_total: r.5,
                rewind_counter: r.6,
                progress_difference_counter: r.7,
                first_time_seen: r.8,
                last_time_seen: r.9,
                ban_delay_window_end_at: r.10,
                fast_pcb_test_execute_at: r.11,
                last_torrent_completed_size: r.12,
            },
        })
        .collect();
        Ok(rows)
    }

    /// 载入 `last_time_seen >= since` 的前缀段状态。
    pub async fn load_pcb_ranges(&self, since: i64) -> Result<Vec<PcbRangeRow>> {
        let rows = sqlx::query_as::<_, (String, String, f64, i64, i64, i64, i64, i64, i64, i64, i64, i64)>(
            "SELECT ip_range, torrent_id, last_report_progress,
                    COALESCE(last_report_uploaded,0), COALESCE(tracking_uploaded_increase_total,0),
                    rewind_counter, progress_difference_counter, first_time_seen, last_time_seen,
                    COALESCE(ban_delay_window_end_at,0), COALESCE(fast_pcb_test_execute_at,0),
                    COALESCE(last_torrent_completed_size,0)
             FROM pcb_range WHERE last_time_seen >= ?",
        )
        .bind(since)
        .fetch_all(self.pool())
        .await?
        .into_iter()
        .map(|r| PcbRangeRow {
            ip_range: r.0,
            torrent_id: r.1,
            a: PcbAnalysis {
                last_report_progress: r.2,
                last_report_uploaded: r.3,
                tracking_uploaded_increase_total: r.4,
                rewind_counter: r.5,
                progress_difference_counter: r.6,
                first_time_seen: r.7,
                last_time_seen: r.8,
                ban_delay_window_end_at: r.9,
                fast_pcb_test_execute_at: r.10,
                last_torrent_completed_size: r.11,
            },
        })
        .collect();
        Ok(rows)
    }

    /// 删除 `last_time_seen < cutoff` 的 PCB 记录（两表）。返回删除行数。
    pub async fn cleanup_pcb(&self, cutoff: i64) -> Result<u64> {
        let a = sqlx::query("DELETE FROM pcb_address WHERE last_time_seen < ?")
            .bind(cutoff)
            .execute(self.pool())
            .await?
            .rows_affected();
        let r = sqlx::query("DELETE FROM pcb_range WHERE last_time_seen < ?")
            .bind(cutoff)
            .execute(self.pool())
            .await?
            .rows_affected();
        Ok(a + r)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn pcb_address_upsert_load_cleanup() {
        let db = Db::open_in_memory().await.unwrap();
        let row = PcbAddressRow {
            ip: "1.2.3.4".into(),
            port: 6881,
            torrent_id: "t1".into(),
            a: PcbAnalysis {
                last_report_progress: 0.5,
                tracking_uploaded_increase_total: 1234,
                first_time_seen: 100,
                last_time_seen: 200,
                ..Default::default()
            },
        };
        db.upsert_pcb_addresses(std::slice::from_ref(&row))
            .await
            .unwrap();
        // 再 upsert（累计更新）。
        let mut row2 = row.clone();
        row2.a.tracking_uploaded_increase_total = 9999;
        row2.a.last_time_seen = 300;
        db.upsert_pcb_addresses(&[row2]).await.unwrap();

        let loaded = db.load_pcb_addresses(0).await.unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].a.tracking_uploaded_increase_total, 9999);
        assert_eq!(loaded[0].a.first_time_seen, 100); // 保留首见

        // cleanup：cutoff=400 → last_time_seen 300 < 400 删除。
        let deleted = db.cleanup_pcb(400).await.unwrap();
        assert_eq!(deleted, 1);
        assert_eq!(db.load_pcb_addresses(0).await.unwrap().len(), 0);
    }

    #[tokio::test]
    async fn pcb_range_roundtrip() {
        let db = Db::open_in_memory().await.unwrap();
        db.upsert_pcb_ranges(&[PcbRangeRow {
            ip_range: "1.2.3.0/24".into(),
            torrent_id: "t1".into(),
            a: PcbAnalysis {
                rewind_counter: 3,
                last_time_seen: 50,
                ..Default::default()
            },
        }])
        .await
        .unwrap();
        let loaded = db.load_pcb_ranges(0).await.unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].a.rewind_counter, 3);
        assert_eq!(loaded[0].ip_range, "1.2.3.0/24");
    }
}
