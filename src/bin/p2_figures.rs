//! Paper 2: figures, drawn from the aggregate result tables in the results
//! folder (never from the data). Output: SVG files in <out>/figures.
//!
//! Usage (after the other p2 programs):
//!     cargo run --release --bin p2_figures -- --out private/results/paper-2

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::Parser;

use breaking_binary::p2::{month_label, parse_month};
use breaking_binary::svgplot::{render, render_sized, Panel, Series};

#[derive(Parser)]
struct Args {
    #[arg(long, default_value = "private/results/paper-2")]
    out: PathBuf,
}

/// Rows of a TSV as maps from column name to value.
fn table(path: &Path) -> Result<Vec<BTreeMap<String, String>>> {
    let text = std::fs::read_to_string(path).with_context(|| format!("cannot read {}", path.display()))?;
    let mut lines = text.lines();
    let header: Vec<&str> = lines.next().context("empty table")?.split('\t').collect();
    Ok(lines
        .map(|l| header.iter().zip(l.split('\t')).map(|(h, v)| (h.to_string(), v.to_string())).collect())
        .collect())
}

fn num(r: &BTreeMap<String, String>, k: &str) -> f64 {
    r.get(k).and_then(|v| v.parse().ok()).unwrap_or(f64::NAN)
}

const REGIMES: [(&str, &str); 3] = [("A", "Flat fee"), ("B", "Early seats priced"), ("C", "Every seat priced")];

