//! Paper 2, section 7: does demand respond to price?
//!
//! Primary test, in ranked groups:
//!     rank_gs = beta × fee_gs + group effect + (seat, length, regime) effect + error
//! rank: join rank of the seat's holder, from 0 (first) to 1 (last);
//! fee: posted fee of the seat at its holder's payment month, in points of
//! the payout. Standard errors clustered by (length, payout, payment month).
//!
//! Checks (sections 7 and 10) run as further specifications of the same
//! model. Secondary, descriptive outputs: regime-switch series, repeat
//! users, and the checks of the rise from 54% to 92%.
//!
//! Usage:
//!     cargo run --release --bin p2_demand -- --data data/anonymised --out private/results/paper-2

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;

use breaking_binary::p2::{
    band, by_group, f, month_label, multi_fee_groups, parse_month, read_groups, read_members,
    regime_of, start_month_weights, write_tsv, Group, Member, Month, Posted,
};
use breaking_binary::stats::{fe_ols, FeFit};

#[derive(Parser)]
struct Args {
    #[arg(long, default_value = "data/anonymised")]
    data: PathBuf,
    #[arg(long, default_value = "private/results/paper-2")]
    out: PathBuf,
}

#[derive(Clone, Copy, PartialEq)]
enum Outcome {
    Rank,
    DaysOnMarket,
}

#[derive(Clone, Copy, PartialEq)]
enum FeeSource {
    /// Posted fee of the seat in the month the group opened (its first
    /// payment month), or at the holder's payment month when that cell is
    /// empty (clarification 3).
    Opening,
    /// Posted fee at the holder's own payment month.
    Holder,
    /// Fee actually paid.
    Paid,
}

#[derive(Clone)]
struct Spec {
    name: &'static str,
    shift: i32,
    fee_source: FeeSource,
    /// Group × payment-month effects instead of group effects.
    group_month: bool,
    /// Keep only observations with a next-month fee, without adding it.
    lead_sample: bool,
    outcome: Outcome,
    holidays: bool,
    lead: bool,
    exclude_multi: bool,
    reweight: bool,
    rate_window: bool,
    /// Diagnostics only: keep one regime, and groups whose fee payments
    /// span this many days (inclusive).
    only_regime: Option<char>,
    span: Option<(i64, i64)>,
}

const PRIMARY: Spec = Spec {
    name: "primary",
    shift: 0,
    fee_source: FeeSource::Opening,
    group_month: false,
    lead_sample: false,
    outcome: Outcome::Rank,
    holidays: false,
    lead: false,
    exclude_multi: false,
    reweight: false,
    rate_window: false,
    only_regime: None,
    span: None,
};

fn specs() -> Vec<Spec> {
    let s = |name, change: &dyn Fn(&mut Spec)| {
        let mut spec = PRIMARY.clone();
        spec.name = name;
        change(&mut spec);
        spec
    };
    vec![
        PRIMARY.clone(),
        s("primary on the pre-trend sample", &|x| x.lead_sample = true),
        s("pre-trend: next month's fee added", &|x| x.lead = true),
        s("fee at the holder's payment month", &|x| x.fee_source = FeeSource::Holder),
        s("group x payment-month effects, fee at holder's month", &|x| {
            x.group_month = true;
            x.fee_source = FeeSource::Holder;
        }),
        s("outcome: days on the market", &|x| x.outcome = Outcome::DaysOnMarket),
        s("holiday indicators added", &|x| x.holidays = true),
        s("cut-offs one month earlier", &|x| x.shift = -1),
        s("cut-offs one month later", &|x| x.shift = 1),
        s("paid fee instead of posted", &|x| x.fee_source = FeeSource::Paid),
        s("groups with multi-fee cells excluded", &|x| x.exclude_multi = true),
        s("reweighted to the start-month mix of regime C", &|x| x.reweight = true),
        s("rate held fixed: payments July 2023 to August 2024", &|x| x.rate_window = true),
        s("diagnostic: regime A only", &|x| x.only_regime = Some('A')),
        s("diagnostic: regime B only", &|x| x.only_regime = Some('B')),
        s("diagnostic: regime C only", &|x| x.only_regime = Some('C')),
        s("diagnostic: groups filled within 7 days", &|x| x.span = Some((0, 7))),
        s("diagnostic: groups filled over 8+ days", &|x| x.span = Some((8, 10_000))),
        s("diagnostic: days, regime B only", &|x| {
            x.outcome = Outcome::DaysOnMarket;
            x.only_regime = Some('B');
        }),
        s("diagnostic: days, regime C only", &|x| {
            x.outcome = Outcome::DaysOnMarket;
            x.only_regime = Some('C');
        }),
        s("diagnostic: days, groups filled within 7 days", &|x| {
            x.outcome = Outcome::DaysOnMarket;
            x.span = Some((0, 7));
        }),
        s("diagnostic: days, groups filled over 8+ days", &|x| {
            x.outcome = Outcome::DaysOnMarket;
            x.span = Some((8, 10_000));
        }),
    ]
}

