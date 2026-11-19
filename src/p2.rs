//! Data access and shared definitions for the Paper 2 programs.
//!
//! Every Paper 2 program reads the anonymised tables only
//! (`data/anonymised/`). Definitions follow the frozen plan
//! (`private/analysis-plan-paper-2.md`) and its clarifications in
//! `private/notes/deviations-paper-2.md`.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use anyhow::{bail, Context, Result};

/// Calendar month as a single number: year × 12 + (month - 1).
pub type Month = i32;

pub fn parse_month(text: &str) -> Option<Month> {
    let (y, m) = text.trim().split_once('-')?;
    let (y, m): (i32, i32) = (y.parse().ok()?, m.parse().ok()?);
    (1..=12).contains(&m).then_some(y * 12 + m - 1)
}

pub fn month_label(m: Month) -> String {
    format!("{}-{:02}", m.div_euclid(12), m.rem_euclid(12) + 1)
}

/// Month of the year, 1 to 12.
pub fn month_of_year(m: Month) -> usize {
    (m.rem_euclid(12) + 1) as usize
}

/// First month of regime B and of regime C, as fixed by the plan.
pub fn b_start() -> Month {
    parse_month("2023-03").unwrap()
}
pub fn c_start() -> Month {
    parse_month("2024-03").unwrap()
}

/// Fee regime of a group from its start month. `shift` moves both
/// cut-offs by that many months (plan section 10).
pub fn regime_of(start: Month, shift: i32) -> char {
    if start < b_start() + shift {
        'A'
    } else if start < c_start() + shift {
        'B'
    } else {
        'C'
    }
}

/// Payout band of clarification 1.
pub fn band(payout: f64) -> &'static str {
    if payout < 5000.0 {
        "under 5000"
    } else if payout < 10000.0 {
        "5000-9999"
    } else if payout < 20000.0 {
        "10000-19999"
    } else {
        "20000 and over"
    }
}

#[derive(Debug, Clone)]
pub struct Member {
    pub user: String,
    pub group: String,
    /// Regime as written in the tables (cut-offs of the plan).
    pub regime: char,
    pub seat: u32,
    pub n: u32,
    pub payout: f64,
    pub monthly: f64,
    pub fee: f64,
    pub start_month: Month,
    pub paid_month: Option<Month>,
    pub days_paid_before_start: Option<i64>,
    pub join_rank: Option<u32>,
    pub usable: bool,
    pub join_order_complete: bool,
    pub payout_month: Month,
    pub fitr45: bool,
    pub adha30: bool,
    pub school30: bool,
    pub fitr30: bool,
    pub fitr60: bool,
    pub adha60: bool,
    pub school60: bool,
}

impl Member {
    /// Payout amount as an integer key.
    pub fn payout_key(&self) -> u64 {
        self.payout.round() as u64
    }
}

#[derive(Debug, Clone)]
pub struct Group {
    pub group: String,
    pub regime: char,
    pub n: u32,
    pub payout: f64,
    pub monthly: f64,
    pub start_month: Month,
    pub days_created_before_start: Option<i64>,
    pub n_observed: u32,
    pub fully_observed: bool,
    pub usable: bool,
}

#[derive(Debug, Clone)]
pub struct User {
    pub user: String,
    pub gender: String,
    pub age_band: String,
    pub income_level: String,
    pub employment_sector: String,
}

fn reader(path: &Path) -> Result<(csv::Reader<std::fs::File>, HashMap<String, usize>)> {
    let mut r = csv::ReaderBuilder::new()
        .delimiter(b'\t')
        .from_path(path)
        .with_context(|| format!("cannot open {}", path.display()))?;
    let cols = r.headers()?.iter().enumerate().map(|(i, h)| (h.to_string(), i)).collect();
    Ok((r, cols))
}

fn col(cols: &HashMap<String, usize>, name: &str) -> Result<usize> {
    cols.get(name).copied().with_context(|| format!("column {name} missing"))
}

