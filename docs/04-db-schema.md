# PeerBanHelper-Rust 数据库 Schema（嵌入式 SQLite）

> 来源：`resources/db/migration/sqlite/V1_1..V1_5` + `databasent/table/*Entity.java` + `mapper/sqlite/*.xml`。
> Rust 端：单文件 `<dataDir>/persist/peerbanhelper-nt.db`，WAL，`busy_timeout=60000`，写池单连接。迁移用 `sqlx::migrate!` 单个**合并版** `V1__initial.sql`（反映 V1_5 后的最终形态）。
> 约定：时间戳 = `INTEGER`(epoch millis)；IP/Inet = `TEXT`(规范串)；JSON/TranslationComponent = `TEXT`(serde_json)；bool = `INTEGER`(0/1)；枚举 = `TEXT`。

## 连接 pragma（连接时设置）
```
journal_mode=WAL; synchronous=NORMAL; busy_timeout=60000;
mmap_size=134217728; journal_size_limit=67108864;
```
写：`SqlitePoolOptions::max_connections(1)`；读可另开小池（WAL 允许并发读）。清理用分块（LIMIT 200 循环）短事务，避免长写锁。

## 表清单

### history — 封禁历史
PK `id` INTEGER AUTOINCREMENT。列：`ban_at` INT, `unban_at` INT, `ip` TEXT, `port` INT, `peer_id` TEXT?, `peer_client_name` TEXT?, `peer_uploaded` INT?, `peer_downloaded` INT?, `peer_progress` REAL, `downloader_progress` REAL, `torrent_id` INT, `module_name` TEXT, `rule_name` TEXT(JSON), `description` TEXT(JSON), `flags` TEXT?, `downloader` TEXT, `structured_data` TEXT?(JSON), `peer_geoip` TEXT?(JSON IPGeoData)。
索引：`downloader`,`ip`,`module_name`,`peer_id`,`torrent_id`,`(ban_at)`,`(peer_uploaded DESC)`,`(peer_downloaded DESC)`。

### banlist — 封禁快照 KV
PK `address` TEXT。`metadata` TEXT(JSON BanMetadata)。（运行时内存权威，此表周期快照）

### pcb_address — PCB 精确 IP 状态
PK `id`。`ip` TEXT, `port` INT, `torrent_id` TEXT, `last_report_progress` REAL, `last_report_uploaded` INT?, `tracking_uploaded_increase_total` INT?, `rewind_counter` INT, `progress_difference_counter` INT, `first_time_seen` INT, `last_time_seen` INT, `downloader` TEXT, `ban_delay_window_end_at` INT, `fast_pcb_test_execute_at` INT, `last_torrent_completed_size` INT。
唯一：`(ip, port, torrent_id, downloader)`。索引：`(last_time_seen)`。

### pcb_range — PCB 前缀聚合状态
PK `id`。`ip_range` TEXT（V1_5 由 `range` 改名）+ 与 `pcb_address` 相同的分析列。
唯一：`(ip_range, torrent_id, downloader)`。索引：`(last_time_seen)`。

### peer_records — peer 记录
PK `id`。`address` TEXT, `port` INT, `torrent_id` INT, `downloader` TEXT, `peer_id` TEXT?, `client_name` TEXT?, `uploaded` INT, `uploaded_offset` INT, `upload_speed` INT, `downloaded` INT, `downloaded_offset` INT, `download_speed` INT, `last_flags` TEXT?, `first_time_seen` INT, `last_time_seen` INT, `peer_geoip` TEXT?(JSON)。
唯一：**`(address, torrent_id, downloader)`**（V1_3 去掉了 port）。索引：`address`,`(last_time_seen)`,`(downloader,uploaded,downloaded,first_time_seen,last_time_seen)`,`(downloader,first_time_seen,last_time_seen)`。
> upsert 含带 offset 的单调累加冲突解决（最难单条 SQL，见下）。

### peer_connection_metrics — 连接指标（聚合）
PK `id`。`timeframe_at` INT, `downloader` TEXT + 16 个 INT 计数器：`total_connections, incoming_connections, remote_refuse_transfer_to_client, remote_accept_transfer_to_client, local_refuse_transfer_to_peer, local_accept_transfer_to_peer, local_not_interested, question_status, optimistic_unchoke, from_dht, from_pex, from_lsd, from_tracker_or_other, rc4_encrypted, plain_text_encrypted, utp_socket, tcp_socket`。
唯一：`(timeframe_at, downloader)`。

### peer_connection_metrics_track — 连接指标（逐 peer）
PK `id`。`timeframe_at` INT, `downloader` TEXT, `torrent_id` INT, `address` TEXT, `port` INT, `peer_id` TEXT?(V1_4 改可空), `client_name` TEXT?, `last_flags` TEXT?。
唯一：`(timeframe_at, downloader, torrent_id, address, port)`。

