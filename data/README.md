# Data

The data files are confidential. They are not distributed with this repository. The anonymised tables are made available to journal editors and reviewers on request, for the purpose of peer review only. The raw export is not shared.

## Source

A user-level export from a digital rotating savings and credit platform. In each group, every member pays a fixed monthly instalment and one member receives the pot each month. Each member chooses a seat, and the seat number is the month of the payout.

## Files

| File | Content | Shared for review |
|---|---|---|
| `data.tsv` | Raw export, one row per user | No |
| `anonymised/users.tsv` | One row per user | Yes |
| `anonymised/groups.tsv` | One row per group | Yes |
| `anonymised/memberships.tsv` | One row per user and group, with the chosen seat | Yes |

The anonymised tables are built by the `anonymise` program (`src/bin/anonymise.rs`). The rules are in `src/anonymity.rs`. All analyses read the anonymised tables only.

## What the anonymisation does

The raw export holds no names, phone numbers or national identifiers. The program reduces the remaining personal data as follows.

| Item | Treatment |
|---|---|
| User and group identifiers | Replaced by random labels. The random seed is not stored, so the labels cannot be traced back. They change on every run. |
| Date of birth | Reduced to five age bands. |
| City | The five largest cities keep a rank label (`city_1` to `city_5`). All others are grouped. |
| Income | Seven bands reduced to six. The labels are monthly income in SAR, written without the currency code. |
| Employment sector | Small categories merged into "other or not stated". |
| Income source | Reduced to salary against everything else. |
| Registration date | Reduced to the year. The first two years are merged. |
| Group start date and fee payment time | Reduced to the month. |
| Time between fee payment and group start | Whole days. |
| Counters (late payments, exits and similar) | Capped at 5. The value 5 means 5 or more. |
| Fee totals per user | Rounded to the nearest 100 and capped. |
| Internal profile status code | Removed. |

A five-person rule is then applied. No combination of gender, age band, city, income and employment sector is shared by fewer than five people. Where a combination is rarer, values are replaced by `suppressed` until the rule holds. The program stops with an error if the rule cannot be met.

In the current tables the rule removed the employment sector for 2,022 users (5.0%), the income band for 499, the city for 45, and the age band and gender for 5.

## What remains

The tables are protected against a reader who knows ordinary facts about a person. They are not protected against every possible reader.

- The platform holds the raw records and could match them to these tables.
- A reader who knows the month, amount, length and seat of a specific person's circle could find that person's row. About a third of members are unique on these four facts.

For this reason the tables stay confidential and are shared under controlled access only.

## Columns

**users.tsv**

| Column | Meaning |
|---|---|
| `user` | Random label |
| `gender`, `age_band`, `city_group`, `income_level`, `employment_sector` | Attributes covered by the five-person rule |
| `income_source` | Salary, or other or not stated |
| `register_period` | Year of registration |
| `verified` | Whether the account has a verification date |
| `n_completed`, `n_running` | Listed memberships in completed and running groups |
| `n_dismissed`, `n_left`, `n_ejected` | Groups cancelled, left, or removed from |
| `n_in_schedule`, `n_not_in_schedule` | Platform counters of participations |
| `n_deferrals`, `n_late_payments` | Payment deferrals and late payments |
| `extra_fees_amount`, `total_join_fees_paid` | Rounded amounts in SAR |

**groups.tsv**

| Column | Meaning |
|---|---|
| `group` | Random label |
| `regime` | Fee regime by start date: A (flat fee), B (early seats priced), C (every seat priced) |
| `n_cycles` | Number of members, months and seats |
| `total_amount`, `monthly_amount` | Payout and instalment in SAR |
| `start_month` | Month of the first instalment |
| `days_created_before_start` | Days between creation of the group and its start |
| `n_observed` | Members listed in the export |
| `fully_observed` | Every seat has a listed holder |
| `join_order_complete` | Every listed holder has a fee payment time |
| `usable_for_choice` | Both conditions hold, so the open seats at each choice can be rebuilt |
| `any_join_time_tied` | Two members share a fee payment time |

**memberships.tsv**

| Column | Meaning |
|---|---|
| `user`, `group` | Random labels |
| `kind` | Completed or running group |
| `seat` | Chosen seat. Seat k is paid in month k |
| `position` | Seat on a scale from 0 (first payout) to 1 (last payout) |
| `fee_amount`, `fee_pct` | Joining fee in SAR and as a percentage of the payout |
| `fee_paid_month` | Month in which the fee was paid |
| `days_paid_before_start` | Days between fee payment and group start |
| `join_rank` | Order of joining within the group, by fee payment time |
| `join_time_tied` | Another member of the group has the same fee payment time |
| `duplicate_seat_in_group` | Two listed members hold the same seat |
| `payout_month` | Month in which the seat is paid. Seat k is paid on the start date plus k - 1 months |
| `pay_before_eid_fitr` | The payout falls 1 to 45 days before Eid al-Fitr |
| `pay_before_eid_adha` | The payout falls 1 to 30 days before Eid al-Adha |
| `pay_before_school` | The payout falls 1 to 30 days before the first school day |
| `pay_before_eid_fitr_30`, `pay_before_eid_fitr_60`, `pay_before_eid_adha_60`, `pay_before_school_60` | The same with other windows (30 or 60 days) |

The holiday flags are computed from the exact start date in local time, which is not written out. The holiday dates are in `src/calendar.rs`. Together with the payout month, the flags narrow the day of the month on which a group started. This raises the risk described under "What remains" a little, which is one more reason for controlled access.

The remaining columns repeat group-level values.
