//! Paper 2, section 6: who pays the price of going first.
//!
//! Time price paid by a member: the posted fee of their seat minus the
//! lowest posted fee among the listed seats of the same group, as a
//! percentage of the payout (clarification 2). Each posted fee is taken at
//! the payment month of the seat's holder.
//!
//! Outputs, by regime (B, C, and B and C pooled):
//!   * mean time price paid by income band, age band, gender and
//!     employment sector, raw and adjusted for length, amount and payment
//!     month fixed effects (standard errors clustered by group);
//!   * each income band's share of all time price paid, in money, next to
//!     its share of payouts received.
//! A check excludes users whose employment sector is suppressed.
//!
//! Usage:
//!     cargo run --release --bin p2_incidence -- --data data/anonymised --out private/results/paper-2

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;

use breaking_binary::p2::{by_group, f, read_members, read_users, write_tsv, Member, Posted, User};
use breaking_binary::stats::fe_ols;

#[derive(Parser)]
struct Args {
    #[arg(long, default_value = "data/anonymised")]
    data: PathBuf,
    #[arg(long, default_value = "private/results/paper-2")]
    out: PathBuf,
}

const ATTRIBUTES: [&str; 4] = ["income_level", "age_band", "gender", "employment_sector"];

fn attribute<'a>(u: &'a User, name: &str) -> &'a str {
    match name {
        "income_level" => &u.income_level,
        "age_band" => &u.age_band,
        "gender" => &u.gender,
        _ => &u.employment_sector,
    }
}

struct Obs<'a> {
    m: &'a Member,
    user: &'a User,
    tp: f64,
    group_index: usize,
}

fn index_of<T: Ord + Clone>(map: &mut BTreeMap<T, usize>, key: T) -> usize {
    let next = map.len();
    *map.entry(key).or_insert(next)
}

fn analyse(obs: &[Obs], label: &str, rows: &mut Vec<Vec<String>>, shares: &mut Vec<Vec<String>>) {
    for attr in ATTRIBUTES {
        let mut cats: BTreeMap<&str, (f64, usize)> = BTreeMap::new();
        for o in obs {
            let e = cats.entry(attribute(o.user, attr)).or_default();
            e.0 += o.tp;
            e.1 += 1;
        }
        // Base category: the largest one.
        let base = cats.iter().max_by_key(|(_, v)| v.1).map(|(k, _)| *k).unwrap_or("");
        let others: Vec<&str> = cats.keys().copied().filter(|c| *c != base).collect();
        let y: Vec<f64> = obs.iter().map(|o| o.tp).collect();
        let x: Vec<Vec<f64>> = others
            .iter()
            .map(|c| obs.iter().map(|o| (attribute(o.user, attr) == *c) as u8 as f64).collect())
            .collect();
        let mut len_ix = BTreeMap::new();
        let mut amt_ix = BTreeMap::new();
        let mut month_ix = BTreeMap::new();
        let fes = vec![
            obs.iter().map(|o| index_of(&mut len_ix, o.m.n)).collect::<Vec<_>>(),
            obs.iter().map(|o| index_of(&mut amt_ix, o.m.payout_key())).collect(),
            obs.iter().map(|o| index_of(&mut month_ix, o.m.paid_month.unwrap())).collect(),
        ];
        let clusters: Vec<usize> = obs.iter().map(|o| o.group_index).collect();
        let fit = fe_ols(&y, &x, &fes, None, &clusters);
        let base_mean = cats[base].0 / cats[base].1 as f64;
        for (c, (sum, count)) in &cats {
            let mean = sum / *count as f64;
            let (adj, se) = match (&fit, others.iter().position(|o| o == c)) {
                (Some(fit), Some(j)) => (fit.coef[j], fit.se[j]),
                _ => (0.0, f64::NAN),
            };
            rows.push(vec![
                label.to_string(),
                attr.to_string(),
                c.to_string(),
                count.to_string(),
                f(mean, 3),
                f(mean - base_mean, 3),
                f(adj, 3),
                f(se, 3),
                (c == &base).to_string(),
            ]);
        }
    }
    // Shares by income band, in money.
    let mut paid: BTreeMap<&str, (f64, f64)> = BTreeMap::new();
    for o in obs {
        let e = paid.entry(o.user.income_level.as_str()).or_default();
        e.0 += o.tp / 100.0 * o.m.payout;
        e.1 += o.m.payout;
    }
    let (tp_total, p_total): (f64, f64) =
        paid.values().fold((0.0, 0.0), |a, v| (a.0 + v.0, a.1 + v.1));
    for (band, (tp, p)) in &paid {
        shares.push(vec![
            label.to_string(),
            band.to_string(),
            f(100.0 * tp / tp_total, 2),
            f(100.0 * p / p_total, 2),
        ]);
    }
}

