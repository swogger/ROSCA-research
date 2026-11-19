//! Shared helpers for the analysis programs.
//!
//! Nothing in this crate contains data values. The data files are
//! confidential and are read from paths given on the command line.

use std::collections::HashMap;

use chrono::{DateTime, TimeZone, Utc};
use rand::seq::SliceRandom;
use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;

pub mod anonymity;
pub mod calendar;
pub mod p2;
pub mod stats;
pub mod svgplot;

/// Number of largest cities that keep their own label.
pub const N_NAMED_CITIES: usize = 5;

/// Fee regime of a group, defined by the start date of the group.
///
/// The cut-off dates were read from the data (share of members paying
/// the flat fee).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Regime {
    /// Flat fee.
    A,
    /// Early seats priced.
    B,
    /// Every seat priced.
    C,
}

impl Regime {
    pub fn b_start() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2023, 3, 1, 0, 0, 0).unwrap()
    }

    pub fn c_start() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2024, 3, 1, 0, 0, 0).unwrap()
    }

    pub fn of(group_start: DateTime<Utc>) -> Regime {
        if group_start < Regime::b_start() {
            Regime::A
        } else if group_start < Regime::c_start() {
            Regime::B
        } else {
            Regime::C
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Regime::A => "A",
            Regime::B => "B",
            Regime::C => "C",
        }
    }
}

/// Parse a timestamp such as `2023-01-30T00:00:00.000Z`.
pub fn parse_time(text: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(text.trim())
        .ok()
        .map(|t| t.with_timezone(&Utc))
}

/// Extract the four-digit birth year. Two date formats occur in the export.
pub fn birth_year(text: &str) -> Option<i32> {
    let bytes = text.as_bytes();
    if bytes.len() < 4 {
        return None;
    }
    for i in 0..=bytes.len() - 4 {
        let window = &bytes[i..i + 4];
        let century = &window[..2];
        if (century == b"19" || century == b"20") && window.iter().all(u8::is_ascii_digit) {
            return std::str::from_utf8(window).ok()?.parse().ok();
        }
    }
    None
}

/// Reduce an age in years to a band. Ages outside 0..200 give `None`.
pub fn age_band(age: i32) -> Option<&'static str> {
    match age {
        0..=24 => Some("under 25"),
        25..=29 => Some("25-29"),
        30..=34 => Some("30-34"),
        35..=44 => Some("35-44"),
        45..=199 => Some("45 and over"),
        _ => None,
    }
}

/// Creation time stored in the first four bytes of a database id.
pub fn object_id_time(id: &str) -> Option<DateTime<Utc>> {
    let seconds = u32::from_str_radix(id.get(..8)?, 16).ok()?;
    Utc.timestamp_opt(i64::from(seconds), 0).single()
}

/// Random generator for the labels that replace user and group ids.
///
/// The seed comes from the operating system and is not stored. The
/// labels therefore cannot be traced back to the original ids, and they
/// differ on every run.
pub fn entropy_rng() -> ChaCha8Rng {
    ChaCha8Rng::from_entropy()
}

/// Random generator with a fixed seed, for tests and for analyses that
/// must give the same result on every run.
pub fn seeded_rng(seed: u64) -> ChaCha8Rng {
    ChaCha8Rng::seed_from_u64(seed)
}

/// Year and month of a timestamp, such as `2023-04`.
pub fn month(t: DateTime<Utc>) -> String {
    t.format("%Y-%m").to_string()
}

/// Map each distinct value to a random label such as `U000123`.
///
/// The labels carry no information about the original values.
pub fn random_labels<'a, I>(values: I, prefix: &str, rng: &mut ChaCha8Rng) -> HashMap<String, String>
where
    I: IntoIterator<Item = &'a str>,
{
    let mut distinct: Vec<&str> = values.into_iter().collect();
    distinct.sort_unstable();
    distinct.dedup();
    let mut order: Vec<usize> = (1..=distinct.len()).collect();
    order.shuffle(rng);
    let width = distinct.len().to_string().len();
    distinct
        .into_iter()
        .zip(order)
        .map(|(value, k)| (value.to_string(), format!("{prefix}{k:0width$}")))
        .collect()
}

/// Format a count with thousands separators, for the printed summary.
pub fn with_commas(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn birth_year_reads_both_formats() {
        assert_eq!(birth_year("1990-05-12"), Some(1990));
        assert_eq!(birth_year("20 Mar, 1985"), Some(1985));
        assert_eq!(birth_year("5 Jan, 2001"), Some(2001));
        assert_eq!(birth_year(""), None);
        assert_eq!(birth_year("unknown"), None);
    }

    #[test]
    fn age_bands_have_the_stated_edges() {
        assert_eq!(age_band(24), Some("under 25"));
        assert_eq!(age_band(25), Some("25-29"));
        assert_eq!(age_band(34), Some("30-34"));
        assert_eq!(age_band(35), Some("35-44"));
        assert_eq!(age_band(45), Some("45 and over"));
        assert_eq!(age_band(-1), None);
        assert_eq!(age_band(200), None);
    }

    #[test]
    fn regime_follows_the_cut_off_dates() {
        let t = |y, m, d| Utc.with_ymd_and_hms(y, m, d, 0, 0, 0).unwrap();
        assert_eq!(Regime::of(t(2023, 2, 28)), Regime::A);
        assert_eq!(Regime::of(t(2023, 3, 1)), Regime::B);
        assert_eq!(Regime::of(t(2024, 2, 29)), Regime::B);
        assert_eq!(Regime::of(t(2024, 3, 1)), Regime::C);
    }

    #[test]
    fn object_id_time_reads_the_leading_seconds() {
        let t = object_id_time("65000000aaaaaaaaaaaaaaaa").unwrap();
        assert_eq!(t.timestamp(), 0x6500_0000);
        assert_eq!(object_id_time("xyz"), None);
    }

    #[test]
    fn labels_are_unique_fixed_and_padded() {
        let values = ["b", "a", "c", "a"];
        let first = random_labels(values, "U", &mut seeded_rng(7));
        let second = random_labels(values, "U", &mut seeded_rng(7));
        assert_eq!(first, second);
        assert_eq!(first.len(), 3);
        let mut labels: Vec<&String> = first.values().collect();
        labels.sort();
        assert_eq!(labels, ["U1", "U2", "U3"]);
    }

    #[test]
    fn commas_group_by_three() {
        assert_eq!(with_commas(7), "7");
        assert_eq!(with_commas(1234), "1,234");
        assert_eq!(with_commas(40466), "40,466");
        assert_eq!(with_commas(1000000), "1,000,000");
    }
}
