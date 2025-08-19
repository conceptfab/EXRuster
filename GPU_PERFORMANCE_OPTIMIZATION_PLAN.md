# GPU Performance Optimization Plan - EXRuster

## Problem Analysis

Current performance results show GPU acceleration is **3x slower** than CPU processing:
- CPU processing: **4,820 ms** for 20 files
- GPU processing: **14,737 ms** for 20 files  
- Hardware: RTX 4070 (5,888 CUDA cores)

## Root Causes Identified

### 1. **Synchronization Overhead** 🚨 HIGH PRIORITY
**Location:** `gpu_processing.rs:122`, `image_cache.rs:755`
- 5-second timeout for each GPU operation
- Blocking CPU-GPU synchronization after every operation
- Excessive use of `recv_timeout` calls

### 2. **Fake GPU Implementation** 🚨 HIGH PRIORITY  
**Location:** `image_cache.rs:100-115`
- "GPU MIP generation" is actually CPU processing with GPU logging
- Missing true GPU compute shader implementation for MIP chains
- Fallback masks real GPU performance issues

### 3. **Suboptimal Workgroup Configuration** 🔶 MEDIUM PRIORITY
**Location:** `shaders/image_processing.wgsl:222`, `gpu_processing.rs:98`
- Current: 8x8 = 64 threads per workgroup
- RTX 4070 optimal: 16x16 = 256+ threads per workgroup
- Poor GPU occupancy and underutilization

### 4. **Memory Transfer Inefficiency** 🔶 MEDIUM PRIORITY
**Location:** `image_cache.rs:774-780`
- Multiple memory copies: GPU → staging → CPU buffer → Vec<u8>
- Chunk-by-chunk copying instead of bulk transfer
- Unnecessary intermediate allocations

### 5. **Buffer Management Overhead** 🔸 LOW PRIORITY
**Location:** `gpu_context.rs:445-470`
- Mutex locking for every buffer operation
- Frequent buffer pool allocation/deallocation
- No batch buffer operations

### 6. **Premature CPU Fallback** 🔶 MEDIUM PRIORITY
**Location:** `image_cache.rs:92-96`
- Automatic fallback to CPU hides GPU issues
- No diagnostic information about GPU failures
- Missing performance metrics comparison

## Optimization Plan

### Phase 1: Quick Wins (1-2 days)

#### 1.1 Optimize Workgroup Size
**Files:** `shaders/image_processing.wgsl`, `gpu_processing.rs`
```wgsl
// Change from:
@compute @workgroup_size(8, 8, 1)
// To:
@compute @workgroup_size(16, 16, 1)  // 256 threads
```

#### 1.2 Remove Synchronous Timeouts  
**Files:** `gpu_processing.rs`, `image_cache.rs`
- Replace `recv_timeout(5s)` with async polling
- Use `device.poll(Maintain::Wait)` for non-blocking sync
- Add timeout only as last resort (30s+)

#### 1.3 Optimize Memory Transfers
**File:** `image_cache.rs:774-780`
```rust
// Replace chunk-by-chunk copying with:
let data = slice.get_mapped_range();
let out_bytes = data.to_vec(); // Single allocation
```

### Phase 2: Core Improvements (3-5 days)

#### 2.1 Implement True GPU MIP Generation
**Files:** `image_cache.rs`, `shaders/mip_generation.wgsl`
- Remove fake GPU MIP implementation
- Create proper compute shader for MIP chain generation
- Implement GPU-native downsampling algorithms

#### 2.2 Add Performance Diagnostics
**Files:** `gpu_metrics.rs`, `gpu_context.rs`
- Measure actual GPU vs CPU processing times
- Add GPU occupancy metrics
- Log memory transfer bottlenecks
- Create performance comparison reports

#### 2.3 Optimize Buffer Management
**File:** `gpu_context.rs`
- Implement persistent buffer pools
- Reduce mutex contention with lockless structures
- Batch buffer operations where possible

### Phase 3: Advanced Optimizations (1-2 weeks)

#### 3.1 Asynchronous GPU Pipeline
**Files:** `gpu_processing.rs`, `image_cache.rs`
- Pipeline multiple operations simultaneously  
- Overlap compute and memory transfers
- Implement work queues for batch processing

#### 3.2 Memory Management Overhaul
- Use GPU-persistent memory pools
- Implement zero-copy buffer sharing where possible
- Optimize for RTX 4070's memory hierarchy

#### 3.3 Specialized GPU Kernels
**Files:** `shaders/`
- Optimize tone mapping shaders for NVIDIA architecture
- Implement SIMD-style operations in WGSL
- Add specialized kernels for different image sizes

### Phase 4: Validation & Tuning (2-3 days)

#### 4.1 Performance Testing
- Benchmark against original CPU times
- Test with various image sizes and batch sizes
- Validate on different GPU architectures

#### 4.2 Adaptive GPU/CPU Selection
**File:** `gpu_scheduler.rs`
- Improve decision algorithms based on real metrics
- Add image-size-based GPU/CPU selection
- Implement dynamic load balancing

## Expected Performance Improvements

### Conservative Estimates:
- **Phase 1:** 2-3x GPU speedup (GPU faster than current CPU)
- **Phase 2:** 4-6x GPU speedup  
- **Phase 3:** 8-12x GPU speedup
- **Phase 4:** 10-15x GPU speedup with optimal selection

### Target Performance:
- **20 files processing time:** ~500-1000ms (vs current 4,820ms CPU)
- **GPU utilization:** >70% (vs current ~5-10%)
- **Memory efficiency:** >90% (reduce copies and allocations)

## Implementation Priority

1. **🚨 Phase 1.2** - Remove synchronous timeouts (biggest impact)
2. **🚨 Phase 1.1** - Optimize workgroup size (easy win)  
3. **🚨 Phase 2.1** - Implement real GPU MIP generation
4. **🔶 Phase 1.3** - Optimize memory transfers
5. **🔶 Phase 2.2** - Add performance diagnostics
6. **🔸 Phases 3-4** - Advanced optimizations

## Success Metrics

- [ ] GPU processing faster than CPU processing
- [ ] <1000ms total time for 20 files
- [ ] >70% GPU utilization during processing  
- [ ] <2 memory copies per operation
- [ ] Reliable GPU/CPU performance prediction

---

**Hardware Target:** RTX 4070 (5,888 CUDA cores, 12GB GDDR6X)
**Framework:** wgpu + WGSL compute shaders
**Primary Bottleneck:** CPU-GPU synchronization overhead