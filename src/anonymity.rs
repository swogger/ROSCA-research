//! Rules that make the user attributes coarse enough to share.
//!
//! Two steps are applied. First, every attribute is reduced to a small
//! set of broad categories. Second, a k-anonymity rule is enforced on
//! the attributes that an outsider could plausibly know about a person.
//! k-anonymity means that every combination of these attributes that
//! occurs in the table is shared by at least k people.

use std::collections::HashMap;

/// Smallest number of people that may share a combination of attributes.
pub const K: usize = 5;

/// Value written in place of an attribute removed by the k-anonymity rule.
pub const SUPPRESSED: &str = "suppressed";

/// Counters are capped at this value. The cap stands for "this many or more".
pub const COUNT_CAP: u32 = 5;

/// One spelling per city. Records spell the same city in more than one
/// way, so without this a city could be counted twice. `aliases` maps each
/// variant spelling to the canonical one. It is read from a private file
/// kept with the raw data, so that no place names appear in the code.
pub fn canonical_city<'a>(raw: &'a str, aliases: &'a HashMap<String, String>) -> &'a str {
    let trimmed = raw.trim();
    aliases.get(trimmed).map(String::as_str).unwrap_or(trimmed)
}

/// Drop the currency code from an income label, so that the bands are
/// written without it. A currency code is a word of three capital letters.
fn without_currency_code(raw: &str) -> String {
    raw.split_whitespace()
        .filter(|w| !(w.len() == 3 && w.chars().all(|c| c.is_ascii_uppercase())))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Monthly income band in local currency units. The two bands between
/// 20,000 and 60,000 are merged. An unknown value gives `None`, so that a
/// new category is never passed through unchecked.
pub fn income_band(raw: &str) -> Option<&'static str> {
    Some(match without_currency_code(raw).as_str() {
        "2000 and below" => "2000 and below",
        "2000 to 5000" => "2000 to 5000",
        "5000 to 10000" => "5000 to 10000",
        "10000 to 20000" => "10000 to 20000",
        "20000 to 40000" | "40000 to 60000" => "20000 to 60000",
        "60000 and above" => "60000 and above",
        "" => "not stated",
        _ => return None,
    })
}

/// Employment sector. Small and sensitive categories are merged.
pub fn sector_group(raw: &str) -> Option<&'static str> {
    Some(match raw.trim() {
        "Private Sector" => "private sector",
        "Government Sector" => "government sector",
        "Military" => "military",
        "Home maker" => "home maker",
        "Student" => "student",
        "Self Employed" => "self employed",
        "Retired" => "retired",
        "Unemployed" => "unemployed",
        "Other" | "Social Security" | "Citizen account" | "Household Labor" | "" => {
            "other or not stated"
        }
        _ => return None,
    })
}

/// Income source, reduced to salary against everything else.
pub fn source_group(raw: &str) -> Option<&'static str> {
    Some(match raw.trim() {
        "Salary" => "salary",
        "Other" | "Investment Return" | "Rental Income" | "Ancestral" | "Real Estate" | "" => {
            "other or not stated"
        }
        _ => return None,
    })
}

/// Registration year, with the first two years merged.
pub fn register_period(year: i32) -> String {
    if year <= 2021 {
        "2021 or earlier".to_string()
    } else {
        year.to_string()
    }
}

/// Cap a counter at `COUNT_CAP`. An empty value stays empty.
pub fn cap_count(raw: &str) -> Option<String> {
    let text = raw.trim();
    if text.is_empty() {
        return Some(String::new());
    }
    let value: f64 = text.parse().ok()?;
    if value < 0.0 {
        return None;
    }
    Some((value.round() as u32).min(COUNT_CAP).to_string())
}

/// Round an amount to the nearest `step` and cap it at `cap`.
pub fn round_amount(raw: &str, step: f64, cap: f64) -> Option<String> {
    let text = raw.trim();
    if text.is_empty() {
        return Some(String::new());
    }
    let value: f64 = text.parse().ok()?;
    Some((((value / step).round() * step).min(cap)).to_string())
}

/// Result of the k-anonymity step.
#[derive(Debug, PartialEq, Eq)]
pub struct KReport {
    /// Number of values removed in each column, in table column order.
    pub suppressed: Vec<usize>,
    /// Size of the smallest combination left in the table.
    pub smallest_group: usize,
}

