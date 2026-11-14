//! The text of SVG images is shaped by HarfBuzz, the library that shapes the text of the
//! document, with the fonts of the document. (The SVG parser does not shape text itself.)

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use harfbuzz_rs::{Blob, Direction, Face, Feature, Font, Owned, Shared, Tag, UnicodeBuffer, shape};
use usvg::{ShapedGlyph, TextShaper};

#[derive(Default)]
pub struct HarfbuzzShaper {
    fonts: Mutex<HashMap<usvg::fontdb::ID, Shared<Font<'static>>>>,
}

impl std::fmt::Debug for HarfbuzzShaper {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HarfbuzzShaper").finish()
    }
}

impl TextShaper for HarfbuzzShaper {
    fn shape(
        &self,
        id: usvg::fontdb::ID,
        data: &[u8],
        index: u32,
        text: &str,
        rtl: bool,
        small_caps: bool,
        kerning: bool,
    ) -> Option<Vec<ShapedGlyph>> {
        let font = {
            let mut fonts = self.fonts.lock().ok()?;
            fonts
                .entry(id)
                .or_insert_with(|| {
                    let blob = Blob::with_bytes_owned(Arc::new(data.to_vec()), |bytes| &**bytes);
                    let font: Owned<Font<'static>> = Font::new(Face::new(blob, index));
                    font.into()
                })
                .clone()
        };

        let direction = if rtl { Direction::Rtl } else { Direction::Ltr };
        let buffer = UnicodeBuffer::new()
            .add_str(text)
            .set_direction(direction)
            .guess_segment_properties();
        let mut features = Vec::new();
        if small_caps {
            features.push(Feature::new(Tag::new('s', 'm', 'c', 'p'), 1, ..));
        }
        if !kerning {
            features.push(Feature::new(Tag::new('k', 'e', 'r', 'n'), 0, ..));
        }
        let output = shape(&font, buffer, &features);
        Some(
            output
                .get_glyph_infos()
                .iter()
                .zip(output.get_glyph_positions())
                .map(|(info, position)| ShapedGlyph {
                    cluster: info.cluster as usize,
                    glyph_id: info.codepoint,
                    x_offset: position.x_offset,
                    y_offset: position.y_offset,
                    x_advance: position.x_advance,
                })
                .collect(),
        )
    }
}
