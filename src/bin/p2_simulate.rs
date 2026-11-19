//! Paper 2, section 11: compare fee schedules with the fitted seat-choice
//! model of section 8.
//!
//! Members arrive one by one and each takes an open seat with the logit
//! probabilities of the fitted model. The last member takes the last open
//! seat. Schedules (fees in points of the payout, clarification 6):
//!   * flat: the median posted fee of each seat in regime A;
//!   * B and C: the median posted schedules of regimes B and C;
//!   * linear: from C's seat 1 fee to C's last-seat fee in equal steps;
//!   * late bonus: C's schedule with a bonus of 1 point of the payout on
//!     the last seat (its fee minus 1).
//! Outputs: mean fill rank and the probability of filling first and last,
//! by seat, and fee income per group. The model is conditional on joining,
//! so it says nothing about how many people join.
//!
//! Start dates: the 15th of every month from January 2022 to December
//! 2024, so that the holiday and month effects average over the calendar.
//!
//! Usage (after p2_calendar):
//!     cargo run --release --bin p2_simulate -- --data data/anonymised --out private/results/paper-2

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::{Datelike, NaiveDate};
use clap::Parser;
use rand::Rng;

use breaking_binary::calendar::{payout_date, pays_before, Event};
use breaking_binary::p2::{f, read_members, write_tsv, Posted};
use breaking_binary::seeded_rng;
use breaking_binary::stats::weighted_median;

const SEED: u64 = 11_2026_11_19;
const RUNS_PER_START: usize = 500;
const LENGTHS: [u32; 6] = [4, 5, 6, 8, 10, 12];

#[derive(Parser)]
struct Args {
    #[arg(long, default_value = "data/anonymised")]
    data: PathBuf,
    #[arg(long, default_value = "private/results/paper-2")]
    out: PathBuf,
}

fn read_params(path: &Path) -> Result<HashMap<String, f64>> {
    let text = std::fs::read_to_string(path).with_context(|| format!("cannot read {}", path.display()))?;
    let mut out = HashMap::new();
    for line in text.lines().skip(1) {
        let mut parts = line.split('\t');
        let (Some(name), Some(value)) = (parts.next(), parts.next()) else { continue };
        out.insert(name.to_string(), value.parse()?);
    }
    Ok(out)
}

struct Model {
    name: &'static str,
    params: HashMap<String, f64>,
    /// Prefix of the seat-effect names ("seat N=" or "seat C N=").
    seat_prefix: &'static str,
}

impl Model {
    fn seat_effect(&self, n: u32, s: u32) -> Option<f64> {
        if s == 1 {
            return Some(0.0);
        }
        self.params.get(&format!("{}{n} s={s}", self.seat_prefix)).copied()
    }
    fn get(&self, name: &str) -> f64 {
        self.params.get(name).copied().unwrap_or(0.0)
    }
}

