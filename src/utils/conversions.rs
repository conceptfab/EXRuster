#![allow(dead_code)]

/// Generic conversion utilities for EXRuster
/// 
/// This module provides helper functions for common data type conversions
/// used throughout the application, particularly for UI data binding.

/// Generic converter for numeric types that can be cast to i32
pub trait ToI32 {
    fn to_i32(self) -> i32;
}

impl ToI32 for u32 {
    fn to_i32(self) -> i32 {
        self as i32
    }
}

impl ToI32 for f32 {
    fn to_i32(self) -> i32 {
        self as i32
    }
}

impl ToI32 for f64 {
    fn to_i32(self) -> i32 {
        self as i32
    }
}

/// Convert a slice of numeric values to Vec<i32>
pub fn convert_to_i32_vec<T: ToI32 + Copy>(data: &[T]) -> Vec<i32> {
    data.iter().map(|&x| x.to_i32()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_u32_to_i32_conversion() {
        let data = vec![0u32, 100, 255, 1000];
        let converted = convert_to_i32_vec(&data);
        assert_eq!(converted, vec![0, 100, 255, 1000]);
    }

    #[test]
    fn test_f32_conversion() {
        let data = vec![0.5f32, 1.2, 2.8, 100.9];
        let converted = convert_to_i32_vec(&data);
        assert_eq!(converted, vec![0, 1, 2, 100]);
    }
}