fn index_of<T: Ord>(map: &mut BTreeMap<T, usize>, key: T) -> usize {
    let next = map.len();
    *map.entry(key).or_insert(next)
}

fn mean(v: &[f64], w: &[f64]) -> f64 {
    v.iter().zip(w).map(|(a, b)| a * b).sum::<f64>() / w.iter().sum::<f64>()
}

struct Context<'a> {
    ranked: Vec<(&'a str, Vec<&'a Member>)>,
    groups: HashMap<&'a str, &'a Group>,
    posted: Posted,
    multi: HashSet<&'a str>,
    weights: HashMap<String, f64>,
}

fn days_on_market(g: &Group, m: &Member) -> Option<f64> {
    Some((g.days_created_before_start? - m.days_paid_before_start?) as f64)
}

/// Fit one specification. Returns the fit, the names of the regressors,
/// the sample means of fee and outcome, and the number of groups.
fn run(ctx: &Context, spec: &Spec) -> Option<(FeFit, Vec<&'static str>, f64, f64, usize)> {
    let window = (parse_month("2023-07").unwrap(), parse_month("2024-08").unwrap());
    let (mut y, mut fee, mut lead, mut hol, mut school, mut w) =
        (vec![], vec![], vec![], vec![], vec![], vec![]);
    let (mut fe_group, mut fe_seat, mut clusters) = (vec![], vec![], vec![]);
    let (mut gi, mut si, mut ci) = (BTreeMap::new(), BTreeMap::new(), BTreeMap::new());
    for (label, ms) in &ctx.ranked {
        if spec.exclude_multi && ctx.multi.contains(label) {
            continue;
        }
        if spec.rate_window
            && !ms.iter().all(|m| m.paid_month.is_some_and(|p| p >= window.0 && p <= window.1))
        {
            continue;
        }
        if spec.only_regime.is_some_and(|r| r != ms[0].regime) {
            continue;
        }
        if let Some((lo, hi)) = spec.span {
            let d: Vec<i64> = ms.iter().filter_map(|m| m.days_paid_before_start).collect();
            let span = d.iter().max().unwrap_or(&0) - d.iter().min().unwrap_or(&0);
            if span < lo || span > hi {
                continue;
            }
        }
        let weight = if spec.reweight { ctx.weights[*label] } else { 1.0 };
        if weight == 0.0 {
            continue;
        }
        let g = ctx.groups[label];
        let Some(open) = ms.iter().filter_map(|m| m.paid_month).min() else { continue };
        for m in ms {
            let Some(pm) = m.paid_month else { continue };
            let fee_value = match spec.fee_source {
                FeeSource::Paid => Some(m.fee),
                FeeSource::Holder => ctx.posted.of(m),
                FeeSource::Opening => ctx
                    .posted
                    .get(open, m.n, m.payout_key(), m.seat)
                    .or_else(|| ctx.posted.of(m)),
            };
            let Some(fee_value) = fee_value else { continue };
            // The lead is read in the month after the fee month actually used.
            let opening_used = spec.fee_source == FeeSource::Opening
                && ctx.posted.get(open, m.n, m.payout_key(), m.seat).is_some();
            let lead_month = if opening_used { open } else { pm };
            let outcome = match spec.outcome {
                Outcome::Rank => {
                    let r = m.join_rank? as f64;
                    (r - 1.0) / (m.n as f64 - 1.0)
                }
                Outcome::DaysOnMarket => match days_on_market(g, m) {
                    Some(d) => d,
                    None => continue,
                },
            };
            let lead_value = if spec.lead || spec.lead_sample {
                match ctx.posted.get(lead_month + 1, m.n, m.payout_key(), m.seat) {
                    Some(l) => 100.0 * l / m.payout,
                    None => continue,
                }
            } else {
                0.0
            };
            let regime = regime_of(m.start_month, spec.shift);
            y.push(outcome);
            fee.push(100.0 * fee_value / m.payout);
            lead.push(lead_value);
            hol.push((m.fitr45 || m.adha30) as u8 as f64);
            school.push(m.school30 as u8 as f64);
            w.push(weight);
            let group_key = if spec.group_month { (*label, pm) } else { (*label, 0) };
            fe_group.push(index_of(&mut gi, group_key));
            fe_seat.push(index_of(&mut si, (m.seat, m.n, regime)));
            clusters.push(index_of(&mut ci, (m.n, m.payout_key(), pm)));
        }
    }
    let mut x = vec![fee.clone()];
    let mut names = vec!["fee"];
    if spec.lead {
        x.push(lead);
        names.push("next month's fee");
    }
    if spec.holidays {
        x.push(hol);
        x.push(school);
        names.push("H (pre-Eid payout)");
        names.push("school");
    }
    let fit = fe_ols(&y, &x, &[fe_group, fe_seat], Some(&w), &clusters)?;
    let n_groups = gi.keys().map(|k: &(&str, Month)| k.0).collect::<HashSet<_>>().len();
    Some((fit, names, mean(&fee, &w), mean(&y, &w), n_groups))
}

