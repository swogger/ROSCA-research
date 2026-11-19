//! Build the anonymised analysis tables from the raw platform export.
//!
//! Input:  the raw user-level export (one row per user, tab-separated).
//! Output: three tab-separated tables in the output folder.
//!
//!     users.tsv        one row per user
//!     groups.tsv       one row per savings circle (group)
//!     memberships.tsv  one row per user and group, with the chosen seat
//!
//! What the program removes or reduces:
//!   * user and group ids are replaced by random labels. The seed is not
//!     stored, so the labels cannot be traced back to the ids;
//!   * the date of birth is reduced to an age band;
//!   * the city is reduced to a rank label for the largest cities;
//!   * income, employment and income source are reduced to broad groups;
//!   * every date is reduced to a month, or to a whole number of days
//!     before the group starts. The payout date of each seat is reduced to
//!     its month and to yes/no flags for the days before Eid al-Fitr,
//!     Eid al-Adha and the first school day;
//!   * counters are capped and amounts are rounded;
//!   * no combination of gender, age band, city, income and employment
//!     is shared by fewer than five people. Values are removed until
//!     this holds.
//!
//! The program contains no data values and prints aggregate counts only.
//! It stops with an error if the five-person rule cannot be met.
//!
//! Usage:
//!     cargo run --release --bin anonymise -- --raw data/data.tsv --out data/anonymised

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use chrono::{DateTime, Datelike, Utc};
use clap::Parser;

use breaking_binary::anonymity::{
    canonical_city, cap_count, enforce_k_anonymity, income_band, register_period, round_amount,
    sector_group, source_group, K,
};
use breaking_binary::calendar::{
    first_covered_payout, last_covered_payout, local_date, pays_before, payout_date, Event,
};
use breaking_binary::{
    age_band, birth_year, entropy_rng, month, object_id_time, parse_time, random_labels,
    with_commas, Regime, N_NAMED_CITIES,
};

/// Column groups that hold memberships: prefix and number of slots.
const MEMBERSHIP_SLOTS: [(&str, usize); 2] = [("completed", 5), ("running", 2)];

/// The blank header cell sits where this column belongs.
const BLANK_HEADER_INDEX: usize = 4;
const BLANK_HEADER_NAME: &str = "completed.0.startDate";

/// Attributes an outsider could plausibly know about a person.
const QUASI_IDENTIFIERS: [&str; 5] =
    ["gender", "age_band", "city_group", "income_level", "employment_sector"];
/// Order in which values are removed to meet the five-person rule.
/// The attribute that matters least for the analysis goes first.
const SUPPRESSION_ORDER: [usize; 5] = [4, 3, 2, 1, 0];

/// Output name and raw column of the capped counters.
const USER_COUNTERS: [(&str, &str); 9] = [
    ("n_completed", "completedJameyaCount"),
    ("n_running", "runningJameyasCount"),
    ("n_dismissed", "dismissedJameyasCount"),
    ("n_left", "jameyaLeftCount"),
    ("n_ejected", "ejectedCount"),
    ("n_in_schedule", "userInScheduleCount"),
    ("n_not_in_schedule", "userNotInScheduleCount"),
    ("n_deferrals", "emhalCount"),
    ("n_late_payments", "latePayments"),
];

/// Output name, event and window in days of the holiday flags
/// (Paper 2 plan, section 4). The first three are the main indicators.
const HOLIDAY_FLAGS: [(&str, Event, i64); 7] = [
    ("pay_before_eid_fitr", Event::EidAlFitr, 45),
    ("pay_before_eid_adha", Event::EidAlAdha, 30),
    ("pay_before_school", Event::SchoolStart, 30),
    ("pay_before_eid_fitr_30", Event::EidAlFitr, 30),
    ("pay_before_eid_fitr_60", Event::EidAlFitr, 60),
    ("pay_before_eid_adha_60", Event::EidAlAdha, 60),
    ("pay_before_school_60", Event::SchoolStart, 60),
];

