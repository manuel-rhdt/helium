//! The metadata of the document: what `<meta>` elements say about the author, the subject and
//! so on, and the dates. They go to the document information dictionary and to the XMP packet
//! of the PDF.
//!
//! | `<meta name=...>`            | Info       | XMP                |
//! | ---------------------------- | ---------- | ------------------ |
//! | `author` (can be repeated)   | `Author`   | `dc:creator`       |
//! | `description`                | `Subject`  | `dc:description`   |
//! | `keywords`                   | `Keywords` | `pdf:Keywords`     |
//! | `generator`, `application-name` | `Creator` | `xmp:CreatorTool` |
//! | `date`                       | `CreationDate`, `ModDate` | `xmp:CreateDate`, `xmp:ModifyDate` |
//!
//! The output must not depend on the time at which it is made (the file identifier is derived
//! from the content), so the PDF has a date only if the document sets one with `<meta
//! name="date">`, or if `SOURCE_DATE_EPOCH` is set (<https://reproducible-builds.org/specs/source-date-epoch/>).

use crate::parser::{ElementData, ParsedData};
use crate::tagging::{attribute, collapse_whitespace};
use crate::tree::{DomRef, TreeLinked};

/// A date and time, with the offset from UTC if it is known.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Timestamp {
    pub year: u16,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
    /// The offset from UTC in minutes; `None` if the local time zone is unknown.
    pub offset_minutes: Option<i16>,
}

impl Timestamp {
    /// A date (and time) as written in `<meta name="date">`: `YYYY-MM-DD`, optionally followed
    /// by `T` (or a space), `HH:MM`, optionally `:SS` (and a fraction, which is dropped) and a
    /// time zone: `Z`, `+HH:MM`, `-HH:MM`, `+HHMM`, or `+HH`.
    pub fn parse(text: &str) -> Option<Timestamp> {
        let text = text.trim();
        let (date, rest) = match text.find(['T', 't', ' ']) {
            Some(index) => (&text[..index], Some(&text[index + 1..])),
            None => (text, None),
        };
        let mut parts = date.split('-');
        let year: u16 = parts.next()?.parse().ok().filter(|_| date.len() >= 10)?;
        let month: u8 = parts.next()?.parse().ok()?;
        let day: u8 = parts.next()?.parse().ok()?;
        if parts.next().is_some() || date.len() != 10 {
            return None;
        }
        let mut timestamp = Timestamp { year, month, day, hour: 0, minute: 0, second: 0, offset_minutes: None };
        if let Some(rest) = rest {
            let (time, zone) = match rest.find(['Z', 'z', '+', '-']) {
                Some(index) => (&rest[..index], Some(&rest[index..])),
                None => (rest, None),
            };
            let mut fields = time.split(':');
            timestamp.hour = fields.next()?.parse().ok()?;
            timestamp.minute = fields.next()?.parse().ok()?;
            if let Some(seconds) = fields.next() {
                // a fraction of a second is dropped
                timestamp.second = seconds.split('.').next()?.parse().ok()?;
            }
            if fields.next().is_some() {
                return None;
            }
            timestamp.offset_minutes = match zone {
                None => None,
                Some("Z") | Some("z") => Some(0),
                Some(zone) => {
                    let sign = if zone.starts_with('-') { -1 } else { 1 };
                    let digits: String = zone[1..].chars().filter(|c| *c != ':').collect();
                    if !digits.chars().all(|c| c.is_ascii_digit()) {
                        return None;
                    }
                    let (hours, minutes) = match digits.len() {
                        2 => (digits.parse::<i16>().ok()?, 0),
                        4 => (digits[..2].parse::<i16>().ok()?, digits[2..].parse::<i16>().ok()?),
                        _ => return None,
                    };
                    if hours > 23 || minutes > 59 {
                        return None;
                    }
                    Some(sign * (hours * 60 + minutes))
                }
            };
        }
        timestamp.is_valid().then_some(timestamp)
    }