fn main() -> Result<()> {
    let args = Args::parse();
    let members = read_members(&args.data)?;
    let groups = read_groups(&args.data)?;
    let mismatched = members.iter().filter(|m| regime_of(m.start_month, 0) != m.regime).count();
    println!("memberships whose table regime differs from the start-month rule: {mismatched}");
    let posted = Posted::build(&members);
    let multi = multi_fee_groups(&members, &posted);
    let grouped = by_group(&members);
    let ranked: Vec<(&str, Vec<&Member>)> = grouped
        .iter()
        .filter(|(_, ms)| ms[0].usable)
        .map(|(g, ms)| (*g, ms.clone()))
        .collect();
    let group_of: HashMap<&str, &Group> = groups.iter().map(|g| (g.group.as_str(), g)).collect();
    let weights = start_month_weights(
        &ranked
            .iter()
            .map(|(g, ms)| (g.to_string(), ms[0].regime, ms[0].start_month))
            .collect::<Vec<_>>(),
    );
    let ctx = Context { ranked, groups: group_of.clone(), posted, multi, weights };

    // ---- primary test and checks
    let mut rows = Vec::new();
    println!("Section 7: rank on fee, group and (seat, length, regime) effects");
    for spec in specs() {
        let Some((fit, names, fee_mean, y_mean, n_groups)) = run(&ctx, &spec) else {
            println!("  {}: not estimable", spec.name);
            continue;
        };
        for (j, name) in names.iter().enumerate() {
            let elasticity = if j == 0 { fit.coef[0] * fee_mean / y_mean } else { f64::NAN };
            rows.push(vec![
                spec.name.to_string(),
                name.to_string(),
                f(fit.coef[j], 5),
                f(fit.se[j], 5),
                f(fit.coef[j] - 1.959_964 * fit.se[j], 5),
                f(fit.coef[j] + 1.959_964 * fit.se[j], 5),
                format!("{:.3e}", fit.p(j)),
                fit.n_obs.to_string(),
                n_groups.to_string(),
                fit.n_clusters.to_string(),
                f(fee_mean, 3),
                f(y_mean, 3),
                f(elasticity, 4),
            ]);
        }
        println!(
            "  {:<50} beta {:>9.5} (se {:.5}, p {:.2e}) n {} groups {} clusters {}",
            spec.name,
            fit.coef[0],
            fit.se[0],
            fit.p(0),
            fit.n_obs,
            n_groups,
            fit.n_clusters
        );
        if spec.lead {
            println!("      next month's fee: {:.5} (se {:.5}, p {:.2e})", fit.coef[1], fit.se[1], fit.p(1));
        }
        if spec.holidays {
            println!("      H: {:.5} (se {:.5}); school: {:.5} (se {:.5})", fit.coef[1], fit.se[1], fit.coef[2], fit.se[2]);
        }
    }
    std::fs::create_dir_all(&args.out)?;
    write_tsv(
        &args.out.join("demand_tests.tsv"),
        &[
            "specification", "term", "coef", "se", "ci_low", "ci_high", "p", "obs", "groups",
            "clusters", "mean_fee_pct", "mean_outcome", "elasticity_at_means",
        ],
        &rows,
    )?;

    secondary(&args, &members, &grouped, &group_of, &ctx.posted)?;
    println!("tables written to {}", args.out.display());
    Ok(())
}