/// Output name, raw column, rounding step and cap of the rounded amounts.
const USER_AMOUNTS: [(&str, &str, f64, f64); 2] = [
    ("extra_fees_amount", "extraFeesAmount", 100.0, 1000.0),
    ("total_join_fees_paid", "totalJoinFeesPaid", 100.0, 5000.0),
];

#[derive(Parser)]
#[command(about = "Build the anonymised analysis tables from the raw platform export")]
struct Args {
    /// Raw user-level export (tab-separated).
    #[arg(long)]
    raw: PathBuf,
    /// Folder for the output tables.
    #[arg(long)]
    out: PathBuf,
    /// Private file of city spellings: variant, tab, canonical spelling.
    /// It stays with the raw data and is not shared.
    #[arg(long, default_value = "data/city_aliases.tsv")]
    city_aliases: PathBuf,
}

/// Read the private table of city spellings.
fn read_city_aliases(path: &Path) -> Result<HashMap<String, String>> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("cannot read the city spellings file {}", path.display()))?;
    let mut aliases = HashMap::new();
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        let (variant, canonical) =
            line.split_once('\t').context("a line of the city spellings file has no tab")?;
        aliases.insert(variant.trim().to_string(), canonical.trim().to_string());
    }
    Ok(aliases)
}

/// The raw export, held as text.
struct Raw {
    columns: HashMap<String, usize>,
    rows: Vec<csv::StringRecord>,
}

impl Raw {
    fn read(path: &Path) -> Result<Raw> {
        let mut reader = csv::ReaderBuilder::new()
            .delimiter(b'\t')
            .quoting(false)
            .from_path(path)
            .with_context(|| format!("cannot open {}", path.display()))?;

        let mut names: Vec<String> = reader.headers()?.iter().map(str::to_string).collect();
        let blank: Vec<usize> = (0..names.len())
            .filter(|&i| names[i].trim().is_empty())
            .collect();
        if !blank.is_empty() {
            if blank != [BLANK_HEADER_INDEX] {
                bail!("unexpected blank header cells at {blank:?}");
            }
            names[BLANK_HEADER_INDEX] = BLANK_HEADER_NAME.to_string();
        }
        let columns: HashMap<String, usize> =
            names.into_iter().enumerate().map(|(i, n)| (n, i)).collect();

        let rows = reader
            .records()
            .collect::<Result<Vec<_>, _>>()
            .context("cannot read the raw export")?;

        let raw = Raw { columns, rows };
        let id = raw.col("_id")?;
        let mut seen = HashSet::new();
        for row in &raw.rows {
            if !seen.insert(&row[id]) {
                bail!("duplicate user ids in the raw export");
            }
        }
        Ok(raw)
    }

    fn col(&self, name: &str) -> Result<usize> {
        self.columns
            .get(name)
            .copied()
            .with_context(|| format!("column {name} is missing"))
    }
}

struct User {
    raw_id: String,
    birth_year: Option<i32>,
    gender: String,
    city: String,
    income_level: &'static str,
    employment_sector: &'static str,
    income_source: &'static str,
    register_period: String,
    verified: bool,
    /// Capped counters and rounded amounts, in output order.
    numbers: Vec<String>,
}

fn build_users(raw: &Raw, city_aliases: &HashMap<String, String>) -> Result<Vec<User>> {
    let id = raw.col("_id")?;
    let dob = raw.col("dateOfBirth")?;
    let gender = raw.col("gender")?;
    let city = raw.col("city")?;
    let income = raw.col("incomeLevel")?;
    let sector = raw.col("employmentSector")?;
    let source = raw.col("incomeSource")?;
    let registered = raw.col("registerDate")?;
    let verified = raw.col("verificationDate")?;
    let counters: Vec<usize> = USER_COUNTERS
        .iter()
        .map(|(_, c)| raw.col(c))
        .collect::<Result<_>>()?;
    let amounts: Vec<usize> = USER_AMOUNTS
        .iter()
        .map(|(_, c, _, _)| raw.col(c))
        .collect::<Result<_>>()?;

    let mut users = Vec::with_capacity(raw.rows.len());
    for row in &raw.rows {
        let mut numbers = Vec::with_capacity(counters.len() + amounts.len());
        for &c in &counters {
            numbers.push(cap_count(&row[c]).context("a counter is not a valid number")?);
        }
        for (&c, &(_, _, step, cap)) in amounts.iter().zip(&USER_AMOUNTS) {
            numbers.push(round_amount(&row[c], step, cap).context("an amount is not a number")?);
        }
        users.push(User {
            raw_id: row[id].to_string(),
            birth_year: birth_year(&row[dob]),
            gender: row[gender].trim().to_string(),
            city: canonical_city(&row[city], city_aliases).to_string(),
            income_level: income_band(&row[income]).context("unknown income category")?,
            employment_sector: sector_group(&row[sector]).context("unknown employment category")?,
            income_source: source_group(&row[source]).context("unknown income source category")?,
            register_period: parse_time(&row[registered])
                .map(|t| register_period(t.year()))
                .unwrap_or_else(|| "not stated".to_string()),
            verified: !row[verified].is_empty(),
            numbers,
        });
    }
    Ok(users)
}

