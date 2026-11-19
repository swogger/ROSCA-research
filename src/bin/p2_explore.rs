//! Paper 2, exploratory analyses E1, E3 and E4 (see "Exploratory analyses
//! added after the confirmatory results" in deviations-paper-2.md).
//!
//! E1. Cost to the seat-1 and seat-2 member at the posted fee.
//! E3. The section 7 demand test on all groups with a complete join order.
//! E4. Quantity demand across circle types (length, payout) by month.
//!
//! Usage:
//!     cargo run --release --bin p2_explore -- --data data/anonymised --out private/results/paper-2 --card 2.5

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;

use breaking_binary::p2::{
    b_start, by_group, f, read_members, read_users, seat_rate, write_tsv, Member, Month, Posted,
};
use breaking_binary::stats::{fe_ols, percentile};

#[derive(Parser)]
struct Args {
    #[arg(long, default_value = "data/anonymised")]
    data: PathBuf,
    #[arg(long, default_value = "private/results/paper-2")]
    out: PathBuf,
    /// Card benchmark, % a month.
    #[arg(long, default_value_t = 2.5)]
    card: f64,
}

fn ix<T: Ord>(m: &mut BTreeMap<T, usize>, k: T) -> usize {
    let n = m.len();
    *m.entry(k).or_insert(n)
}