fn flag(text: &str) -> Result<bool> {
    match text {
        "true" => Ok(true),
        "false" => Ok(false),
        other => bail!("not a yes/no value: {other}"),
    }
}

fn opt_i64(text: &str) -> Result<Option<i64>> {
    if text.trim().is_empty() {
        Ok(None)
    } else {
        Ok(Some(text.trim().parse()?))
    }
}

pub fn read_members(dir: &Path) -> Result<Vec<Member>> {
    let (mut r, c) = reader(&dir.join("memberships.tsv"))?;
    let names = [
        "user", "group", "regime", "seat", "n_cycles", "total_amount", "monthly_amount",
        "fee_amount", "start_month", "fee_paid_month", "days_paid_before_start", "join_rank",
        "usable_for_choice", "join_order_complete", "payout_month", "pay_before_eid_fitr",
        "pay_before_eid_adha", "pay_before_school", "pay_before_eid_fitr_30",
        "pay_before_eid_fitr_60", "pay_before_eid_adha_60", "pay_before_school_60",
    ];
    let i: Vec<usize> = names.iter().map(|n| col(&c, n)).collect::<Result<_>>()?;
    let mut out = Vec::new();
    for rec in r.records() {
        let rec = rec?;
        out.push(Member {
            user: rec[i[0]].to_string(),
            group: rec[i[1]].to_string(),
            regime: rec[i[2]].chars().next().context("empty regime")?,
            seat: rec[i[3]].parse()?,
            n: rec[i[4]].parse()?,
            payout: rec[i[5]].parse()?,
            monthly: rec[i[6]].parse()?,
            fee: rec[i[7]].parse()?,
            start_month: parse_month(&rec[i[8]]).context("bad start month")?,
            paid_month: parse_month(&rec[i[9]]),
            days_paid_before_start: opt_i64(&rec[i[10]])?,
            join_rank: opt_i64(&rec[i[11]])?.map(|v| v as u32),
            usable: flag(&rec[i[12]])?,
            join_order_complete: flag(&rec[i[13]])?,
            payout_month: parse_month(&rec[i[14]]).context("bad payout month")?,
            fitr45: flag(&rec[i[15]])?,
            adha30: flag(&rec[i[16]])?,
            school30: flag(&rec[i[17]])?,
            fitr30: flag(&rec[i[18]])?,
            fitr60: flag(&rec[i[19]])?,
            adha60: flag(&rec[i[20]])?,
            school60: flag(&rec[i[21]])?,
        });
    }
    Ok(out)
}

pub fn read_groups(dir: &Path) -> Result<Vec<Group>> {
    let (mut r, c) = reader(&dir.join("groups.tsv"))?;
    let names = [
        "group", "regime", "n_cycles", "total_amount", "monthly_amount", "start_month",
        "days_created_before_start", "n_observed", "fully_observed", "usable_for_choice",
    ];
    let i: Vec<usize> = names.iter().map(|n| col(&c, n)).collect::<Result<_>>()?;
    let mut out = Vec::new();
    for rec in r.records() {
        let rec = rec?;
        out.push(Group {
            group: rec[i[0]].to_string(),
            regime: rec[i[1]].chars().next().context("empty regime")?,
            n: rec[i[2]].parse()?,
            payout: rec[i[3]].parse()?,
            monthly: rec[i[4]].parse()?,
            start_month: parse_month(&rec[i[5]]).context("bad start month")?,
            days_created_before_start: opt_i64(&rec[i[6]])?,
            n_observed: rec[i[7]].parse()?,
            fully_observed: flag(&rec[i[8]])?,
            usable: flag(&rec[i[9]])?,
        });
    }
    Ok(out)
}

