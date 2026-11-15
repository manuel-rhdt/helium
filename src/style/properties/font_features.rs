//! The properties that choose OpenType features: `font-feature-settings`, `font-kerning` and the
//! `font-variant-*` longhands with the `font-variant` shorthand.
//! https://www.w3.org/TR/css-fonts-4/#feature-variation-precedence
//!
//! Every property is inherited and kept on its own in the computed values; `Features::of` turns
//! them into the list of features that is handed to the shaper. The order is the one of the
//! specification: the features of `font-kerning` and the `font-variant-*` properties come first
//! and `font-feature-settings` last, so that it overrides them.
//!
//! Not supported: `font-variant-alternates` (it needs `@font-feature-values`) and
//! `font-variant-emoji`, and the synthesis of small caps, subscripts and superscripts for fonts
//! without the features for them.

use cssparser::{ParseError, Parser, Token, match_ignore_ascii_case};
use helium_derive::PropertyValue;

use super::specified::PropertyValue;
use crate::style::parse::StyleParseError;

/// An OpenType feature tag, four ASCII characters.
pub type Tag = [u8; 4];

/// An OpenType feature with its value: 0 turns it off, 1 on, and a larger value selects an
/// alternate of the feature.
pub type Feature = (Tag, u32);

fn tag(name: &str) -> Tag {
    let bytes = name.as_bytes();
    [bytes[0], bytes[1], bytes[2], bytes[3]]
}

/// https://www.w3.org/TR/css-fonts-4/#font-kerning-prop
#[derive(Debug, Copy, Clone, Default, PartialEq, Eq, PropertyValue)]
pub enum FontKerning {
    /// The same as `normal` here: kerning is on unless the font says otherwise.
    #[default]
    Auto,
    Normal,
    None,
}

/// https://www.w3.org/TR/css-fonts-4/#font-variant-caps-prop
#[derive(Debug, Copy, Clone, Default, PartialEq, Eq, PropertyValue)]
pub enum FontVariantCaps {
    #[default]
    Normal,
    SmallCaps,
    AllSmallCaps,
    PetiteCaps,
    AllPetiteCaps,
    Unicase,
    TitlingCaps,
}

/// https://www.w3.org/TR/css-fonts-4/#font-variant-position-prop
#[derive(Debug, Copy, Clone, Default, PartialEq, Eq, PropertyValue)]
pub enum FontVariantPosition {
    #[default]
    Normal,
    Sub,
    Super,
}

/// Reads an identifier of `input` that is one of `names`, and returns its index.
fn keyword(input: &mut Parser<'_>, names: &[&str]) -> Option<usize> {
    input
        .try_parse(|input| {
            let ident = input.expect_ident()?.clone();
            names
                .iter()
                .position(|name| ident.eq_ignore_ascii_case(name))
                .ok_or_else(|| input.new_error_for_next_token::<()>())
        })
        .ok()
}

fn unexpected<T>() -> Result<T, ()> {
    Err(())
}

/// https://www.w3.org/TR/css-fonts-4/#font-variant-ligatures-prop: `None` leaves the choice to
/// the font (the initial value `normal`).
#[derive(Debug, Copy, Clone, Default, PartialEq, Eq)]
pub struct FontVariantLigatures {
    pub common: Option<bool>,
    pub discretionary: Option<bool>,
    pub historical: Option<bool>,
    pub contextual: Option<bool>,
}

impl FontVariantLigatures {
    const NAMES: [&'static str; 8] = [
        "common-ligatures",
        "no-common-ligatures",
        "discretionary-ligatures",
        "no-discretionary-ligatures",
        "historical-ligatures",
        "no-historical-ligatures",
        "contextual",
        "no-contextual",
    ];

    fn none() -> Self {
        FontVariantLigatures {
            common: Some(false),
            discretionary: Some(false),
            historical: Some(false),
            contextual: Some(false),
        }
    }

    /// Reads one component; false if there is none. A group that is set twice is an error.
    fn component(&mut self, input: &mut Parser<'_>) -> Result<bool, ()> {
        let Some(index) = keyword(input, &Self::NAMES) else {
            return Ok(false);
        };
        let slot = match index / 2 {
            0 => &mut self.common,
            1 => &mut self.discretionary,
            2 => &mut self.historical,
            _ => &mut self.contextual,
        };
        if slot.is_some() {
            return unexpected();
        }
        *slot = Some(index % 2 == 0);
        Ok(true)
    }

