//! Paper 2, section 5: the price of an earlier payout.
//!
//! Reads the anonymised memberships table and writes aggregate tables only.
//!
//! Definitions (frozen plan, section 5):
//!   * posted fee of a seat: the most common fee paid for the same payment
//!     month, group length, payout amount and seat;
//!   * measure 1, price of going first: the monthly rate r with
//!         P * (1 - (1 + r)^-(N - 1)) = f_1 - f_N;
//!   * measure 2, price of one month: (f_k - f_(k+1)) / P, from the median
//!     posted schedule;
//!   * measure 3, seat rate: monthly internal rate of return of the seat's
//!     cash flows, fee at month 0, instalments at months 0..N-1, payout at
//!     month k - 1.
//!
//! Usage:
//!     cargo run --release --bin p2_price -- --data data/anonymised --out private/results/paper-2

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::Parser;
use rand::Rng;

use breaking_binary::seeded_rng;

/// Seed of the bootstrap, fixed so that the intervals are reproducible.
const BOOTSTRAP_SEED: u64 = 2026_11_19;
const BOOTSTRAP_DRAWS: usize = 1000;
/// Lengths reported on their own (clarification 1).
const REPORTED_LENGTHS: [u32; 6] = [4, 5, 6, 8, 10, 12];

#[derive(Parser)]
struct Args {
    #[arg(long, default_value = "data/anonymised")]
    data: PathBuf,
    #[arg(long, default_value = "private/results/paper-2")]
    out: PathBuf,
}

struct Row {
    group: String,
    regime: String,
    seat: u32,
    n: u32,
    payout: f64,
    monthly: f64,
    fee: f64,
    paid_month: String,
}

fn read_rows(path: &Path) -> Result<Vec<Row>> {
    let mut reader = csv::ReaderBuilder::new()
        .delimiter(b'\t')
        .from_path(path)
        .with_context(|| format!("cannot open {}", path.display()))?;
    let headers = reader.headers()?.clone();
    let col = |name: &str| {
        headers
            .iter()
            .position(|h| h == name)
            .with_context(|| format!("column {name} missing"))
    };
    let (g, r, s, n) = (col("group")?, col("regime")?, col("seat")?, col("n_cycles")?);
    let (p, m, f, pm) =
        (col("total_amount")?, col("monthly_amount")?, col("fee_amount")?, col("fee_paid_month")?);
    let mut rows = Vec::new();
    for record in reader.records() {
        let record = record?;
        rows.push(Row {
            group: record[g].to_string(),
            regime: record[r].to_string(),
            seat: record[s].parse()?,
            n: record[n].parse()?,
            payout: record[p].parse()?,
            monthly: record[m].parse()?,
            fee: record[f].parse()?,
            paid_month: record[pm].to_string(),
        });
    }
    Ok(rows)
}

/// Payout band in local currency units (clarification 1).
fn band(payout: f64) -> &'static str {
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

/// Most common fee per (payment month, length, payout, seat). Ties go to the
/// lower fee, so that the choice does not depend on the order of the rows.
fn posted_fees(rows: &[Row]) -> HashMap<(String, u32, u64, u32), f64> {
    let mut counts: HashMap<(String, u32, u64, u32), BTreeMap<u64, usize>> = HashMap::new();
    for r in rows.iter().filter(|r| !r.paid_month.is_empty()) {
        let key = (r.paid_month.clone(), r.n, r.payout.round() as u64, r.seat);
        // Fees are kept in hundredths of a currency unit to count them exactly.
        *counts.entry(key).or_default().entry((r.fee * 100.0).round() as u64).or_default() += 1;
    }
    counts
        .into_iter()
        .map(|(key, fees)| {
            let top = fees.values().max().copied().unwrap_or(0);
            let fee = fees.iter().find(|(_, &c)| c == top).map(|(&f, _)| f).unwrap();
            (key, fee as f64 / 100.0)
        })
        .collect()
}

/// Measure 1. `None` when the gap is not below the payout.
fn price_of_going_first(f1: f64, f_last: f64, payout: f64, n: u32) -> Option<f64> {
    let share = 1.0 - (f1 - f_last) / payout;
    (share > 0.0 && n > 1).then(|| share.powf(-1.0 / f64::from(n - 1)) - 1.0)
}

