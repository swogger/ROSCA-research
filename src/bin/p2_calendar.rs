//! Paper 2, section 8: dated need or earliness?
//!
//! Conditional logit on the choice sets of ranked groups. Each member
//! picks one seat among the seats still open when they paid. The score of
//! open seat s is
//!     seat effect(s, length) + theta × H_s + lambda × school_s
//!     + effect of the calendar month of the payout - beta_c × fee_s.
//! H_s: the payout falls 1 to 45 days before Eid al-Fitr or 1 to 30 days
//! before Eid al-Adha. fee_s enters in regimes B and C only (zero in A).
//!
//! Primary: theta, fitted on the 80% fitting sample (the hold-out is read
//! from `holdout_groups.tsv`), 1,000 bootstrap draws of whole groups,
//! Holm correction with the section 7 test. Prediction check on the
//! hold-out. Secondary analyses and section 10 checks follow.
//!
//! Usage (after p2_holdout and p2_demand):
//!     cargo run --release --bin p2_calendar -- --data data/anonymised --out private/results/paper-2

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;

use anyhow::{Context as _, Result};
use clap::Parser;
use rand::Rng;

use breaking_binary::p2::{
    by_group, f, month_label, month_of_year, multi_fee_groups, read_members, regime_of,
    start_month_weights, write_tsv, Member, Posted,
};
use breaking_binary::seeded_rng;
use breaking_binary::stats::{
    clogit_fit, clogit_loglik_each, clogit_se, fe_ols, holm, percentile, sd, two_sided_p, Alt,
    Choice, ClogitFit,
};

const BOOTSTRAP_SEED: u64 = 8_2026_11_19;
const BOOTSTRAP_DRAWS: usize = 1000;

#[derive(Parser)]
struct Args {
    #[arg(long, default_value = "data/anonymised")]
    data: PathBuf,
    #[arg(long, default_value = "private/results/paper-2")]
    out: PathBuf,
    /// Bootstrap draws for the primary test.
    #[arg(long, default_value_t = BOOTSTRAP_DRAWS)]
    draws: usize,
}

#[derive(Clone, Copy, PartialEq)]
enum Window {
    Main,
    Days30,
    Days60,
}

#[derive(Clone)]
struct Spec {
    name: &'static str,
    window: Window,
    split_eid: bool,
    with_h: bool,
    month_effects: bool,
    seat_by_regime: bool,
    paid_fee: bool,
    shift: i32,
    regimes: &'static str,
    exclude_multi: bool,
    reweight: bool,
}

const PRIMARY: Spec = Spec {
    name: "primary",
    window: Window::Main,
    split_eid: false,
    with_h: true,
    month_effects: true,
    seat_by_regime: false,
    paid_fee: false,
    shift: 0,
    regimes: "ABC",
    exclude_multi: false,
    reweight: false,
};

fn variant(name: &'static str, change: &dyn Fn(&mut Spec)) -> Spec {
    let mut s = PRIMARY.clone();
    s.name = name;
    change(&mut s);
    s
}

/// Parameter names, created as they first appear.
#[derive(Default, Clone)]
struct Layout {
    names: Vec<String>,
    index: HashMap<String, usize>,
}

impl Layout {
    fn idx(&mut self, name: String) -> usize {
        if let Some(&i) = self.index.get(&name) {
            return i;
        }
        self.names.push(name.clone());
        self.index.insert(name, self.names.len() - 1);
        self.names.len() - 1
    }
    fn get(&self, name: &str) -> Option<usize> {
        self.index.get(name).copied()
    }
}

struct Ranked<'a> {
    label: &'a str,
    members: Vec<&'a Member>,
    held: bool,
    multi: bool,
    weight: f64,
}

struct Built {
    choices: Vec<Choice>,
    /// Index into the ranked groups for each choice.
    group: Vec<usize>,
    /// Length of the group of each choice.
    length: Vec<u32>,
    /// Regime (with the spec's cut-offs) of each choice.
    regime: Vec<char>,
    layout: Layout,
    fee_fallbacks: usize,
}

fn h_flags(m: &Member, w: Window) -> (bool, bool, bool) {
    match w {
        Window::Main => (m.fitr45, m.adha30, m.school30),
        Window::Days30 => (m.fitr30, m.adha30, m.school30),
        Window::Days60 => (m.fitr60, m.adha60, m.school60),
    }
}

