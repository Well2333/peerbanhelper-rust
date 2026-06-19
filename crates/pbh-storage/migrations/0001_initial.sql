-- PeerBanHelper-Rust v2 精简表集（嵌入式 SQLite）。
-- 约定：时间戳/布尔/计数 = INTEGER(epoch millis / 0,1)；IP/JSON/枚举 = TEXT；进度 = REAL。
-- 详见 memory/design/db-schema.md。

-- 种子（history.torrent_id 引用其 id）
CREATE TABLE IF NOT EXISTS torrents (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    info_hash       TEXT    NOT NULL,
    name            TEXT    NOT NULL,
    size            INTEGER NOT NULL,
    private_torrent INTEGER
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_torrents_info_hash ON torrents(info_hash);
CREATE INDEX IF NOT EXISTS idx_torrents_name ON torrents(name);

-- 封禁历史
CREATE TABLE IF NOT EXISTS history (
    id                  INTEGER PRIMARY KEY AUTOINCREMENT,
    ban_at              INTEGER NOT NULL,
    unban_at            INTEGER NOT NULL,
    ip                  TEXT    NOT NULL,
    port                INTEGER NOT NULL,
    peer_id             TEXT,
    peer_client_name    TEXT,
    peer_uploaded       INTEGER,
    peer_downloaded     INTEGER,
    peer_progress       REAL    NOT NULL,
    downloader_progress REAL    NOT NULL,
    torrent_id          INTEGER NOT NULL,
    module_name         TEXT    NOT NULL,
    rule_name           TEXT    NOT NULL,   -- v2: 纯字符串(无 i18n)
    description         TEXT    NOT NULL,   -- v2: 纯字符串
    flags               TEXT,
    downloader          TEXT    NOT NULL,
    structured_data     TEXT,               -- JSON
    peer_geoip          TEXT                -- JSON(IpGeoData)
);
CREATE INDEX IF NOT EXISTS idx_history_ban_at ON history(ban_at);
CREATE INDEX IF NOT EXISTS idx_history_ip ON history(ip);
CREATE INDEX IF NOT EXISTS idx_history_downloader ON history(downloader);
CREATE INDEX IF NOT EXISTS idx_history_module_name ON history(module_name);
CREATE INDEX IF NOT EXISTS idx_history_torrent_id ON history(torrent_id);

-- 内存封禁表的周期快照（K=address, V=BanMetadata JSON）
CREATE TABLE IF NOT EXISTS banlist (
    address  TEXT PRIMARY KEY,
    metadata TEXT NOT NULL
);

-- PCB 精确 IP 状态
CREATE TABLE IF NOT EXISTS pcb_address (
    id                               INTEGER PRIMARY KEY AUTOINCREMENT,
    ip                               TEXT    NOT NULL,
    port                             INTEGER NOT NULL,
    torrent_id                       TEXT    NOT NULL,
    last_report_progress             REAL    NOT NULL,
    last_report_uploaded             INTEGER,
    tracking_uploaded_increase_total INTEGER,
    rewind_counter                   INTEGER NOT NULL,
    progress_difference_counter      INTEGER NOT NULL,
    first_time_seen                  INTEGER NOT NULL,
    last_time_seen                   INTEGER NOT NULL,
    downloader                       TEXT    NOT NULL,
    ban_delay_window_end_at          INTEGER,
    fast_pcb_test_execute_at         INTEGER,
    last_torrent_completed_size      INTEGER
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_pcb_address_unique ON pcb_address(ip, port, torrent_id, downloader);
CREATE INDEX IF NOT EXISTS idx_pcb_address_last_time_seen ON pcb_address(last_time_seen);

-- PCB 前缀聚合状态
CREATE TABLE IF NOT EXISTS pcb_range (
    id                               INTEGER PRIMARY KEY AUTOINCREMENT,
    ip_range                         TEXT    NOT NULL,
    torrent_id                       TEXT    NOT NULL,
    last_report_progress             REAL    NOT NULL,
    last_report_uploaded             INTEGER,
    tracking_uploaded_increase_total INTEGER,
    rewind_counter                   INTEGER NOT NULL,
    progress_difference_counter      INTEGER NOT NULL,
    first_time_seen                  INTEGER NOT NULL,
    last_time_seen                   INTEGER NOT NULL,
    downloader                       TEXT    NOT NULL,
    ban_delay_window_end_at          INTEGER,
    fast_pcb_test_execute_at         INTEGER,
    last_torrent_completed_size      INTEGER
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_pcb_range_unique ON pcb_range(ip_range, torrent_id, downloader);
CREATE INDEX IF NOT EXISTS idx_pcb_range_last_time_seen ON pcb_range(last_time_seen);

-- peer 记录（供 BTN 上行 submit_history）
CREATE TABLE IF NOT EXISTS peer_records (
    id                INTEGER PRIMARY KEY AUTOINCREMENT,
    address           TEXT    NOT NULL,
    port              INTEGER NOT NULL,
    torrent_id        INTEGER NOT NULL,
    downloader        TEXT    NOT NULL,
    peer_id           TEXT,
    client_name       TEXT,
    uploaded          INTEGER NOT NULL,
    uploaded_offset   INTEGER NOT NULL,
    upload_speed      INTEGER NOT NULL,
    downloaded        INTEGER NOT NULL,
    downloaded_offset INTEGER NOT NULL,
    download_speed    INTEGER NOT NULL,
    last_flags        TEXT,
    first_time_seen   INTEGER NOT NULL,
    last_time_seen    INTEGER NOT NULL,
    peer_geoip        TEXT
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_peer_records_unique ON peer_records(address, torrent_id, downloader);
CREATE INDEX IF NOT EXISTS idx_peer_records_address ON peer_records(address);
CREATE INDEX IF NOT EXISTS idx_peer_records_last_time_seen ON peer_records(last_time_seen);

-- 订阅规则信息
CREATE TABLE IF NOT EXISTS rule_sub_info (
    rule_id     TEXT PRIMARY KEY,
    enabled     INTEGER NOT NULL,
    rule_name   TEXT    NOT NULL,
    sub_url     TEXT    NOT NULL,
    last_update INTEGER,
    ent_count   INTEGER
);

-- 订阅更新日志
CREATE TABLE IF NOT EXISTS rule_sub_log (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    rule_id     TEXT    NOT NULL,
    update_time INTEGER NOT NULL,
    count       INTEGER NOT NULL,
    update_type TEXT    NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_rule_sub_log_rule_id ON rule_sub_log(rule_id, update_time DESC);

-- KV 元数据（BTN 游标/缓存等）
CREATE TABLE IF NOT EXISTS metadata (
    k TEXT PRIMARY KEY,
    v TEXT
);

-- 当前 swarm（供 BTN 上行 submit_swarm；启动可重置）
CREATE TABLE IF NOT EXISTS tracked_swarm (
    id                  INTEGER PRIMARY KEY AUTOINCREMENT,
    ip                  TEXT    NOT NULL,
    port                INTEGER NOT NULL,
    info_hash           TEXT    NOT NULL,
    torrent_is_private  INTEGER NOT NULL,
    torrent_size        INTEGER NOT NULL,
    downloader          TEXT    NOT NULL,
    downloader_progress REAL    NOT NULL,
    peer_id             TEXT,
    client_name         TEXT,
    peer_progress       REAL    NOT NULL,
    uploaded            INTEGER NOT NULL,
    uploaded_offset     INTEGER NOT NULL,
    upload_speed        INTEGER NOT NULL,
    downloaded          INTEGER NOT NULL,
    downloaded_offset   INTEGER NOT NULL,
    download_speed      INTEGER NOT NULL,
    last_flags          TEXT,
    first_time_seen     INTEGER NOT NULL,
    last_time_seen      INTEGER NOT NULL,
    download_speed_max  INTEGER NOT NULL,
    upload_speed_max    INTEGER NOT NULL
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_tracked_swarm_unique ON tracked_swarm(ip, port, info_hash, downloader);
CREATE INDEX IF NOT EXISTS idx_tracked_swarm_last_time_seen ON tracked_swarm(last_time_seen DESC);
