# PeerBanHelper-Rust 数据库 Schema（嵌入式 SQLite · v2 精简表集）

> 本文件只列 **v2 实际保留的表**。已砍除的纯图表/告警表（`traffic_journal_v3`、
> `peer_connection_metrics(+track)`、`alert`，详见 `02-strategy-and-roadmap.md` §3）不再收录。
> 上游完整 14 表结构如需查阅见 `source/.../databasent/` 与 `resources/db/migration/sqlite/`。

> Rust 端：单文件 `<dataDir>/persist/peerbanhelper-nt.db`，WAL，`busy_timeout=60000`，写池单连接。迁移用 `sqlx::migrate!` 单个**合并版** `V1__initial.sql`。
> 约定：时间戳 = `INTEGER`(epoch millis)；IP/Inet = `TEXT`(规范串)；JSON = `TEXT`(serde_json)；bool = `INTEGER`(0/1)；枚举 = `TEXT`。
> 注：v2 已砍 i18n，原 `TranslationComponent` 字段（如 history.rule_name/description）改存**纯字符串**。

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

### rule_sub_info — 订阅规则信息
PK `rule_id` TEXT（调用方提供）。`enabled` INT(bool), `rule_name` TEXT, `sub_url` TEXT, `last_update` INT?, `ent_count` INT?。索引：`(rule_id)`。

### rule_sub_log — 订阅更新日志
PK `id`。`rule_id` TEXT, `update_time` INT, `count` INT, `update_type` TEXT(枚举 AUTO/MANUAL)。索引：`(rule_id, update_time DESC)`。

### torrents — 种子
PK `id`。`info_hash` TEXT, `name` TEXT, `size` INT, `private_torrent` INT?(bool)。唯一：`(info_hash)`。索引：`name`,`private_torrent`。
> upsert：`ON CONFLICT(info_hash)` 保留更大 size / 非空 private。

### metadata — KV（游标/缓存）
PK `k` TEXT。`v` TEXT?。键：`btn.submithistory.timestamp`, `BtnAbilitySubmitBans.cursor`, `BtnAbilitySubmitSwarm.cursor`(`"lastTimeSeen,id"`), `btn.ability.rules.cache`, `btn.ability.ip_denylist.cache.{version,value}`, `btn.ability.ip_allowlist.cache.{version,value}`。游标须重启续传。

### tracked_swarm — 当前 swarm（临时表，启动重置）
PK `id`。`ip` TEXT, `port` INT, `info_hash` TEXT, `torrent_is_private` INT, `torrent_size` INT, `downloader` TEXT, `downloader_progress` REAL, `peer_id` TEXT?, `client_name` TEXT?, `peer_progress` REAL(*注：原 DDL 误写 TEXT，Rust 用 REAL*), `uploaded` INT, `uploaded_offset` INT, `upload_speed` INT, `downloaded` INT, `downloaded_offset` INT, `download_speed` INT, `last_flags` TEXT?, `first_time_seen` INT, `last_time_seen` INT, `download_speed_max` INT, `upload_speed_max` INT。
唯一：`(ip, port, info_hash, downloader)`。索引：`(last_time_seen DESC)`。

## 需手工移植的关键 SQL（v2 精简）

v2 砍掉了图表/统计分析,故原 `sumField`/`countField`/`getBannedIps`/`queryClientAnalyse`/`traffic_journal 聚合`/`torrents.search` 等分析查询**均不移植**。v2 实际需要的非平凡 SQL 只有几条 upsert：

1. **peer_records upsert（最难,供 BTN 上行）：** `ON CONFLICT(address,torrent_id,downloader) DO UPDATE`，按时间戳 last-write-wins + 带 offset 的单调流量累加（`uploaded = peer.uploaded + excluded.uploaded - offset`，clamp）。逐 CASE 照搬。
2. **tracked_swarm upsert（供 BTN 上行）** 与 **torrents upsert**（`ON CONFLICT(info_hash)` 保留更大 size / 非空 private）。
3. **pcb_address / pcb_range upsert**（按唯一键更新分析列,见 M5）。

其余为平凡 CRUD：history 插入 + `/api/bans/history` 分页查询、banlist 全表 save/load、rule_sub 增改查、metadata KV。所有动态排序/过滤字段须**枚举白名单**,绝不拼接用户输入。

## 不做的事
- 不支持 MySQL/PostgreSQL/H2（删方言分支）。
- 不做 legacy ORMLite→MyBatis 数据迁移（用户不需历史数据）。
- 不做 Flyway 的 `db/repeat`（现状为空）。
