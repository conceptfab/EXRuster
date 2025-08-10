use std::io::Write;

use crate::exr_layers::PsdLayer;

pub fn write_psd<W: Write>(_out: W, _layers: &[PsdLayer], _composite: &PsdLayer) -> anyhow::Result<()> {
    // Placeholder: implementacja zapisu PSD pojawi się w kolejnych krokach
    anyhow::bail!("psd_writer::write_psd() not implemented yet")
}


