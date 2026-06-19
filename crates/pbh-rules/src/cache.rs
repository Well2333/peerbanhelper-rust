//! 按模块的结果记忆化缓存。对应上游 `util/rule/ModuleMatchCache`。
//!
//! 提供两种写入策略：
//! - `get_or_insert`：命中即返回，未命中则计算并缓存（缓存 pass + ban）。
//! - `get_or_insert_pass_only`：未命中则计算，**仅当结果是 pass 时缓存**（ban 每次重算，
//!   对应上游 `readCacheButWritePassOnly`，避免把短时 ban 结果钉死）。

use std::time::Duration;

use moka::sync::Cache;

/// 泛型记忆化缓存（键为字符串，如 `peer.cacheKey()` 或 `module + ip`）。
#[derive(Clone)]
pub struct ModuleMatchCache<V>
where
    V: Clone + Send + Sync + 'static,
{
    cache: Cache<String, V>,
}

impl<V> ModuleMatchCache<V>
where
    V: Clone + Send + Sync + 'static,
{
    /// `max_capacity` 条，`ttl` 后过期。
    pub fn new(max_capacity: u64, ttl: Duration) -> Self {
        ModuleMatchCache {
            cache: Cache::builder()
                .max_capacity(max_capacity)
                .time_to_live(ttl)
                .build(),
        }
    }

    /// 命中即返回;否则计算、缓存并返回。
    pub fn get_or_insert(&self, key: &str, compute: impl FnOnce() -> V) -> V {
        if let Some(v) = self.cache.get(key) {
            return v;
        }
        let v = compute();
        self.cache.insert(key.to_string(), v.clone());
        v
    }

    /// 命中即返回;否则计算，**仅当 `is_pass(&v)` 为真时缓存**，再返回。
    pub fn get_or_insert_pass_only(
        &self,
        key: &str,
        compute: impl FnOnce() -> V,
        is_pass: impl FnOnce(&V) -> bool,
    ) -> V {
        if let Some(v) = self.cache.get(key) {
            return v;
        }
        let v = compute();
        if is_pass(&v) {
            self.cache.insert(key.to_string(), v.clone());
        }
        v
    }

    /// 失效全部（配置重载时调用）。
    pub fn invalidate_all(&self) {
        self.cache.invalidate_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn caches_and_reuses() {
        let c: ModuleMatchCache<i32> = ModuleMatchCache::new(100, Duration::from_secs(60));
        let calls = Cell::new(0);
        let f = || {
            calls.set(calls.get() + 1);
            42
        };
        assert_eq!(c.get_or_insert("k", f), 42);
        assert_eq!(c.get_or_insert("k", || 99), 42); // 命中缓存，不重算
        assert_eq!(calls.get(), 1);
    }

    #[test]
    fn pass_only_does_not_cache_ban() {
        let c: ModuleMatchCache<&str> = ModuleMatchCache::new(100, Duration::from_secs(60));
        let is_pass = |v: &&str| *v == "pass";
        // ban 不缓存：第二次仍重算。
        assert_eq!(c.get_or_insert_pass_only("k", || "ban", is_pass), "ban");
        assert_eq!(c.get_or_insert_pass_only("k", || "pass", is_pass), "pass");
        // 现在缓存了 pass。
        assert_eq!(c.get_or_insert_pass_only("k", || "ban", is_pass), "pass");
    }
}
