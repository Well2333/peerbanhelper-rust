//! IP 黑名单订阅持久化（`rule_sub_info` / `rule_sub_log`）。供 M6 IPBlackRuleList。

use crate::{Db, Result};

/// 订阅元信息行。
#[derive(Debug, Clone, serde::Serialize)]
pub struct RuleSubInfo {
    pub rule_id: String,
    pub enabled: bool,
    pub rule_name: String,
    pub sub_url: String,
    pub last_update: Option<i64>,
    pub ent_count: Option<i64>,
}

/// 订阅更新日志行。
#[derive(Debug, Clone, serde::Serialize)]
pub struct RuleSubLog {
    pub id: i64,
    pub rule_id: String,
    pub update_time: i64,
    pub count: i64,
    pub update_type: String,
}

impl Db {
    /// upsert 订阅元信息（按 rule_id）。
    pub async fn upsert_rule_sub(&self, info: &RuleSubInfo) -> Result<()> {
        sqlx::query(
            "INSERT INTO rule_sub_info(rule_id, enabled, rule_name, sub_url, last_update, ent_count)
             VALUES(?,?,?,?,?,?)
             ON CONFLICT(rule_id) DO UPDATE SET
                enabled = excluded.enabled,
                rule_name = excluded.rule_name,
                sub_url = excluded.sub_url,
                last_update = COALESCE(excluded.last_update, rule_sub_info.last_update),
                ent_count = COALESCE(excluded.ent_count, rule_sub_info.ent_count)",
        )
        .bind(&info.rule_id)
        .bind(info.enabled as i64)
        .bind(&info.rule_name)
        .bind(&info.sub_url)
        .bind(info.last_update)
        .bind(info.ent_count)
        .execute(self.pool())
        .await?;
        Ok(())
    }

    /// 列出所有订阅。
    pub async fn list_rule_subs(&self) -> Result<Vec<RuleSubInfo>> {
        let rows = sqlx::query_as::<_, (String, i64, String, String, Option<i64>, Option<i64>)>(
            "SELECT rule_id, enabled, rule_name, sub_url, last_update, ent_count
             FROM rule_sub_info ORDER BY rule_id",
        )
        .fetch_all(self.pool())
        .await?
        .into_iter()
        .map(|r| RuleSubInfo {
            rule_id: r.0,
            enabled: r.1 != 0,
            rule_name: r.2,
            sub_url: r.3,
            last_update: r.4,
            ent_count: r.5,
        })
        .collect();
        Ok(rows)
    }

    /// 删除订阅（连带日志）。
    pub async fn delete_rule_sub(&self, rule_id: &str) -> Result<()> {
        sqlx::query("DELETE FROM rule_sub_info WHERE rule_id = ?")
            .bind(rule_id)
            .execute(self.pool())
            .await?;
        sqlx::query("DELETE FROM rule_sub_log WHERE rule_id = ?")
            .bind(rule_id)
            .execute(self.pool())
            .await?;
        Ok(())
    }

    /// 写一条更新日志。
    pub async fn insert_rule_sub_log(
        &self,
        rule_id: &str,
        update_time: i64,
        count: i64,
        update_type: &str,
    ) -> Result<()> {
        sqlx::query(
            "INSERT INTO rule_sub_log(rule_id, update_time, count, update_type) VALUES(?,?,?,?)",
        )
        .bind(rule_id)
        .bind(update_time)
        .bind(count)
        .bind(update_type)
        .execute(self.pool())
        .await?;
        Ok(())
    }

    /// 查询某订阅的更新日志（按时间倒序）。
    pub async fn query_rule_sub_logs(&self, rule_id: &str, limit: i64) -> Result<Vec<RuleSubLog>> {
        let rows = sqlx::query_as::<_, (i64, String, i64, i64, String)>(
            "SELECT id, rule_id, update_time, count, update_type
             FROM rule_sub_log WHERE rule_id = ? ORDER BY update_time DESC LIMIT ?",
        )
        .bind(rule_id)
        .bind(limit)
        .fetch_all(self.pool())
        .await?
        .into_iter()
        .map(|r| RuleSubLog {
            id: r.0,
            rule_id: r.1,
            update_time: r.2,
            count: r.3,
            update_type: r.4,
        })
        .collect();
        Ok(rows)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn rule_sub_crud() {
        let db = Db::open_in_memory().await.unwrap();
        db.upsert_rule_sub(&RuleSubInfo {
            rule_id: "all".into(),
            enabled: true,
            rule_name: "all-in-one".into(),
            sub_url: "https://example.com/all.txt".into(),
            last_update: None,
            ent_count: None,
        })
        .await
        .unwrap();
        // 更新计数。
        db.upsert_rule_sub(&RuleSubInfo {
            rule_id: "all".into(),
            enabled: true,
            rule_name: "all-in-one".into(),
            sub_url: "https://example.com/all.txt".into(),
            last_update: Some(123),
            ent_count: Some(456),
        })
        .await
        .unwrap();
        let list = db.list_rule_subs().await.unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].ent_count, Some(456));

        db.insert_rule_sub_log("all", 100, 456, "MANUAL")
            .await
            .unwrap();
        let logs = db.query_rule_sub_logs("all", 10).await.unwrap();
        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0].update_type, "MANUAL");

        db.delete_rule_sub("all").await.unwrap();
        assert_eq!(db.list_rule_subs().await.unwrap().len(), 0);
        assert_eq!(db.query_rule_sub_logs("all", 10).await.unwrap().len(), 0);
    }
}
