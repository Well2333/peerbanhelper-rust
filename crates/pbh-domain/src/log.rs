//! 进程内日志环形缓冲（供后续 WS 日志流 `/api/logs/stream` 与 `/api/logs` 历史）。
//!
//! std-only：用 `Mutex<VecDeque>` + 单调 `seq`。实时推送（broadcast）在 M7 接 WS 时再加。
//! 对应上游 web 日志环形缓冲 + `NewLogEntryCreatedEvent`。

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

/// 一条日志。`seq` 单调递增，供 WS `?offset=` 增量回放。
#[derive(Debug, Clone)]
pub struct LogEntry {
    pub seq: u64,
    pub time_ms: i64,
    pub level: String,
    pub target: String,
    pub message: String,
}

/// 有界环形日志缓冲。
#[derive(Debug)]
pub struct LogBuffer {
    inner: Mutex<VecDeque<LogEntry>>,
    cap: usize,
    seq: AtomicU64,
}

impl LogBuffer {
    pub fn new(cap: usize) -> Arc<Self> {
        Arc::new(LogBuffer {
            inner: Mutex::new(VecDeque::with_capacity(cap.min(1024))),
            cap: cap.max(1),
            seq: AtomicU64::new(0),
        })
    }

    /// 追加一条，返回其（含分配好的 seq/time）。超出容量丢最旧。
    pub fn push(&self, level: &str, target: &str, message: String) -> LogEntry {
        let seq = self.seq.fetch_add(1, Ordering::Relaxed) + 1;
        let entry = LogEntry {
            seq,
            time_ms: now_ms(),
            level: level.to_string(),
            target: target.to_string(),
            message,
        };
        let mut q = self.inner.lock().unwrap();
        if q.len() >= self.cap {
            q.pop_front();
        }
        q.push_back(entry.clone());
        entry
    }

    /// 全量快照（旧→新）。
    pub fn snapshot(&self) -> Vec<LogEntry> {
        self.inner.lock().unwrap().iter().cloned().collect()
    }

    /// 仅返回 `seq > after` 的条目（WS 回放）。
    pub fn since(&self, after: u64) -> Vec<LogEntry> {
        self.inner
            .lock()
            .unwrap()
            .iter()
            .filter(|e| e.seq > after)
            .cloned()
            .collect()
    }

    /// 当前最大 seq。
    pub fn last_seq(&self) -> u64 {
        self.seq.load(Ordering::Relaxed)
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ring_evicts_oldest_and_tracks_seq() {
        let buf = LogBuffer::new(2);
        buf.push("INFO", "t", "a".into());
        buf.push("INFO", "t", "b".into());
        buf.push("WARN", "t", "c".into()); // 挤掉 a
        let snap = buf.snapshot();
        assert_eq!(snap.len(), 2);
        assert_eq!(snap[0].message, "b");
        assert_eq!(snap[1].message, "c");
        assert_eq!(buf.last_seq(), 3);
    }

    #[test]
    fn since_returns_newer_only() {
        let buf = LogBuffer::new(10);
        let a = buf.push("INFO", "t", "a".into());
        buf.push("INFO", "t", "b".into());
        let newer = buf.since(a.seq);
        assert_eq!(newer.len(), 1);
        assert_eq!(newer[0].message, "b");
    }
}
