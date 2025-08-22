/// Ultra-fast EXR metadata parser optimized for channel and layer analysis
/// Based on the optimized implementation from read/fast_exr.rs
/// Only parses metadata, bypassing pixel data for maximum performance

use std::collections::HashMap;
use std::io::Read;
use std::path::Path;
use memmap2::MmapOptions;

#[derive(Debug, Clone)]
pub struct FastEXRMetadata {
    pub channels: Vec<ChannelInfo>,
    pub display_window: (i32, i32, i32, i32), // x_min, y_min, x_max, y_max
    pub pixel_aspect: f32,
    pub compression: String,
    pub line_order: String,
    pub layer_name: Option<String>,
    pub custom_attributes: HashMap<String, String>,
}

#[derive(Debug, Clone)]
pub struct ChannelInfo {
    pub name: String,
    pub sample_type: SampleType,
    pub sampling: (i32, i32), // x_sampling, y_sampling
    pub quantize_linearly: bool,
}

#[derive(Debug, Clone)]
pub enum SampleType {
    UInt,
    Half,
    Float,
}

impl SampleType {
    fn from_u32(value: u32) -> Result<Self, String> {
        match value {
            0 => Ok(SampleType::UInt),
            1 => Ok(SampleType::Half), 
            2 => Ok(SampleType::Float),
            _ => Err(format!("Unknown sample type: {}", value)),
        }
    }
}

/// Ultra-fast EXR parser optimized for metadata-only reading
pub struct FastEXRParser {
    data: Vec<u8>,
    position: usize,
}

impl FastEXRParser {
    pub fn from_file(path: &Path) -> anyhow::Result<Self> {
        let file = std::fs::File::open(path)?;
        let file_size = file.metadata()?.len() as usize;
        
        // For very large files, only read the header portion (first ~64KB should be enough)
        let read_size = std::cmp::min(file_size, 65536);
        
        if file_size > 1024 * 1024 { // >1MB files use memory mapping for header
            let mmap = unsafe { MmapOptions::new().len(read_size).map(&file)? };
            Ok(FastEXRParser {
                data: mmap[..read_size].to_vec(),
                position: 0,
            })
        } else {
            // Small files - read directly
            let mut data = vec![0u8; read_size];
            let mut file = file;
            file.read_exact(&mut data)?;
            Ok(FastEXRParser { data, position: 0 })
        }
    }
    
    pub fn parse_metadata(&mut self) -> Result<FastEXRMetadata, Box<dyn std::error::Error + Send + Sync>> {
        // Check magic number
        let magic = self.read_u32()?;
        if magic != 20000630 {
            return Err("Invalid EXR magic number".into());
        }
        
        // Read version field
        let version = self.read_u32()?;
        let _file_version = version & 0xFF;
        let _is_tiled = (version & 0x200) != 0;
        let _is_long_names = (version & 0x400) != 0;
        let _is_multipart = (version & 0x1000) != 0;
        
        let mut metadata = FastEXRMetadata {
            channels: Vec::new(),
            display_window: (0, 0, 0, 0),
            pixel_aspect: 1.0,
            compression: "Unknown".to_string(),
            line_order: "Increasing".to_string(),
            layer_name: None,
            custom_attributes: HashMap::new(),
        };
        
        // Parse header attributes until we hit the null terminator
        while self.position < self.data.len() {
            let attr_name = self.read_null_terminated_string()?;
            if attr_name.is_empty() {
                break; // End of header
            }
            
            let _attr_type = self.read_null_terminated_string()?;
            let attr_size = self.read_u32()? as usize;
            
            match attr_name.as_str() {
                "channels" => {
                    metadata.channels = self.parse_channels(attr_size)?;
                },
                "displayWindow" => {
                    if attr_size >= 16 {
                        metadata.display_window = (
                            self.read_i32()?,
                            self.read_i32()?,
                            self.read_i32()?,
                            self.read_i32()?,
                        );
                    } else {
                        self.skip(attr_size)?;
                    }
                },
                "pixelAspectRatio" => {
                    if attr_size >= 4 {
                        metadata.pixel_aspect = self.read_f32()?;
                    } else {
                        self.skip(attr_size)?;
                    }
                },
                "compression" => {
                    metadata.compression = self.read_compression(attr_size)?;
                },
                "lineOrder" => {
                    metadata.line_order = self.read_line_order(attr_size)?;
                },
                "name" => {
                    if attr_size > 0 {
                        metadata.layer_name = Some(self.read_fixed_string(attr_size)?);
                    } else {
                        self.skip(attr_size)?;
                    }
                },
                _ => {
                    // Skip binary attributes that can't be displayed as text
                    if attr_size > 0 && attr_size <= 64 { // Only small, likely text attributes
                        // Try to read as string, but validate it's printable ASCII
                        let start_pos = self.position;
                        if let Ok(value) = self.read_fixed_string(attr_size) {
                            // Only store if it's printable ASCII or valid UTF-8
                            if value.chars().all(|c| c.is_ascii_graphic() || c.is_ascii_whitespace()) {
                                metadata.custom_attributes.insert(attr_name, value);
                            }
                        } else {
                            self.position = start_pos;
                            self.skip(attr_size)?;
                        }
                    } else {
                        self.skip(attr_size)?;
                    }
                }
            }
        }
        
        Ok(metadata)
    }
    