/// Label the largest cities by rank and group the rest.
fn city_groups(users: &[User]) -> HashMap<&str, String> {
    let mut counts: HashMap<&str, (usize, usize)> = HashMap::new();
    for (i, user) in users.iter().enumerate() {
        if !user.city.is_empty() {
            counts.entry(user.city.as_str()).or_insert((0, i)).0 += 1;
        }
    }
    let mut ranked: Vec<(&str, usize, usize)> =
        counts.into_iter().map(|(c, (n, first))| (c, n, first)).collect();
    // Largest first. Ties are broken by first appearance in the export.
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then(a.2.cmp(&b.2)));
    ranked
        .into_iter()
        .take(N_NAMED_CITIES)
        .enumerate()
        .map(|(k, (city, _, _))| (city, format!("city_{}", k + 1)))
        .collect()
}

struct Membership {
    user_raw: String,
    group_raw: String,
    kind: &'static str,
    start: DateTime<Utc>,
    fee_paid: Option<DateTime<Utc>>,
    total_amount: f64,
    monthly_amount: f64,
    fee_amount: f64,
    seat: u32,
    n_cycles: u32,
    regime: Regime,
    // Fields that describe the group, filled by `add_group_fields`.
    n_observed: usize,
    duplicate_seat_in_group: bool,
    fully_observed: bool,
    join_order_complete: bool,
    join_rank: Option<usize>,
    join_time_tied: bool,
}

impl Membership {
    /// Relative payout position: 0 is the first payout, 1 is the last.
    fn position(&self) -> f64 {
        f64::from(self.seat - 1) / f64::from(self.n_cycles - 1)
    }

    /// Fee as a percentage of the payout.
    fn fee_pct(&self) -> f64 {
        100.0 * self.fee_amount / self.total_amount
    }

    /// Whole days between the fee payment and the start of the group.
    fn days_paid_before_start(&self) -> Option<i64> {
        self.fee_paid.map(|paid| whole_days(self.start - paid))
    }

    /// Date on which this seat is paid, in local time.
    fn payout(&self) -> chrono::NaiveDate {
        payout_date(local_date(self.start), self.seat)
    }

    /// A group can be used to rebuild choice sets when every seat has a
    /// listed holder and every holder has a fee payment time.
    fn usable_for_choice(&self) -> bool {
        self.fully_observed && self.join_order_complete
    }
}

fn whole_days(span: chrono::Duration) -> i64 {
    (span.num_seconds() as f64 / 86_400.0).round() as i64
}

fn number(text: &str, what: &str) -> Result<f64> {
    text.trim()
        .parse()
        .with_context(|| format!("{what} is not a number"))
}