fn main() -> Result<()> {
    let args = Args::parse();
    let members = read_members(&args.data)?;
    let users = read_users(&args.data)?;
    let user_of: HashMap<&str, &User> = users.iter().map(|u| (u.user.as_str(), u)).collect();
    let posted = Posted::build(&members);

    let mut obs = Vec::new();
    let mut no_fee = 0usize;
    for (gi, (_, ms)) in by_group(&members).iter().enumerate() {
        let fees: Vec<Option<f64>> = ms.iter().map(|m| posted.of(m)).collect();
        let Some(low) = fees.iter().flatten().copied().reduce(f64::min) else {
            no_fee += ms.len();
            continue;
        };
        for (m, fee) in ms.iter().zip(&fees) {
            match fee {
                Some(fee) => obs.push(Obs {
                    m,
                    user: user_of[m.user.as_str()],
                    tp: 100.0 * (fee - low) / m.payout,
                    group_index: gi,
                }),
                None => no_fee += 1,
            }
        }
    }
    println!("memberships with a time price: {}; without a posted fee: {no_fee}", obs.len());

    let mut rows = Vec::new();
    let mut shares = Vec::new();
    for (label, keep) in [
        ("A", "A"),
        ("B", "B"),
        ("C", "C"),
        ("B and C", "BC"),
    ] {
        let sample: Vec<Obs> = obs
            .iter()
            .filter(|o| keep.contains(o.m.regime))
            .map(|o| Obs { m: o.m, user: o.user, tp: o.tp, group_index: o.group_index })
            .collect();
        analyse(&sample, label, &mut rows, &mut shares);
        let mean = sample.iter().map(|o| o.tp).sum::<f64>() / sample.len() as f64;
        println!("  {label}: {} memberships, mean time price paid {mean:.2}% of payout", sample.len());
    }
    // Check (section 10): users with a suppressed employment sector removed.
    let kept: Vec<Obs> = obs
        .iter()
        .filter(|o| "BC".contains(o.m.regime) && o.user.employment_sector != "suppressed")
        .map(|o| Obs { m: o.m, user: o.user, tp: o.tp, group_index: o.group_index })
        .collect();
    let dropped = obs
        .iter()
        .filter(|o| "BC".contains(o.m.regime) && o.user.employment_sector == "suppressed")
        .count();
    analyse(&kept, "B and C, suppressed sector excluded", &mut rows, &mut shares);
    println!("  check: {dropped} memberships of users with a suppressed sector excluded");

    std::fs::create_dir_all(&args.out)?;
    write_tsv(
        &args.out.join("incidence_by_attribute.tsv"),
        &[
            "sample", "attribute", "category", "memberships", "mean_time_price_pct",
            "raw_gap_to_base", "adjusted_gap_to_base", "adjusted_gap_se", "is_base",
        ],
        &rows,
    )?;
    write_tsv(
        &args.out.join("incidence_income_shares.tsv"),
        &["sample", "income_level", "share_of_time_price_paid_pct", "share_of_payouts_pct"],
        &shares,
    )?;
    println!("tables written to {}", args.out.display());
    Ok(())
}
