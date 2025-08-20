use std::sync::{Arc, Mutex};
use std::collections::VecDeque;

/// Buffer pool for reusing Vec<f32> and Vec<u8> allocations
/// Eliminates frequent allocations in hot image processing paths
pub struct BufferPool {
    f32_buffers: Mutex<VecDeque<Vec<f32>>>,
    #[allow(dead_code)]
    u8_buffers: Mutex<VecDeque<Vec<u8>>>,
    max_pool_size: usize,
}

impl BufferPool {
    /// Create a new buffer pool with specified maximum pool size
    pub fn new(max_pool_size: usize) -> Self {
        Self {
            f32_buffers: Mutex::new(VecDeque::with_capacity(max_pool_size)),
            u8_buffers: Mutex::new(VecDeque::with_capacity(max_pool_size)),
            max_pool_size,
        }
    }

    /// Get a Vec<f32> buffer with at least the specified capacity
    /// Returns a reused buffer if available, otherwise creates a new one
    pub fn get_f32_buffer(&self, min_capacity: usize) -> Vec<f32> {
        if let Ok(mut pool) = self.f32_buffers.lock() {
            // Look for a buffer with sufficient capacity
            if let Some(mut buffer) = pool.pop_back() {
                if buffer.capacity() >= min_capacity {
                    buffer.clear();
                    return buffer;
                }
                // Buffer too small, put it back and create a new one
                if pool.len() < self.max_pool_size {
                    pool.push_back(buffer);
                }
            }
        }
        
        // Create new buffer with some extra capacity to reduce future reallocations
        let capacity = (min_capacity * 5 / 4).max(1024); // 25% extra capacity, min 1024
        Vec::with_capacity(capacity)
    }

    /// Return a Vec<f32> buffer to the pool for reuse
    #[allow(dead_code)]
    pub fn return_f32_buffer(&self, buffer: Vec<f32>) {
        if let Ok(mut pool) = self.f32_buffers.lock() {
            if pool.len() < self.max_pool_size && buffer.capacity() > 0 {
                pool.push_back(buffer);
            }
            // If pool is full or buffer has no capacity, just drop it
        }
    }

    /// Get a Vec<u8> buffer with at least the specified capacity
    #[allow(dead_code)]
    pub fn get_u8_buffer(&self, min_capacity: usize) -> Vec<u8> {
        if let Ok(mut pool) = self.u8_buffers.lock() {
            if let Some(mut buffer) = pool.pop_back() {
                if buffer.capacity() >= min_capacity {
                    buffer.clear();
                    return buffer;
                }
                // Buffer too small, put it back and create a new one
                if pool.len() < self.max_pool_size {
                    pool.push_back(buffer);
                }
            }
        }
        
        let capacity = (min_capacity * 5 / 4).max(1024); // 25% extra capacity, min 1024
        Vec::with_capacity(capacity)
    }

    /// Return a Vec<u8> buffer to the pool for reuse
    #[allow(dead_code)]
    pub fn return_u8_buffer(&self, buffer: Vec<u8>) {
        if let Ok(mut pool) = self.u8_buffers.lock() {
            if pool.len() < self.max_pool_size && buffer.capacity() > 0 {
                pool.push_back(buffer);
            }
        }
    }

    /// Get pool statistics for debugging/monitoring
    #[allow(dead_code)]
    pub fn stats(&self) -> BufferPoolStats {
        let f32_count = self.f32_buffers.lock().map(|pool| pool.len()).unwrap_or(0);
        let u8_count = self.u8_buffers.lock().map(|pool| pool.len()).unwrap_or(0);
        
        BufferPoolStats {
            f32_buffers_available: f32_count,
            u8_buffers_available: u8_count,
            max_pool_size: self.max_pool_size,
        }
    }

    /// Clear all buffers from the pool (useful for memory cleanup)
    #[allow(dead_code)]
    pub fn clear(&self) {
        if let Ok(mut pool) = self.f32_buffers.lock() {
            pool.clear();
        }
        if let Ok(mut pool) = self.u8_buffers.lock() {
            pool.clear();
        }
    }
}

impl Default for BufferPool {
    fn default() -> Self {
        Self::new(16) // Default pool size of 16 buffers per type
    }
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct BufferPoolStats {
    pub f32_buffers_available: usize,
    pub u8_buffers_available: usize,
    pub max_pool_size: usize,
}

/// RAII wrapper for automatic buffer return to pool
#[allow(dead_code)]
pub struct PooledF32Buffer {
    buffer: Option<Vec<f32>>,
    pool: Arc<BufferPool>,
}

#[allow(dead_code)]
impl PooledF32Buffer {
    pub fn new(pool: Arc<BufferPool>, min_capacity: usize) -> Self {
        let buffer = pool.get_f32_buffer(min_capacity);
        Self {
            buffer: Some(buffer),
            pool,
        }
    }

    pub fn as_mut(&mut self) -> &mut Vec<f32> {
        self.buffer.as_mut().expect("Buffer already taken")
    }

    pub fn as_ref(&self) -> &Vec<f32> {
        self.buffer.as_ref().expect("Buffer already taken")
    }

    /// Take the buffer out of the wrapper (buffer won't be returned to pool automatically)
    pub fn take(mut self) -> Vec<f32> {
        self.buffer.take().expect("Buffer already taken")
    }
}

impl Drop for PooledF32Buffer {
    fn drop(&mut self) {
        if let Some(buffer) = self.buffer.take() {
            self.pool.return_f32_buffer(buffer);
        }
    }
}

/// RAII wrapper for automatic buffer return to pool
#[allow(dead_code)]
pub struct PooledU8Buffer {
    buffer: Option<Vec<u8>>,
    pool: Arc<BufferPool>,
}

#[allow(dead_code)]
impl PooledU8Buffer {
    pub fn new(pool: Arc<BufferPool>, min_capacity: usize) -> Self {
        let buffer = pool.get_u8_buffer(min_capacity);
        Self {
            buffer: Some(buffer),
            pool,
        }
    }

    pub fn as_mut(&mut self) -> &mut Vec<u8> {
        self.buffer.as_mut().expect("Buffer already taken")
    }

    pub fn as_ref(&self) -> &Vec<u8> {
        self.buffer.as_ref().expect("Buffer already taken")
    }

    pub fn take(mut self) -> Vec<u8> {
        self.buffer.take().expect("Buffer already taken")
    }
}

impl Drop for PooledU8Buffer {
    fn drop(&mut self) {
        if let Some(buffer) = self.buffer.take() {
            self.pool.return_u8_buffer(buffer);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_buffer_pool_reuse() {
        let pool = Arc::new(BufferPool::new(4));
        
        // Get and return a buffer
        {
            let buffer = pool.get_f32_buffer(100);
            assert!(buffer.capacity() >= 100);
            pool.return_f32_buffer(buffer);
        }
        
        // Get another buffer - should reuse the previous one
        let buffer2 = pool.get_f32_buffer(50);
        assert!(buffer2.capacity() >= 100); // Should have the capacity from previous buffer
    }

    #[test] 
    fn test_pooled_buffer_raii() {
        let pool = Arc::new(BufferPool::new(4));
        
        {
            let mut pooled = PooledF32Buffer::new(pool.clone(), 100);
            pooled.as_mut().push(1.0);
            pooled.as_mut().push(2.0);
            assert_eq!(pooled.as_ref().len(), 2);
        } // Buffer automatically returned here
        
        // Verify buffer was returned by checking pool stats
        let stats = pool.stats();
        assert_eq!(stats.f32_buffers_available, 1);
    }
}