fn build(spec: &Spec, ranked: &[Ranked], posted: &Posted) -> Built {
    let mut layout = Layout::default();
    let (mut choices, mut group, mut length, mut regimes) = (vec![], vec![], vec![], vec![]);
    let mut fee_fallbacks = 0;
    for (gi, g) in ranked.iter().enumerate() {
        let first = g.members[0];
        let regime = regime_of(first.start_month, spec.shift);
        if !spec.regimes.contains(regime) || (spec.exclude_multi && g.multi) {
            continue;
        }
        if spec.reweight && g.weight == 0.0 {
            continue;
        }
        let n = first.n;
        let holder: HashMap<u32, &Member> = g.members.iter().map(|m| (m.seat, *m)).collect();
        let mut taken: HashSet<u32> = HashSet::new();
        for chooser in &g.members[..g.members.len() - 1] {
            let open: Vec<u32> = (1..=n).filter(|s| !taken.contains(s)).collect();
            let mut alts = Vec::with_capacity(open.len());
            let mut chosen = usize::MAX;
            for (a, &s) in open.iter().enumerate() {
                if s == chooser.seat {
                    chosen = a;
                }
                let h = holder[&s];
                let mut x = Vec::new();
                if s > 1 {
                    let key = if spec.seat_by_regime {
                        format!("seat {regime} N={n} s={s}")
                    } else {
                        format!("seat N={n} s={s}")
                    };
                    x.push((layout.idx(key), 1.0));
                }
                let (fitr, adha, school) = h_flags(h, spec.window);
                if spec.with_h {
                    if spec.split_eid {
                        if fitr {
                            x.push((layout.idx("H fitr".into()), 1.0));
                        }
                        if adha {
                            x.push((layout.idx("H adha".into()), 1.0));
                        }
                    } else if fitr || adha {
                        x.push((layout.idx("H".into()), 1.0));
                    }
                }
                if school {
                    x.push((layout.idx("school".into()), 1.0));
                }
                if spec.month_effects {
                    let moy = month_of_year(h.payout_month);
                    if moy > 1 {
                        x.push((layout.idx(format!("payout month {moy:02}")), 1.0));
                    }
                }
                if regime != 'A' {
                    let fee = if spec.paid_fee {
                        Some(h.fee)
                    } else {
                        chooser
                            .paid_month
                            .and_then(|pm| posted.get(pm, n, h.payout_key(), s))
                            .or_else(|| {
                                fee_fallbacks += 1;
                                posted.of(h)
                            })
                    };
                    if let Some(fee) = fee {
                        // Minus sign: the score falls with the fee, so the
                        // coefficient is beta_c itself.
                        x.push((layout.idx("fee (beta_c)".into()), -100.0 * fee / h.payout));
                    }
                }
                alts.push(Alt { x });
            }
            assert!(chosen != usize::MAX, "a chooser's seat is not among the open seats");
            choices.push(Choice { alts, chosen, cluster: gi });
            group.push(gi);
            length.push(n);
            regimes.push(regime);
            taken.insert(chooser.seat);
        }
    }
    Built { choices, group, length, regime: regimes, layout, fee_fallbacks }
}

/// Weights of the fitting sample: 1 for fitting groups (or the reweighting
/// weight), 0 for the hold-out.
fn fit_weights(b: &Built, ranked: &[Ranked], spec: &Spec) -> Vec<f64> {
    b.group
        .iter()
        .map(|&g| {
            if ranked[g].held {
                0.0
            } else if spec.reweight {
                ranked[g].weight
            } else {
                1.0
            }
        })
        .collect()
}

/// d1: mean gap between the seat effects of adjacent seats in the first
/// half of the group, averaged over lengths with the number of fitting
/// choices as weights (clarification 4).
fn d1(beta: &[f64], b: &Built, w: &[f64]) -> f64 {
    // Weights by (regime, length); seat effects are looked up under the
    // common name, or under the regime-specific name when the
    // specification has seat effects by regime.
    let mut cells: BTreeMap<(char, u32), f64> = BTreeMap::new();
    for ((&n, &r), &wi) in b.length.iter().zip(&b.regime).zip(w) {
        *cells.entry((r, n)).or_default() += wi;
    }
    let (mut num, mut den) = (0.0, 0.0);
    for (&(r, n), &count) in &cells {
        let h = n / 2;
        if h < 2 || count == 0.0 {
            continue;
        }
        let j = b
            .layout
            .get(&format!("seat N={n} s={h}"))
            .or_else(|| b.layout.get(&format!("seat {r} N={n} s={h}")));
        let Some(j) = j else { continue };
        let gap = -beta[j] / (h as f64 - 1.0);
        num += count * gap;
        den += count;
    }
    num / den
}

