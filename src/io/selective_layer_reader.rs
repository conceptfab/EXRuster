//! Selective layer reading for EXR files.
//! Uses ReadFirstValidLayer with a name filter to load only the target layer,
//! avoiding decoding of other layers (10-20x less I/O for multi-layer files).

use ::exr::error::Error as ExrError;
use ::exr::image::read::any_channels::ReadAnyChannels;
use ::exr::image::read::image::ReadLayers;
use ::exr::image::read::layers::ReadChannels;
use ::exr::image::read::samples::ReadFlatSamples;
use ::exr::image::{AnyChannels, FlatSamples, Layer};
use ::exr::meta::header::Header;
use exr::prelude as exr;
use std::borrow::Cow;
use std::io::{BufRead, Seek};

/// Wraps ReadChannels and filters by layer name — create_channels_reader returns Ok only for matching layer.
/// Used with first_valid_layer() to read a single layer by name.
#[derive(Clone, Debug)]
struct LayerNameFilter {
    inner: ReadAnyChannels<ReadFlatSamples>,
    target_layer_name: String,
}

impl LayerNameFilter {
    fn new(inner: ReadAnyChannels<ReadFlatSamples>, target_layer_name: String) -> Self {
        Self {
            inner,
            target_layer_name,
        }
    }

    fn layer_matches(&self, header: &Header) -> bool {
        let header_name = header
            .own_attributes
            .layer_name
            .as_ref()
            .map(|t| t.to_string());
        match (self.target_layer_name.as_str(), header_name.as_deref()) {
            ("Beauty", None) | ("Beauty", Some("")) => true,
            (target, Some(h)) => target.eq_ignore_ascii_case(h),
            _ => false,
        }
    }
}

impl<'s> ReadChannels<'s> for LayerNameFilter {
    type Reader = <ReadAnyChannels<ReadFlatSamples> as ReadChannels<'s>>::Reader;

    fn create_channels_reader(&'s self, header: &Header) -> Result<Self::Reader, ExrError> {
        if self.layer_matches(header) {
            self.inner.create_channels_reader(header)
        } else {
            Err(ExrError::Invalid(Cow::Borrowed(
                "Layer name does not match target (filtered out)",
            )))
        }
    }
}

/// Read a single layer by name from an EXR source.
/// Only decodes the target layer's blocks — other layers are skipped.
/// Requires Seek for resolution level selection.
pub fn read_single_layer_by_name<R: BufRead + Seek>(
    source: R,
    layer_name: &str,
) -> Result<Layer<AnyChannels<FlatSamples>>, ExrError> {
    let all_channels = exr::read()
        .no_deep_data()
        .largest_resolution_level()
        .all_channels();

    let filtered = LayerNameFilter::new(all_channels, layer_name.to_string());
    let first_valid = filtered.first_valid_layer();

    let image = first_valid.all_attributes().from_buffered(source)?;
    Ok(image.layer_data)
}
