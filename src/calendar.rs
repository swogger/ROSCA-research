//! Payout dates and the holiday indicators of the Paper 2 analysis plan
//! (section 4).
//!
//! Seat k of a group is paid on the start date plus k - 1 months. The
//! indicators say whether a payout falls in the days before Eid al-Fitr,
//! Eid al-Adha or the first school day. Only the indicators are written to
//! the anonymised tables, never the exact date.
//!
//! The dates below are public. They follow the Eid dates officially
//! announced in the platform's market, and the first public school day as
//! reported by the press. The plan's table lists the same dates.

use chrono::{DateTime, Duration, Months, NaiveDate, Utc};

/// Local time is three hours ahead of UTC, with no daylight saving.
const LOCAL_OFFSET_HOURS: i64 = 3;

/// The dated expenses of the plan.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    EidAlFitr,
    EidAlAdha,
    SchoolStart,
}

/// (year, month, day) of each event, 2021 to 2026.
const EID_AL_FITR: [(i32, u32, u32); 6] =
    [(2021, 5, 13), (2022, 5, 2), (2023, 4, 21), (2024, 4, 10), (2025, 3, 30), (2026, 3, 20)];
const EID_AL_ADHA: [(i32, u32, u32); 6] =
    [(2021, 7, 20), (2022, 7, 9), (2023, 6, 28), (2024, 6, 16), (2025, 6, 6), (2026, 5, 27)];
const SCHOOL_START: [(i32, u32, u32); 5] =
    [(2021, 8, 29), (2022, 8, 28), (2023, 8, 20), (2024, 8, 18), (2025, 8, 24)];

/// Payout dates must fall inside this range, so that every window of up
/// to 60 days before a payout's next event is covered by the tables.
pub fn first_covered_payout() -> NaiveDate {
    NaiveDate::from_ymd_opt(2021, 1, 1).unwrap()
}
pub fn last_covered_payout() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 5, 1).unwrap()
}

impl Event {
    fn dates(self) -> impl Iterator<Item = NaiveDate> {
        let table: &[(i32, u32, u32)] = match self {
            Event::EidAlFitr => &EID_AL_FITR,
            Event::EidAlAdha => &EID_AL_ADHA,
            Event::SchoolStart => &SCHOOL_START,
        };
        table.iter().map(|&(y, m, d)| NaiveDate::from_ymd_opt(y, m, d).unwrap())
    }
}

/// Local calendar date of a timestamp stored in UTC.
pub fn local_date(t: DateTime<Utc>) -> NaiveDate {
    (t + Duration::hours(LOCAL_OFFSET_HOURS)).date_naive()
}

/// Date on which seat `seat` is paid: the start date plus seat - 1 months.
/// A start on the 31st is paid on the last day of shorter months.
pub fn payout_date(start: NaiveDate, seat: u32) -> NaiveDate {
    start
        .checked_add_months(Months::new(seat - 1))
        .expect("payout date out of range")
}

/// True when the payout falls 1 to `window_days` days before the event.
/// A payout on the day of the event does not count.
pub fn pays_before(payout: NaiveDate, event: Event, window_days: i64) -> bool {
    event.dates().any(|day| {
        let lead = (day - payout).num_days();
        (1..=window_days).contains(&lead)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    #[test]
    fn local_date_moves_late_utc_times_to_the_next_day() {
        let midnight = Utc.with_ymd_and_hms(2023, 3, 1, 0, 0, 0).unwrap();
        assert_eq!(local_date(midnight), date(2023, 3, 1));
        let late = Utc.with_ymd_and_hms(2023, 2, 28, 21, 0, 0).unwrap();
        assert_eq!(local_date(late), date(2023, 3, 1));
    }

    #[test]
    fn seat_k_is_paid_k_minus_one_months_after_the_start() {
        assert_eq!(payout_date(date(2023, 1, 15), 1), date(2023, 1, 15));
        assert_eq!(payout_date(date(2023, 1, 15), 4), date(2023, 4, 15));
        assert_eq!(payout_date(date(2023, 1, 31), 2), date(2023, 2, 28));
    }

    #[test]
    fn windows_count_the_days_before_the_event_only() {
        // Eid al-Fitr 2023 fell on 21 April.
        assert!(pays_before(date(2023, 4, 20), Event::EidAlFitr, 45));
        assert!(!pays_before(date(2023, 4, 21), Event::EidAlFitr, 45));
        assert!(!pays_before(date(2023, 4, 25), Event::EidAlFitr, 45));
        assert!(pays_before(date(2023, 3, 7), Event::EidAlFitr, 45));
        assert!(!pays_before(date(2023, 3, 6), Event::EidAlFitr, 45));
        assert!(!pays_before(date(2023, 3, 7), Event::EidAlFitr, 30));
        // Eid al-Adha 2024 fell on 16 June; school 2024 opened on 18 August.
        assert!(pays_before(date(2024, 6, 1), Event::EidAlAdha, 30));
        assert!(pays_before(date(2024, 8, 1), Event::SchoolStart, 30));
        assert!(!pays_before(date(2024, 6, 1), Event::SchoolStart, 60));
    }
}
