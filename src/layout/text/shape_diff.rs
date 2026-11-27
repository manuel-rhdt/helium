//! Differential tests for the port from harfbuzz to harfrust and read-fonts: every bundled font
//! is shaped and measured with both stacks, and the results are compared.
//!
//! The tests report every difference they find (up to a limit) rather than stopping at the first.
//! `KNOWN_DIFFERENCES` lists the ones that are understood and accepted.

use super::font::{BUNDLED_FONTS, Font};
use harfbuzz_rs::{ClusterLevel, UnicodeBuffer, shape as hb_shape};
use harfrust::{Buffer, Direction, Feature, Language, Script, ShaperFont, ShapeOptions};
use read_fonts::model::Font as RfFont;
use read_fonts::TableProvider;
use read_fonts::types::{GlyphId, NameId, Tag};
use std::fmt::Write as _;
use std::sync::Arc;

/// Differences that are understood. A difference is matched by a prefix of its description.
const KNOWN_DIFFERENCES: &[&str] = &[];

struct Case {
    /// ISO 15924 tag of the script of the run, as umbruch passes it.
    script: &'static str,
    lang: Option<&'static str>,
    rtl: bool,
    text: &'static str,
    /// Features as `(tag, value)`, like the author's `font-feature-settings`.
    features: &'static [(&'static [u8; 4], u32)],
}

const fn case(script: &'static str, text: &'static str) -> Case {
    Case { script, lang: None, rtl: false, text, features: &[] }
}

const NO_LIGA: &[(&[u8; 4], u32)] = &[(b"liga", 0), (b"clig", 0)];
const SMALL_CAPS: &[(&[u8; 4], u32)] = &[(b"smcp", 1), (b"onum", 1)];

fn cases() -> Vec<Case> {
    vec![
        case("Latn", "Hello, world! office affluent fi fl ffi AVA To Ty Wa 1234567890"),
        case("Latn", "Åland naïve façade Łódź Đà Lạt ǆ ǅ ß ẞ ﬁ œ æ"),
        case("Latn", "e\u{301} a\u{308} o\u{302}\u{323} n\u{303} z\u{30C}"),
        Case { features: NO_LIGA, ..case("Latn", "office affluent fi fl ffi") },
        Case { features: SMALL_CAPS, ..case("Latn", "Small Caps 0123456789") },
        Case { lang: Some("tr"), ..case("Latn", "İstanbul ıı fi") },
        Case { lang: Some("nl"), ..case("Latn", "IJsselmeer ijs") },
        case("Grek", "Αλφάβητο ήλιος ΐ ΰ Ὀδυσσεύς ᾳ ᾼ"),
        case("Cyrl", "Привет, мир! Ђ ђ Љ љ Ꙋ ѣ"),
        case("Armn", "Հայերեն այբուբեն ﬓ"),
        case("Geor", "ქართული ანბანი Ა"),
        Case { rtl: true, ..case("Hebr", "שלום עולם בְּרֵאשִׁית") },
        Case { rtl: true, ..case("Arab", "مرحبا بالعالم السلام عليكم ﷲ") },
        Case { rtl: true, ..case("Arab", "لا لله الله بِسْمِ ٱللَّهِ") },
        Case { rtl: true, ..case("Arab", "٠١٢٣٤٥٦٧٨٩ یہ اردو ہے") },
        Case { rtl: true, ..case("Syrc", "ܫܠܡܐ ܥܠܡܐ") },
        Case { rtl: true, ..case("Thaa", "ދިވެހި ބަސް") },
        case("Deva", "हिन्दी भाषा क्षत्रिय श्रीमान् कर्मण्येवाधिकारस्ते"),
        case("Beng", "বাংলা ভাষা ক্ষত্রিয় কর্ম্ম"),
        case("Guru", "ਪੰਜਾਬੀ ਭਾਸ਼ਾ"),
        case("Gujr", "ગુજરાતી ભાષા ક્ષત્રિય"),
        case("Orya", "ଓଡ଼ିଆ ଭାଷା"),
        case("Taml", "தமிழ் மொழி ஸ்ரீ கொ"),
        case("Telu", "తెలుగు భాష క్ష్మ్య"),
        case("Knda", "ಕನ್ನಡ ಭಾಷೆ ಕ್ಷ"),
        case("Mlym", "മലയാളം ഭാഷ ക്ഷ ന്റെ"),
        case("Sinh", "සිංහල භාෂාව ක්‍රෝ"),
        case("Thai", "ภาษาไทย สวัสดีชาวโลก ก็ น้ำ"),
        case("Laoo", "ພາສາລາວ ສະບາຍດີ"),
        case("Khmr", "ភាសាខ្មែរ ស្រុកខ្មែរ ក្រៅ"),
        case("Mymr", "မြန်မာဘာသာ ကျွန်ုပ်"),
        case("Ethi", "አማርኛ ግዕዝ"),
        Case { lang: Some("ja"), ..case("Hani", "日本語の漢字 直 骨 令 刃") },
        Case { lang: Some("zh-Hans"), ..case("Hani", "简体中文 直 骨 令 刃") },
        Case { lang: Some("zh-Hant"), ..case("Hani", "繁體中文 直 骨 令 刃") },
        Case { lang: Some("ko"), ..case("Hang", "한국어 조선글 ᄒᆞᆫ") },
        Case { lang: Some("ja"), ..case("Kana", "ひらがなカタカナ ガ ぎ ゛") },
        case("Zyyy", "→ ← ∑ ∫ ∂ √ ≤ ≥ ≠ ∞ ℝ ℂ ⊕ ⟨ ⟩ ① ② ■ ▲ ★ ✓ ♪"),
        case("Zyyy", "😀 👍🏽 🇩🇪 ❤️ 👨‍👩‍👧"),
        case("Zyyy", "a b  c\u{00A0}d\u{200B}e\u{200D}f\u{2060}g"),
    ]
}