/// Descriptive outputs of section 7.
fn secondary(
    args: &Args,
    members: &[Member],
    grouped: &BTreeMap<&str, Vec<&Member>>,
    group_of: &HashMap<&str, &Group>,
    posted: &Posted,
) -> Result<()> {
    // Users with two or more memberships, and users seen in several regimes.
    let mut per_user: HashMap<&str, Vec<&Member>> = HashMap::new();
    for m in members {
        per_user.entry(m.user.as_str()).or_default().push(m);
    }
    let repeat: HashSet<&str> =
        per_user.iter().filter(|(_, v)| v.len() >= 2).map(|(u, _)| *u).collect();
    let multi_regime: HashSet<&str> = per_user
        .iter()
        .filter(|(_, v)| v.iter().map(|m| m.regime).collect::<HashSet<_>>().len() >= 2)
        .map(|(u, _)| *u)
        .collect();

    // ---- first joiners: series by start month and the 54% to 92% checks
    struct First<'a> {
        regime: char,
        start: Month,
        n: u32,
        band: &'static str,
        seat1: bool,
        position: f64,
        ranked: bool,
        user: &'a str,
        days_to_first_fee: Option<f64>,
        mean_days_on_market: Option<f64>,
    }
    let mut firsts = Vec::new();
    for (label, ms) in grouped {
        if !ms[0].join_order_complete {
            continue;
        }
        let Some(first) = ms.iter().find(|m| m.join_rank == Some(1)) else { continue };
        let g = group_of[label];
        let max_paid = ms.iter().filter_map(|m| m.days_paid_before_start).max();
        let dom: Vec<f64> = ms.iter().filter_map(|m| days_on_market(g, m)).collect();
        firsts.push(First {
            regime: first.regime,
            start: first.start_month,
            n: first.n,
            band: band(first.payout),
            seat1: first.seat == 1,
            position: (first.seat as f64 - 1.0) / (first.n as f64 - 1.0),
            ranked: first.usable,
            user: first.user.as_str(),
            days_to_first_fee: g
                .days_created_before_start
                .zip(max_paid)
                .map(|(c, p)| (c - p) as f64),
            mean_days_on_market: (!dom.is_empty())
                .then(|| dom.iter().sum::<f64>() / dom.len() as f64),
        });
    }
    let share = |v: &[&First]| v.iter().filter(|x| x.seat1).count() as f64 / v.len() as f64;
    let avg = |v: &[f64]| if v.is_empty() { f64::NAN } else { v.iter().sum::<f64>() / v.len() as f64 };

    // Series by start month.
    let mut rows = Vec::new();
    let months: Vec<Month> = {
        let mut m: Vec<Month> = firsts.iter().map(|x| x.start).collect();
        m.sort_unstable();
        m.dedup();
        m
    };
    for sample in ["groups with complete join order", "ranked groups"] {
        for &mo in &months {
            let v: Vec<&First> = firsts
                .iter()
                .filter(|x| x.start == mo && (sample.starts_with("groups") || x.ranked))
                .collect();
            if v.is_empty() {
                continue;
            }
            let pos: Vec<f64> = v.iter().map(|x| x.position).collect();
            let dtf: Vec<f64> = v.iter().filter_map(|x| x.days_to_first_fee).collect();
            let dom: Vec<f64> = v.iter().filter_map(|x| x.mean_days_on_market).collect();
            rows.push(vec![
                sample.to_string(),
                month_label(mo),
                v[0].regime.to_string(),
                v.len().to_string(),
                f(100.0 * share(&v), 1),
                f(avg(&pos), 3),
                f(avg(&dtf), 1),
                f(avg(&dom), 1),
            ]);
        }
    }
    write_tsv(
        &args.out.join("demand_switch_series.tsv"),
        &[
            "sample", "start_month", "regime", "groups", "first_joiner_seat1_pct",
            "first_joiner_mean_position", "days_creation_to_first_fee", "mean_days_on_market",
        ],
        &rows,
    )?;

    // The rise from 54% to 92%.
    let mut rows = Vec::new();
    let mut push = |check: &str, sub: String, regime: char, v: Vec<&First>| {
        if v.is_empty() {
            return;
        }
        let dtf: Vec<f64> = v.iter().filter_map(|x| x.days_to_first_fee).collect();
        rows.push(vec![
            check.to_string(),
            sub,
            regime.to_string(),
            v.len().to_string(),
            f(100.0 * share(&v), 1),
            f(avg(&dtf), 1),
        ]);
    };
    for regime in ['A', 'B', 'C'] {
        let base: Vec<&First> = firsts.iter().filter(|x| x.regime == regime).collect();
        push("baseline", "groups with complete join order".into(), regime, base.clone());
        push("baseline", "ranked groups".into(), regime, base.iter().copied().filter(|x| x.ranked).collect());
        push("(a) different users", "first joiner has 2+ memberships".into(), regime,
            base.iter().copied().filter(|x| repeat.contains(x.user)).collect());
        push("(a) different users", "first joiner seen in 2+ regimes".into(), regime,
            base.iter().copied().filter(|x| multi_regime.contains(x.user)).collect());
        for n in [4, 5, 6, 8, 10, 12] {
            for b in ["under 5000", "5000-9999", "10000-19999", "20000 and over"] {
                push("(c) within length and amount", format!("length {n}, payout {b}"), regime,
                    base.iter().copied().filter(|x| x.n == n && x.band == b).collect());
            }
        }
    }
    // (c) reweighted: each regime given the pooled mix of (length, band) cells.
    let mut cell_total: HashMap<(u32, &str), f64> = HashMap::new();
    for x in &firsts {
        *cell_total.entry((x.n, x.band)).or_default() += 1.0;
    }
    let total = firsts.len() as f64;
    for regime in ['A', 'B', 'C'] {
        let mut by_cell: HashMap<(u32, &str), (f64, f64)> = HashMap::new();
        for x in firsts.iter().filter(|x| x.regime == regime) {
            let e = by_cell.entry((x.n, x.band)).or_default();
            e.0 += x.seat1 as u8 as f64;
            e.1 += 1.0;
        }
        let (mut num, mut den) = (0.0, 0.0);
        for (cell, (s, c)) in &by_cell {
            let wt = cell_total[cell] / total;
            num += wt * s / c;
            den += wt;
        }
        rows.push(vec![
            "(c) within length and amount".into(),
            "reweighted to the pooled cell mix (cells present in the regime)".into(),
            regime.to_string(),
            by_cell.values().map(|v| v.1 as usize).sum::<usize>().to_string(),
            f(100.0 * num / den, 1),
            String::new(),
        ]);
    }
    // (b) supply of circles: groups starting per month and days from
    // creation to the first fee, by regime (the series file has them by month).
    write_tsv(
        &args.out.join("demand_rise_54_92.tsv"),
        &["check", "subsample", "regime", "groups", "first_joiner_seat1_pct", "days_creation_to_first_fee"],
        &rows,
    )?;

    let mut supply = Vec::new();
    let mut started: BTreeMap<Month, (char, usize, usize)> = BTreeMap::new();
    for g in group_of.values() {
        let e = started.entry(g.start_month).or_insert((g.regime, 0, 0));
        e.1 += 1;
        e.2 += g.n_observed as usize;
    }
    // Pending circles: created but not started, counted at the first day of
    // each month. The start day is not in the tables; mid-month is assumed.
    let created: Vec<(f64, f64)> = group_of
        .values()
        .filter_map(|g| {
            let start = g.start_month as f64 * 30.4375 + 15.0;
            g.days_created_before_start.map(|d| (start - d as f64, start))
        })
        .collect();
    for (mo, (regime, n_groups, n_members)) in &started {
        let t = *mo as f64 * 30.4375;
        let pending = created.iter().filter(|(c, s)| *c <= t && t < *s).count();
        supply.push(vec![
            month_label(*mo),
            regime.to_string(),
            n_groups.to_string(),
            n_members.to_string(),
            pending.to_string(),
        ]);
    }
    write_tsv(
        &args.out.join("demand_supply_of_circles.tsv"),
        &["start_month", "regime", "groups_started", "listed_members", "pending_at_month_start_approx"],
        &supply,
    )?;

    repeat_users(args, &per_user, posted)?;
    Ok(())
}