    fn features(self, out: &mut Vec<Feature>) {
        let mut add = |value: Option<bool>, tags: &[&str]| {
            if let Some(on) = value {
                out.extend(tags.iter().map(|name| (tag(name), on as u32)));
            }
        };
        add(self.common, &["liga", "clig"]);
        add(self.discretionary, &["dlig"]);
        add(self.historical, &["hlig"]);
        add(self.contextual, &["calt"]);
    }
}

impl<'i> PropertyValue<'i> for FontVariantLigatures {
    type Error = StyleParseError;

    fn from_css_property_value(input: &mut Parser<'i>) -> Result<Self, ParseError<StyleParseError>> {
        if keyword(input, &["normal"]).is_some() {
            return Ok(Self::default());
        }
        if keyword(input, &["none"]).is_some() {
            return Ok(Self::none());
        }
        let mut result = Self::default();
        if !result.component(input).map_err(|_| input.new_error_for_next_token())? {
            return Err(input.new_error_for_next_token());
        }
        while result.component(input).map_err(|_| input.new_error_for_next_token())? {}
        Ok(result)
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum NumericFigure {
    Lining,
    Oldstyle,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum NumericSpacing {
    Proportional,
    Tabular,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum NumericFraction {
    Diagonal,
    Stacked,
}

/// https://www.w3.org/TR/css-fonts-4/#font-variant-numeric-prop
#[derive(Debug, Copy, Clone, Default, PartialEq, Eq)]
pub struct FontVariantNumeric {
    pub ordinal: bool,
    pub slashed_zero: bool,
    pub figure: Option<NumericFigure>,
    pub spacing: Option<NumericSpacing>,
    pub fraction: Option<NumericFraction>,
}

impl FontVariantNumeric {
    const NAMES: [&'static str; 9] = [
        "ordinal",
        "slashed-zero",
        "lining-nums",
        "oldstyle-nums",
        "proportional-nums",
        "tabular-nums",
        "diagonal-fractions",
        "stacked-fractions",
        "",
    ];

    fn component(&mut self, input: &mut Parser<'_>) -> Result<bool, ()> {
        let Some(index) = keyword(input, &Self::NAMES[..8]) else {
            return Ok(false);
        };
        let taken = match index {
            0 => std::mem::replace(&mut self.ordinal, true),
            1 => std::mem::replace(&mut self.slashed_zero, true),
            2 | 3 => self
                .figure
                .replace(if index == 2 { NumericFigure::Lining } else { NumericFigure::Oldstyle })
                .is_some(),
            4 | 5 => self
                .spacing
                .replace(if index == 4 { NumericSpacing::Proportional } else { NumericSpacing::Tabular })
                .is_some(),
            _ => self
                .fraction
                .replace(if index == 6 { NumericFraction::Diagonal } else { NumericFraction::Stacked })
                .is_some(),
        };
        if taken {
            return unexpected();
        }
        Ok(true)
    }

    fn features(self, out: &mut Vec<Feature>) {
        if self.ordinal {
            out.push((tag("ordn"), 1));
        }
        if self.slashed_zero {
            out.push((tag("zero"), 1));
        }
        match self.figure {
            Some(NumericFigure::Lining) => out.push((tag("lnum"), 1)),
            Some(NumericFigure::Oldstyle) => out.push((tag("onum"), 1)),
            None => {}
        }
        match self.spacing {
            Some(NumericSpacing::Proportional) => out.push((tag("pnum"), 1)),
            Some(NumericSpacing::Tabular) => out.push((tag("tnum"), 1)),
            None => {}
        }
        match self.fraction {
            Some(NumericFraction::Diagonal) => out.push((tag("frac"), 1)),
            Some(NumericFraction::Stacked) => out.push((tag("afrc"), 1)),
            None => {}
        }
    }
}

impl<'i> PropertyValue<'i> for FontVariantNumeric {
    type Error = StyleParseError;

    fn from_css_property_value(input: &mut Parser<'i>) -> Result<Self, ParseError<StyleParseError>> {
        if keyword(input, &["normal"]).is_some() {
            return Ok(Self::default());
        }
        let mut result = Self::default();
        if !result.component(input).map_err(|_| input.new_error_for_next_token())? {
            return Err(input.new_error_for_next_token());
        }
        while result.component(input).map_err(|_| input.new_error_for_next_token())? {}
        Ok(result)
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum EastAsianVariant {
    Jis78,
    Jis83,
    Jis90,
    Jis04,
    Simplified,
    Traditional,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum EastAsianWidth {
    FullWidth,
    ProportionalWidth,
}

/// https://www.w3.org/TR/css-fonts-4/#font-variant-east-asian-prop
#[derive(Debug, Copy, Clone, Default, PartialEq, Eq)]
pub struct FontVariantEastAsian {
    pub variant: Option<EastAsianVariant>,
    pub width: Option<EastAsianWidth>,
    pub ruby: bool,
}

impl FontVariantEastAsian {
    const NAMES: [&'static str; 9] = [
        "jis78",
        "jis83",
        "jis90",
        "jis04",
        "simplified",
        "traditional",
        "full-width",
        "proportional-width",
        "ruby",
    ];

    fn component(&mut self, input: &mut Parser<'_>) -> Result<bool, ()> {
        use EastAsianVariant as V;
        let Some(index) = keyword(input, &Self::NAMES) else {
            return Ok(false);
        };
        let taken = match index {
            0..=5 => self
                .variant
                .replace([V::Jis78, V::Jis83, V::Jis90, V::Jis04, V::Simplified, V::Traditional][index])
                .is_some(),
            6 | 7 => self
                .width
                .replace(if index == 6 { EastAsianWidth::FullWidth } else { EastAsianWidth::ProportionalWidth })
                .is_some(),
            _ => std::mem::replace(&mut self.ruby, true),
        };
        if taken {
            return unexpected();
        }
        Ok(true)
    }

    fn features(self, out: &mut Vec<Feature>) {
        use EastAsianVariant as V;
        if let Some(variant) = self.variant {
            let name = match variant {
                V::Jis78 => "jp78",
                V::Jis83 => "jp83",
                V::Jis90 => "jp90",
                V::Jis04 => "jp04",
                V::Simplified => "smpl",
                V::Traditional => "trad",
            };
            out.push((tag(name), 1));
        }
        match self.width {
            Some(EastAsianWidth::FullWidth) => out.push((tag("fwid"), 1)),
            Some(EastAsianWidth::ProportionalWidth) => out.push((tag("pwid"), 1)),
            None => {}
        }
        if self.ruby {
            out.push((tag("ruby"), 1));
        }
    }
}

impl<'i> PropertyValue<'i> for FontVariantEastAsian {
    type Error = StyleParseError;

    fn from_css_property_value(input: &mut Parser<'i>) -> Result<Self, ParseError<StyleParseError>> {
        if keyword(input, &["normal"]).is_some() {
            return Ok(Self::default());
        }
        let mut result = Self::default();
        if !result.component(input).map_err(|_| input.new_error_for_next_token())? {
            return Err(input.new_error_for_next_token());
        }
        while result.component(input).map_err(|_| input.new_error_for_next_token())? {}
        Ok(result)
    }
}

/// https://www.w3.org/TR/css-fonts-4/#font-feature-settings-prop: `normal` (no features) or a
/// list of tags with values. A tag that is listed twice takes the value of the last one.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FontFeatureSettings(pub Vec<Feature>);

impl<'i> PropertyValue<'i> for FontFeatureSettings {
    type Error = StyleParseError;

    fn from_css_property_value(input: &mut Parser<'i>) -> Result<Self, ParseError<StyleParseError>> {
        if keyword(input, &["normal"]).is_some() {
            return Ok(Self::default());
        }
        let mut features: Vec<Feature> = Vec::new();
        loop {
            let name = input.expect_string()?.clone();
            let valid = name.len() == 4 && name.bytes().all(|b| (0x20..=0x7e).contains(&b));
            if !valid {
                return Err(input.new_error_for_next_token());
            }
            let value = input
                .try_parse(|input| match input.next()?.clone() {
                    Token::Number { int_value: Some(value), .. } if value >= 0 => Ok(value as u32),
                    Token::Ident(ident) => match_ignore_ascii_case! { &ident,
                        "on" => Ok(1),
                        "off" => Ok(0),
                        _ => Err(input.new_error_for_next_token::<()>()),
                    },
                    _ => Err(input.new_error_for_next_token::<()>()),
                })
                .unwrap_or(1);
            let name = tag(&name);
            match features.iter_mut().find(|(other, _)| *other == name) {
                Some(known) => known.1 = value,
                None => features.push((name, value)),
            }
            if input.try_parse(Parser::expect_comma).is_err() {
                return Ok(FontFeatureSettings(features));
            }
        }
    }
}

/// The value of the `font-variant` shorthand.
/// https://www.w3.org/TR/css-fonts-4/#font-variant-prop
#[derive(Debug, Clone, Default)]
pub struct FontVariant {
    ligatures: FontVariantLigatures,
    caps: FontVariantCaps,
    numeric: FontVariantNumeric,
    east_asian: FontVariantEastAsian,
    position: FontVariantPosition,
}

impl FontVariant {
    /// The `font-variant` of CSS 2 (`normal | small-caps`) that the `font` shorthand takes.
    pub fn small_caps() -> Self {
        FontVariant { caps: FontVariantCaps::SmallCaps, ..FontVariant::default() }
    }

    pub fn ligatures(&self) -> FontVariantLigatures {
        self.ligatures
    }

    pub fn caps(&self) -> FontVariantCaps {
        self.caps
    }

    pub fn numeric(&self) -> FontVariantNumeric {
        self.numeric
    }

    pub fn east_asian(&self) -> FontVariantEastAsian {
        self.east_asian
    }

    pub fn position(&self) -> FontVariantPosition {
        self.position
    }
}

impl<'i> PropertyValue<'i> for FontVariant {
    type Error = StyleParseError;

    fn from_css_property_value(input: &mut Parser<'i>) -> Result<Self, ParseError<StyleParseError>> {
        if keyword(input, &["normal"]).is_some() {
            return Ok(Self::default());
        }
        if keyword(input, &["none"]).is_some() {
            return Ok(FontVariant { ligatures: FontVariantLigatures::none(), ..FontVariant::default() });
        }
        let mut result = Self::default();
        let (mut caps, mut position, mut any) = (false, false, false);
        loop {
            let fail = |input: &mut Parser<'i>| input.new_error_for_next_token();
            if result.ligatures.component(input).map_err(|_| fail(input))?
                || result.numeric.component(input).map_err(|_| fail(input))?
                || result.east_asian.component(input).map_err(|_| fail(input))?
            {
            } else if !caps && let Ok(value) = input.try_parse(FontVariantCaps::from_css_property_value) {
                // `normal` was handled above
                caps = true;
                result.caps = value;
            } else if !position && let Ok(value) = input.try_parse(FontVariantPosition::from_css_property_value) {
                position = true;
                result.position = value;
            } else if any {
                return Ok(result);
            } else {
                return Err(fail(input));
            }
            any = true;
        }
    }
}

/// The OpenType features that the computed font properties ask for.
pub struct Features;

impl Features {
    /// The features of the properties, in the order of precedence (the later ones override the
    /// earlier ones).
    pub fn of(font: &crate::style::properties::computed::Font) -> Vec<Feature> {
        let mut out = Vec::new();
        match font.font_kerning {
            FontKerning::Auto => {}
            FontKerning::Normal => out.push((tag("kern"), 1)),
            FontKerning::None => out.push((tag("kern"), 0)),
        }
        font.font_variant_ligatures.features(&mut out);
        match font.font_variant_caps {
            FontVariantCaps::Normal => {}
            FontVariantCaps::SmallCaps => out.push((tag("smcp"), 1)),
            FontVariantCaps::AllSmallCaps => out.extend([(tag("smcp"), 1), (tag("c2sc"), 1)]),
            FontVariantCaps::PetiteCaps => out.push((tag("pcap"), 1)),
            FontVariantCaps::AllPetiteCaps => out.extend([(tag("pcap"), 1), (tag("c2pc"), 1)]),
            FontVariantCaps::Unicase => out.push((tag("unic"), 1)),
            FontVariantCaps::TitlingCaps => out.push((tag("titl"), 1)),
        }
        font.font_variant_numeric.features(&mut out);
        font.font_variant_east_asian.features(&mut out);
        match font.font_variant_position {
            FontVariantPosition::Normal => {}
            FontVariantPosition::Sub => out.push((tag("subs"), 1)),
            FontVariantPosition::Super => out.push((tag("sups"), 1)),
        }
        out.extend(font.font_feature_settings.0.iter().copied());
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse<T: for<'i> PropertyValue<'i, Error = StyleParseError>>(css: &str) -> Option<T> {
        let mut parser = Parser::new(css);
        let value = T::from_css_property_value(&mut parser).ok()?;
        parser.is_exhausted().then_some(value)
    }

    #[test]
    fn feature_settings_have_tags_and_values() {
        let settings: FontFeatureSettings = parse("\"liga\" 0, \"smcp\", \"ss01\" on, 'zero' off, \"salt\" 3").unwrap();
        assert_eq!(
            settings.0,
            vec![(*b"liga", 0), (*b"smcp", 1), (*b"ss01", 1), (*b"zero", 0), (*b"salt", 3)]
        );
    }

    #[test]
    fn a_feature_that_is_listed_twice_takes_the_last_value() {
        let settings: FontFeatureSettings = parse("\"liga\" 0, \"kern\", \"liga\" 1").unwrap();
        assert_eq!(settings.0, vec![(*b"liga", 1), (*b"kern", 1)]);
    }

    #[test]
    fn invalid_feature_settings_are_rejected() {
        assert!(parse::<FontFeatureSettings>("liga").is_none());
        assert!(parse::<FontFeatureSettings>("\"lig\"").is_none());
        assert!(parse::<FontFeatureSettings>("\"liga\" -1").is_none());
        assert!(parse::<FontFeatureSettings>("\"liga\" 1.5").is_none());
        assert!(parse::<FontFeatureSettings>("\"liga\",").is_none());
        assert_eq!(parse::<FontFeatureSettings>("normal"), Some(FontFeatureSettings::default()));
    }

    #[test]
    fn ligatures_are_set_by_group() {
        let value: FontVariantLigatures = parse("no-common-ligatures discretionary-ligatures").unwrap();
        assert_eq!(value.common, Some(false));
        assert_eq!(value.discretionary, Some(true));
        assert_eq!(value.historical, None);
        assert_eq!(parse::<FontVariantLigatures>("none"), Some(FontVariantLigatures::none()));
        assert_eq!(parse::<FontVariantLigatures>("normal"), Some(FontVariantLigatures::default()));
        // a group twice
        assert!(parse::<FontVariantLigatures>("common-ligatures no-common-ligatures").is_none());
        assert!(parse::<FontVariantLigatures>("normal contextual").is_none());
        assert!(parse::<FontVariantLigatures>("small-caps").is_none());
    }

    #[test]
    fn numeric_east_asian_and_keyword_properties_parse() {
        let numeric: FontVariantNumeric = parse("oldstyle-nums tabular-nums slashed-zero").unwrap();
        assert_eq!(numeric.figure, Some(NumericFigure::Oldstyle));
        assert_eq!(numeric.spacing, Some(NumericSpacing::Tabular));
        assert!(numeric.slashed_zero && !numeric.ordinal);
        assert!(parse::<FontVariantNumeric>("lining-nums oldstyle-nums").is_none());
        let east_asian: FontVariantEastAsian = parse("jis04 full-width ruby").unwrap();
        assert_eq!(east_asian.variant, Some(EastAsianVariant::Jis04));
        assert!(east_asian.ruby);
        assert_eq!(parse::<FontVariantCaps>("all-small-caps"), Some(FontVariantCaps::AllSmallCaps));
        assert_eq!(parse::<FontVariantPosition>("super"), Some(FontVariantPosition::Super));
        assert_eq!(parse::<FontKerning>("none"), Some(FontKerning::None));
        assert!(parse::<FontKerning>("small-caps").is_none());
    }

    #[test]
    fn the_font_variant_shorthand_takes_the_components_in_any_order() {
        let value: FontVariant = parse("small-caps oldstyle-nums no-common-ligatures").unwrap();
        assert_eq!(value.caps(), FontVariantCaps::SmallCaps);
        assert_eq!(value.numeric().figure, Some(NumericFigure::Oldstyle));
        assert_eq!(value.ligatures().common, Some(false));
        let none: FontVariant = parse("none").unwrap();
        assert_eq!(none.ligatures(), FontVariantLigatures::none());
        assert_eq!(none.caps(), FontVariantCaps::Normal);
        let normal: FontVariant = parse("normal").unwrap();
        assert_eq!(normal.ligatures(), FontVariantLigatures::default());
        assert!(parse::<FontVariant>("small-caps all-small-caps").is_none());
        assert!(parse::<FontVariant>("bold").is_none());
    }
}
