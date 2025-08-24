# Export System Documentation

## Overview

EXRuster's export system allows users to export different layer groups from EXR files with configurable format and processing options. The system is built around grouped layer export based on channel classification.

## Export UI Controls

### Format Options
- **32 Bit Checkbox**: 
  - `false` → PNG 16-bit format
  - `true` → TIFF 32-bit format

### Processing Options  
- **Apply Corrections Checkbox**:
  - `false` → Export raw layer data
  - `true` → Apply user corrections (Exposure, Gamma, Tonemap)

### Export Buttons

| Button | Callback | Layer Group | Description |
|--------|----------|-------------|-------------|
| Export Beauty | `export-beauty()` | base | Exports main beauty/RGB channels |
| Export All | `export-all()` | all | Exports all available layers |  
| Export Scene | `export-scene()` | scene | Exports scene elements (Background, Translucency, etc.) |
| Export Objects | `export-objects()` | scene_objects | Exports object layers (ID*, leather, scratch, _*) |
| Export Cryptomatte | `export-cryptomatte()` | cryptomatte | Exports Cryptomatte channels |
| Export Lights | `export-lights()` | light | Exports lighting layers (Sky, Sun, LightMix, Light*) |

## Layer Groups (from channel_groups.json)

### Base Group
- **Channels**: Beauty
- **Type**: Basic RGB layers
- **Purpose**: Main rendered image

### Scene Group  
- **Channels**: Background, Translucency, Translucency0, VirtualBeauty, ZDepth
- **Type**: Scene elements
- **Purpose**: Background and scene composition layers

### Scene Objects Group
- **Channels**: ID*, leather, scratch, _*
- **Type**: Object identification and materials
- **Purpose**: Individual object and material layers

### Cryptomatte Group
- **Channels**: Cryptomatte, Cryptomatte0  
- **Type**: Object/material ID passes
- **Purpose**: Advanced compositing and selection

### Light Group
- **Channels**: Sky, Sun, LightMix, Light*
- **Type**: Lighting passes
- **Purpose**: Light contribution layers

### Technical Group
- **Channels**: RenderStamp, RenderStamp0
- **Type**: Technical/metadata
- **Purpose**: Render information (not exported via buttons)

## Export Process Flow

1. **User clicks export button**
2. **System determines target layer group**
3. **Reads checkbox states**:
   - Format selection (PNG 16-bit vs TIFF 32-bit)
   - Processing options (Apply Corrections)
4. **Collects matching layers** from EXR based on group classification
5. **Applies corrections if enabled**:
   - Exposure adjustment
   - Gamma correction  
   - Tone mapping (Linear, ACES, Reinhard, Filmic, Hable, Local)
6. **Exports each layer** in selected format
7. **Reports progress and results** to console

## Implementation Files

- **UI Definition**: `ui/appwindow.slint` - Export section UI
- **Export Logic**: `src/ui/export_handlers.rs` - Core export functions  
- **UI Integration**: `src/ui/setup.rs` - Callback connections
- **Layer Processing**: `src/processing/layer_export.rs` - Format handling
- **Channel Classification**: `src/processing/channel_classification.rs` - Group determination

## File Naming Convention

Exported files follow the pattern:
```
{base_filename}_{layer_name}.{extension}
```

Where:
- `base_filename` - Original EXR filename without extension
- `layer_name` - Individual layer/channel name
- `extension` - `png` or `tiff` based on 32-bit checkbox

## Error Handling

- **Missing layers**: Logged to console, export continues for available layers
- **Format errors**: Specific error messages for each failed layer
- **File system errors**: Directory creation and permission issues handled
- **Processing errors**: Tone mapping and correction failures reported

## Future Enhancements

- Batch export across multiple EXR files
- Custom output directory selection
- Layer filtering within groups  
- Additional export formats (OpenEXR, JPEG)
- Metadata preservation options