fn fit(b: &Built, w: &[f64]) -> ClogitFit {
    let k = b.layout.names.len();
    clogit_fit(&b.choices, w, k, &vec![0.0; k])
}


fn main() -> Result<()> {
    let args = Args::parse();
    let members = read_members(&args.data)?;
    let posted = Posted::build(&members);
    let multi = multi_fee_groups(&members, &posted);
    let held: HashSet<String> = std::fs::read_to_string(args.out.join("holdout_groups.tsv"))
        .context("run p2_holdout first")?
        .lines()
        .skip(1)
        .map(str::to_string)
        .collect();
    let grouped = by_group(&members);
    let mut ranked: Vec<Ranked> = grouped
        .iter()
        .filter(|(_, ms)| ms[0].usable)
        .map(|(g, ms)| {
            let mut ms = ms.clone();
            ms.sort_by_key(|m| m.join_rank);
            Ranked {
                label: g,
                members: ms,
                held: held.contains(*g),
                multi: multi.contains(g),
                weight: 1.0,
            }
        })
        .collect();
    let found = ranked.iter().filter(|g| g.held).count();
    anyhow::ensure!(found == held.len(), "hold-out list does not match the tables ({found} of {})", held.len());
    // Start-month weights from the fitting sample only; hold-out groups
    // never enter a fit.
    let weights = start_month_weights(
        &ranked
            .iter()
            .filter(|g| !g.held)
            .map(|g| (g.label.to_string(), g.members[0].regime, g.members[0].start_month))
            .collect::<Vec<_>>(),
    );
    for g in &mut ranked {
        g.weight = weights.get(g.label).copied().unwrap_or(0.0);
    }

    // ------------------------------------------------------------ primary
    let b = build(&PRIMARY, &ranked, &posted);
    let w = fit_weights(&b, &ranked, &PRIMARY);
    let n_fit = w.iter().filter(|x| **x > 0.0).count();
    let n_hold = b.choices.len() - n_fit;
    println!(
        "choices {} (fitting {n_fit}, hold-out {n_hold}); parameters {}; fee read from the holder's month in {} alternatives",
        b.choices.len(),
        b.layout.names.len(),
        b.fee_fallbacks
    );
    let primary = fit(&b, &w);
    println!("primary fit: log-likelihood {:.2}, {} iterations, converged {}", primary.loglik, primary.iterations, primary.converged);
    let (se_model, se_cluster) = clogit_se(&b.choices, &w, &primary);
    let th = b.layout.get("H").unwrap();
    let fee_ix = b.layout.get("fee (beta_c)").unwrap();
    let theta = primary.beta[th];
    let beta_c = primary.beta[fee_ix];
    let d1_point = d1(&primary.beta, &b, &w);

    // Bootstrap over fitting groups.
    let fit_groups: Vec<usize> = {
        let mut v: Vec<usize> = b.group.iter().zip(&w).filter(|(_, wi)| **wi > 0.0).map(|(g, _)| *g).collect();
        v.dedup();
        v.sort_unstable();
        v.dedup();
        v
    };
    let mut rng = seeded_rng(BOOTSTRAP_SEED);
    let mut draws: Vec<[f64; 5]> = Vec::with_capacity(args.draws);
    let mut not_converged = 0;
    for _ in 0..args.draws {
        let mut mult: HashMap<usize, f64> = HashMap::new();
        for _ in 0..fit_groups.len() {
            *mult.entry(fit_groups[rng.gen_range(0..fit_groups.len())]).or_default() += 1.0;
        }
        let wb: Vec<f64> = b.group.iter().map(|g| mult.get(g).copied().unwrap_or(0.0)).collect();
        let fb = clogit_fit(&b.choices, &wb, b.layout.names.len(), &primary.beta);
        if !fb.converged {
            not_converged += 1;
        }
        let d = d1(&fb.beta, &b, &wb);
        draws.push([fb.beta[th], d, fb.beta[th] / d, fb.beta[fee_ix], fb.beta[th] / fb.beta[fee_ix]]);
    }
    let col = |j: usize| {
        let mut v: Vec<f64> = draws.iter().map(|d| d[j]).collect();
        v.sort_by(f64::total_cmp);
        v
    };
    let stats_names = ["theta", "d1", "theta / d1 (months of earliness)", "beta_c", "theta / beta_c (points of payout)"];
    let points = [theta, d1_point, theta / d1_point, beta_c, theta / beta_c];
    let se_boot_theta = sd(&col(0));
    let p_theta = two_sided_p(theta / se_boot_theta);

    // Holm with the section 7 primary test.
    let demand = std::fs::read_to_string(args.out.join("demand_tests.tsv")).context("run p2_demand first")?;
    let p_demand: f64 = demand
        .lines()
        .find(|l| l.starts_with("primary\tfee\t"))
        .and_then(|l| l.split('\t').nth(6))
        .and_then(|v| v.parse().ok())
        .context("primary demand p-value not found")?;
    let adjusted = holm(&[p_demand, p_theta]);

    let mut rows = Vec::new();
    for (j, name) in stats_names.iter().enumerate() {
        let v = col(j);
        rows.push(vec![
            name.to_string(),
            f(points[j], 5),
            f(sd(&v), 5),
            f(percentile(&v, 0.025), 5),
            f(percentile(&v, 0.975), 5),
        ]);
    }
    rows.push(vec![format!("draws {} (not converged: {not_converged})", args.draws), String::new(), String::new(), String::new(), String::new()]);
    write_tsv(&args.out.join("calendar_primary_bootstrap.tsv"), &["statistic", "estimate", "bootstrap_se", "ci_low", "ci_high"], &rows)?;
    let theta_ci = (percentile(&col(0), 0.025), percentile(&col(0), 0.975));
    let supported = theta > 0.0 && theta_ci.0 > 0.0 && adjusted[1] < 0.05;
    let small = theta_ci.1 < d1_point;
    let reading = if supported && small {
        "dated need supported (theta > 0, interval excludes 0), and smaller than one month of earliness"
    } else if supported {
        "dated need supported (theta > 0, interval excludes 0)"
    } else if small {
        "no dated need of practical size (upper end of theta's interval below d1)"
    } else {
        "inconclusive"
    };
    println!("theta {theta:.4} (bootstrap se {se_boot_theta:.4}, 95% {:.4} to {:.4}); model se {:.4}; cluster se {:.4}", theta_ci.0, theta_ci.1, se_model[th], se_cluster[th]);
    println!("d1 {d1_point:.4}; theta/d1 {:.3}; beta_c {beta_c:.4}; theta/beta_c {:.3} points", theta / d1_point, theta / beta_c);
    println!("p theta {p_theta:.3e}; p demand {p_demand:.3e}; Holm adjusted: demand {:.3e}, theta {:.3e}", adjusted[0], adjusted[1]);
    println!("bootstrap draws {} (not converged {not_converged})", args.draws);
    println!("reading: {reading}");
    write_tsv(
        &args.out.join("calendar_primary_tests.tsv"),
        &["test", "estimate", "p_unadjusted", "p_holm", "reading"],
        &[
            vec!["section 7: beta (demand)".into(), String::new(), format!("{p_demand:.3e}"), format!("{:.3e}", adjusted[0]), String::new()],
            vec!["section 8: theta (dated need)".into(), f(theta, 5), format!("{p_theta:.3e}"), format!("{:.3e}", adjusted[1]), reading.into()],
        ],
    )?;

    // All primary parameters, for the paper and for the simulation.
    let rows: Vec<Vec<String>> = b
        .layout
        .names
        .iter()
        .enumerate()
        .map(|(j, n)| vec![n.clone(), f(primary.beta[j], 6), f(se_model[j], 6), f(se_cluster[j], 6)])
        .collect();
    write_tsv(&args.out.join("calendar_primary_params.tsv"), &["parameter", "estimate", "se_model", "se_cluster_group"], &rows)?;

    // ---------------------------------------------------- prediction check
    let mut no_h = PRIMARY.clone();
    no_h.with_h = false;
    let b0 = build(&no_h, &ranked, &posted);
    let w0 = fit_weights(&b0, &ranked, &no_h);
    let fit0 = fit(&b0, &w0);
    let ll1 = clogit_loglik_each(&b.choices, &primary.beta);
    let ll0 = clogit_loglik_each(&b0.choices, &fit0.beta);
    let hold_groups: Vec<usize> = {
        let mut v: Vec<usize> = b.group.iter().zip(&w).filter(|(_, wi)| **wi == 0.0).map(|(g, _)| *g).collect();
        v.sort_unstable();
        v.dedup();
        v
    };
    let mut per_group: HashMap<usize, (f64, f64)> = HashMap::new();
    for i in 0..b.choices.len() {
        if w[i] == 0.0 {
            let e = per_group.entry(b.group[i]).or_default();
            e.0 += ll1[i] - ll0[i];
            e.1 += 1.0;
        }
    }
    let diff_of = |gs: &[usize]| {
        let (s, c) = gs.iter().fold((0.0, 0.0), |a, g| (a.0 + per_group[g].0, a.1 + per_group[g].1));
        s / c
    };
    let point_diff = diff_of(&hold_groups);
    let mut rng = seeded_rng(BOOTSTRAP_SEED + 1);
    let mut diffs: Vec<f64> = (0..args.draws)
        .map(|_| {
            let draw: Vec<usize> = (0..hold_groups.len()).map(|_| hold_groups[rng.gen_range(0..hold_groups.len())]).collect();
            diff_of(&draw)
        })
        .collect();
    diffs.sort_by(f64::total_cmp);
    let mean_ll = |ll: &[f64]| {
        let (s, c) = ll.iter().zip(&w).filter(|(_, wi)| **wi == 0.0).fold((0.0, 0.0), |a, (l, _)| (a.0 + l, a.1 + 1.0));
        s / c
    };
    println!(
        "hold-out: mean log-likelihood per choice with H {:.5}, without H {:.5}, difference {point_diff:.5} (95% {:.5} to {:.5})",
        mean_ll(&ll1),
        mean_ll(&ll0),
        percentile(&diffs, 0.025),
        percentile(&diffs, 0.975)
    );
    write_tsv(
        &args.out.join("calendar_prediction_check.tsv"),
        &["holdout_choices", "mean_ll_with_H", "mean_ll_without_H", "difference", "ci_low", "ci_high", "point_higher_with_H", "interval_excludes_zero"],
        &[vec![
            n_hold.to_string(),
            f(mean_ll(&ll1), 5),
            f(mean_ll(&ll0), 5),
            f(point_diff, 5),
            f(percentile(&diffs, 0.025), 5),
            f(percentile(&diffs, 0.975), 5),
            (point_diff > 0.0).to_string(),
            (percentile(&diffs, 0.025) > 0.0 || percentile(&diffs, 0.975) < 0.0).to_string(),
        ]],
    )?;

    // ------------------------------------- secondary analyses and checks
    let others = vec![
        variant("flat regime only (no price)", &|s| s.regimes = "A"),
        variant("regimes B and C only", &|s| s.regimes = "BC"),
        variant("Eid al-Fitr and Eid al-Adha separately", &|s| s.split_eid = true),
        variant("without calendar-month effects", &|s| s.month_effects = false),
        variant("seat effects by regime", &|s| s.seat_by_regime = true),
        variant("windows of 30 days", &|s| s.window = Window::Days30),
        variant("windows of 60 days", &|s| s.window = Window::Days60),
        variant("cut-offs one month earlier", &|s| s.shift = -1),
        variant("cut-offs one month later", &|s| s.shift = 1),
        variant("paid fee instead of posted", &|s| s.paid_fee = true),
        variant("groups with multi-fee cells excluded", &|s| s.exclude_multi = true),
        variant("reweighted to the start-month mix of regime C", &|s| s.reweight = true),
    ];
    let mut rows = Vec::new();
    let mut report = |name: &str, bb: &Built, ww: &[f64], ff: &ClogitFit| {
        let n_choices = ww.iter().filter(|x| **x > 0.0).count();
        let (_, se) = clogit_se(&bb.choices, ww, ff);
        for term in ["H", "H fitr", "H adha", "school", "fee (beta_c)"] {
            if let Some(j) = bb.layout.get(term) {
                rows.push(vec![
                    name.to_string(),
                    term.to_string(),
                    f(ff.beta[j], 5),
                    f(se[j], 5),
                    format!("{:.3e}", two_sided_p(ff.beta[j] / se[j])),
                    n_choices.to_string(),
                    f(d1(&ff.beta, bb, ww), 5),
                    ff.converged.to_string(),
                ]);
            }
        }
        let j = bb.layout.get("H").or(bb.layout.get("H fitr"));
        if let Some(j) = j {
            println!("  {name:<48} {} {:.4} (cluster se {:.4}) choices {}", bb.layout.names[j], ff.beta[j], se[j], n_choices);
        }
    };
    report("primary (cluster-robust se)", &b, &w, &primary);
    for spec in &others {
        let bb = build(spec, &ranked, &posted);
        let ww = fit_weights(&bb, &ranked, spec);
        let ff = fit(&bb, &ww);
        report(spec.name, &bb, &ww, &ff);
        if spec.seat_by_regime {
            // Parameters for the simulation's sensitivity run (section 11).
            let rows: Vec<Vec<String>> = bb
                .layout
                .names
                .iter()
                .enumerate()
                .map(|(j, n)| vec![n.clone(), f(ff.beta[j], 6)])
                .collect();
            write_tsv(&args.out.join("calendar_params_seat_by_regime.tsv"), &["parameter", "estimate"], &rows)?;
        }
    }
    write_tsv(
        &args.out.join("calendar_checks.tsv"),
        &["specification", "term", "estimate", "se_cluster_group", "p", "choices", "d1", "converged"],
        &rows,
    )?;

    // Seat effects and month effects of the primary, for figures.
    let rows: Vec<Vec<String>> = b
        .layout
        .names
        .iter()
        .enumerate()
        .filter(|(_, n)| n.starts_with("seat") || n.starts_with("payout month"))
        .map(|(j, n)| vec![n.clone(), f(primary.beta[j], 5), f(se_cluster[j], 5)])
        .collect();
    write_tsv(&args.out.join("calendar_seat_and_month_effects.tsv"), &["parameter", "estimate", "se_cluster_group"], &rows)?;

    other_secondaries(&args, &members, &ranked, &held)?;
    exploratory_wtp(&args, &ranked, &posted)?;
    println!("tables written to {}", args.out.display());
    Ok(())
}

