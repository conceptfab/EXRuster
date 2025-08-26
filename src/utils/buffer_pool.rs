use std::collections::BTreeMap;
use std::ops::{Deref, DerefMut};
use std::sync::{Arc, Mutex, Weak};

/// Wrapper that automatically returns buffer to pool when dropped
pub struct PooledBuffer<T: Send + 'static> {
    data: Option<Vec<T>>,
    pool: Weak<BufferPool>,
}

impl<T: Send + 'static> PooledBuffer<T> {
    fn new(data: Vec<T>, pool: Weak<BufferPool>) -> Self {
        Self {
            data: Some(data),
            pool,
        }
    }

    /// Take ownership of the inner Vec, preventing return to pool
    pub fn into_inner(mut self) -> Vec<T> {
        self.data.take().unwrap_or_default()
    }
}

impl<T: Send + 'static> Deref for PooledBuffer<T> {
    type Target = Vec<T>;

    fn deref(&self) -> &Self::Target {
        self.data.as_ref().unwrap()
    }
}

impl<T: Send + 'static> DerefMut for PooledBuffer<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.data.as_mut().unwrap()
    }
}

impl<T: Send + 'static> Drop for PooledBuffer<T> {
    fn drop(&mut self) {
        if let (Some(data), Some(pool)) = (self.data.take(), self.pool.upgrade()) {
            pool.return_buffer(data);
        }
    }
}

/// Buffer pool for reusing Vec<f32> allocations
/// Eliminates frequent allocations in hot image processing paths
/// Uses BTreeMap for O(log n) buffer selection by size
pub struct BufferPool {
    f32_buffers_by_size: Mutex<BTreeMap<usize, Vec<Vec<f32>>>>,
    max_pool_size: usize,
}

impl BufferPool {
    /// Create a new buffer pool with size limit
    pub fn new(max_pool_size: usize) -> Self {
        Self {
            f32_buffers_by_size: Mutex::new(BTreeMap::new()),
            max_pool_size,
        }
    }

    /// Get a pooled buffer with at least the specified capacity
    /// Returns a reused buffer if available, otherwise creates a new one
    /// Buffer is automatically returned to pool when dropped
    pub fn get_f32_buffer(self: &Arc<Self>, min_capacity: usize) -> PooledBuffer<f32> {
        let buffer = if let Ok(mut pool) = self.f32_buffers_by_size.lock() {
            // Find the smallest buffer with sufficient capacity in O(log n)
            if let Some((_, buffers)) = pool.range_mut(min_capacity..).next() {
                if let Some(mut buffer) = buffers.pop() {
                    buffer.clear();
                    // Remove empty size entry to keep BTreeMap clean
                    if buffers.is_empty() {
                        let capacity = buffer.capacity();
                        pool.remove(&capacity);
                    }
                    buffer
                } else {
                    // Create new buffer with extra capacity
                    let capacity = (min_capacity * 5 / 4).max(1024);
                    Vec::with_capacity(capacity)
                }
            } else {
                // Create new buffer with extra capacity
                let capacity = (min_capacity * 5 / 4).max(1024);
                Vec::with_capacity(capacity)
            }
        } else {
            // Fallback if lock fails
            let capacity = (min_capacity * 5 / 4).max(1024);
            Vec::with_capacity(capacity)
        };

        PooledBuffer::new(buffer, Arc::downgrade(self))
    }

    /// Return a buffer to the pool (called automatically by PooledBuffer::drop)
    fn return_buffer<T>(&self, buffer: Vec<T>)
    where
        T: 'static + Send,
    {
        // Only handle f32 buffers for now
        if std::any::TypeId::of::<T>() == std::any::TypeId::of::<f32>() {
            // Safety: We just checked the type ID matches f32
            let buffer_f32: Vec<f32> = unsafe { std::mem::transmute(buffer) };

            if let Ok(mut pool) = self.f32_buffers_by_size.lock() {
                let capacity = buffer_f32.capacity();
                let buffers = pool.entry(capacity).or_insert_with(Vec::new);

                // Enforce pool size limit - only store if under limit
                if buffers.len() < self.max_pool_size {
                    buffers.push(buffer_f32);
                }
                // If over limit, buffer is just dropped (deallocated)
            }
        }
    }
}

impl Default for BufferPool {
    fn default() -> Self {
        Self::new(16) // Parameter ignored but kept for API compatibility
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_buffer_pool_basic_allocation() {
        let pool = Arc::new(BufferPool::new(4));

        // Get a buffer
        let buffer = pool.get_f32_buffer(100);
        assert!(buffer.capacity() >= 100);
        assert_eq!(buffer.len(), 0);
    }

    #[test]
    fn test_buffer_pool_capacity() {
        let pool = Arc::new(BufferPool::new(4));

        // Get a buffer and use it
        let mut buffer = pool.get_f32_buffer(100);
        buffer.push(1.0);
        buffer.push(2.0);
        assert_eq!(buffer.len(), 2);
        assert!(buffer.capacity() >= 100);

        // Get another buffer with different capacity
        let buffer2 = pool.get_f32_buffer(200);
        assert!(buffer2.capacity() >= 200);
        assert_eq!(buffer2.len(), 0);
    }

    #[test]
    fn test_buffer_pool_reuse() {
        let pool = Arc::new(BufferPool::new(4));
        let capacity = {
            // Get a buffer, use it, then drop it
            let mut buffer = pool.get_f32_buffer(100);
            buffer.push(42.0);
            let cap = buffer.capacity();
            // Buffer is automatically returned to pool when dropped
            cap
        };

        // Get another buffer of same size - should reuse the previous one
        let mut buffer2 = pool.get_f32_buffer(100);
        assert_eq!(buffer2.capacity(), capacity);
        assert_eq!(buffer2.len(), 0); // Buffer should be cleared when reused

        buffer2.push(1.0);
        assert_eq!(buffer2[0], 1.0);
    }

    #[test]
    fn test_buffer_pool_size_limit() {
        let pool = Arc::new(BufferPool::new(2)); // Only 2 buffers per size

        // Create and drop 3 buffers of same capacity
        for i in 0..3 {
            let mut buffer = pool.get_f32_buffer(100);
            buffer.push(i as f32);
            // Buffer automatically returned on drop
        }

        // Pool should only keep 2 buffers due to size limit
        let pool_state = pool.f32_buffers_by_size.lock().unwrap();
        if let Some(buffers) = pool_state.values().next() {
            assert!(buffers.len() <= 2, "Pool should respect size limit");
        }
    }
}