/// Turn the repeated column groups into one row per membership.
fn build_memberships(raw: &Raw) -> Result<Vec<Membership>> {
    let user_id = raw.col("_id")?;
    let mut memberships = Vec::new();

    for (kind, n_slots) in MEMBERSHIP_SLOTS {
        for slot in 0..n_slots {
            let col = |field: &str| raw.col(&format!("{kind}.{slot}.{field}"));
            let (group, start) = (col("_id")?, col("startDate")?);
            let (total, monthly) = (col("totalAmount")?, col("monthlyAmount")?);
            let (seat, fee, paid) = (col("seat")?, col("feeAmount")?, col("feePaidDate")?);

            for row in &raw.rows {
                if row[group].is_empty() {
                    continue;
                }
                let total_amount = number(&row[total], "total amount")?;
                let monthly_amount = number(&row[monthly], "monthly amount")?;
                let ratio = total_amount / monthly_amount;
                if (ratio - ratio.round()).abs() > 1e-6 || ratio < 2.0 {
                    bail!("total amount is not a whole multiple of the instalment");
                }
                let n_cycles = ratio.round() as u32;
                let seat_number = number(&row[seat], "seat")? as u32;
                if seat_number < 1 || seat_number > n_cycles {
                    bail!("seat outside 1..n_cycles");
                }
                let start_time =
                    parse_time(&row[start]).context("a group start date cannot be read")?;

                memberships.push(Membership {
                    user_raw: row[user_id].to_string(),
                    group_raw: row[group].to_string(),
                    kind,
                    start: start_time,
                    fee_paid: parse_time(&row[paid]),
                    total_amount,
                    monthly_amount,
                    fee_amount: number(&row[fee], "fee amount")?,
                    seat: seat_number,
                    n_cycles,
                    regime: Regime::of(start_time),
                    n_observed: 0,
                    duplicate_seat_in_group: false,
                    fully_observed: false,
                    join_order_complete: false,
                    join_rank: None,
                    join_time_tied: false,
                });
            }
        }
    }
    Ok(memberships)
}

/// Add the join order and the flags that describe each group.
fn add_group_fields(memberships: &mut [Membership]) {
    let mut by_group: HashMap<String, Vec<usize>> = HashMap::new();
    for (i, m) in memberships.iter().enumerate() {
        by_group.entry(m.group_raw.clone()).or_default().push(i);
    }

    for members in by_group.values() {
        let n_observed = members.len();
        let seats: HashSet<u32> = members.iter().map(|&i| memberships[i].seat).collect();
        let duplicate = seats.len() != n_observed;
        let n_cycles = memberships[members[0]].n_cycles as usize;
        let fully_observed = seats.len() == n_cycles && !duplicate;
        let complete = members.iter().all(|&i| memberships[i].fee_paid.is_some());

        // Join order: by fee payment time. Equal times keep export order.
        let mut joined: Vec<usize> = members
            .iter()
            .copied()
            .filter(|&i| memberships[i].fee_paid.is_some())
            .collect();
        joined.sort_by_key(|&i| memberships[i].fee_paid);
        let mut time_counts: HashMap<DateTime<Utc>, usize> = HashMap::new();
        for &i in &joined {
            *time_counts.entry(memberships[i].fee_paid.unwrap()).or_default() += 1;
        }

        for (rank, &i) in joined.iter().enumerate() {
            memberships[i].join_rank = Some(rank + 1);
            memberships[i].join_time_tied = time_counts[&memberships[i].fee_paid.unwrap()] > 1;
        }
        for &i in members {
            let m = &mut memberships[i];
            m.n_observed = n_observed;
            m.duplicate_seat_in_group = duplicate;
            m.fully_observed = fully_observed;
            m.join_order_complete = complete;
        }
    }
}

fn writer(path: &Path) -> Result<csv::Writer<std::fs::File>> {
    csv::WriterBuilder::new()
        .delimiter(b'\t')
        .from_path(path)
        .with_context(|| format!("cannot write {}", path.display()))
}

/// One row of the summary by fee regime.
#[derive(Default)]
struct RegimeSummary {
    groups: usize,
    fully_observed: usize,
    usable_for_choice: usize,
    with_tied_join_times: usize,
    members_in_usable: usize,
    informative_choices: usize,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let raw = Raw::read(&args.raw)?;
    let city_aliases = read_city_aliases(&args.city_aliases)?;
    let users = build_users(&raw, &city_aliases)?;
    let mut memberships = build_memberships(&raw)?;
    add_group_fields(&mut memberships);
    if memberships
        .iter()
        .any(|m| m.payout() < first_covered_payout() || m.payout() > last_covered_payout())
    {
        bail!("a payout date lies outside the range covered by the holiday tables");
    }