    /// The date of `SOURCE_DATE_EPOCH`, in UTC: the seconds since 1970-01-01.
    pub fn from_unix(seconds: u64) -> Option<Timestamp> {
        // the civil date of a day count (H. Hinnant's algorithm)
        let days = (seconds / 86_400) as i64;
        let time = seconds % 86_400;
        let z = days + 719_468;
        let era = z.div_euclid(146_097);
        let day_of_era = z.rem_euclid(146_097);
        let year_of_era = (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
        let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
        let mp = (5 * day_of_year + 2) / 153;
        let day = (day_of_year - (153 * mp + 2) / 5 + 1) as u8;
        let month = if mp < 10 { mp + 3 } else { mp - 9 } as u8;
        let year = year_of_era + era * 400 + i64::from(month <= 2);
        Some(Timestamp {
            year: u16::try_from(year).ok().filter(|year| *year <= 9999)?,
            month,
            day,
            hour: (time / 3600) as u8,
            minute: (time % 3600 / 60) as u8,
            second: (time % 60) as u8,
            offset_minutes: Some(0),
        })
    }

    /// The date of the environment variable `SOURCE_DATE_EPOCH`, if it is set to a number.
    pub fn from_environment() -> Option<Timestamp> {
        Timestamp::from_unix(std::env::var("SOURCE_DATE_EPOCH").ok()?.trim().parse().ok()?)
    }

    fn is_valid(&self) -> bool {
        const DAYS: [u8; 12] = [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
        (1..=12).contains(&self.month)
            && self.day >= 1
            && self.day <= DAYS[self.month as usize - 1]
            && self.hour < 24
            && self.minute < 60
            && self.second < 60
    }

    /// A PDF date string: `D:YYYYMMDDHHmmSSZ` or `D:YYYYMMDDHHmmSS+HH'mm'`.
    pub fn pdf(&self) -> String {
        let mut text = format!(
            "D:{:04}{:02}{:02}{:02}{:02}{:02}",
            self.year, self.month, self.day, self.hour, self.minute, self.second
        );
        match self.offset_minutes {
            None => {}
            Some(0) => text.push('Z'),
            Some(offset) => {
                text += &format!("{}{:02}'{:02}'", if offset < 0 { '-' } else { '+' }, offset.abs() / 60, offset.abs() % 60)
            }
        }
        text
    }

    /// An XMP date (ISO 8601): `YYYY-MM-DDTHH:MM:SSZ`.
    pub fn xmp(&self) -> String {
        let mut text = format!(
            "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}",
            self.year, self.month, self.day, self.hour, self.minute, self.second
        );
        match self.offset_minutes {
            None => {}
            Some(0) => text.push('Z'),
            Some(offset) => {
                text += &format!("{}{:02}:{:02}", if offset < 0 { '-' } else { '+' }, offset.abs() / 60, offset.abs() % 60)
            }
        }
        text
    }
}

/// What the `<meta>` elements of the document say.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Metadata {
    pub authors: Vec<String>,
    pub description: Option<String>,
    pub keywords: Option<String>,
    /// The program that made the document (`generator`, `application-name`).
    pub creator: Option<String>,
    pub date: Option<Timestamp>,
}

impl Metadata {
    /// Reads the `<meta name content>` elements of `dom`. The first of `description`, `keywords`,
    /// `generator` and `date` counts; every `author` does. Names are not case-sensitive and
    /// whitespace of the content is collapsed; empty content and a `date` that cannot be read
    /// are ignored.
    pub fn from_dom(dom: &DomRef) -> Metadata {
        let mut metadata = Metadata::default();
        metadata.collect(dom);
        metadata
    }

    fn collect(&mut self, node: &DomRef) {
        if let ParsedData::Element(element) = &node.html_data {
            if element.name.local == html5ever::local_name!("meta") {
                self.add(element);
            }
        }
        for child in TreeLinked::children(node) {
            self.collect(&child);
        }
    }

    fn add(&mut self, element: &ElementData) {
        let (Some(name), Some(content)) = (attribute(element, "name"), attribute(element, "content")) else {
            return;
        };
        let content = collapse_whitespace(&content);
        if content.is_empty() {
            return;
        }
        match name.trim().to_ascii_lowercase().as_str() {
            "author" => self.authors.push(content),
            "description" => {
                self.description.get_or_insert(content);
            }
            "keywords" => {
                self.keywords.get_or_insert(content);
            }
            "generator" | "application-name" => {
                self.creator.get_or_insert(content);
            }
            "date" => {
                if self.date.is_none() {
                    self.date = Timestamp::parse(&content);
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_are_parsed() {
        let date = Timestamp::parse("2024-03-05").unwrap();
        assert_eq!(date.pdf(), "D:20240305000000");
        assert_eq!(date.xmp(), "2024-03-05T00:00:00");
        let date = Timestamp::parse("2024-03-05T14:30:07Z").unwrap();
        assert_eq!(date.pdf(), "D:20240305143007Z");
        assert_eq!(date.xmp(), "2024-03-05T14:30:07Z");
        let date = Timestamp::parse("2024-03-05 14:30+05:30").unwrap();
        assert_eq!(date.pdf(), "D:20240305143000+05'30'");
        assert_eq!(date.xmp(), "2024-03-05T14:30:00+05:30");
        let date = Timestamp::parse("2024-03-05T14:30:07.123-0800").unwrap();
        assert_eq!(date.xmp(), "2024-03-05T14:30:07-08:00");
        assert_eq!(date.pdf(), "D:20240305143007-08'00'");
    }

    #[test]
    fn invalid_dates_are_rejected() {
        for text in ["", "2024", "2024-03", "2024-13-01", "2024-02-30", "2024-03-05T25:00", "2024-03-05T10", "2024-03-05T10:00+24:00", "yesterday", "24-03-05"] {
            assert_eq!(Timestamp::parse(text), None, "{text:?}");
        }
    }

    #[test]
    fn unix_time_is_converted() {
        assert_eq!(Timestamp::from_unix(0).unwrap().xmp(), "1970-01-01T00:00:00Z");
        assert_eq!(Timestamp::from_unix(1_709_649_007).unwrap().xmp(), "2024-03-05T14:30:07Z");
        // a leap day
        assert_eq!(Timestamp::from_unix(1_709_164_800).unwrap().xmp(), "2024-02-29T00:00:00Z");
        assert_eq!(Timestamp::from_unix(253_402_300_799).unwrap().xmp(), "9999-12-31T23:59:59Z");
        assert_eq!(Timestamp::from_unix(253_402_300_800), None);
    }
}
