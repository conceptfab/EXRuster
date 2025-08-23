use std::sync::Mutex;
use std::collections::BTreeMap;

/// Buffer pool for reusing Vec<f32> allocations
/// Eliminates frequent allocations in hot image processing paths
/// Uses BTreeMap for O(log n) buffer selection by size
pub struct BufferPool {
    f32_buffers_by_size: Mutex<BTreeMap<usize, Vec<Vec<f32>>>>,
}

impl BufferPool {
    /// Create a new buffer pool
    pub fn new(_max_pool_size: usize) -> Self {
        Self {
            f32_buffers_by_size: Mutex::new(BTreeMap::new()),
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
        let pool = BufferPool::new(4);
        
        // Get a buffer
        let buffer = pool.get_f32_buffer(100);
        assert!(buffer.capacity() >= 100);
        assert_eq!(buffer.len(), 0);
    }

    #[test]
    fn test_buffer_pool_capacity() {
        let pool = BufferPool::new(4);
        
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
}