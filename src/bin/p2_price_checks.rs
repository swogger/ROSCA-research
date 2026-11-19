//! Paper 2, section 10: checks of measure 1 (price of going first).
//!
//! Recomputes the regime medians of measure 1 under each check of the plan
//! that applies to it, with 1,000 bootstrap draws of groups each. The
//! primary figures stay in the tables written by `p2_price`.
//!
//! Usage:
//!     cargo run --release --bin p2_price_checks -- --data data/anonymised --out private/results/paper-2

use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;
use rand::Rng;

use breaking_binary::p2::{
    by_group, f, multi_fee_groups, read_members, regime_of, start_month_weights, write_tsv, Posted,
};
use breaking_binary::seeded_rng;
use breaking_binary::stats::{percentile, weighted_median};

const SEED: u64 = 2026_11_19;

#[derive(Parser)]
struct Args {
    #[arg(long, default_value = "data/anonymised")]
    data: PathBuf,
    #[arg(long, default_value = "private/results/paper-2")]
    out: PathBuf,
}

fn price_of_going_first(f1: f64, f_last: f64, payout: f64, n: u32) -> Option<f64> {
    let share = 1.0 - (f1 - f_last) / payout;
    (share > 0.0 && n > 1).then(|| share.powf(-1.0 / f64::from(n - 1)) - 1.0)
}

struct G {
    label: String,
    regime: char,
    start: i32,
    rate: f64,
    weight: f64,
    multi: bool,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let members = read_members(&args.data)?;
    let posted = Posted::build(&members);
    let multi = multi_fee_groups(&members, &posted);
    let grouped = by_group(&members);

    let build = |paid: bool, shift: i32| -> Vec<G> {
        let mut out = Vec::new();
        for (label, ms) in &grouped {
            let fee = |seat: u32| {
                ms.iter().find(|m| m.seat == seat).and_then(|m| if paid { Some(m.fee) } else { posted.of(m) })
            };
            let any = ms[0];
            let (Some(f1), Some(fl)) = (fee(1), fee(any.n)) else { continue };
            let Some(rate) = price_of_going_first(f1, fl, any.payout, any.n) else { continue };
            out.push(G {
                label: label.to_string(),
                regime: regime_of(any.start_month, shift),
                start: any.start_month,
                rate,
                weight: ms.len() as f64,
                multi: multi.contains(label),
            });
        }
        out
    };

    let mut rows = Vec::new();
    let mut rng = seeded_rng(SEED);
    let mut report = |name: &str, gs: &[G], extra: &dyn Fn(&G) -> f64| {
        for regime in ['A', 'B', 'C'] {
            let v: Vec<(f64, f64)> = gs
                .iter()
                .filter(|g| g.regime == regime)
                .map(|g| (g.rate, g.weight * extra(g)))
                .filter(|x| x.1 > 0.0)
                .collect();
            if v.is_empty() {
                continue;
            }
            let median = weighted_median(&mut v.clone());
            let mut draws: Vec<f64> = (0..1000)
                .map(|_| {
                    let mut s: Vec<(f64, f64)> = (0..v.len()).map(|_| v[rng.gen_range(0..v.len())]).collect();
                    weighted_median(&mut s)
                })
                .collect();
            draws.sort_by(f64::total_cmp);
            rows.push(vec![
                name.to_string(),
                regime.to_string(),
                v.len().to_string(),
                f(100.0 * median, 3),
                f(100.0 * percentile(&draws, 0.025), 3),
                f(100.0 * percentile(&draws, 0.975), 3),
            ]);
            if regime == 'C' {
                println!("  {name:<48} regime C median {:.2}% per month ({} groups)", 100.0 * median, v.len());
            }
        }
    };
    let base = build(false, 0);
    let one = |_: &G| 1.0;
    report("primary (as p2_price)", &base, &one);
    report("cut-offs one month earlier", &build(false, -1), &one);
    report("cut-offs one month later", &build(false, 1), &one);
    report("paid fee instead of posted", &build(true, 0), &one);
    report("groups with multi-fee cells excluded", &base, &|g: &G| (!g.multi) as u8 as f64);
    let weights: BTreeMap<String, f64> = start_month_weights(
        &base.iter().map(|g| (g.label.clone(), g.regime, g.start)).collect::<Vec<_>>(),
    )
    .into_iter()
    .collect();
    report("reweighted to the start-month mix of regime C", &base, &|g: &G| weights[&g.label]);
    write_tsv(
        &args.out.join("measure1_checks.tsv"),
        &["check", "regime", "groups", "weighted_median_pct_per_month", "ci_low", "ci_high"],
        &rows,
    )?;
    Ok(())
}
