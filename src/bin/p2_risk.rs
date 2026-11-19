//! Paper 2, section 9: price of time or risk premium?
//!
//! 1. Break-even default rate of seat k: the probability of default after
//!    the payout, with nothing recovered, at which the fee gap f_k - f_N
//!    would just pay for the expected loss:
//!        q_k = (f_k - f_N) / ((N - k) × m).
//!    Computed from the median posted schedule of each regime and length
//!    (as in p2_price), and as the median over groups (clarification 5).
//! 2. Vacancy by seat and regime: the share of seats with no listed holder.
//!    Summarised as the vacancy of the first third of seats minus the last
//!    third, by regime, with a bootstrap over groups.
//!
//! Usage:
//!     cargo run --release --bin p2_risk -- --data data/anonymised --out private/results/paper-2

use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;
use rand::Rng;

use breaking_binary::p2::{by_group, f, read_groups, read_members, write_tsv, Posted};
use breaking_binary::seeded_rng;
use breaking_binary::stats::{percentile, weighted_median};

const REPORTED_LENGTHS: [u32; 6] = [4, 5, 6, 8, 10, 12];
const SEED: u64 = 9_2026_11_19;

#[derive(Parser)]
struct Args {
    #[arg(long, default_value = "data/anonymised")]
    data: PathBuf,
    #[arg(long, default_value = "private/results/paper-2")]
    out: PathBuf,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let members = read_members(&args.data)?;
    let groups = read_groups(&args.data)?;
    let posted = Posted::build(&members);

    // ---- 1. break-even default rates
    let mut cells: BTreeMap<(char, u32, u32), Vec<(f64, f64)>> = BTreeMap::new();
    for m in &members {
        let Some(fee) = posted.of(m) else { continue };
        cells.entry((m.regime, m.n, m.seat)).or_default().push((100.0 * fee / m.payout, 1.0));
    }
    let schedule: BTreeMap<(char, u32, u32), f64> =
        cells.iter_mut().map(|(k, v)| (*k, weighted_median(v))).collect();
    // Per group: q_k from the group's own posted fees (seat k and seat N).
    let mut per_group: BTreeMap<(char, u32, u32), Vec<(f64, f64)>> = BTreeMap::new();
    for ms in by_group(&members).values() {
        let last = ms.iter().find(|m| m.seat == m.n).and_then(|m| posted.of(m));
        let Some(f_last) = last else { continue };
        for m in ms.iter().filter(|m| m.seat < m.n) {
            let Some(fk) = posted.of(m) else { continue };
            let q = (fk - f_last) / ((m.n - m.seat) as f64 * m.monthly);
            per_group.entry((m.regime, m.n, m.seat)).or_default().push((q, 1.0));
        }
    }
    let mut rows = Vec::new();
    for (&(regime, n, seat), &fk) in &schedule {
        if regime == 'A' || seat == n || !REPORTED_LENGTHS.contains(&n) {
            continue;
        }
        let Some(&f_last) = schedule.get(&(regime, n, n)) else { continue };
        // Fees are in points of the payout P = N m, so
        // q_k = (f_k - f_N) / 100 × N / (N - k).
        let q = (fk - f_last) / 100.0 * n as f64 / (n - seat) as f64;
        let group_median = per_group
            .get_mut(&(regime, n, seat))
            .map(|v| weighted_median(v))
            .unwrap_or(f64::NAN);
        rows.push(vec![
            regime.to_string(),
            n.to_string(),
            seat.to_string(),
            f(fk, 3),
            f(f_last, 3),
            f(100.0 * q, 2),
            f(100.0 * group_median, 2),
        ]);
    }
    std::fs::create_dir_all(&args.out)?;
    write_tsv(
        &args.out.join("risk_break_even_default.tsv"),
        &[
            "regime", "length", "seat", "median_posted_fee_pct", "median_last_seat_fee_pct",
            "break_even_default_pct_schedule", "break_even_default_pct_group_median",
        ],
        &rows,
    )?;
    for r in rows.iter().filter(|r| r[0] == "C" && r[2] == "1") {
        println!("  regime C, length {}, seat 1: break-even default {}% (group median {}%)", r[1], r[5], r[6]);
    }

