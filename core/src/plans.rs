//! Planned and recurring transactions. Plans affect the books only when posted.
use crate::model::{Book, Frequency, Plan};
use crate::num::D;
use chrono::{Days, Months, NaiveDate};
use serde::Serialize;

/// Occurrence dates of a plan within `[from, to]` (inclusive), capped at 5000.
pub fn occurrences(plan: &Plan, from: NaiveDate, to: NaiveDate) -> Vec<NaiveDate> {
    let mut out = vec![];
    let end = plan.end.map_or(to, |e| e.min(to));
    for i in 0..5000u32 {
        let step = i.saturating_mul(plan.interval.max(1));
        let date = match plan.frequency {
            Frequency::Once if i > 0 => break,
            Frequency::Once => Some(plan.start),
            Frequency::Weekly => plan.start.checked_add_days(Days::new(u64::from(step) * 7)),
            Frequency::Monthly => plan.start.checked_add_months(Months::new(step)),
            Frequency::Yearly => plan
                .start
                .checked_add_months(Months::new(step.saturating_mul(12))),
        };
        let Some(date) = date else { break };
        if date > end {
            break;
        }
        if date >= from {
            out.push(date);
        }
    }
    out
}
pub fn source(plan: i64, occurrence: NaiveDate) -> String {
    format!("plan:{plan}:{occurrence}")
}
pub fn posted(book: &Book, plan: i64, occurrence: NaiveDate) -> bool {
    let key = source(plan, occurrence);
    book.entries.iter().any(|e| !e.void && e.source == key)
}

#[derive(Clone, Debug, Serialize)]
pub struct Occurrence {
    pub plan: i64,
    pub name: String,
    pub date: NaiveDate,
    pub debit: i64,
    pub credit: i64,
    pub amount: D,
    pub posted: bool,
}
/// Occurrences of active plans in a period, with posting state.
pub fn calendar(book: &Book, from: NaiveDate, to: NaiveDate) -> Vec<Occurrence> {
    let mut out = vec![];
    for p in book.plans.iter().filter(|p| p.active) {
        for date in occurrences(p, from, to) {
            out.push(Occurrence {
                plan: p.id,
                name: p.name.clone(),
                date,
                debit: p.debit,
                credit: p.credit,
                amount: D(p.amount),
                posted: posted(book, p.id, date),
            });
        }
    }
    out.sort_by(|a, b| a.date.cmp(&b.date).then(a.plan.cmp(&b.plan)));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::num::Decimal;
    fn d(s: &str) -> NaiveDate {
        s.parse().unwrap()
    }
    #[test]
    fn monthly_plans_keep_the_anchor_day_and_respect_end() {
        let plan = Plan {
            id: 1,
            name: "家賃".into(),
            debit: 1,
            credit: 2,
            amount: Decimal::from(80000),
            start: d("2026-01-31"),
            end: Some(d("2026-05-15")),
            frequency: Frequency::Monthly,
            interval: 1,
            partner: None,
            active: true,
            memo: String::new(),
        };
        assert_eq!(
            occurrences(&plan, d("2026-01-01"), d("2026-12-31")),
            vec![
                d("2026-01-31"),
                d("2026-02-28"),
                d("2026-03-31"),
                d("2026-04-30")
            ]
        );
    }
}