/// What both stacks return for a text: glyph ids, clusters and positions in font units.
#[derive(Debug, PartialEq)]
struct Shaped {
    glyphs: Vec<(u32, u32, i32, i32, i32)>,
}

fn script_tag(script: &str) -> u32 {
    u32::from_be_bytes(script.as_bytes().try_into().unwrap())
}

fn shape_harfbuzz(font: &Font, case: &Case, text: &str) -> Shaped {
    let mut buffer = UnicodeBuffer::new()
        .add_str(text)
        .set_cluster_level(ClusterLevel::MonotoneCharacters)
        .set_direction(if case.rtl { harfbuzz_rs::Direction::Rtl } else { harfbuzz_rs::Direction::Ltr });
    if !matches!(case.script, "Zyyy" | "Zinh" | "Zzzz") {
        buffer = buffer.set_script(harfbuzz_rs::Tag(script_tag(case.script)));
    }
    if let Some(lang) = case.lang {
        buffer = buffer.set_language(lang.parse().unwrap());
    }
    buffer = buffer.guess_segment_properties();
    let features: Vec<_> = case
        .features
        .iter()
        .map(|&(tag, value)| {
            harfbuzz_rs::Feature::new(
                harfbuzz_rs::Tag::new(tag[0] as char, tag[1] as char, tag[2] as char, tag[3] as char),
                value,
                ..,
            )
        })
        .collect();
    let result = hb_shape(&font.hb_font, buffer, &features);
    Shaped {
        glyphs: result
            .get_glyph_infos()
            .iter()
            .zip(result.get_glyph_positions())
            .map(|(info, pos)| (info.codepoint, info.cluster, pos.x_advance, pos.x_offset, pos.y_offset))
            .collect(),
    }
}

