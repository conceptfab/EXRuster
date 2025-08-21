use std::collections::HashMap;
use std::hash::Hash;
use std::sync::{Arc, Mutex};
use lru::LruCache;
use std::num::NonZeroUsize;

/// Generic cache trait with different eviction policies
pub trait Cache<K, V> {
    /// Get value from cache
    fn get(&mut self, key: &K) -> Option<&V>;
    
    /// Insert or update value in cache
    fn put(&mut self, key: K, value: V);
    
    /// Clear all entries
    fn clear(&mut self);
}

/// LRU cache implementation
pub struct LruCacheImpl<K, V> 
where 
    K: Hash + Eq,
{
    cache: LruCache<K, V>,
}

impl<K, V> LruCacheImpl<K, V> 
where 
    K: Hash + Eq,
{
    pub fn new(capacity: usize) -> Self {
        Self {
            cache: LruCache::new(NonZeroUsize::new(capacity).unwrap()),
        }
    }
}

impl<K, V> Cache<K, V> for LruCacheImpl<K, V> 
where 
    K: Hash + Eq,
{
    fn get(&mut self, key: &K) -> Option<&V> {
        self.cache.get(key)
    }
    
    fn put(&mut self, key: K, value: V) {
        self.cache.put(key, value);
    }
    
    
    fn clear(&mut self) {
        self.cache.clear();
    }
}

/// FIFO cache with size limit (like the color matrix cache)
pub struct FifoCacheImpl<K, V> 
where 
    K: Hash + Eq + Clone,
{
    cache: HashMap<K, V>,
    insertion_order: Vec<K>,
    capacity: usize,
}

impl<K, V> FifoCacheImpl<K, V> 
where 
    K: Hash + Eq + Clone,
{
    pub fn new(capacity: usize) -> Self {
        Self {
            cache: HashMap::new(),
            insertion_order: Vec::new(),
            capacity,
        }
    }
}

impl<K, V> Cache<K, V> for FifoCacheImpl<K, V> 
where 
    K: Hash + Eq + Clone,
{
    fn get(&mut self, key: &K) -> Option<&V> {
        self.cache.get(key)
    }
    
    fn put(&mut self, key: K, value: V) {
        if self.cache.contains_key(&key) {
            // Update existing entry
            self.cache.insert(key, value);
        } else {
            // New entry - check capacity
            if self.cache.len() >= self.capacity {
                // Remove oldest entry
                if let Some(oldest_key) = self.insertion_order.first().cloned() {
                    self.cache.remove(&oldest_key);
                    self.insertion_order.retain(|k| k != &oldest_key);
                }
            }
            self.insertion_order.push(key.clone());
            self.cache.insert(key, value);
        }
    }
    
    
    fn clear(&mut self) {
        self.cache.clear();
        self.insertion_order.clear();
    }
}

/// Thread-safe cache wrapper
pub struct ThreadSafeCache<K, V, C> 
where 
    C: Cache<K, V>,
{
    cache: Arc<Mutex<C>>,
    _phantom: std::marker::PhantomData<(K, V)>,
}

impl<K, V, C> ThreadSafeCache<K, V, C> 
where 
    C: Cache<K, V>,
{
    pub fn new(cache: C) -> Self {
        Self {
            cache: Arc::new(Mutex::new(cache)),
            _phantom: std::marker::PhantomData,
        }
    }
    
    pub fn get<F, R>(&self, key: &K, f: F) -> Option<R>
    where 
        F: FnOnce(&V) -> R,
    {
        if let Ok(mut cache) = self.cache.lock() {
            cache.get(key).map(f)
        } else {
            None
        }
    }
    
    pub fn put(&self, key: K, value: V) {
        if let Ok(mut cache) = self.cache.lock() {
            cache.put(key, value);
        }
    }
    
    pub fn clear(&self) {
        if let Ok(mut cache) = self.cache.lock() {
            cache.clear();
        }
    }
    
}

impl<K, V, C> Clone for ThreadSafeCache<K, V, C> 
where 
    C: Cache<K, V>,
{
    fn clone(&self) -> Self {
        Self {
            cache: Arc::clone(&self.cache),
            _phantom: std::marker::PhantomData,
        }
    }
}

/// Cache statistics for monitoring
#[derive(Debug, Default)]
pub struct CacheStats {
    pub hits: u64,
    pub misses: u64,
}

impl CacheStats {
    pub fn hit_rate(&self) -> f64 {
        if self.hits + self.misses == 0 {
            0.0
        } else {
            self.hits as f64 / (self.hits + self.misses) as f64
        }
    }
}

/// Cache with statistics tracking
pub struct StatsCache<K, V, C> 
where 
    C: Cache<K, V>,
{
    cache: C,
    hits: u64,
    misses: u64,
    _phantom: std::marker::PhantomData<(K, V)>,
}

impl<K, V, C> StatsCache<K, V, C> 
where 
    C: Cache<K, V>,
{
    pub fn new(cache: C) -> Self {
        Self {
            cache,
            hits: 0,
            misses: 0,
            _phantom: std::marker::PhantomData,
        }
    }
    
    pub fn stats(&self) -> CacheStats {
        CacheStats {
            hits: self.hits,
            misses: self.misses,
        }
    }
}

impl<K, V, C> Cache<K, V> for StatsCache<K, V, C> 
where 
    C: Cache<K, V>,
{
    fn get(&mut self, key: &K) -> Option<&V> {
        match self.cache.get(key) {
            Some(value) => {
                self.hits += 1;
                Some(value)
            }
            None => {
                self.misses += 1;
                None
            }
        }
    }
    
    fn put(&mut self, key: K, value: V) {
        self.cache.put(key, value);
    }
    
    
    fn clear(&mut self) {
        self.cache.clear();
        self.hits = 0;
        self.misses = 0;
    }
}

/// Convenience type aliases
pub type LruThreadSafeCache<K, V> = ThreadSafeCache<K, V, LruCacheImpl<K, V>>;
pub type ThreadSafeStatsFifoCache<K, V> = ThreadSafeCache<K, V, StatsCache<K, V, FifoCacheImpl<K, V>>>;

/// Create a thread-safe LRU cache
pub fn new_thread_safe_lru_cache<K, V>(capacity: usize) -> LruThreadSafeCache<K, V> 
where 
    K: Hash + Eq,
{
    ThreadSafeCache::new(LruCacheImpl::new(capacity))
}

/// Create a thread-safe FIFO cache with statistics
pub fn new_thread_safe_stats_fifo_cache<K, V>(capacity: usize) -> ThreadSafeStatsFifoCache<K, V> 
where 
    K: Hash + Eq + Clone,
{
    ThreadSafeCache::new(StatsCache::new(FifoCacheImpl::new(capacity)))
}

impl<K, V> ThreadSafeStatsFifoCache<K, V> 
where 
    K: Hash + Eq + Clone,
{
    /// Get cache statistics
    pub fn stats(&self) -> CacheStats {
        if let Ok(cache) = self.cache.lock() {
            cache.stats()
        } else {
            CacheStats::default()
        }
    }
}