    let mut rng = entropy_rng();
    let user_label = random_labels(users.iter().map(|u| u.raw_id.as_str()), "U", &mut rng);
    let group_label = random_labels(memberships.iter().map(|m| m.group_raw.as_str()), "G", &mut rng);

    let snapshot_year = memberships
        .iter()
        .filter_map(|m| m.fee_paid)
        .max()
        .context("no fee payment time in the export")?
        .year();

    // Attributes an outsider could know, one row per user, then the
    // five-person rule.
    let cities = city_groups(&users);
    let mut attributes: Vec<Vec<String>> = users
        .iter()
        .map(|u| {
            let band = u
                .birth_year
                .and_then(|year| age_band(snapshot_year - year))
                .unwrap_or("not stated");
            let city_group = if u.city.is_empty() {
                "not stated".to_string()
            } else {
                cities.get(u.city.as_str()).cloned().unwrap_or_else(|| "other".to_string())
            };
            vec![
                u.gender.clone(),
                band.to_string(),
                city_group,
                u.income_level.to_string(),
                u.employment_sector.to_string(),
            ]
        })
        .collect();
    let report = enforce_k_anonymity(&mut attributes, &SUPPRESSION_ORDER, K);
    if report.smallest_group < K {
        bail!(
            "the {K}-person rule cannot be met: the smallest group has {} people",
            report.smallest_group
        );
    }

    std::fs::create_dir_all(&args.out)?;

    // users.tsv, in label order. The labels are random, so the row order
    // carries no information.
    let mut rows: Vec<Vec<String>> = users
        .iter()
        .zip(&attributes)
        .map(|(u, attrs)| {
            let mut row = vec![user_label[&u.raw_id].clone()];
            row.extend(attrs.iter().cloned());
            row.push(u.income_source.to_string());
            row.push(u.register_period.clone());
            row.push(u.verified.to_string());
            row.extend(u.numbers.iter().cloned());
            row
        })
        .collect();
    rows.sort();
    let mut out = writer(&args.out.join("users.tsv"))?;
    let mut header = vec!["user"];
    header.extend(QUASI_IDENTIFIERS);
    header.extend(["income_source", "register_period", "verified"]);
    header.extend(USER_COUNTERS.iter().map(|(name, _)| *name));
    header.extend(USER_AMOUNTS.iter().map(|(name, _, _, _)| *name));
    out.write_record(&header)?;
    for row in &rows {
        out.write_record(row)?;
    }
    out.flush()?;

    // memberships.tsv
    let mut order: Vec<usize> = (0..memberships.len()).collect();
    order.sort_by(|&a, &b| {
        let (ma, mb) = (&memberships[a], &memberships[b]);
        (&group_label[&ma.group_raw], ma.seat, &user_label[&ma.user_raw]).cmp(&(
            &group_label[&mb.group_raw],
            mb.seat,
            &user_label[&mb.user_raw],
        ))
    });
    let mut out = writer(&args.out.join("memberships.tsv"))?;
    let mut header = vec![
        "user", "group", "kind", "regime", "seat", "n_cycles", "position", "total_amount",
        "monthly_amount", "fee_amount", "fee_pct", "start_month", "fee_paid_month",
        "days_paid_before_start", "join_rank", "join_time_tied", "n_observed",
        "fully_observed", "join_order_complete", "usable_for_choice", "duplicate_seat_in_group",
        "payout_month",
    ];
    header.extend(HOLIDAY_FLAGS.iter().map(|(name, _, _)| *name));
    out.write_record(&header)?;
    for &i in &order {
        let m = &memberships[i];
        let payout = m.payout();
        let mut record = vec![
            user_label[&m.user_raw].clone(),
            group_label[&m.group_raw].clone(),
            m.kind.to_string(),
            m.regime.label().to_string(),
            m.seat.to_string(),
            m.n_cycles.to_string(),
            m.position().to_string(),
            m.total_amount.to_string(),
            m.monthly_amount.to_string(),
            m.fee_amount.to_string(),
            m.fee_pct().to_string(),
            month(m.start),
            m.fee_paid.map(month).unwrap_or_default(),
            m.days_paid_before_start().map(|d| d.to_string()).unwrap_or_default(),
            m.join_rank.map(|r| r.to_string()).unwrap_or_default(),
            m.join_time_tied.to_string(),
            m.n_observed.to_string(),
            m.fully_observed.to_string(),
            m.join_order_complete.to_string(),
            m.usable_for_choice().to_string(),
            m.duplicate_seat_in_group.to_string(),
            payout.format("%Y-%m").to_string(),
        ];
        record.extend(
            HOLIDAY_FLAGS
                .iter()
                .map(|&(_, event, window)| pays_before(payout, event, window).to_string()),
        );
        out.write_record(&record)?;
    }
    out.flush()?;

