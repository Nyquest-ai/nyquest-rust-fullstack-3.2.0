//! Bounded per-message compression cache.
//!
//! Agent conversations resend old message history every turn — re-running the
//! whole rule pipeline on identical content is wasted work. This cache stores
//! `(compressed_text, hits)` keyed by `hash(content) + level + profile + mode`
//! so that a repeated message returns instantly.
//!
//! Bounded: backed by `lru::LruCache` with a default capacity of 2048 entries.
//! Thread-safe: protected by a `Mutex`. Atomic counters expose hit/miss/eviction
//! totals for the analytics dashboard.
//!
//! Safety: callers MUST decide what is cacheable before calling. We refuse to
//! cache `tool_use` / `image` blocks and very large payloads. See
//! `is_cacheable` for the exact rules.

use lru::LruCache;
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::num::NonZeroUsize;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

/// Mode discriminator for the cache key — request prompts and response
/// (assistant) text run different rule pipelines, so they must not collide.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Mode {
    Request,
    Response,
}

impl Mode {
    fn as_byte(self) -> u8 {
        match self {
            Mode::Request => b'q',
            Mode::Response => b's',
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct CacheKey([u8; 32]);

#[derive(Debug, Clone)]
struct Entry {
    compressed: String,
    hits: usize,
}

/// Default cache capacity. Documented in the repair brief as 1k–5k.
const DEFAULT_CAPACITY: usize = 2048;

/// Cap on the size of cached payloads. Larger inputs (huge tool results,
/// dumps) are not worth caching — they're rarely repeated and would evict
/// many small entries. Tunable via `CompressionCache::set_max_entry_size`.
const DEFAULT_MAX_ENTRY_SIZE: usize = 64 * 1024;

pub struct CompressionCache {
    inner: Mutex<LruCache<CacheKey, Entry>>,
    pub hits: AtomicUsize,
    pub misses: AtomicUsize,
    pub evictions: AtomicUsize,
    pub stores: AtomicUsize,
    max_entry_size: AtomicUsize,
    enabled: AtomicUsize, // 0 = off, 1 = on
}

impl CompressionCache {
    pub fn new(capacity: usize) -> Self {
        let cap = NonZeroUsize::new(capacity.max(1)).unwrap();
        Self {
            inner: Mutex::new(LruCache::new(cap)),
            hits: AtomicUsize::new(0),
            misses: AtomicUsize::new(0),
            evictions: AtomicUsize::new(0),
            stores: AtomicUsize::new(0),
            max_entry_size: AtomicUsize::new(DEFAULT_MAX_ENTRY_SIZE),
            enabled: AtomicUsize::new(1),
        }
    }

    pub fn set_enabled(&self, on: bool) {
        self.enabled
            .store(if on { 1 } else { 0 }, Ordering::Relaxed);
    }

    pub fn enabled(&self) -> bool {
        self.enabled.load(Ordering::Relaxed) != 0
    }

    pub fn set_max_entry_size(&self, bytes: usize) {
        self.max_entry_size.store(bytes, Ordering::Relaxed);
    }

    /// Build the cache key from the message content + the decisions that affect
    /// the compression output. We fold `level`, `profile_name`, and `mode` into
    /// the hash so a config change naturally invalidates the cache.
    fn make_key(&self, content: &str, level: f64, profile_name: &str, mode: Mode) -> CacheKey {
        // Use SHA-256 (already pulled in via `sha2`) for a 32-byte digest.
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(content.as_bytes());
        hasher.update([mode.as_byte()]);
        hasher.update(profile_name.as_bytes());
        hasher.update(level.to_le_bytes());
        let out = hasher.finalize();
        let mut buf = [0u8; 32];
        buf.copy_from_slice(&out);
        CacheKey(buf)
    }

    /// Decide whether `content` is worth caching. Returns false for empty,
    /// oversized, or obviously non-prose content.
    pub fn is_cacheable(&self, content: &str) -> bool {
        if !self.enabled() {
            return false;
        }
        if content.is_empty() {
            return false;
        }
        if content.len() > self.max_entry_size.load(Ordering::Relaxed) {
            return false;
        }
        true
    }

    pub fn get(
        &self,
        content: &str,
        level: f64,
        profile_name: &str,
        mode: Mode,
    ) -> Option<(String, usize)> {
        if !self.is_cacheable(content) {
            return None;
        }
        let key = self.make_key(content, level, profile_name, mode);
        let mut guard = self.inner.lock().ok()?;
        if let Some(entry) = guard.get(&key) {
            self.hits.fetch_add(1, Ordering::Relaxed);
            Some((entry.compressed.clone(), entry.hits))
        } else {
            self.misses.fetch_add(1, Ordering::Relaxed);
            None
        }
    }

    pub fn put(
        &self,
        content: &str,
        level: f64,
        profile_name: &str,
        mode: Mode,
        compressed: String,
        hits: usize,
    ) {
        if !self.is_cacheable(content) {
            return;
        }
        let key = self.make_key(content, level, profile_name, mode);
        if let Ok(mut guard) = self.inner.lock() {
            // `LruCache::push` returns the displaced (key, value) when an LRU
            // eviction happens, OR the prior value when the same key is
            // overwritten. Distinguish by comparing the returned key against
            // the inserted key — only a different key indicates a true eviction.
            let key_inserted = key.clone();
            let prev = guard.push(key, Entry { compressed, hits });
            self.stores.fetch_add(1, Ordering::Relaxed);
            if let Some((returned_key, _)) = prev {
                if returned_key != key_inserted {
                    self.evictions.fetch_add(1, Ordering::Relaxed);
                }
            }
        }
    }

    pub fn snapshot(&self) -> CacheSnapshot {
        let len = self.inner.lock().map(|g| g.len()).unwrap_or(0);
        CacheSnapshot {
            entries: len,
            capacity: self.inner.lock().map(|g| g.cap().get()).unwrap_or(0),
            hits: self.hits.load(Ordering::Relaxed),
            misses: self.misses.load(Ordering::Relaxed),
            evictions: self.evictions.load(Ordering::Relaxed),
            stores: self.stores.load(Ordering::Relaxed),
            enabled: self.enabled(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheSnapshot {
    pub entries: usize,
    pub capacity: usize,
    pub hits: usize,
    pub misses: usize,
    pub evictions: usize,
    pub stores: usize,
    pub enabled: bool,
}

/// Process-global compression cache instance.
///
/// We expose a single shared cache because compression is called from many
/// request handlers — they all share the same key space (hash + level +
/// profile + mode), so keeping one bounded cache amortizes nicely across
/// concurrent agent conversations.
///
/// Three environment variables can override defaults at process start (read
/// the first time the cache is accessed):
///
/// * `NYQUEST_CACHE_CAPACITY` — number of entries (default: 2048)
/// * `NYQUEST_CACHE_MAX_ENTRY_SIZE` — max content bytes per entry (default: 65536)
/// * `NYQUEST_CACHE_ENABLED` — "true"/"1"/"yes"/"on" to enable (default: true)
///
/// All three are optional. Unset values fall back to the defaults above. This
/// lets containerized deployments (Docker, k8s) tune the cache without
/// rebuilding the image.
pub static GLOBAL_CACHE: Lazy<CompressionCache> = Lazy::new(|| {
    let capacity = std::env::var("NYQUEST_CACHE_CAPACITY")
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
        .filter(|&n| n > 0)
        .unwrap_or(DEFAULT_CAPACITY);
    let cache = CompressionCache::new(capacity);

    if let Ok(s) = std::env::var("NYQUEST_CACHE_MAX_ENTRY_SIZE") {
        if let Ok(n) = s.parse::<usize>() {
            cache.set_max_entry_size(n);
        }
    }
    if let Ok(s) = std::env::var("NYQUEST_CACHE_ENABLED") {
        let on = matches!(s.to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on");
        cache.set_enabled(on);
    }
    cache
});

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_hit_returns_value_and_increments_counter() {
        let cache = CompressionCache::new(8);
        cache.put(
            "hello world",
            1.0,
            "aggressive",
            Mode::Request,
            "hi".into(),
            3,
        );
        let got = cache
            .get("hello world", 1.0, "aggressive", Mode::Request)
            .unwrap();
        assert_eq!(got.0, "hi");
        assert_eq!(got.1, 3);
        assert_eq!(cache.hits.load(Ordering::Relaxed), 1);
        assert_eq!(cache.misses.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn cache_miss_when_level_changes() {
        let cache = CompressionCache::new(8);
        cache.put(
            "same text",
            0.5,
            "aggressive",
            Mode::Request,
            "out".into(),
            1,
        );
        // Different level → key changes → cache miss.
        assert!(cache
            .get("same text", 1.0, "aggressive", Mode::Request)
            .is_none());
        assert_eq!(cache.misses.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn cache_miss_when_profile_changes() {
        let cache = CompressionCache::new(8);
        cache.put("text", 1.0, "aggressive", Mode::Request, "a".into(), 0);
        assert!(cache
            .get("text", 1.0, "conservative", Mode::Request)
            .is_none());
    }

    #[test]
    fn cache_miss_when_mode_changes() {
        let cache = CompressionCache::new(8);
        cache.put("text", 1.0, "aggressive", Mode::Request, "a".into(), 0);
        assert!(cache
            .get("text", 1.0, "aggressive", Mode::Response)
            .is_none());
    }

    #[test]
    fn capacity_is_bounded_and_evictions_are_counted() {
        let cache = CompressionCache::new(2);
        cache.put("a", 1.0, "p", Mode::Request, "A".into(), 0);
        cache.put("b", 1.0, "p", Mode::Request, "B".into(), 0);
        cache.put("c", 1.0, "p", Mode::Request, "C".into(), 0);
        assert_eq!(cache.evictions.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn oversize_content_is_not_cached() {
        let cache = CompressionCache::new(8);
        cache.set_max_entry_size(16);
        cache.put(
            "this is a very long string",
            1.0,
            "p",
            Mode::Request,
            "x".into(),
            0,
        );
        assert!(cache
            .get("this is a very long string", 1.0, "p", Mode::Request)
            .is_none());
        assert_eq!(cache.stores.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn disabled_cache_no_ops() {
        let cache = CompressionCache::new(8);
        cache.set_enabled(false);
        cache.put("x", 1.0, "p", Mode::Request, "y".into(), 0);
        assert!(cache.get("x", 1.0, "p", Mode::Request).is_none());
    }
}