fn main() -> Result<()> {
    let args = Args::parse();
    let members = read_members(&args.data)?;
    let posted = Posted::build(&members);
    let mut cells: BTreeMap<(char, u32, u32), Vec<(f64, f64)>> = BTreeMap::new();
    for m in &members {
        if let Some(fee) = posted.of(m) {
            cells.entry((m.regime, m.n, m.seat)).or_default().push((100.0 * fee / m.payout, 1.0));
        }
    }
    let median: BTreeMap<(char, u32, u32), f64> =
        cells.iter_mut().map(|(k, v)| (*k, weighted_median(v))).collect();

    let models = [
        Model { name: "primary", params: read_params(&args.out.join("calendar_primary_params.tsv"))?, seat_prefix: "seat N=" },
        Model {
            name: "seat effects by regime (regime C seat effects)",
            params: read_params(&args.out.join("calendar_params_seat_by_regime.tsv"))?,
            seat_prefix: "seat C N=",
        },
    ];
    let starts: Vec<NaiveDate> = (2022..=2024)
        .flat_map(|y| (1..=12).map(move |m| NaiveDate::from_ymd_opt(y, m, 15).unwrap()))
        .collect();

    let mut rng = seeded_rng(SEED);
    let (mut rows, mut summary) = (Vec::new(), Vec::new());
    for model in &models {
        let theta = model.get("H");
        let lambda = model.get("school");
        let beta_c = model.get("fee (beta_c)");
        for n in LENGTHS {
            let Some(delta) = (1..=n).map(|s| model.seat_effect(n, s)).collect::<Option<Vec<f64>>>() else {
                continue;
            };
            let sched = |r: char| (1..=n).map(|s| median.get(&(r, n, s)).copied()).collect::<Option<Vec<f64>>>();
            let (Some(a), Some(b), Some(c)) = (sched('A'), sched('B'), sched('C')) else { continue };
            let linear: Vec<f64> =
                (0..n).map(|i| c[0] + (c[n as usize - 1] - c[0]) * i as f64 / (n as f64 - 1.0)).collect();
            let mut bonus = c.clone();
            bonus[n as usize - 1] -= 1.0;
            let schedules = [("flat (regime A)", a), ("regime B", b), ("regime C", c), ("linear", linear), ("late-seat bonus", bonus)];
            for (sname, fees) in &schedules {
                let mut rank_sum = vec![0.0; n as usize];
                let mut first = vec![0.0; n as usize];
                let mut last = vec![0.0; n as usize];
                let mut total = 0.0;
                for start in &starts {
                    // Fixed part of each seat's score at this start date.
                    let base: Vec<f64> = (1..=n)
                        .map(|s| {
                            let pay = payout_date(*start, s);
                            let h = pays_before(pay, Event::EidAlFitr, 45) || pays_before(pay, Event::EidAlAdha, 30);
                            let school = pays_before(pay, Event::SchoolStart, 30);
                            let moy = pay.month();
                            let month = if moy > 1 { model.get(&format!("payout month {moy:02}")) } else { 0.0 };
                            delta[s as usize - 1]
                                + theta * h as u8 as f64
                                + lambda * school as u8 as f64
                                + month
                                - beta_c * fees[s as usize - 1]
                        })
                        .collect();
                    for _ in 0..RUNS_PER_START {
                        let mut open: Vec<usize> = (0..n as usize).collect();
                        for rank in 0..n as usize {
                            let pick = if open.len() == 1 {
                                0
                            } else {
                                let w: Vec<f64> = open.iter().map(|&s| base[s].exp()).collect();
                                let u = rng.gen::<f64>() * w.iter().sum::<f64>();
                                let mut acc = 0.0;
                                let mut k = w.len() - 1;
                                for (i, wi) in w.iter().enumerate() {
                                    acc += wi;
                                    if u < acc {
                                        k = i;
                                        break;
                                    }
                                }
                                k
                            };
                            let s = open.remove(pick);
                            rank_sum[s] += (rank + 1) as f64;
                            if rank == 0 {
                                first[s] += 1.0;
                            }
                            if rank == n as usize - 1 {
                                last[s] += 1.0;
                            }
                        }
                        total += 1.0;
                    }
                }
                for s in 0..n as usize {
                    rows.push(vec![
                        model.name.to_string(),
                        n.to_string(),
                        sname.to_string(),
                        (s + 1).to_string(),
                        f(fees[s], 3),
                        f(rank_sum[s] / total, 3),
                        f(100.0 * first[s] / total, 1),
                        f(100.0 * last[s] / total, 1),
                    ]);
                }
                let income: f64 = fees.iter().sum();
                let last_seat = (0..n as usize).max_by(|&i, &j| last[i].total_cmp(&last[j])).unwrap() + 1;
                summary.push(vec![
                    model.name.to_string(),
                    n.to_string(),
                    sname.to_string(),
                    f(income, 3),
                    f(100.0 * first[0] / total, 1),
                    f(rank_sum[n as usize - 1] / total, 3),
                    last_seat.to_string(),
                    f(100.0 * last[last_seat - 1] / total, 1),
                ]);
            }
        }
    }
    write_tsv(
        &args.out.join("simulation_by_seat.tsv"),
        &["model", "length", "schedule", "seat", "fee_pct", "mean_fill_rank", "fills_first_pct", "fills_last_pct"],
        &rows,
    )?;
    write_tsv(
        &args.out.join("simulation_summary.tsv"),
        &[
            "model", "length", "schedule", "fee_income_pct_of_payout", "seat1_taken_first_pct",
            "last_seat_mean_fill_rank", "seat_most_often_last", "its_fills_last_pct",
        ],
        &summary,
    )?;
    println!("simulation tables written to {}", args.out.display());
    Ok(())
}