/// Repeat users: change in seat between consecutive memberships of the
/// same length, by whether the earlier seat's relative fee rose.
fn repeat_users(args: &Args, per_user: &HashMap<&str, Vec<&Member>>, posted: &Posted) -> Result<()> {
    // Relative fee of seat s: its posted fee minus the mean posted fee of
    // the seats posted in the same cell, in points of the payout.
    let relative = |pm: Month, n: u32, payout: u64, seat: u32| -> Option<f64> {
        let fees: Vec<f64> = (1..=n).filter_map(|s| posted.get(pm, n, payout, s)).collect();
        if fees.len() < 2 {
            return None;
        }
        let own = posted.get(pm, n, payout, seat)?;
        Some(100.0 * (own - fees.iter().sum::<f64>() / fees.len() as f64) / payout as f64)
    };
    let mut rows = Vec::new();
    for (label, same_regime) in [("all consecutive pairs", false), ("pairs within one regime", true)] {
        let (mut y, mut rose, mut number, mut gap, mut clusters) = (vec![], vec![], vec![], vec![], vec![]);
        let mut users = BTreeMap::new();
        for (user, ms) in per_user {
            let mut ms: Vec<&&Member> = ms.iter().filter(|m| m.paid_month.is_some()).collect();
            ms.sort_by_key(|m| (m.paid_month, m.start_month, m.seat));
            for (k, pair) in ms.windows(2).enumerate() {
                let (a, b) = (pair[0], pair[1]);
                if a.n != b.n || (same_regime && a.regime != b.regime) {
                    continue;
                }
                let (pa, pb) = (a.paid_month.unwrap(), b.paid_month.unwrap());
                let (Some(r1), Some(r2)) = (
                    relative(pa, a.n, a.payout_key(), a.seat),
                    relative(pb, b.n, b.payout_key(), a.seat),
                ) else {
                    continue;
                };
                y.push((b.seat as f64 - a.seat as f64) / (a.n as f64 - 1.0));
                rose.push((r2 - r1 > 0.1) as u8 as f64);
                number.push((k + 2) as f64);
                gap.push((pb - pa) as f64);
                let next = users.len();
                clusters.push(*users.entry(*user).or_insert(next));
            }
        }
        let intercept = vec![0usize; y.len()];
        if let Some(fit) = fe_ols(&y, &[rose.clone(), number, gap], &[intercept], None, &clusters) {
            let n_rose = rose.iter().filter(|r| **r > 0.0).count();
            rows.push(vec![
                label.to_string(),
                y.len().to_string(),
                n_rose.to_string(),
                fit.n_clusters.to_string(),
                f(fit.coef[0], 4),
                f(fit.se[0], 4),
                format!("{:.3e}", fit.p(0)),
            ]);
            println!(
                "  repeat users, {label}: change in position when the earlier seat's relative fee rose {:.4} (se {:.4}), pairs {} ({} with a rise)",
                fit.coef[0], fit.se[0], y.len(), n_rose
            );
        }
    }
    write_tsv(
        &args.out.join("demand_repeat_users.tsv"),
        &["sample", "pairs", "pairs_with_rise", "users", "coef_rise_on_position_change", "se", "p"],
        &rows,
    )?;
    Ok(())
}