pub fn read_users(dir: &Path) -> Result<Vec<User>> {
    let (mut r, c) = reader(&dir.join("users.tsv"))?;
    let names = ["user", "gender", "age_band", "income_level", "employment_sector"];
    let i: Vec<usize> = names.iter().map(|n| col(&c, n)).collect::<Result<_>>()?;
    let mut out = Vec::new();
    for rec in r.records() {
        let rec = rec?;
        out.push(User {
            user: rec[i[0]].to_string(),
            gender: rec[i[1]].to_string(),
            age_band: rec[i[2]].to_string(),
            income_level: rec[i[3]].to_string(),
            employment_sector: rec[i[4]].to_string(),
        });
    }
    Ok(out)
}

/// Key of a posted fee: payment month, length, payout, seat.
pub type FeeKey = (Month, u32, u64, u32);

/// Posted fees (plan section 5): the most common fee paid for the same
/// payment month, length, payout and seat. Ties go to the lower fee.
/// Also returns the cells in which more than one distinct fee was paid.
pub struct Posted {
    pub fee: HashMap<FeeKey, f64>,
    pub multi: HashMap<FeeKey, bool>,
}

impl Posted {
    pub fn build(members: &[Member]) -> Posted {
        let mut counts: HashMap<FeeKey, BTreeMap<u64, usize>> = HashMap::new();
        for m in members {
            let Some(pm) = m.paid_month else { continue };
            let key = (pm, m.n, m.payout_key(), m.seat);
            *counts.entry(key).or_default().entry((m.fee * 100.0).round() as u64).or_default() +=
                1;
        }
        let mut fee = HashMap::new();
        let mut multi = HashMap::new();
        for (key, fees) in counts {
            let top = fees.values().max().copied().unwrap_or(0);
            let f = fees.iter().find(|(_, &c)| c == top).map(|(&f, _)| f).unwrap();
            fee.insert(key, f as f64 / 100.0);
            multi.insert(key, fees.len() > 1);
        }
        Posted { fee, multi }
    }

    pub fn get(&self, month: Month, n: u32, payout: u64, seat: u32) -> Option<f64> {
        self.fee.get(&(month, n, payout, seat)).copied()
    }

    /// Posted fee of a member's own seat at their payment month.
    pub fn of(&self, m: &Member) -> Option<f64> {
        self.get(m.paid_month?, m.n, m.payout_key(), m.seat)
    }

    /// Whether a member's fee cell held more than one fee.
    pub fn is_multi(&self, m: &Member) -> bool {
        m.paid_month
            .and_then(|pm| self.multi.get(&(pm, m.n, m.payout_key(), m.seat)).copied())
            .unwrap_or(false)
    }
}

/// Memberships of each group, in table order.
pub fn by_group(members: &[Member]) -> BTreeMap<&str, Vec<&Member>> {
    let mut out: BTreeMap<&str, Vec<&Member>> = BTreeMap::new();
    for m in members {
        out.entry(m.group.as_str()).or_default().push(m);
    }
    out
}

/// Groups in which some member's fee cell held more than one fee
/// (plan section 10).
pub fn multi_fee_groups<'a>(members: &'a [Member], posted: &Posted) -> std::collections::HashSet<&'a str> {
    members.iter().filter(|m| posted.is_multi(m)).map(|m| m.group.as_str()).collect()
}

/// Weights that give regimes A and B the start-month mix of regime C
/// (clarification of plan section 10). Groups whose month of the year
/// does not occur in regime C get weight 0. Weights average 1 within each
/// regime among the groups kept. Keyed by group.
pub fn start_month_weights(groups: &[(String, char, Month)]) -> HashMap<String, f64> {
    let mut count: HashMap<(char, usize), f64> = HashMap::new();
    let mut total: HashMap<char, f64> = HashMap::new();
    for (_, r, m) in groups {
        *count.entry((*r, month_of_year(*m))).or_default() += 1.0;
        *total.entry(*r).or_default() += 1.0;
    }
    let c_total = total.get(&'C').copied().unwrap_or(0.0);
    let target = |moy: usize| count.get(&('C', moy)).copied().unwrap_or(0.0) / c_total;
    let mut kept: HashMap<char, f64> = HashMap::new();
    for (_, r, m) in groups {
        if target(month_of_year(*m)) > 0.0 {
            *kept.entry(*r).or_default() += 1.0;
        }
    }
    groups
        .iter()
        .map(|(g, r, m)| {
            let moy = month_of_year(*m);
            let t = target(moy);
            let w = if t > 0.0 {
                let own = count[&(*r, moy)] / kept[r];
                t / own
            } else {
                0.0
            };
            (g.clone(), w)
        })
        .collect()
}

