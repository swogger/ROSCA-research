# ROSCA research

Code for two studies of a digital rotating savings and credit association (ROSCA). In a circle of N members, each member pays the same instalment every month and one member receives the whole pot. Members choose their seat, and seat k is paid in month k. A payout at the start works like a loan. A payout at the end works like a savings plan. The platform first charged every seat the same join fee, then priced early seats higher.

- **Paper 2, "The Price of Going First"** (in preparation). What an earlier payout costs, whether members respond to seat prices, who pays, and whether members pay for earliness or for a payout timed before a dated expense such as Eid.
- **Paper 1, "Breaking the Binary"** (extension of a conference paper, in preparation). How people choose between borrowing-like and saving-like positions.

All code is written in Rust.

## Data

The data are confidential and are not part of this repository. They are available to journal editors and reviewers on request, for peer review only. See `data/README.md` for what the anonymised tables contain and how they were built.

## Layout

| Path | Content |
|---|---|
| `Cargo.toml`, `Cargo.lock` | Package definition and exact dependency versions |
| `src/lib.rs` | Shared helpers |
| `src/anonymity.rs` | Anonymisation rules and their tests |
| `src/calendar.rs` | Payout dates and holiday indicators (Eid al-Fitr, Eid al-Adha, first school day) |
| `src/p2.rs` | Paper 2: reading the anonymised tables, posted fees, regimes, seat rates |
| `src/stats.rs` | Estimators: least squares with absorbed fixed effects and clustered errors, conditional logit, Holm correction |
| `src/svgplot.rs` | Static SVG charts for the figures |
| `src/bin/` | One program per analysis step (below) |
| `data/README.md` | Description of the data |

## Running the code

```
cargo test
cargo run --release --bin anonymise -- --raw data/data.tsv --out data/anonymised
```

The first command runs the tests. The second builds the anonymised tables from the raw export. All analyses read the anonymised tables only, and every program prints and writes aggregate figures only.

Paper 2, in order (each program reads `data/anonymised` and writes to `--out`, by default `private/results/paper-2`):

| Program | Plan section | What it does |
|---|---|---|
| `p2_price` | 5 | Price of going first, price of one month, seat rates |
| `p2_incidence` | 6 | Who pays the price of going first |
| `p2_demand` | 7 | Demand test (seat's join rank on its fee) and its checks |
| `p2_holdout` | 4 | Draws the 20% hold-out of groups, once, before any choice model |
| `p2_calendar` | 8 | Conditional logit of seat choice: pre-Eid payouts against earliness |
| `p2_risk` | 9 | Break-even default rates and vacancy by seat |
| `p2_price_checks` | 10 | Checks of the price measure |
| `p2_simulate` | 11 | Fee schedules compared with the fitted choice model |
| `p2_explore` | after the tests | Exploratory analyses E1, E3 and E4 (E2 is in `p2_calendar`) |
| `p2_figures` | | Figures from the result tables |

## Analysis plan

The Paper 2 analysis plan was frozen on 19 November 2026, before any confirmatory estimate was computed. It was written after some descriptive figures had been inspected, and it lists them. The plan, its clarifications and every deviation will be published with the paper. Its SHA-256 at the freeze was

    0182d10455f721efe331069ed7fe6fcc321c1da4c0091f95f94bd481cc5f430e

This commit records that fingerprint. It is not a third-party registration.