fn main() -> Result<()> {
    let args = Args::parse();
    let dir = args.out.join("figures");
    std::fs::create_dir_all(&dir)?;
    let legend: Vec<(usize, String)> = REGIMES.iter().enumerate().map(|(i, (_, n))| (i, n.to_string())).collect();

    // Figure 1: share of first joiners taking seat 1, by start month.
    let t = table(&args.out.join("demand_switch_series.tsv"))?;
    let pts: Vec<(f64, f64)> = t
        .iter()
        .filter(|r| r["sample"] == "groups with complete join order" && num(r, "groups") >= 20.0)
        .map(|r| (parse_month(&r["start_month"]).unwrap() as f64, num(r, "first_joiner_seat1_pct")))
        .collect();
    let (b, c) = (parse_month("2023-03").unwrap() as f64, parse_month("2024-03").unwrap() as f64);
    let last = pts.last().map(|p| p.0).unwrap_or(c);
    let first = pts.first().map(|p| p.0).unwrap_or(b);
    let ticks: Vec<(f64, String)> = (first as i32..=last as i32)
        .filter(|m| m.rem_euclid(6) == 0)
        .map(|m| (m as f64, month_label(m)))
        .collect();
    let panel = Panel {
        title: "First joiners who take seat 1".into(),
        x_label: "Month the circle started (months with 20+ circles)".into(),
        y_label: "% of circles".into(),
        series: vec![Series { name: String::new(), slot: 0, points: pts, interval: None, line: true }],
        bands: vec![(b - 0.5, c - 0.5, "Early seats priced".into()), (c - 0.5, last + 0.5, "All".into())],
        hlines: vec![],
        x_ticks: ticks,
        y_range: Some((0.0, 100.0)),
        x_range: Some((first - 0.5, last + 0.5)),
        direct_labels: false,
    };
    std::fs::write(dir.join("fig1_first_joiner_seat1.svg"), render_sized("Figure 1. The first seat goes to the first joiner more and more often", &[panel], 1, &[], 680.0, 300.0))?;

    // Figure 2: posted fee schedule by seat.
    let t = table(&args.out.join("measures2_3_schedule.tsv"))?;
    let panels: Vec<Panel> = [4u32, 6, 12]
        .iter()
        .map(|&n| Panel {
            title: format!("{n}-month circles"),
            x_label: "Seat".into(),
            y_label: "Median posted fee, % of payout".into(),
            series: REGIMES
                .iter()
                .enumerate()
                .map(|(i, (r, name))| Series {
                    name: name.to_string(),
                    slot: i,
                    points: t
                        .iter()
                        .filter(|x| x["regime"] == *r && x["length"] == n.to_string())
                        .map(|x| (num(x, "seat"), num(x, "median_posted_fee_pct")))
                        .collect(),
                    interval: None,
                    line: true,
                })
                .collect(),
            x_ticks: (1..=n).filter(|s| n <= 6 || s % 2 == 1 || *s == n).map(|s| (s as f64, s.to_string())).collect(),
            ..Default::default()
        })
        .collect();
    std::fs::write(dir.join("fig2_posted_schedules.svg"), render("Figure 2. Posted join fees by seat", &panels, 3, &legend))?;

    // Figure 3: seat effects of the primary choice model.
    let t = table(&args.out.join("calendar_seat_and_month_effects.tsv"))?;
    let panels: Vec<Panel> = [4u32, 6, 12]
        .iter()
        .map(|&n| {
            let mut pts = vec![(1.0, 0.0)];
            let mut iv = vec![(0.0, 0.0)];
            for s in 2..=n {
                if let Some(r) = t.iter().find(|r| r["parameter"] == format!("seat N={n} s={s}")) {
                    let (e, se) = (num(r, "estimate"), num(r, "se_cluster_group"));
                    pts.push((s as f64, e));
                    iv.push((e - 1.96 * se, e + 1.96 * se));
                }
            }
            Panel {
                title: format!("{n}-month circles"),
                x_label: "Seat".into(),
                y_label: "Seat effect (seat 1 = 0)".into(),
                series: vec![Series { name: String::new(), slot: 0, points: pts, interval: Some(iv), line: true }],
                x_ticks: (1..=n).filter(|s| n <= 6 || s % 2 == 1 || *s == n).map(|s| (s as f64, s.to_string())).collect(),
                ..Default::default()
            }
        })
        .collect();
    std::fs::write(dir.join("fig3_seat_effects.svg"), render("Figure 3. How much members value each seat (choice model, 95% intervals)", &panels, 3, &[]))?;

    // Figure 4: the pre-Eid effect across specifications.
    let t = table(&args.out.join("calendar_checks.tsv"))?;
    let specs: [(&str, &str, &str); 12] = [
        ("primary (cluster-robust se)", "H", "Primary"),
        ("flat regime only (no price)", "H", "Flat fee only"),
        ("regimes B and C only", "H", "Priced only"),
        ("Eid al-Fitr and Eid al-Adha separately", "H fitr", "Fitr alone"),
        ("Eid al-Fitr and Eid al-Adha separately", "H adha", "Adha alone"),
        ("without calendar-month effects", "H", "No month effects"),
        ("seat effects by regime", "H", "Seat x regime"),
        ("windows of 30 days", "H", "30-day window"),
        ("windows of 60 days", "H", "60-day window"),
        ("paid fee instead of posted", "H", "Paid fee"),
        ("groups with multi-fee cells excluded", "H", "One fee per cell"),
        ("reweighted to the start-month mix of regime C", "H", "Reweighted"),
    ];
    let mut pts = Vec::new();
    let mut iv = Vec::new();
    let mut ticks = Vec::new();
    for (i, (spec, term, label)) in specs.iter().enumerate() {
        let r = t.iter().find(|r| r["specification"] == *spec && r["term"] == *term).context("missing spec")?;
        let (e, se) = (num(r, "estimate"), num(r, "se_cluster_group"));
        pts.push((i as f64 + 1.0, e));
        iv.push((e - 1.96 * se, e + 1.96 * se));
        ticks.push((i as f64 + 1.0, label.to_string()));
    }
    let panel = Panel {
        title: "Effect of a payout in the weeks before Eid, by specification".into(),
        x_label: "Specification".into(),
        y_label: "Coefficient (one month of earliness = 1.30)".into(),
        series: vec![Series { name: String::new(), slot: 0, points: pts, interval: Some(iv), line: false }],
        hlines: vec![(0.0, "zero".into())],
        x_ticks: ticks,
        x_range: Some((0.4, 12.6)),
        ..Default::default()
    };
    let svg = render_sized("Figure 4. No specification finds a positive Eid effect", &[panel], 1, &[], 1020.0, 300.0);
    std::fs::write(dir.join("fig4_eid_effect.svg"), svg)?;

    // Figure 5: vacancy by seat.
    let t = table(&args.out.join("risk_vacancy_by_seat.tsv"))?;
    let panels: Vec<Panel> = [4u32, 6, 12]
        .iter()
        .map(|&n| Panel {
            title: format!("{n}-month circles"),
            x_label: "Seat".into(),
            y_label: "Seats with no listed holder, %".into(),
            series: REGIMES
                .iter()
                .enumerate()
                .map(|(i, (r, name))| Series {
                    name: name.to_string(),
                    slot: i,
                    points: t
                        .iter()
                        .filter(|x| x["regime"] == *r && x["length"] == n.to_string() && num(x, "groups") >= 30.0)
                        .map(|x| (num(x, "seat"), num(x, "vacancy_pct")))
                        .collect(),
                    interval: None,
                    line: true,
                })
                .collect(),
            x_ticks: (1..=n).filter(|s| n <= 6 || s % 2 == 1 || *s == n).map(|s| (s as f64, s.to_string())).collect(),
            ..Default::default()
        })
        .collect();
    std::fs::write(dir.join("fig5_vacancy.svg"), render("Figure 5. Under pricing, the empty seats move to the end", &panels, 3, &legend))?;

    // Figure 6: the platform against verified benchmarks (notes/benchmark-verification.md).
    let m1 = table(&args.out.join("measure1_bootstrap.tsv"))?;
    let c = m1.iter().find(|r| r["regime"] == "C").context("no regime C")?;
    let e1 = table(&args.out.join("explore_e1_seat_cost.tsv"))?;
    let seat1 = |n: &str| {
        e1.iter()
            .find(|r| r["regime"] == "C" && r["seat"] == "1" && r["length"] == n && r["income_level"] == "all")
            .map(|r| (num(r, "median_pct_month"), num(r, "p25_pct_month"), num(r, "p75_pct_month")))
            .unwrap_or((f64::NAN, f64::NAN, f64::NAN))
    };
    // (label, estimate, low, high, slot): slot 0 = this platform, 1 = literature, 2 = consumer credit.
    let left: Vec<(String, f64, f64, f64, usize)> = vec![
        ("This platform".into(), num(c, "median"), num(c, "ci_low"), num(c, "ci_high"), 0),
        ("E&M, before cap".into(), 1.46, 1.46, 1.46, 1),
        ("E&M, after cap".into(), 1.10, 1.10, 1.10, 1),
        ("C&R, first draw".into(), 3.29, 3.29, 3.29, 1),
    ];
    let (a, al, ah) = seat1("4");
    let (b6, bl, bh) = seat1("6");
    let (d, dl, dh) = seat1("12");
    let right: Vec<(String, f64, f64, f64, usize)> = vec![
        ("4-month".into(), a, al, ah, 0),
        ("6-month".into(), b6, bl, bh, 0),
        ("12-month".into(), d, dl, dh, 0),
        ("Klonner".into(), 1.6, 1.6, 1.6, 1),
        ("Card A".into(), 2.5, 2.5, 2.5, 2),
        ("Card B".into(), 3.0, 3.0, 3.0, 2),
    ];
    let mk = |title: &str, y: &str, rows: &[(String, f64, f64, f64, usize)]| {
        let series: Vec<Series> = (0..3)
            .filter_map(|slot| {
                let pts: Vec<(usize, &(String, f64, f64, f64, usize))> =
                    rows.iter().enumerate().filter(|(_, r)| r.4 == slot).collect();
                (!pts.is_empty()).then(|| Series {
                    name: String::new(),
                    slot,
                    points: pts.iter().map(|(i, r)| (*i as f64 + 1.0, r.1)).collect(),
                    interval: Some(pts.iter().map(|(_, r)| (r.2, r.3)).collect()),
                    line: false,
                })
            })
            .collect();
        Panel {
            title: title.into(),
            x_label: String::new(),
            y_label: y.into(),
            series,
            x_ticks: rows.iter().enumerate().map(|(i, r)| (i as f64 + 1.0, r.0.clone())).collect(),
            x_range: Some((0.4, rows.len() as f64 + 0.6)),
            y_range: Some((0.0, 6.0)),
            ..Default::default()
        }
    };
    let panels = vec![
        mk("Price of time: discount on an early pot", "% a month", &left),
        mk("Cost to the first recipient (seat 1: median, quartiles)", "% a month", &right),
    ];
    let legend = vec![(0, "This platform (every seat priced)".to_string()), (1, "ROSCA literature".to_string()), (2, "Saudi Islamic credit cards".to_string())];
    std::fs::write(
        dir.join("fig6_benchmarks.svg"),
        render_sized("Figure 6. Time is priced like a chit fund; the first seat costs more than a credit card", &panels, 2, &legend, 520.0, 280.0),
    )?;
    println!("figures written to {}", dir.display());
    Ok(())
}