    // groups.tsv: one row per group, taken from its lowest listed seat.
    let mut first_member: BTreeMap<&str, usize> = BTreeMap::new();
    let mut any_tied: HashMap<&str, bool> = HashMap::new();
    for &i in &order {
        let label = group_label[&memberships[i].group_raw].as_str();
        first_member.entry(label).or_insert(i);
        *any_tied.entry(label).or_default() |= memberships[i].join_time_tied;
    }
    let mut out = writer(&args.out.join("groups.tsv"))?;
    out.write_record([
        "group", "regime", "n_cycles", "total_amount", "monthly_amount", "start_month",
        "days_created_before_start", "n_observed", "fully_observed", "join_order_complete",
        "usable_for_choice", "any_join_time_tied",
    ])?;
    let mut summary: BTreeMap<Regime, RegimeSummary> = BTreeMap::new();
    for (&label, &i) in &first_member {
        let m = &memberships[i];
        let created_before = object_id_time(&m.group_raw)
            .map(|created| whole_days(m.start - created).to_string())
            .unwrap_or_default();
        out.write_record([
            label.to_string(),
            m.regime.label().to_string(),
            m.n_cycles.to_string(),
            m.total_amount.to_string(),
            m.monthly_amount.to_string(),
            month(m.start),
            created_before,
            m.n_observed.to_string(),
            m.fully_observed.to_string(),
            m.join_order_complete.to_string(),
            m.usable_for_choice().to_string(),
            any_tied[label].to_string(),
        ])?;

        let s = summary.entry(m.regime).or_default();
        s.groups += 1;
        s.fully_observed += usize::from(m.fully_observed);
        s.with_tied_join_times += usize::from(any_tied[label]);
        if m.usable_for_choice() {
            s.usable_for_choice += 1;
            s.members_in_usable += m.n_cycles as usize;
            // The last person to join has one seat left and so makes no choice.
            s.informative_choices += m.n_cycles as usize - 1;
        }
    }
    out.flush()?;

    // Aggregate summary only. No individual values are printed.
    let with_membership: HashSet<&str> = memberships.iter().map(|m| m.user_raw.as_str()).collect();
    println!("users                     {:>7}", with_commas(users.len()));
    println!("users with a membership   {:>7}", with_commas(with_membership.len()));
    println!("memberships               {:>7}", with_commas(memberships.len()));
    println!("groups                    {:>7}", with_commas(first_member.len()));

    println!("\nFive-person rule on {}", QUASI_IDENTIFIERS.join(", "));
    println!("smallest group left       {:>7}", with_commas(report.smallest_group));
    for (name, count) in QUASI_IDENTIFIERS.iter().zip(&report.suppressed) {
        println!("values removed: {:<18}{:>7}", name, with_commas(*count));
    }

    println!("\nSample by fee regime");
    println!(
        "{:<7}{:>8}{:>16}{:>19}{:>22}{:>19}{:>21}",
        "regime", "groups", "fully_observed", "usable_for_choice", "with_tied_join_times",
        "members_in_usable", "informative_choices"
    );
    for (regime, s) in &summary {
        println!(
            "{:<7}{:>8}{:>16}{:>19}{:>22}{:>19}{:>21}",
            regime.label(), s.groups, s.fully_observed, s.usable_for_choice,
            s.with_tied_join_times, s.members_in_usable, s.informative_choices
        );
    }
    Ok(())
}