fn group_sizes(rows: &[Vec<String>]) -> HashMap<&[String], usize> {
    let mut sizes: HashMap<&[String], usize> = HashMap::new();
    for row in rows {
        *sizes.entry(row.as_slice()).or_default() += 1;
    }
    sizes
}

/// Enforce k-anonymity by removing values.
///
/// Each row holds the attributes of one person. The columns listed in
/// `order` are visited one at a time. In each pass, every person whose
/// combination of attributes is shared by fewer than `k` people loses
/// the value in that column. A combination that is already large enough
/// is never changed, because all people in a small combination are
/// treated together.
pub fn enforce_k_anonymity(rows: &mut [Vec<String>], order: &[usize], k: usize) -> KReport {
    let width = rows.first().map_or(0, Vec::len);
    let mut suppressed = vec![0; width];

    for &column in order {
        let small: Vec<bool> = {
            let sizes = group_sizes(rows);
            rows.iter().map(|row| sizes[row.as_slice()] < k).collect()
        };
        for (row, is_small) in rows.iter_mut().zip(small) {
            if is_small && row[column] != SUPPRESSED {
                row[column] = SUPPRESSED.to_string();
                suppressed[column] += 1;
            }
        }
    }

    merge_leftovers(rows, k, &mut suppressed);

    let smallest_group = group_sizes(rows).values().copied().min().unwrap_or(0);
    KReport { suppressed, smallest_group }
}

/// Remove every value in a row, and count what was removed.
fn suppress_row(row: &mut [String], suppressed: &mut [usize]) {
    for (value, count) in row.iter_mut().zip(suppressed.iter_mut()) {
        if value != SUPPRESSED {
            *value = SUPPRESSED.to_string();
            *count += 1;
        }
    }
}

fn is_fully_suppressed(row: &[String]) -> bool {
    row.iter().all(|value| value == SUPPRESSED)
}