    // ---- 2. vacancy by seat and regime
    let listed: HashSet<(&str, u32)> = members.iter().map(|m| (m.group.as_str(), m.seat)).collect();
    let mut vac: BTreeMap<(char, u32, u32), (usize, usize)> = BTreeMap::new();
    // Per group: vacancy share of the first third and last third of seats.
    let mut thirds: Vec<(char, f64, f64)> = Vec::new();
    for g in &groups {
        let (mut early, mut early_n, mut late, mut late_n) = (0.0, 0.0, 0.0, 0.0);
        for s in 1..=g.n {
            let empty = !listed.contains(&(g.group.as_str(), s));
            let e = vac.entry((g.regime, g.n, s)).or_default();
            e.0 += empty as usize;
            e.1 += 1;
            let pos = (s as f64 - 1.0) / (g.n as f64 - 1.0);
            if pos < 1.0 / 3.0 {
                early += empty as u8 as f64;
                early_n += 1.0;
            } else if pos > 2.0 / 3.0 {
                late += empty as u8 as f64;
                late_n += 1.0;
            }
        }
        if early_n > 0.0 && late_n > 0.0 {
            thirds.push((g.regime, early / early_n, late / late_n));
        }
    }
    let rows: Vec<Vec<String>> = vac
        .iter()
        .filter(|((_, n, _), _)| REPORTED_LENGTHS.contains(n))
        .map(|((r, n, s), (e, t))| {
            vec![r.to_string(), n.to_string(), s.to_string(), t.to_string(), f(100.0 * *e as f64 / *t as f64, 1)]
        })
        .collect();
    write_tsv(&args.out.join("risk_vacancy_by_seat.tsv"), &["regime", "length", "seat", "groups", "vacancy_pct"], &rows)?;

    let mut rng = seeded_rng(SEED);
    let mut rows = Vec::new();
    let summary = |v: &[&(char, f64, f64)]| {
        let n = v.len() as f64;
        let e = v.iter().map(|x| x.1).sum::<f64>() / n;
        let l = v.iter().map(|x| x.2).sum::<f64>() / n;
        (e, l, e - l)
    };
    let mut by_regime: BTreeMap<char, Vec<&(char, f64, f64)>> = BTreeMap::new();
    for t in &thirds {
        by_regime.entry(t.0).or_default().push(t);
    }
    let base = summary(&by_regime[&'A']).2;
    let mut boot_a: Vec<f64> = Vec::new();
    for (regime, v) in &by_regime {
        let (e, l, d) = summary(v);
        let mut draws: Vec<f64> = (0..1000)
            .map(|_| {
                let s: Vec<&(char, f64, f64)> = (0..v.len()).map(|_| v[rng.gen_range(0..v.len())]).collect();
                summary(&s).2
            })
            .collect();
        if *regime == 'A' {
            boot_a = draws.clone();
        }
        let diff_vs_a: Vec<f64> = draws.iter().zip(&boot_a).map(|(x, a)| x - a).collect();
        let mut diff_sorted = diff_vs_a.clone();
        diff_sorted.sort_by(f64::total_cmp);
        draws.sort_by(f64::total_cmp);
        rows.push(vec![
            regime.to_string(),
            v.len().to_string(),
            f(100.0 * e, 1),
            f(100.0 * l, 1),
            f(100.0 * d, 1),
            f(100.0 * percentile(&draws, 0.025), 1),
            f(100.0 * percentile(&draws, 0.975), 1),
            f(100.0 * (d - base), 1),
            f(100.0 * percentile(&diff_sorted, 0.025), 1),
            f(100.0 * percentile(&diff_sorted, 0.975), 1),
        ]);
        println!("  vacancy, regime {regime}: first third {:.1}%, last third {:.1}%, gap {:.1} points", 100.0 * e, 100.0 * l, 100.0 * d);
    }
    write_tsv(
        &args.out.join("risk_vacancy_summary.tsv"),
        &[
            "regime", "groups", "vacancy_first_third_pct", "vacancy_last_third_pct", "gap_points",
            "gap_ci_low", "gap_ci_high", "gap_minus_regime_A", "ci_low", "ci_high",
        ],
        &rows,
    )?;
    println!("tables written to {}", args.out.display());
    Ok(())
}