    fn parse_channels(&mut self, size: usize) -> Result<Vec<ChannelInfo>, Box<dyn std::error::Error + Send + Sync>> {
        let start_pos = self.position;
        let mut channels = Vec::new();
        
        while self.position < start_pos + size {
            let name = self.read_null_terminated_string()?;
            if name.is_empty() {
                break;
            }
            
            let pixel_type = self.read_u32()?;
            let p_linear = self.read_u8()?;
            self.skip(3)?; // Reserved bytes
            let x_sampling = self.read_i32()?;
            let y_sampling = self.read_i32()?;
            
            channels.push(ChannelInfo {
                name,
                sample_type: SampleType::from_u32(pixel_type).map_err(|e| anyhow::anyhow!(e))?,
                sampling: (x_sampling, y_sampling),
                quantize_linearly: p_linear != 0,
            });
        }
        
        Ok(channels)
    }
    
    fn read_compression(&mut self, size: usize) -> anyhow::Result<String> {
        if size >= 1 {
            let comp = self.read_u8()?;
            self.skip(size - 1)?;
            Ok(match comp {
                0 => "None".to_string(),
                1 => "RLE".to_string(),
                2 => "ZIPS".to_string(),
                3 => "ZIP".to_string(),
                4 => "PIZ".to_string(),
                5 => "PXR24".to_string(),
                6 => "B44".to_string(),
                7 => "B44A".to_string(),
                8 => "DWAA".to_string(),
                9 => "DWAB".to_string(),
                _ => format!("Unknown({})", comp),
            })
        } else {
            Ok("Unknown".to_string())
        }
    }
    
    fn read_line_order(&mut self, size: usize) -> anyhow::Result<String> {
        if size >= 1 {
            let order = self.read_u8()?;
            self.skip(size - 1)?;
            Ok(match order {
                0 => "Increasing".to_string(),
                1 => "Decreasing".to_string(),
                2 => "Random".to_string(),
                _ => format!("Unknown({})", order),
            })
        } else {
            Ok("Increasing".to_string())
        }
    }
    
    // Low-level reading functions
    fn read_u8(&mut self) -> anyhow::Result<u8> {
        if self.position >= self.data.len() {
            return Err(anyhow::anyhow!("Unexpected end of data"));
        }
        let value = self.data[self.position];
        self.position += 1;
        Ok(value)
    }
    
    fn read_u32(&mut self) -> anyhow::Result<u32> {
        if self.position + 4 > self.data.len() {
            return Err(anyhow::anyhow!("Unexpected end of data"));
        }
        let value = u32::from_le_bytes([
            self.data[self.position],
            self.data[self.position + 1],
            self.data[self.position + 2],
            self.data[self.position + 3],
        ]);
        self.position += 4;
        Ok(value)
    }
    
    fn read_i32(&mut self) -> anyhow::Result<i32> {
        Ok(self.read_u32()? as i32)
    }
    
    fn read_f32(&mut self) -> anyhow::Result<f32> {
        Ok(f32::from_bits(self.read_u32()?))
    }
    
    fn read_null_terminated_string(&mut self) -> anyhow::Result<String> {
        let mut result = Vec::new();
        loop {
            if self.position >= self.data.len() {
                break;
            }
            let byte = self.data[self.position];
            self.position += 1;
            if byte == 0 {
                break;
            }
            result.push(byte);
        }
        Ok(String::from_utf8_lossy(&result).to_string())
    }
    
    fn read_fixed_string(&mut self, size: usize) -> anyhow::Result<String> {
        if self.position + size > self.data.len() {
            return Err(anyhow::anyhow!("Unexpected end of data"));
        }
        let result = String::from_utf8_lossy(&self.data[self.position..self.position + size]).to_string();
        self.position += size;
        Ok(result)
    }
    
    fn skip(&mut self, count: usize) -> anyhow::Result<()> {
        if self.position + count > self.data.len() {
            return Err(anyhow::anyhow!("Unexpected end of data"));
        }
        self.position += count;
        Ok(())
    }
}

/// Ultra-fast metadata reader function
pub fn read_exr_metadata_ultra_fast(path: &Path) -> anyhow::Result<FastEXRMetadata> {
    let mut parser = FastEXRParser::from_file(path)?;
    parser.parse_metadata().map_err(|e| anyhow::anyhow!("Failed to parse EXR metadata: {}", e))
}

/// Convert FastEXRMetadata to the existing ExrMetadata format for compatibility
impl From<FastEXRMetadata> for crate::io::exr_metadata::ExrMetadata {
    fn from(fast_meta: FastEXRMetadata) -> Self {
        use crate::io::exr_metadata::{ExrMetadata, MetadataGroup, LayerMetadata};
        
        // Create basic metadata groups
        let mut general_items = Vec::new();
        general_items.push(("Display Window".into(), format!("{:?}", fast_meta.display_window)));
        general_items.push(("Pixel Aspect Ratio".into(), format!("{:.3}", fast_meta.pixel_aspect)));
        general_items.push(("Compression".into(), fast_meta.compression.clone()));
        general_items.push(("Line Order".into(), fast_meta.line_order.clone()));
        if let Some(ref layer_name) = fast_meta.layer_name {
            general_items.push(("Layer Name".into(), layer_name.clone()));
        }
        
        let mut groups = vec![
            MetadataGroup {
                name: "Ogólne".into(),
                items: general_items,
            }
        ];
        
        // Add custom attributes as a separate group if any exist
        if !fast_meta.custom_attributes.is_empty() {
            let custom_items: Vec<(String, String)> = fast_meta.custom_attributes
                .into_iter()
                .collect();
            groups.push(MetadataGroup {
                name: "Niestandardowe atrybuty".into(),
                items: custom_items,
            });
        }
        
        // Create layer metadata
        let layer_name = fast_meta.layer_name.unwrap_or_else(|| "".to_string());
        let layers = vec![LayerMetadata {
            name: layer_name,
            width: 1920, // Default values - could be extracted from display_window
            height: 1080,
            attributes: Vec::new(),
        }];
        
        ExrMetadata { groups, layers }
    }
}