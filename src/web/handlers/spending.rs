//! The `/spending` page (feature 032): confirmed bill/receipt amounts,
//! summed by month, for the trailing 12 calendar months (this month
//! included). Only the confirmed `amount` column ever counts —
//! `ocr_suggested_amount` never feeds this view, mirroring how
//! `is_expiry_eligible`/`is_amount_eligible` gate on confirmed facts only.
//!
//! Askama templates have no arithmetic, so every percentage/formatted
//! string the chart needs is computed once here, not per-render in the
//! template.

use axum::extract::State;
use tracing::Instrument;

use crate::web::error::AppWebError;
use crate::web::nav;
use crate::web::state::AppState;
use crate::web::templates::{MonthOverMonth, SpendingMonth, SpendingTemplate};
use crate::web::tenancy::TenantContext;

const MONTH_ABBR: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

fn month_label(month: u8) -> &'static str {
    MONTH_ABBR[(month - 1) as usize]
}

/// The 12 (year, month) pairs ending at `today`'s month, oldest first —
/// always exactly 12 distinct calendar months, so plain 3-letter month
/// abbreviations (no year) are never ambiguous within one chart.
fn last_12_months(today: time::Date) -> Vec<(i32, u8)> {
    let mut months = Vec::with_capacity(12);
    let mut year = today.year();
    let mut month = today.month() as i32;
    for _ in 0..12 {
        months.push((year, month as u8));
        month -= 1;
        if month == 0 {
            month = 12;
            year -= 1;
        }
    }
    months.reverse();
    months
}

/// Thousands-comma-grouped integer, e.g. `4812` -> `"4,812"`. Hand-rolled
/// rather than pulling in a formatting crate for one call site.
fn format_grouped(n: i64) -> String {
    let digits = n.unsigned_abs().to_string();
    let mut grouped = String::new();
    for (i, c) in digits.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(c);
    }
    let mut result: String = grouped.chars().rev().collect();
    if n < 0 {
        result.insert(0, '-');
    }
    result
}

/// Cents rounded to whole units and comma-grouped — this page shows
/// whole-unit figures throughout (stat tiles, axis, bars, tooltip),
/// matching the signed-off mockup; individual documents' own Amount
/// field is the only place cents precision is shown (feature 032).
fn cents_to_grouped_units(cents: i64) -> String {
    format_grouped(((cents as f64) / 100.0).round() as i64)
}

/// A "nice" round axis top: the smallest 1/2/5-times-a-power-of-ten value
/// at or above `max_cents`, so gridlines land on clean numbers (600, 450,
/// 300, 150, 0 rather than 587, 440, ...). Never called with
/// `max_cents <= 0` — that's the `is_empty` case, which skips the chart
/// entirely.
fn nice_axis_max_cents(max_cents: i64) -> i64 {
    let max_units = (max_cents as f64) / 100.0;
    let magnitude = 10f64.powf(max_units.log10().floor());
    let normalized = max_units / magnitude;
    // Finer than the classic {1, 2, 5, 10} "nice number" set — those leave
    // up to ~50% of the chart's vertical space empty for a max like 587
    // (which would round all the way up to 1000). This step list trades a
    // little of that "very clean" roundness for a tighter fit.
    const NICE_STEPS: [f64; 10] = [1.0, 1.5, 2.0, 2.5, 3.0, 4.0, 5.0, 6.0, 8.0, 10.0];
    let nice_step = NICE_STEPS.into_iter().find(|&step| step >= normalized).unwrap_or(10.0);
    ((nice_step * magnitude).round() as i64) * 100
}

fn y_axis_labels(axis_max_cents: i64) -> Vec<String> {
    [4, 3, 2, 1, 0].iter().map(|quarter| cents_to_grouped_units(axis_max_cents * quarter / 4)).collect()
}

struct MonthTotals {
    bill_cents: i64,
    receipt_cents: i64,
}

