use bytemuck::{Pod, Zeroable};

/// Konsolidowana struktura parametrów GPU dla image processing
/// Przeniesiona z image_cache.rs aby uniknąć duplikacji
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct ParamsStd140 {
    pub exposure: f32,
    pub gamma: f32,
    pub tonemap_mode: u32,
    pub width: u32,
    pub height: u32,
    pub local_adaptation_radius: u32,
    pub _pad0: u32,
    pub _pad1: [u32; 2],
    pub color_matrix: [[f32; 4]; 3],
    pub has_color_matrix: u32,
    pub _pad2: [u32; 3],
}

impl Default for ParamsStd140 {
    fn default() -> Self {
        Self {
            exposure: 0.0,
            gamma: 2.2,
            tonemap_mode: 0, // ACES
            width: 0,
            height: 0,
            local_adaptation_radius: 16,
            _pad0: 0,
            _pad1: [0; 2],
            color_matrix: [
                [1.0, 0.0, 0.0, 0.0],
                [0.0, 1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
            ],
            has_color_matrix: 0,
            _pad2: [0; 3],
        }
    }
}
