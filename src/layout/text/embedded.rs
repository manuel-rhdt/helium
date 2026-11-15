//! The fonts built into Helium: deflated by `build.rs` and inflated when first used.
//!
//! Registering a font in the font database does not need its data, only its metadata, which
//! `build.rs` reads once and stores next to the compressed font (`EmbeddedFaceInfo`). The data
//! of a font is inflated the first time something asks for it, so a document pays only for the
//! fonts it uses.

use flate2::read::DeflateDecoder;
use std::io::Read;
use std::sync::{Arc, OnceLock};

/// The metadata that `fontdb` reads from a font, determined at build time.
pub(crate) struct EmbeddedFaceInfo {
    pub families: &'static [(&'static str, fontdb::Language)],
    pub post_script_name: &'static str,
    pub style: fontdb::Style,
    pub weight: u16,
    pub stretch: fontdb::Stretch,
    pub monospaced: bool,
}

/// A font, deflated if that pays, and its metadata.
pub(crate) struct EmbeddedFont {
    bytes: &'static [u8],
    /// Whether `bytes` is deflated; fonts that hardly shrink are stored as they are.
    deflated: bool,
    raw_len: usize,
    info: EmbeddedFaceInfo,
    data: OnceLock<Arc<Vec<u8>>>,
}

impl EmbeddedFont {
    pub(crate) const fn new(bytes: &'static [u8], raw_len: usize, deflated: bool, info: EmbeddedFaceInfo) -> Self {
        EmbeddedFont { bytes, deflated, raw_len, info, data: OnceLock::new() }
    }

    /// The font file, inflated on the first call.
    pub(crate) fn data(&self) -> &Arc<Vec<u8>> {
        self.data.get_or_init(|| {
            if !self.deflated {
                return Arc::new(self.bytes.to_vec());
            }
            let mut data = Vec::with_capacity(self.raw_len);
            DeflateDecoder::new(self.bytes)
                .read_to_end(&mut data)
                .expect("the embedded fonts are valid deflate streams");
            Arc::new(data)
        })
    }

    /// Whether the font has been inflated.
    #[cfg(test)]
    pub(crate) fn is_inflated(&self) -> bool {
        self.data.get().is_some()
    }

    pub(crate) fn info(&self) -> &EmbeddedFaceInfo {
        &self.info
    }

    /// The entry for the font database, whose data is inflated only when it is first read.
    pub(crate) fn face_info(&'static self) -> fontdb::FaceInfo {
        fontdb::FaceInfo {
            id: fontdb::ID::dummy(),
            source: fontdb::Source::Binary(Arc::new(LazySource(self))),
            index: 0,
            families: self.info.families.iter().map(|&(name, language)| (name.to_owned(), language)).collect(),
            post_script_name: self.info.post_script_name.to_owned(),
            style: self.info.style,
            weight: fontdb::Weight(self.info.weight),
            stretch: self.info.stretch,
            monospaced: self.info.monospaced,
        }
    }
}

/// The data source of a font in the font database.
struct LazySource(&'static EmbeddedFont);

impl AsRef<[u8]> for LazySource {
    fn as_ref(&self) -> &[u8] {
        self.0.data().as_slice()
    }
}