/// Secondary analyses of section 8 that are not conditional logits.
fn other_secondaries(args: &Args, members: &[Member], ranked: &[Ranked], held: &HashSet<String>) -> Result<()> {
    fn ix<T: Ord>(m: &mut BTreeMap<T, usize>, k: T) -> usize {
        let n = m.len();
        *m.entry(k).or_insert(n)
    }
    let mut rows = Vec::new();
    // (1) Join rank on H with seat, start-month and payout-month effects.
    {
        let (mut y, mut h, mut school) = (vec![], vec![], vec![]);
        let (mut fs, mut fst, mut fpm, mut cl) = (vec![], vec![], vec![], vec![]);
        let (mut a, mut bm, mut c, mut d) = (BTreeMap::new(), BTreeMap::new(), BTreeMap::new(), BTreeMap::new());
        for g in ranked.iter().filter(|g| !g.held) {
            for m in &g.members {
                y.push((m.join_rank.unwrap() as f64 - 1.0) / (m.n as f64 - 1.0));
                h.push((m.fitr45 || m.adha30) as u8 as f64);
                school.push(m.school30 as u8 as f64);
                fs.push(ix(&mut a, (m.seat, m.n)));
                fst.push(ix(&mut bm, m.start_month));
                fpm.push(ix(&mut c, m.payout_month));
                cl.push(ix(&mut d, g.label));
            }
        }
        if let Some(fit) = fe_ols(&y, &[h, school], &[fs, fst, fpm], None, &cl) {
            println!("  join rank on H (fitting sample): {:.4} (se {:.4}); school {:.4} (se {:.4})", fit.coef[0], fit.se[0], fit.coef[1], fit.se[1]);
            rows.push(vec!["join rank on H, seat, start-month and payout-month effects".into(), "H".into(), f(fit.coef[0], 5), f(fit.se[0], 5), format!("{:.3e}", fit.p(0)), fit.n_obs.to_string()]);
            rows.push(vec!["join rank on H, seat, start-month and payout-month effects".into(), "school".into(), f(fit.coef[1], 5), f(fit.se[1], 5), format!("{:.3e}", fit.p(1)), fit.n_obs.to_string()]);
        }
    }
    // (2) Repeat users: does a pre-Eid payout follow a pre-Eid payout?
    {
        let mut per_user: HashMap<&str, Vec<&Member>> = HashMap::new();
        for m in members {
            per_user.entry(m.user.as_str()).or_default().push(m);
        }
        for (label, with_seat) in [("next start month x length effects", false), ("plus next seat effects", true)] {
            let (mut y, mut x, mut f1, mut f2, mut cl) = (vec![], vec![], vec![], vec![], vec![]);
            let (mut a, mut bm, mut d) = (BTreeMap::new(), BTreeMap::new(), BTreeMap::new());
            for (u, ms) in &per_user {
                let mut ms = ms.clone();
                ms.sort_by_key(|m| (m.start_month, m.seat));
                for p in ms.windows(2) {
                    let (prev, next) = (p[0], p[1]);
                    if prev.group == next.group || held.contains(&prev.group) || held.contains(&next.group) {
                        continue;
                    }
                    y.push((next.fitr45 || next.adha30) as u8 as f64);
                    x.push((prev.fitr45 || prev.adha30) as u8 as f64);
                    f1.push(ix(&mut a, (next.start_month, next.n, if with_seat { next.seat } else { 0 })));
                    f2.push(ix(&mut bm, (prev.start_month, prev.n, prev.seat)));
                    cl.push(ix(&mut d, *u));
                }
            }
            if let Some(fit) = fe_ols(&y, &[x], &[f1, f2], None, &cl) {
                println!("  repeat users, pre-Eid after pre-Eid ({label}): {:.4} (se {:.4}), pairs {}", fit.coef[0], fit.se[0], fit.n_obs);
                rows.push(vec![format!("repeat users: H_next on H_prev, {label}, previous (start month, length, seat) effects"), "H_prev".into(), f(fit.coef[0], 5), f(fit.se[0], 5), format!("{:.3e}", fit.p(0)), fit.n_obs.to_string()]);
            }
        }
    }
    // (3) Choice of circle: do pre-Eid early seats sell faster?
    {
        let groups = breaking_binary::p2::read_groups(&args.data)?;
        let created: HashMap<&str, Option<i64>> = groups.iter().map(|g| (g.group.as_str(), g.days_created_before_start)).collect();
        let (mut y, mut h, mut fs, mut fst, mut fb, mut cl) = (vec![], vec![], vec![], vec![], vec![], vec![]);
        let (mut a, mut bm, mut c, mut d) = (BTreeMap::new(), BTreeMap::new(), BTreeMap::new(), BTreeMap::new());
        for m in members.iter().filter(|m| m.seat <= 2 && !held.contains(&m.group)) {
            let (Some(cr), Some(pd)) = (created[m.group.as_str()], m.days_paid_before_start) else { continue };
            y.push((cr - pd) as f64);
            h.push((m.fitr45 || m.adha30) as u8 as f64);
            fs.push(ix(&mut a, (m.seat, m.n)));
            fst.push(ix(&mut bm, m.start_month));
            fb.push(ix(&mut c, breaking_binary::p2::band(m.payout)));
            cl.push(ix(&mut d, m.group.as_str()));
        }
        if let Some(fit) = fe_ols(&y, &[h], &[fs, fst, fb], None, &cl) {
            println!("  seats 1-2, days on the market on H: {:.3} days (se {:.3}), n {}", fit.coef[0], fit.se[0], fit.n_obs);
            rows.push(vec!["circle choice: days on the market of seats 1-2 on H, (seat, length), start-month and band effects".into(), "H".into(), f(fit.coef[0], 4), f(fit.se[0], 4), format!("{:.3e}", fit.p(0)), fit.n_obs.to_string()]);
        }
    }
    write_tsv(&args.out.join("calendar_other_secondaries.tsv"), &["analysis", "term", "estimate", "se", "p", "obs"], &rows)?;
    let _ = month_label;
    Ok(())
}