### traffic_journal_v3 — 流量账（小时分桶）
PK `id`。`timestamp` INT(小时桶), `downloader` TEXT + 8 个 INT：`data_overall_uploaded_at_start, data_overall_uploaded, data_overall_downloaded_at_start, data_overall_downloaded, protocol_overall_uploaded_at_start, protocol_overall_uploaded, protocol_overall_downloaded_at_start, protocol_overall_downloaded`。
唯一：`(timestamp, downloader)`。

### rule_sub_info — 订阅规则信息
PK `rule_id` TEXT（调用方提供）。`enabled` INT(bool), `rule_name` TEXT, `sub_url` TEXT, `last_update` INT?, `ent_count` INT?。索引：`(rule_id)`。

### rule_sub_log — 订阅更新日志
PK `id`。`rule_id` TEXT, `update_time` INT, `count` INT, `update_type` TEXT(枚举 AUTO/MANUAL)。索引：`(rule_id, update_time DESC)`。

### alert — 告警
PK `id`。`create_at` INT, `read_at` INT?, `level` TEXT(INFO/WARN/ERROR/FATAL), `identifier` TEXT, `title` TEXT(JSON), `content` TEXT(JSON)。索引：`(read_at,identifier)`,`(read_at)`,`(create_at,read_at)`。

### torrents — 种子
PK `id`。`info_hash` TEXT, `name` TEXT, `size` INT, `private_torrent` INT?(bool)。唯一：`(info_hash)`。索引：`name`,`private_torrent`。
> upsert：`ON CONFLICT(info_hash)` 保留更大 size / 非空 private。

### metadata — KV（游标/缓存）
PK `k` TEXT。`v` TEXT?。键：`btn.submithistory.timestamp`, `BtnAbilitySubmitBans.cursor`, `BtnAbilitySubmitSwarm.cursor`(`"lastTimeSeen,id"`), `btn.ability.rules.cache`, `btn.ability.ip_denylist.cache.{version,value}`, `btn.ability.ip_allowlist.cache.{version,value}`。游标须重启续传。

### tracked_swarm — 当前 swarm（临时表，启动重置）
PK `id`。`ip` TEXT, `port` INT, `info_hash` TEXT, `torrent_is_private` INT, `torrent_size` INT, `downloader` TEXT, `downloader_progress` REAL, `peer_id` TEXT?, `client_name` TEXT?, `peer_progress` REAL(*注：原 DDL 误写 TEXT，Rust 用 REAL*), `uploaded` INT, `uploaded_offset` INT, `upload_speed` INT, `downloaded` INT, `downloaded_offset` INT, `download_speed` INT, `last_flags` TEXT?, `first_time_seen` INT, `last_time_seen` INT, `download_speed_max` INT, `upload_speed_max` INT。
唯一：`(ip, port, info_hash, downloader)`。索引：`(last_time_seen DESC)`。

## 需手工移植的关键 SQL（驱动仪表盘）

> 所有 `${field}`/`${orderBy}` **必须做枚举白名单映射到固定列**，绝不拼接用户输入。

1. **peer_records upsert（最难）：** `ON CONFLICT(address,torrent_id,downloader) DO UPDATE`，按时间戳 last-write-wins + 带 offset 的单调流量累加（`uploaded = peer.uploaded + excluded.uploaded - offset`，clamp）。逐 CASE 照搬。
2. **history `sumField`/`countField`：** CTE(`filtered_data` 可选 `SUBSTR`)→`total_sum`→percent=`CAST(.. AS REAL)/total`→`HAVING percent > ?`→`ORDER BY count DESC`，可选 `JOIN torrents`。
3. **history `getBannedIps`：** `SELECT ip, COUNT(*) ... [WHERE ip LIKE ?||'%'] GROUP BY ip ORDER BY count DESC`（分页）。
4. **peer_records `queryClientAnalyse`：** `GROUP BY peer_id, client_name`，SUM 上下行 + COUNT，`first_time_seen>=? AND last_time_seen<=? AND uploaded>0 AND downloaded>0`，动态 `ORDER BY`（白名单）。
5. **traffic_journal 聚合：** `GROUP BY timestamp`（已是小时桶整数），逐行 `MAX(0, x - x_at_start)`。
6. **torrents.search：** `ORDER BY (SELECT COUNT(*) FROM history|peer_records WHERE torrent_id = torrents.id)`（子查询表白名单）。

> **注意：** 服务于被删 PBH_PLUS 图表的查询（sessionAnalyse/clientAnalyse 等）本期可不移植；`/api/statistic/*`（counter/field/date/banTrends）保留。

## 不做的事
- 不支持 MySQL/PostgreSQL/H2（删方言分支）。
- 不做 legacy ORMLite→MyBatis 数据迁移（用户不需历史数据）。
- 不做 Flyway 的 `db/repeat`（现状为空）。