/// Net present value of a seat's cash flows at monthly rate r: fee at
/// month 0, instalment m at months 0 to N - 1, payout at month k - 1.
pub fn seat_npv(r: f64, seat: u32, n: u32, payout: f64, monthly: f64, fee: f64) -> f64 {
    (0..n)
        .map(|t| {
            let mut flow = -monthly;
            if t == seat - 1 {
                flow += payout;
            }
            if t == 0 {
                flow -= fee;
            }
            flow / (1.0 + r).powi(t as i32)
        })
        .sum()
}

/// Seat rate (measure 3), as in `p2_price`: reported only when the net
/// present value changes sign exactly once on the grid -0.5 to 0.5.
pub fn seat_rate(seat: u32, n: u32, payout: f64, monthly: f64, fee: f64) -> Option<f64> {
    if fee == 0.0 {
        return Some(0.0);
    }
    let npv = |r| seat_npv(r, seat, n, payout, monthly, fee);
    let grid: Vec<f64> = (0..=400).map(|i| -0.5 + f64::from(i) / 400.0).collect();
    let mut brackets = grid.windows(2).filter(|w| npv(w[0]).signum() != npv(w[1]).signum());
    let (mut lo, mut hi) = match (brackets.next(), brackets.next()) {
        (Some(w), None) => (w[0], w[1]),
        _ => return None,
    };
    for _ in 0..100 {
        let mid = 0.5 * (lo + hi);
        if npv(lo).signum() == npv(mid).signum() {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    Some(0.5 * (lo + hi))
}

/// Write a tab-separated table.
pub fn write_tsv(path: &Path, header: &[&str], rows: &[Vec<String>]) -> Result<()> {
    let mut w = csv::WriterBuilder::new().delimiter(b'\t').from_path(path)?;
    w.write_record(header)?;
    for r in rows {
        w.write_record(r)?;
    }
    w.flush()?;
    Ok(())
}

pub fn f(v: f64, digits: usize) -> String {
    if v.is_finite() {
        format!("{v:.digits$}")
    } else {
        String::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn months_round_trip_and_regimes_shift() {
        let m = parse_month("2023-03").unwrap();
        assert_eq!(month_label(m), "2023-03");
        assert_eq!(month_of_year(m), 3);
        assert_eq!(month_label(m - 3), "2022-12");
        assert_eq!(regime_of(m, 0), 'B');
        assert_eq!(regime_of(m, 1), 'A');
        assert_eq!(regime_of(m - 1, -1), 'B');
        assert_eq!(regime_of(parse_month("2024-03").unwrap(), 0), 'C');
    }

    #[test]
    fn start_month_weights_match_the_target_mix() {
        let g = |i: usize, r: char, m: &str| (format!("g{i}"), r, parse_month(m).unwrap());
        let groups = vec![
            g(1, 'C', "2024-03"),
            g(2, 'C', "2024-04"),
            g(3, 'A', "2022-03"),
            g(4, 'A', "2022-03"),
            g(5, 'A', "2022-03"),
            g(6, 'A', "2022-04"),
            g(7, 'A', "2022-08"),
        ];
        let w = start_month_weights(&groups);
        // A keeps four groups: March gets half the weight, April the other half.
        assert!((w["g3"] + w["g4"] + w["g5"] - 2.0).abs() < 1e-12);
        assert!((w["g6"] - 2.0).abs() < 1e-12);
        assert_eq!(w["g7"], 0.0);
        assert!((w["g1"] - 1.0).abs() < 1e-12);
    }
}