/// Net present value of a seat's cash flows at monthly rate r.
fn seat_npv(r: f64, seat: u32, n: u32, payout: f64, monthly: f64, fee: f64) -> f64 {
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

/// Measure 3. The rate is reported only when the net present value changes
/// sign exactly once on the search grid, so that it is unique there.
fn seat_rate(seat: u32, n: u32, payout: f64, monthly: f64, fee: f64) -> Option<f64> {
    if fee == 0.0 {
        return Some(0.0);
    }
    let npv = |r| seat_npv(r, seat, n, payout, monthly, fee);
    let grid: Vec<f64> = (0..=400).map(|i| -0.5 + 1.0 * f64::from(i) / 400.0).collect();
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

/// Median with weights: the smallest value at which the cumulative weight
/// reaches half of the total.
fn weighted_median(values: &mut [(f64, f64)]) -> f64 {
    values.sort_by(|a, b| a.0.total_cmp(&b.0));
    let total: f64 = values.iter().map(|v| v.1).sum();
    let mut cumulative = 0.0;
    for &(value, weight) in values.iter() {
        cumulative += weight;
        if cumulative >= 0.5 * total {
            return value;
        }
    }
    f64::NAN
}

struct GroupPrice {
    regime: String,
    n: u32,
    band: &'static str,
    rate: f64,
    weight: f64,
}

fn percentile(sorted: &[f64], p: f64) -> f64 {
    let i = ((sorted.len() - 1) as f64 * p).round() as usize;
    sorted[i]
}

fn main() -> Result<()> {
    let args = Args::parse();
    std::fs::create_dir_all(&args.out)?;
    let rows = read_rows(&args.data.join("memberships.tsv"))?;
    let posted = posted_fees(&rows);
    let posted_of = |r: &Row| {
        posted.get(&(r.paid_month.clone(), r.n, r.payout.round() as u64, r.seat)).copied()
    };

    // Measure 1 per group, from the posted fees of seat 1 and seat N.
    let mut by_group: HashMap<&str, Vec<&Row>> = HashMap::new();
    for r in &rows {
        by_group.entry(r.group.as_str()).or_default().push(r);
    }
    let mut prices = Vec::new();
    let mut skipped = 0usize;
    for members in by_group.values() {
        let first = members.iter().find(|r| r.seat == 1).and_then(|r| posted_of(r));
        let last = members.iter().find(|r| r.seat == r.n).and_then(|r| posted_of(r));
        let any = members[0];
        match (first, last) {
            (Some(f1), Some(f_last)) => match price_of_going_first(f1, f_last, any.payout, any.n) {
                Some(rate) => prices.push(GroupPrice {
                    regime: any.regime.clone(),
                    n: any.n,
                    band: band(any.payout),
                    rate,
                    weight: members.len() as f64,
                }),
                None => skipped += 1,
            },
            _ => skipped += 1,
        }
    }

    let mut out = csv::WriterBuilder::new()
        .delimiter(b'\t')
        .from_path(args.out.join("measure1_price_of_going_first.tsv"))?;
    out.write_record(["regime", "split", "value", "groups", "weighted_median_pct_per_month"])?;
    let mut summary: BTreeMap<(String, String, String), Vec<(f64, f64)>> = BTreeMap::new();
    for p in &prices {
        let mut push = |split: &str, value: String| {
            summary
                .entry((p.regime.clone(), split.to_string(), value))
                .or_default()
                .push((p.rate, p.weight));
        };
        push("all", "all".into());
        if REPORTED_LENGTHS.contains(&p.n) {
            push("length", p.n.to_string());
        }
        push("payout band", p.band.into());
    }
    println!("Measure 1, price of going first (% per month, median weighted by memberships)");
    println!("groups without a usable seat 1 and last-seat fee: {skipped}");
    for ((regime, split, value), values) in &mut summary {
        let median = 100.0 * weighted_median(values);
        out.write_record([
            regime.as_str(),
            split,
            value,
            &values.len().to_string(),
            &format!("{median:.3}"),
        ])?;
        if split == "all" {
            println!("  regime {regime}: {median:.2}% per month over {} groups", values.len());
        }
    }
    out.flush()?;

    // Bootstrap over groups for the regime medians.
    let mut rng = seeded_rng(BOOTSTRAP_SEED);
    let mut ci = csv::WriterBuilder::new()
        .delimiter(b'\t')
        .from_path(args.out.join("measure1_bootstrap.tsv"))?;
    ci.write_record(["regime", "median", "ci_low", "ci_high", "draws"])?;
    for regime in ["A", "B", "C"] {
        let groups: Vec<&GroupPrice> = prices.iter().filter(|p| p.regime == regime).collect();
        if groups.is_empty() {
            continue;
        }
        let mut point: Vec<(f64, f64)> = groups.iter().map(|p| (p.rate, p.weight)).collect();
        let median = weighted_median(&mut point);
        let mut draws: Vec<f64> = (0..BOOTSTRAP_DRAWS)
            .map(|_| {
                let mut sample: Vec<(f64, f64)> = (0..groups.len())
                    .map(|_| {
                        let p = groups[rng.gen_range(0..groups.len())];
                        (p.rate, p.weight)
                    })
                    .collect();
                weighted_median(&mut sample)
            })
            .collect();
        draws.sort_by(f64::total_cmp);
        let (lo, hi) = (percentile(&draws, 0.025), percentile(&draws, 0.975));
        ci.write_record([
            regime,
            &format!("{:.3}", 100.0 * median),
            &format!("{:.3}", 100.0 * lo),
            &format!("{:.3}", 100.0 * hi),
            &BOOTSTRAP_DRAWS.to_string(),
        ])?;
        println!(
            "  regime {regime}: 95% interval {:.2}% to {:.2}% per month",
            100.0 * lo,
            100.0 * hi
        );
    }
    ci.flush()?;

    // Measures 2 and 3: posted schedule and seat rates by regime, length, seat.
    let mut cells: BTreeMap<(String, u32, u32), (Vec<(f64, f64)>, Vec<(f64, f64)>)> =
        BTreeMap::new();
    let mut no_rate = 0usize;
    for r in &rows {
        let Some(fee) = posted_of(r) else { continue };
        if !REPORTED_LENGTHS.contains(&r.n) {
            continue;
        }
        let cell = cells.entry((r.regime.clone(), r.n, r.seat)).or_default();
        cell.0.push((100.0 * fee / r.payout, 1.0));
        match seat_rate(r.seat, r.n, r.payout, r.monthly, fee) {
            Some(rate) => cell.1.push((100.0 * rate, 1.0)),
            None => no_rate += 1,
        }
    }
    let mut out = csv::WriterBuilder::new()
        .delimiter(b'\t')
        .from_path(args.out.join("measures2_3_schedule.tsv"))?;
    out.write_record([
        "regime", "length", "seat", "memberships", "median_posted_fee_pct",
        "price_of_one_month_pct", "median_seat_rate_pct_per_month", "seat_rates_defined",
    ])?;
    let medians: BTreeMap<(String, u32, u32), f64> = cells
        .iter_mut()
        .map(|(key, (fees, _))| (key.clone(), weighted_median(fees)))
        .collect();
    for ((regime, n, seat), (fees, rates)) in &mut cells {
        let next = medians.get(&(regime.clone(), *n, *seat + 1));
        let one_month = next
            .map(|f_next| format!("{:.3}", medians[&(regime.clone(), *n, *seat)] - f_next))
            .unwrap_or_default();
        let rate = if rates.is_empty() {
            String::new()
        } else {
            format!("{:.3}", weighted_median(rates))
        };
        out.write_record([
            regime.as_str(),
            &n.to_string(),
            &seat.to_string(),
            &fees.len().to_string(),
            &format!("{:.3}", medians[&(regime.clone(), *n, *seat)]),
            &one_month,
            &rate,
            &rates.len().to_string(),
        ])?;
    }
    out.flush()?;
    println!("seat rates not unique on the grid (left blank): {no_rate}");
    println!("tables written to {}", args.out.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_fee_gap_means_no_price_of_time() {
        assert_eq!(price_of_going_first(199.0, 199.0, 12000.0, 12), Some(0.0));
    }

    #[test]
    fn price_of_going_first_solves_its_equation() {
        let (f1, f_last, payout, n) = (1500.0, 20.0, 10000.0, 10);
        let r = price_of_going_first(f1, f_last, payout, n).unwrap();
        let value = payout * (1.0 - (1.0 + r).powi(-(n as i32 - 1)));
        assert!((value - (f1 - f_last)).abs() < 1e-6);
    }

    #[test]
    fn first_seat_pays_and_last_seat_earns() {
        // Ten seats of 1,000: seat 1 pays a fee, so it borrows at a positive
        // rate; seat 10 pays a fee too, so its saving return is negative.
        let borrow = seat_rate(1, 10, 10000.0, 1000.0, 500.0).unwrap();
        let save = seat_rate(10, 10, 10000.0, 1000.0, 50.0).unwrap();
        assert!(borrow > 0.0 && save < 0.0);
        assert!(seat_npv(borrow, 1, 10, 10000.0, 1000.0, 500.0).abs() < 1e-6);
    }

    #[test]
    fn weighted_median_uses_the_weights() {
        let mut v = vec![(1.0, 1.0), (2.0, 1.0), (3.0, 5.0)];
        assert_eq!(weighted_median(&mut v), 3.0);
    }
}