fn main() -> Result<()> {
    let args = Args::parse();
    let members = read_members(&args.data)?;
    let users = read_users(&args.data)?;
    let income: HashMap<&str, &str> = users.iter().map(|u| (u.user.as_str(), u.income_level.as_str())).collect();
    let posted = Posted::build(&members);
    std::fs::create_dir_all(&args.out)?;

    // ---------------------------------------------------------------- E1
    let mut cells: BTreeMap<(char, u32, u32, String), Vec<f64>> = BTreeMap::new();
    for m in members.iter().filter(|m| m.regime != 'A' && m.seat <= 2) {
        let Some(fee) = posted.of(m) else { continue };
        let Some(r) = seat_rate(m.seat, m.n, m.payout, m.monthly, fee) else { continue };
        let pct = 100.0 * r;
        cells.entry((m.regime, m.seat, m.n, "all".into())).or_default().push(pct);
        cells.entry((m.regime, m.seat, 0, "all".into())).or_default().push(pct);
        cells.entry((m.regime, m.seat, 0, income[m.user.as_str()].to_string())).or_default().push(pct);
    }
    let mut rows = Vec::new();
    for ((regime, seat, n, inc), v) in &mut cells {
        v.sort_by(f64::total_cmp);
        let med = percentile(v, 0.5);
        let above = v.iter().filter(|x| **x > args.card).count() as f64 / v.len() as f64;
        rows.push(vec![
            regime.to_string(),
            seat.to_string(),
            if *n == 0 { "all".into() } else { n.to_string() },
            inc.clone(),
            v.len().to_string(),
            f(percentile(v, 0.25), 2),
            f(med, 2),
            f(percentile(v, 0.75), 2),
            f(100.0 * ((1.0 + med / 100.0).powi(12) - 1.0), 0),
            f(100.0 * above, 1),
        ]);
        if *n == 0 && inc == "all" {
            println!(
                "E1 regime {regime} seat {seat}: median {med:.2}% a month ({:.0}% a year), {:.0}% of members above {}%/month",
                100.0 * ((1.0 + med / 100.0).powi(12) - 1.0),
                100.0 * above,
                args.card
            );
        }
    }
    write_tsv(
        &args.out.join("explore_e1_seat_cost.tsv"),
        &["regime", "seat", "length", "income_level", "memberships", "p25_pct_month", "median_pct_month", "p75_pct_month", "median_pct_year", "share_above_card_pct"],
        &rows,
    )?;

    // ---------------------------------------------------------------- E3
    {
        let (mut y, mut fee, mut fg, mut fs, mut cl) = (vec![], vec![], vec![], vec![], vec![]);
        let (mut a, mut b, mut c) = (BTreeMap::new(), BTreeMap::new(), BTreeMap::new());
        let mut n_groups = 0;
        let mut n_usable = 0;
        for (label, ms) in by_group(&members) {
            if !ms[0].join_order_complete || ms.len() < 2 {
                continue;
            }
            n_groups += 1;
            if ms[0].usable {
                n_usable += 1;
            }
            let Some(open) = ms.iter().filter_map(|m| m.paid_month).min() else { continue };
            let mut ordered: Vec<&&Member> = ms.iter().collect();
            ordered.sort_by_key(|m| m.join_rank);
            let k = ordered.len() as f64;
            for (r, m) in ordered.iter().enumerate() {
                let Some(pm) = m.paid_month else { continue };
                let Some(fv) = posted.get(open, m.n, m.payout_key(), m.seat).or_else(|| posted.of(m)) else { continue };
                y.push(r as f64 / (k - 1.0));
                fee.push(100.0 * fv / m.payout);
                fg.push(ix(&mut a, label));
                fs.push(ix(&mut b, (m.seat, m.n, m.regime)));
                cl.push(ix(&mut c, (m.n, m.payout_key(), pm)));
            }
        }
        let fit = fe_ols(&y, &[fee], &[fg, fs], None, &cl).expect("E3 fit");
        println!(
            "E3 demand test, all groups with a complete join order ({n_groups} groups, {n_usable} of them ranked): beta {:.5} (se {:.5}, p {:.2e}), n {}",
            fit.coef[0], fit.se[0], fit.p(0), fit.n_obs
        );
        write_tsv(
            &args.out.join("explore_e3_wider_demand.tsv"),
            &["sample", "groups", "obs", "clusters", "beta", "se", "p"],
            &[vec![
                "groups with complete join order, 2+ listed".into(),
                n_groups.to_string(),
                fit.n_obs.to_string(),
                fit.n_clusters.to_string(),
                f(fit.coef[0], 5),
                f(fit.se[0], 5),
                format!("{:.3e}", fit.p(0)),
            ]],
        )?;
    }

    // ---------------------------------------------------------------- E4
    {
        // Q per (length, payout, payment month), regimes B and C.
        let mut q: BTreeMap<(u32, u64, Month), f64> = BTreeMap::new();
        for m in members.iter().filter(|m| m.regime != 'A') {
            let Some(pm) = m.paid_month else { continue };
            if pm < b_start() {
                continue;
            }
            *q.entry((m.n, m.payout_key(), pm)).or_default() += 1.0;
        }
        let mut rows = Vec::new();
        for (price_name, seat1) in [("mean posted fee over seats", false), ("seat-1 posted fee", true)] {
            for strict in [false, true] {
                let (mut y, mut x, mut fc, mut fm, mut cl) = (vec![], vec![], vec![], vec![], vec![]);
                let (mut a, mut b, mut c) = (BTreeMap::new(), BTreeMap::new(), BTreeMap::new());
                for (&(n, p, pm), &count) in &q {
                    let price = if seat1 {
                        match posted.get(pm, n, p, 1) {
                            Some(v) => 100.0 * v / p as f64,
                            None => continue,
                        }
                    } else {
                        let fees: Vec<f64> = (1..=n).filter_map(|s| posted.get(pm, n, p, s)).collect();
                        if (fees.len() as f64) < n as f64 / 2.0 {
                            continue;
                        }
                        100.0 * fees.iter().sum::<f64>() / fees.len() as f64 / p as f64
                    };
                    y.push(count.ln());
                    x.push(price);
                    fc.push(ix(&mut a, (n, p)));
                    fm.push(ix(&mut b, if strict { (pm, n) } else { (pm, 0) }));
                    cl.push(ix(&mut c, (n, p)));
                }
                if let Some(fit) = fe_ols(&y, &[x], &[fc, fm], None, &cl) {
                    let label = if strict { "cell and (length, month) effects" } else { "cell and month effects" };
                    println!(
                        "E4 log quantity on {price_name}, {label}: {:.4} (se {:.4}, p {:.2e}), cell-months {}, cells {}",
                        fit.coef[0], fit.se[0], fit.p(0), fit.n_obs, fit.n_clusters
                    );
                    rows.push(vec![
                        price_name.to_string(),
                        label.to_string(),
                        fit.n_obs.to_string(),
                        fit.n_clusters.to_string(),
                        f(fit.coef[0], 5),
                        f(fit.se[0], 5),
                        format!("{:.3e}", fit.p(0)),
                    ]);
                }
            }
        }
        write_tsv(
            &args.out.join("explore_e4_quantity.tsv"),
            &["price", "effects", "cell_months", "cells", "coef_log_quantity_per_point", "se", "p"],
            &rows,
        )?;
    }
    Ok(())
}
