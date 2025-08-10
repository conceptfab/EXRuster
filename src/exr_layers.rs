use std::path::Path;

pub struct PsdLayer {
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub r: Vec<u16>,
    pub g: Vec<u16>,
    pub b: Vec<u16>,
    pub a: Option<Vec<u16>>,
}

pub fn read_layers(_path: &Path) -> anyhow::Result<(Vec<PsdLayer>, PsdLayer)> {
    // Placeholder minimalny: zwróć błąd dopóki nie zaimplementujemy mapowania EXR → warstwy
    anyhow::bail!("exr_layers::read_layers() not implemented yet")
}