fn month_over_month(this_month_cents: i64, last_month_cents: i64) -> Option<MonthOverMonth> {
    if last_month_cents <= 0 {
        return None;
    }
    let delta_pct = ((this_month_cents - last_month_cents) as f64 / last_month_cents as f64) * 100.0;
    // `.round()` (half away from zero), not `{:.0}` formatting (which
    // rounds half to even) — 12.5% should read as "13%", not "12%".
    Some(MonthOverMonth { is_up: delta_pct >= 0.0, pct_display: format!("{}", delta_pct.abs().round() as i64) })
}

#[tracing::instrument(skip(state, tenancy))]
pub async fn show(tenancy: TenantContext, State(state): State<AppState>) -> Result<SpendingTemplate, AppWebError> {
    let nav_avatar_url = nav::avatar_url(&state.pool, &state.blob, tenancy.user_id.0).await?;

    let today = time::OffsetDateTime::now_utc().date();
    let months = last_12_months(today);
    let (start_year, start_month) = months[0];
    let window_start = time::Date::from_calendar_date(start_year, time::Month::try_from(start_month).unwrap(), 1)
        .expect("first-of-month is always a valid date");

    let amount_eligible_doc_types: Vec<String> =
        crate::doc_type_extract::AMOUNT_ELIGIBLE_DOC_TYPES.iter().map(|s| s.to_string()).collect();

    let rows = sqlx::query!(
        r#"select extract(year from date_issued)::int4 as "year!", extract(month from date_issued)::int4 as "month!",
                  doc_type as "doc_type!", sum(amount)::bigint as "total_cents!"
           from documents
           where tenant_id = $1
             and doc_type = any($2)
             and amount is not null
             and date_issued is not null
             and date_issued >= $3
           group by 1, 2, 3"#,
        tenancy.tenant_id.0,
        &amount_eligible_doc_types,
        window_start,
    )
    .fetch_all(&state.pool)
    .instrument(tracing::info_span!("db.query"))
    .await?;

    let mut buckets: std::collections::HashMap<(i32, u8), MonthTotals> = std::collections::HashMap::new();
    for row in rows {
        let entry =
            buckets.entry((row.year, row.month as u8)).or_insert(MonthTotals { bill_cents: 0, receipt_cents: 0 });
        match row.doc_type.as_str() {
            "bill" => entry.bill_cents += row.total_cents,
            "receipt" => entry.receipt_cents += row.total_cents,
            _ => {}
        }
    }

    let month_totals: Vec<(i32, u8, i64, i64)> = months
        .iter()
        .map(|&(year, month)| {
            let totals = buckets.get(&(year, month));
            (year, month, totals.map(|t| t.bill_cents).unwrap_or(0), totals.map(|t| t.receipt_cents).unwrap_or(0))
        })
        .collect();

    let max_total_cents = month_totals.iter().map(|(_, _, bill, receipt)| bill + receipt).max().unwrap_or(0);
    let total_cents: i64 = month_totals.iter().map(|(_, _, bill, receipt)| bill + receipt).sum();
    let is_empty = total_cents <= 0;

    if is_empty {
        return Ok(SpendingTemplate {
            active_tab: "spending",
            authenticated: true,
            nav_avatar_url,
            is_empty: true,
            months: Vec::new(),
            total_display: String::new(),
            this_month_display: String::new(),
            monthly_average_display: String::new(),
            month_over_month: None,
            y_axis_labels: Vec::new(),
        });
    }

    let axis_max_cents = nice_axis_max_cents(max_total_cents);

    let spending_months: Vec<SpendingMonth> = month_totals
        .iter()
        .map(|&(_, month, bill_cents, receipt_cents)| {
            let total = bill_cents + receipt_cents;
            let total_pct = (total as f64 / axis_max_cents as f64) * 100.0;
            let (bill_pct_of_bar, receipt_pct_of_bar) = if total > 0 {
                (bill_cents as f64 / total as f64 * 100.0, receipt_cents as f64 / total as f64 * 100.0)
            } else {
                (0.0, 0.0)
            };
            SpendingMonth {
                label: month_label(month).to_string(),
                bill_display: cents_to_grouped_units(bill_cents),
                receipt_display: cents_to_grouped_units(receipt_cents),
                total_display: cents_to_grouped_units(total),
                total_pct,
                bill_pct_of_bar,
                receipt_pct_of_bar,
                is_max: total == max_total_cents && max_total_cents > 0,
            }
        })
        .collect();

    let this_month_cents = month_totals.last().map(|(_, _, bill, receipt)| bill + receipt).unwrap_or(0);
    let last_month_cents =
        month_totals.get(month_totals.len().saturating_sub(2)).map(|(_, _, bill, receipt)| bill + receipt).unwrap_or(0);
    let monthly_average_cents = total_cents / months.len() as i64;

    Ok(SpendingTemplate {
        active_tab: "spending",
        authenticated: true,
        nav_avatar_url,
        is_empty: false,
        months: spending_months,
        total_display: cents_to_grouped_units(total_cents),
        this_month_display: cents_to_grouped_units(this_month_cents),
        monthly_average_display: cents_to_grouped_units(monthly_average_cents),
        month_over_month: month_over_month(this_month_cents, last_month_cents),
        y_axis_labels: y_axis_labels(axis_max_cents),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(year: i32, month: u8, day: u8) -> time::Date {
        time::Date::from_calendar_date(year, time::Month::try_from(month).unwrap(), day).unwrap()
    }

    #[test]
    fn last_12_months_ends_at_todays_month_oldest_first() {
        let months = last_12_months(date(2026, 7, 19));
        assert_eq!(months.len(), 12);
        assert_eq!(months.first(), Some(&(2025, 8)));
        assert_eq!(months.last(), Some(&(2026, 7)));
    }

    #[test]
    fn last_12_months_handles_a_january_start_crossing_years() {
        let months = last_12_months(date(2026, 1, 15));
        assert_eq!(months.first(), Some(&(2025, 2)));
        assert_eq!(months.last(), Some(&(2026, 1)));
    }

    #[test]
    fn format_grouped_adds_commas_every_three_digits() {
        assert_eq!(format_grouped(4812), "4,812");
        assert_eq!(format_grouped(128), "128");
        assert_eq!(format_grouped(1234567), "1,234,567");
        assert_eq!(format_grouped(0), "0");
    }

    #[test]
    fn cents_to_grouped_units_rounds_to_whole_units() {
        assert_eq!(cents_to_grouped_units(481250), "4,813");
        assert_eq!(cents_to_grouped_units(481249), "4,812");
    }

    #[test]
    fn nice_axis_max_rounds_up_to_a_clean_number() {
        assert_eq!(nice_axis_max_cents(58700), 60000);
        assert_eq!(nice_axis_max_cents(44000), 50000);
        assert_eq!(nice_axis_max_cents(21000), 25000);
        assert_eq!(nice_axis_max_cents(10000), 10000);
    }

    #[test]
    fn y_axis_labels_are_evenly_spaced_from_zero_to_the_max() {
        let labels = y_axis_labels(60000);
        assert_eq!(labels, vec!["600", "450", "300", "150", "0"]);
    }

    #[test]
    fn month_over_month_is_none_when_last_month_had_no_spend() {
        assert!(month_over_month(5000, 0).is_none());
    }

    #[test]
    fn month_over_month_computes_a_positive_delta() {
        let delta = month_over_month(4500, 4000).unwrap();
        assert!(delta.is_up);
        assert_eq!(delta.pct_display, "13");
    }

    #[test]
    fn month_over_month_computes_a_negative_delta() {
        let delta = month_over_month(3000, 4000).unwrap();
        assert!(!delta.is_up);
        assert_eq!(delta.pct_display, "25");
    }
}