/// Deal with the few people who are still in a small combination after
/// all columns have been visited.
///
/// These people lose all their values. If fewer than `k` people have
/// lost all values, a few more people are taken from one large
/// combination and treated the same way. The donor combination is the
/// one that has already lost the most values, and it must stay at `k`
/// people or more.
fn merge_leftovers(rows: &mut [Vec<String>], k: usize, suppressed: &mut [usize]) {
    loop {
        let small: Vec<bool> = {
            let sizes = group_sizes(rows);
            rows.iter().map(|row| sizes[row.as_slice()] < k).collect()
        };
        if !small.iter().any(|&s| s) {
            return;
        }
        for (row, is_small) in rows.iter_mut().zip(&small) {
            if *is_small {
                suppress_row(row, suppressed);
            }
        }

        let n_full = rows.iter().filter(|row| is_fully_suppressed(row)).count();
        if n_full >= k {
            continue;
        }
        let need = k - n_full;

        let donor: Option<Vec<String>> = {
            let sizes = group_sizes(rows);
            let mut candidates: Vec<(&[String], usize)> = sizes
                .into_iter()
                .filter(|(key, size)| !is_fully_suppressed(key) && *size >= k + need)
                .collect();
            // Most values already removed first, then the largest, then a
            // fixed order so that the result does not depend on chance.
            candidates.sort_by(|a, b| {
                let removed = |key: &[String]| key.iter().filter(|v| *v == SUPPRESSED).count();
                removed(b.0).cmp(&removed(a.0)).then(b.1.cmp(&a.1)).then(a.0.cmp(b.0))
            });
            candidates.first().map(|(key, _)| key.to_vec())
        };
        let Some(donor) = donor else {
            return; // The report will show that k was not reached.
        };
        let mut taken = 0;
        for row in rows.iter_mut() {
            if taken < need && *row == donor {
                suppress_row(row, suppressed);
                taken += 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(values: [&str; 3]) -> Vec<String> {
        values.iter().map(|v| v.to_string()).collect()
    }

    #[test]
    fn large_groups_are_left_alone_and_small_ones_lose_values() {
        // Five people share (a, x, p). Two people differ only in the last column.
        let mut rows: Vec<Vec<String>> = (0..5).map(|_| row(["a", "x", "p"])).collect();
        rows.push(row(["a", "x", "q"]));
        rows.push(row(["a", "x", "r"]));
        // Three more people make the suppressed group large enough.
        rows.extend((0..3).map(|i| row(["a", "x", ["s", "t", "u"][i]])));

        let report = enforce_k_anonymity(&mut rows, &[2, 1, 0], 5);

        assert_eq!(rows[0], row(["a", "x", "p"]));
        assert_eq!(rows[5], row(["a", "x", SUPPRESSED]));
        assert_eq!(report.suppressed, vec![0, 0, 5]);
        assert_eq!(report.smallest_group, 5);
    }

    #[test]
    fn suppression_moves_to_the_next_column_when_needed() {
        let mut rows: Vec<Vec<String>> = (0..5).map(|_| row(["a", "x", "p"])).collect();
        rows.extend((0..3).map(|_| row(["a", "y", "q"])));
        rows.extend((0..2).map(|_| row(["a", "z", "r"])));

        let report = enforce_k_anonymity(&mut rows, &[2, 1, 0], 5);

        assert_eq!(rows[5], row(["a", SUPPRESSED, SUPPRESSED]));
        assert_eq!(rows[9], row(["a", SUPPRESSED, SUPPRESSED]));
        assert_eq!(report.suppressed, vec![0, 5, 5]);
        assert_eq!(report.smallest_group, 5);
    }

    #[test]
    fn the_report_shows_when_k_cannot_be_reached() {
        let mut rows: Vec<Vec<String>> = (0..5).map(|_| row(["a", "x", "p"])).collect();
        rows.push(row(["b", "y", "q"]));

        let report = enforce_k_anonymity(&mut rows, &[2, 1, 0], 5);

        assert_eq!(rows[5], row([SUPPRESSED, SUPPRESSED, SUPPRESSED]));
        assert_eq!(report.smallest_group, 1);
    }

    #[test]
    fn leftovers_are_merged_with_people_from_a_large_group() {
        let mut rows: Vec<Vec<String>> = (0..10).map(|_| row(["a", "x", "p"])).collect();
        rows.push(row(["b", "y", "q"]));

        let report = enforce_k_anonymity(&mut rows, &[2, 1, 0], 5);

        // The one leftover person and four donors lose all values.
        let full = rows.iter().filter(|r| is_fully_suppressed(r)).count();
        assert_eq!(full, 5);
        assert_eq!(rows.iter().filter(|r| **r == row(["a", "x", "p"])).count(), 6);
        assert_eq!(report.suppressed, vec![5, 5, 5]);
        assert_eq!(report.smallest_group, 5);
    }

    #[test]
    fn categories_are_merged_and_unknown_values_are_rejected() {
        assert_eq!(income_band("XYZ 40000 to XYZ 60000"), Some("20000 to 60000"));
        assert_eq!(income_band("2000 and below"), Some("2000 and below"));
        assert_eq!(income_band(""), Some("not stated"));
        assert_eq!(income_band("a new band"), None);
        assert_eq!(sector_group("Social Security"), Some("other or not stated"));
        assert_eq!(sector_group("Military"), Some("military"));
        assert_eq!(sector_group("a new sector"), None);
        assert_eq!(source_group("Rental Income"), Some("other or not stated"));
        assert_eq!(source_group("a new source"), None);
        assert_eq!(register_period(2020), "2021 or earlier");
        assert_eq!(register_period(2023), "2023");
    }

    #[test]
    fn counters_are_capped_and_amounts_are_rounded() {
        assert_eq!(cap_count("3").as_deref(), Some("3"));
        assert_eq!(cap_count("11").as_deref(), Some("5"));
        assert_eq!(cap_count("").as_deref(), Some(""));
        assert_eq!(cap_count("x"), None);
        assert_eq!(round_amount("199", 100.0, 5000.0).as_deref(), Some("200"));
        assert_eq!(round_amount("1234.56", 100.0, 5000.0).as_deref(), Some("1200"));
        assert_eq!(round_amount("18905", 100.0, 5000.0).as_deref(), Some("5000"));
    }

    #[test]
    fn city_spellings_are_merged() {
        let aliases: HashMap<String, String> =
            [("Town A".to_string(), "town a".to_string())].into_iter().collect();
        assert_eq!(canonical_city("Town A", &aliases), canonical_city("town a", &aliases));
        assert_eq!(canonical_city(" Town A ", &aliases), "town a");
        assert_eq!(canonical_city("Town B", &aliases), "Town B");
    }
}
