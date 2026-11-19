//! Paper 2, section 4: draw the 20% hold-out of ranked groups, once,
//! before any model of section 8 is fitted.
//!
//! The ranked groups are sorted by label and shuffled with a fixed seed.
//! The first 20% (rounded) form the hold-out. The list is written to a
//! file and is never drawn again: the program stops if the file exists.
//!
//! The group labels are random and change whenever `anonymise` is rerun.
//! The list therefore belongs to one build of the tables. The program also
//! writes the size of the groups table, so that a mismatch can be seen.
//!
//! Usage:
//!     cargo run --release --bin p2_holdout -- --data data/anonymised --out private/results/paper-2

use std::path::PathBuf;

use anyhow::{bail, Result};
use clap::Parser;
use rand::seq::SliceRandom;

use breaking_binary::p2::{read_groups, write_tsv};
use breaking_binary::seeded_rng;

/// Seed of the hold-out draw.
pub const HOLDOUT_SEED: u64 = 4_2026_11_19;
const SHARE: f64 = 0.2;

#[derive(Parser)]
struct Args {
    #[arg(long, default_value = "data/anonymised")]
    data: PathBuf,
    #[arg(long, default_value = "private/results/paper-2")]
    out: PathBuf,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let path = args.out.join("holdout_groups.tsv");
    if path.exists() {
        bail!("{} exists. The hold-out is drawn once only.", path.display());
    }
    let groups = read_groups(&args.data)?;
    let mut ranked: Vec<&str> =
        groups.iter().filter(|g| g.usable).map(|g| g.group.as_str()).collect();
    ranked.sort_unstable();
    let mut rng = seeded_rng(HOLDOUT_SEED);
    ranked.shuffle(&mut rng);
    let k = (SHARE * ranked.len() as f64).round() as usize;
    let mut held: Vec<&str> = ranked[..k].to_vec();
    held.sort_unstable();
    std::fs::create_dir_all(&args.out)?;
    let rows: Vec<Vec<String>> = held.iter().map(|g| vec![g.to_string()]).collect();
    write_tsv(&path, &["group"], &rows)?;
    println!(
        "ranked groups {}; hold-out {} (seed {HOLDOUT_SEED}); groups table rows {}",
        ranked.len(),
        k,
        groups.len()
    );
    Ok(())
}