fn shape_harfrust(font: &RfFont, case: &Case, text: &str) -> Shaped {
    let mut buffer = Buffer::new();
    buffer.push_str(text);
    buffer.set_cluster_level(harfrust::ClusterLevel::MonotoneCharacters);
    buffer.set_direction(if case.rtl { Direction::RightToLeft } else { Direction::LeftToRight });
    if !matches!(case.script, "Zyyy" | "Zinh" | "Zzzz") {
        buffer.set_script(Script::from_iso15924_tag(Tag::new(case.script.as_bytes().try_into().unwrap())));
    }
    if let Some(lang) = case.lang {
        buffer.set_language(Language::new(lang));
    }
    buffer.guess_segment_properties();
    let features: Vec<_> = case.features.iter().map(|&(tag, value)| Feature::new(Tag::new(tag), value, ..)).collect();
    let shaper_font = ShaperFont::new(font);
    harfrust::shape(&shaper_font, &mut buffer, ShapeOptions::new().features(&features)).expect("harfrust shapes");
    Shaped {
        glyphs: buffer
            .glyph_infos()
            .iter()
            .zip(buffer.glyph_positions())
            .map(|(info, pos)| (info.glyph_id, info.cluster, pos.x_advance, pos.x_offset, pos.y_offset))
            .collect(),
    }
}

fn is_known(difference: &str) -> bool {
    KNOWN_DIFFERENCES.iter().any(|known| difference.starts_with(known))
}

fn bundled() -> Vec<(String, Arc<Vec<u8>>)> {
    BUNDLED_FONTS
        .iter()
        .map(|font| {
            let data = font.data().clone();
            let font = Font::load(data.clone(), 0).expect("the bundled fonts are valid");
            (font.postscript_name(), data)
        })
        .collect()
}

fn report(differences: &[String], compared: usize, what: &str) {
    let unknown: Vec<_> = differences.iter().filter(|d| !is_known(d)).collect();
    let mut message = String::new();
    for d in unknown.iter().take(40) {
        writeln!(message, "  {d}").unwrap();
    }
    assert!(
        unknown.is_empty(),
        "{} of {compared} {what} differ between harfbuzz and harfrust:\n{message}",
        unknown.len()
    );
    eprintln!("{what}: {compared} compared, {} known differences", differences.len());
}

/// The characters of `text` that `font` has glyphs for, so that a font is only compared on text
/// it can set (a missing glyph is the notdef glyph in both stacks, which is not interesting).
fn covered(font: &Font, text: &str) -> String {
    text.chars().filter(|&c| c.is_whitespace() || font.covers(c)).collect()
}

#[test]
fn shaping_of_the_bundled_fonts_is_the_same() {
    let mut differences = Vec::new();
    let mut compared = 0;
    for (name, data) in bundled() {
        let font = Font::load(data.clone(), 0).unwrap();
        let rf = RfFont::new(data.clone() as Arc<dyn AsRef<[u8]> + Send + Sync>, 0).expect("read-fonts parses it");
        for case in cases() {
            let text = covered(&font, case.text);
            if text.chars().filter(|c| !c.is_whitespace()).count() < 3 {
                continue;
            }
            // the case is only meaningful for a font if most of the text survived
            if text.chars().count() * 2 < case.text.chars().count() {
                continue;
            }
            compared += 1;
            let hb = shape_harfbuzz(&font, &case, &text);
            let rust = shape_harfrust(&rf, &case, &text);
            if hb != rust {
                let first = hb.glyphs.iter().zip(&rust.glyphs).position(|(a, b)| a != b);
                differences.push(format!(
                    "{name} {} {:?}{}: {} vs {} glyphs, first difference at {:?}: {:?} vs {:?}",
                    case.script,
                    text.chars().take(24).collect::<String>(),
                    if case.features.is_empty() { "" } else { " (features)" },
                    hb.glyphs.len(),
                    rust.glyphs.len(),
                    first,
                    first.map(|i| hb.glyphs[i]),
                    first.map(|i| rust.glyphs[i]),
                ));
            }
        }
    }
    assert!(compared > 200, "only {compared} cases were compared");
    report(&differences, compared, "shaped texts");
}