/// Exploratory analysis E2: willingness to pay for a month of earliness,
/// from the model with seat effects by regime.
fn exploratory_wtp(args: &Args, ranked: &[Ranked], posted: &Posted) -> Result<()> {
    let spec = variant("seat effects by regime", &|s| s.seat_by_regime = true);
    let b = build(&spec, ranked, posted);
    let w = fit_weights(&b, ranked, &spec);
    let fit0 = fit(&b, &w);
    let fee_ix = b.layout.get("fee (beta_c)").context("no fee parameter")?;
    // d1 of one regime, from that regime's seat effects.
    let d1_of = |beta: &[f64], ww: &[f64], regime: char| {
        let mut cells: BTreeMap<u32, f64> = BTreeMap::new();
        for ((&n, &r), &wi) in b.length.iter().zip(&b.regime).zip(ww) {
            if r == regime {
                *cells.entry(n).or_default() += wi;
            }
        }
        let (mut num, mut den) = (0.0, 0.0);
        for (&n, &count) in &cells {
            let h = n / 2;
            if h < 2 || count == 0.0 {
                continue;
            }
            let Some(j) = b.layout.get(&format!("seat {regime} N={n} s={h}")) else { continue };
            num += count * (-beta[j] / (h as f64 - 1.0));
            den += count;
        }
        num / den
    };
    // Posted price of a month on the same seats: (f_1 - f_h) / (h - 1),
    // at each chooser's payment month, averaged over fitting choices.
    let price_of = |regime: char| {
        let (mut num, mut den) = (0.0, 0.0);
        for g in ranked.iter().filter(|g| !g.held && g.members[0].regime == regime) {
            let n = g.members[0].n;
            let h = n / 2;
            if h < 2 {
                continue;
            }
            for c in &g.members[..g.members.len() - 1] {
                let Some(pm) = c.paid_month else { continue };
                let p = c.payout_key();
                let (Some(f1), Some(fh)) = (posted.get(pm, n, p, 1), posted.get(pm, n, p, h)) else { continue };
                num += 100.0 * (f1 - fh) / c.payout / (h as f64 - 1.0);
                den += 1.0;
            }
        }
        num / den
    };
    let fit_groups: Vec<usize> = {
        let mut v: Vec<usize> = b.group.iter().zip(&w).filter(|(_, wi)| **wi > 0.0).map(|(g, _)| *g).collect();
        v.sort_unstable();
        v.dedup();
        v
    };
    let mut rng = seeded_rng(12_2026_11_19);
    let mut draws: Vec<[f64; 5]> = Vec::new();
    let mut not_converged = 0;
    for _ in 0..args.draws {
        let mut mult: HashMap<usize, f64> = HashMap::new();
        for _ in 0..fit_groups.len() {
            *mult.entry(fit_groups[rng.gen_range(0..fit_groups.len())]).or_default() += 1.0;
        }
        let wb: Vec<f64> = b.group.iter().map(|g| mult.get(g).copied().unwrap_or(0.0)).collect();
        let fb = clogit_fit(&b.choices, &wb, b.layout.names.len(), &fit0.beta);
        if !fb.converged {
            not_converged += 1;
        }
        let bc = fb.beta[fee_ix];
        let (db, dc) = (d1_of(&fb.beta, &wb, 'B'), d1_of(&fb.beta, &wb, 'C'));
        draws.push([bc, db, dc, db / bc, dc / bc]);
    }
    let bc = fit0.beta[fee_ix];
    let (db, dc) = (d1_of(&fit0.beta, &w, 'B'), d1_of(&fit0.beta, &w, 'C'));
    let points = [bc, db, dc, db / bc, dc / bc];
    let names = ["beta_c", "d1 regime B", "d1 regime C", "WTP for a month, regime B (points of payout)", "WTP for a month, regime C (points of payout)"];
    let mut rows = Vec::new();
    for (j, name) in names.iter().enumerate() {
        let mut v: Vec<f64> = draws.iter().map(|d| d[j]).collect();
        v.sort_by(f64::total_cmp);
        rows.push(vec![name.to_string(), f(points[j], 4), f(percentile(&v, 0.025), 4), f(percentile(&v, 0.975), 4)]);
    }
    let (pb, pc) = (price_of('B'), price_of('C'));
    rows.push(vec!["posted price of a month on the same seats, regime B".into(), f(pb, 4), String::new(), String::new()]);
    rows.push(vec!["posted price of a month on the same seats, regime C".into(), f(pc, 4), String::new(), String::new()]);
    rows.push(vec![format!("draws {} (not converged: {not_converged})", args.draws), String::new(), String::new(), String::new()]);
    write_tsv(&args.out.join("explore_e2_wtp.tsv"), &["statistic", "estimate", "ci_low", "ci_high"], &rows)?;
    for r in &rows {
        println!("  E2 {}: {} [{} to {}]", r[0], r[1], r[2], r[3]);
    }
    Ok(())
}
