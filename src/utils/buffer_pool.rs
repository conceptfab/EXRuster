use std::sync::Mutex;
use std::collections::BTreeMap;

/// Buffer pool for reusing Vec<f32> and Vec<u8> allocations
/// Eliminates frequent allocations in hot image processing paths
/// Uses BTreeMap for O(log n) buffer selection by size
pub struct BufferPool {
    f32_buffers_by_size: Mutex<BTreeMap<usize, Vec<Vec<f32>>>>,
    #[allow(dead_code)]
    u8_buffers_by_size: Mutex<BTreeMap<usize, Vec<Vec<u8>>>>,
    max_pool_size: usize,
}

impl BufferPool {
    /// Create a new buffer pool with specified maximum pool size
    pub fn new(max_pool_size: usize) -> Self {
        Self {
            f32_buffers_by_size: Mutex::new(BTreeMap::new()),
            u8_buffers_by_size: Mutex::new(BTreeMap::new()),
            max_pool_size,
        }
    }

    /// Get a Vec<f32> buffer with at least the specified capacity
    /// Returns a reused buffer if available, otherwise creates a new one
    /// Uses O(log n) search to find the smallest suitable buffer
    pub fn get_f32_buffer(&self, min_capacity: usize) -> Vec<f32> {
        if let Ok(mut pool) = self.f32_buffers_by_size.lock() {
            // Find the smallest buffer with sufficient capacity in O(log n)
            if let Some((_, buffers)) = pool.range_mut(min_capacity..).next() {
                if let Some(mut buffer) = buffers.pop() {
                    buffer.clear();
                    // Remove empty size entry to keep BTreeMap clean
                    if buffers.is_empty() {
                        let capacity = buffer.capacity();
                        pool.remove(&capacity);
                    }
                    return buffer;
                }
            }
        }
        
        // Create new buffer with some extra capacity to reduce future reallocations
        let capacity = (min_capacity * 5 / 4).max(1024); // 25% extra capacity, min 1024
        Vec::with_capacity(capacity)
    }

    /// Return a Vec<f32> buffer to the pool for reuse
    /// Organizes buffers by capacity for efficient retrieval
    #[allow(dead_code)]
    pub fn return_f32_buffer(&self, buffer: Vec<f32>) {
        if buffer.capacity() == 0 {
            return; // Don't store empty buffers
        }
        
        if let Ok(mut pool) = self.f32_buffers_by_size.lock() {
            // Count total buffers across all sizes
            let total_buffers: usize = pool.values().map(|v| v.len()).sum();
            
            if total_buffers < self.max_pool_size {
                let capacity = buffer.capacity();
                pool.entry(capacity).or_insert_with(Vec::new).push(buffer);
            }
            // If pool is full, just drop the buffer
        }
    }

    /// Get a Vec<u8> buffer with at least the specified capacity
    /// Uses O(log n) search to find the smallest suitable buffer
    #[allow(dead_code)]
    pub fn get_u8_buffer(&self, min_capacity: usize) -> Vec<u8> {
        if let Ok(mut pool) = self.u8_buffers_by_size.lock() {
            // Find the smallest buffer with sufficient capacity in O(log n)
            if let Some((_, buffers)) = pool.range_mut(min_capacity..).next() {
                if let Some(mut buffer) = buffers.pop() {
                    buffer.clear();
                    // Remove empty size entry to keep BTreeMap clean
                    if buffers.is_empty() {
                        let capacity = buffer.capacity();
                        pool.remove(&capacity);
                    }
                    return buffer;
                }
            }
        }
        
        let capacity = (min_capacity * 5 / 4).max(1024); // 25% extra capacity, min 1024
        Vec::with_capacity(capacity)
    }

    /// Return a Vec<u8> buffer to the pool for reuse
    /// Organizes buffers by capacity for efficient retrieval
    #[allow(dead_code)]
    pub fn return_u8_buffer(&self, buffer: Vec<u8>) {
        if buffer.capacity() == 0 {
            return; // Don't store empty buffers
        }
        
        if let Ok(mut pool) = self.u8_buffers_by_size.lock() {
            // Count total buffers across all sizes
            let total_buffers: usize = pool.values().map(|v| v.len()).sum();
            
            if total_buffers < self.max_pool_size {
                let capacity = buffer.capacity();
                pool.entry(capacity).or_insert_with(Vec::new).push(buffer);
            }
            // If pool is full, just drop the buffer
        }
    }

    /// Get pool statistics for debugging/monitoring
    #[allow(dead_code)]
    pub fn stats(&self) -> BufferPoolStats {
        let f32_count = self.f32_buffers_by_size.lock()
            .map(|pool| pool.values().map(|v| v.len()).sum())
            .unwrap_or(0);
        let u8_count = self.u8_buffers_by_size.lock()
            .map(|pool| pool.values().map(|v| v.len()).sum())
            .unwrap_or(0);
        
        BufferPoolStats {
            f32_buffers_available: f32_count,
            u8_buffers_available: u8_count,
            max_pool_size: self.max_pool_size,
        }
    }

    /// Clear all buffers from the pool (useful for memory cleanup)
    #[allow(dead_code)]
    pub fn clear(&self) {
        if let Ok(mut pool) = self.f32_buffers_by_size.lock() {
            pool.clear();
        }
        if let Ok(mut pool) = self.u8_buffers_by_size.lock() {
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