#[test]
fn the_font_queries_of_the_bundled_fonts_are_the_same() {
    let mut differences = Vec::new();
    let mut compared = 0;
    for (name, data) in bundled() {
        let font = Font::load(data.clone(), 0).unwrap();
        let rf = RfFont::new(data.clone() as Arc<dyn AsRef<[u8]> + Send + Sync>, 0).unwrap();
        let mut check = |what: &str, hb: String, rust: String| {
            compared += 1;
            if hb != rust {
                differences.push(format!("{name} {what}: harfbuzz {hb}, harfrust {rust}"));
            }
        };
        check("upem", font.upem.to_string(), rf.units_per_em().to_string());
        check("glyph count", font.hb_font.face().glyph_count().to_string(), rf.num_glyphs().to_string());
        check("is truetype", font.is_truetype().to_string(), rf.tables().glyf().is_ok().to_string());
        check("has MATH", font.has_math_table().to_string(), rf.tables().math().is_ok().to_string());
        check(
            "has COLR",
            font.has_color_glyphs().to_string(),
            rf.tables().colr().is_ok().to_string(),
        );
        let names = rf.names();
        let n = |id| names.get(id).and_then(|n| n.english_or_first()).map(|l| l.name().chars().collect::<String>());
        check("postscript name", format!("{:?}", Some(font.postscript_name())), format!("{:?}", n(NameId::POSTSCRIPT_NAME)));
        check(
            "family name",
            format!("{:?}", super::font::name_of(&font.hb_font.face(), harfbuzz_rs::sys::HB_OT_NAME_ID_FONT_FAMILY)),
            format!("{:?}", n(NameId::FAMILY_NAME)),
        );

        // ascent and descent: harfbuzz's font extents
        let extents = font.hb_font.get_font_h_extents().unwrap_or_default();
        let metrics = rf.metrics();
        let (asc, desc) = line_extents(metrics);
        check("ascender", extents.ascender.to_string(), asc.to_string());
        check("descender", extents.descender.to_string(), desc.to_string());
        check(
            "x-height",
            format!("{:?}", font.ot_metric(harfbuzz_rs::sys::HB_OT_METRICS_TAG_X_HEIGHT)),
            // read-fonts reports a zero `sxHeight` as a value, harfbuzz as absent
            format!("{:?}", metrics.x_height.map(|v| v.to_i32()).filter(|&v| v != 0)),
        );
        check(
            "cap-height",
            format!("{:?}", font.ot_metric(harfbuzz_rs::sys::HB_OT_METRICS_TAG_CAP_HEIGHT)),
            format!("{:?}", metrics.cap_height.map(|v| v.to_i32()).filter(|&v| v != 0)),
        );

        // the cmap and the advances for every character of the samples
        let charmap = rf.charmap();
        let glyph_metrics = rf.glyph_metrics();
        let mut chars: Vec<char> = cases().iter().flat_map(|c| c.text.chars()).collect();
        chars.sort();
        chars.dedup();
        for ch in chars {
            let hb = font.hb_font.get_nominal_glyph(ch);
            let rust = charmap.map_unicode(ch as u32).map(|g| g.to_u32());
            check("nominal glyph", format!("U+{:04X} {hb:?}", ch as u32), format!("U+{:04X} {rust:?}", ch as u32));
            if let (Some(glyph), Some(_)) = (hb, rust) {
                let hb_advance = font.hb_font.get_glyph_h_advance(glyph);
                let rust_advance = glyph_metrics.h_advance(GlyphId::new(glyph));
                check("advance", format!("U+{:04X} {hb_advance}", ch as u32), format!("U+{:04X} {rust_advance}", ch as u32));
            }
        }
    }
    report(&differences, compared, "font queries");
}

/// The ascender and descender (as harfbuzz reports them, descender negative) of a font.
fn line_extents(metrics: &read_fonts::model::metrics::Metrics) -> (i32, i32) {
    // placeholder for the port: harfbuzz uses the typo metrics if the font asks for them, then
    // hhea, then typo, then win
    let line = if metrics.use_typo_metrics { metrics.typo_line.or(metrics.hhea_line) } else { metrics.hhea_line };
    let line = line.filter(|l| l.ascender.to_i32() != 0 || l.descender.to_i32() != 0).or(metrics.typo_line);
    match line {
        Some(l) => (l.ascender.to_i32(), l.descender.to_i32()),
        None => metrics.win_line.map_or((0, 0), |w| (w.ascender.to_i32(), w.descender.to_i32())),
    }
}
