//! Receivables/payables: outstanding balances, repayment schedules and aging.
use crate::model::{Book, Claim, Repayment, Side};
use crate::num::{self, D, Decimal};
use crate::{Result, invalid};
use chrono::{Months, NaiveDate};
use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
pub struct Installment {
    pub number: u32,
    pub date: NaiveDate,
    pub principal: D,
    pub interest: D,
    pub payment: D,
    pub remaining: D,
}

/// Principal repaid so far (lines that reduce the claim), excluding void entries.
pub fn repaid(book: &Book, claim: &Claim, through: Option<NaiveDate>) -> Decimal {
    book.entries
        .iter()
        .filter(|e| !e.void && through.is_none_or(|t| e.date <= t))
        .flat_map(|e| &e.lines)
        .filter(|l| l.claim == Some(claim.id))
        .map(|l| match claim.side {
            Side::Receivable => l.credit,
            Side::Payable => l.debit,
        })
        .sum()
}
pub fn outstanding(book: &Book, claim: &Claim) -> Decimal {
    claim.principal - repaid(book, claim, None)
}

fn first_date(claim: &Claim) -> NaiveDate {
    claim.due.unwrap_or_else(|| {
        claim
            .issued
            .checked_add_months(Months::new(1))
            .unwrap_or(claim.issued)
    })
}

pub fn schedule(book: &Book, claim: &Claim) -> Result<Vec<Installment>> {
    let p = claim.principal;
    let first = first_date(claim);
    if claim.repayment == Repayment::Lump {
        let days = Decimal::from((first - claim.issued).num_days().max(0));
        let interest =
            book.round(p * claim.annual_rate / num::hundred() * days / Decimal::from(365));
        return Ok(vec![Installment {
            number: 1,
            date: first,
            principal: D(p),
            interest: D(interest),
            payment: D(p + interest),
            remaining: D(Decimal::ZERO),
        }]);
    }
    let n = claim.installments.max(1);
    let r = claim.annual_rate / num::hundred() / Decimal::from(12);
    let level = if claim.repayment == Repayment::EqualPayment && !r.is_zero() {
        let f = num::powi(Decimal::ONE + r, n)?;
        Some(book.round(p * r * f / (f - Decimal::ONE)))
    } else {
        None
    };
    let mut remaining = p;
    let mut rows = Vec::with_capacity(n as usize);
    for i in 0..n {
        let date = first
            .checked_add_months(Months::new(i))
            .ok_or_else(|| invalid("返済日が範囲外です"))?;
        let interest = book.round(remaining * r);
        let mut principal = match level {
            Some(pay) => pay - interest,
            None => book.round(p / Decimal::from(n)),
        };
        if i + 1 == n || principal > remaining {
            principal = remaining;
        }
        if principal.is_sign_negative() {
            return Err(invalid("金利が高すぎて返済額が利息に満たません"));
        }
        remaining -= principal;
        rows.push(Installment {
            number: i + 1,
            date,
            principal: D(principal),
            interest: D(interest),
            payment: D(principal + interest),
            remaining: D(remaining),
        });
    }
    Ok(rows)
}

#[derive(Clone, Debug, Serialize)]
pub struct ClaimStatus {
    pub id: i64,
    pub side: Side,
    pub title: String,
    pub partner: Option<String>,
    pub account: String,
    pub principal: D,
    pub repaid: D,
    pub outstanding: D,
    /// Principal that should have been repaid by the report date but was not.
    pub arrears: D,
    pub days_overdue: i64,
    pub bucket: &'static str,
    pub next: Option<Installment>,
    pub closed: bool,
    pub schedule: Vec<Installment>,
}
#[derive(Clone, Debug, Default, Serialize)]
pub struct AgingTotals {
    pub outstanding: D,
    pub arrears: D,
    pub current: D,
    pub overdue_1_30: D,
    pub overdue_31_90: D,
    pub overdue_91_plus: D,
}
#[derive(Clone, Debug, Serialize)]
pub struct ClaimReport {
    pub as_of: NaiveDate,
    pub claims: Vec<ClaimStatus>,
    pub receivable: AgingTotals,
    pub payable: AgingTotals,
}

pub fn report(book: &Book, as_of: NaiveDate, include_closed: bool) -> Result<ClaimReport> {
    let mut claims = vec![];
    let mut receivable = AgingTotals::default();
    let mut payable = AgingTotals::default();
    for c in book.claims.iter().filter(|c| include_closed || !c.closed) {
        let schedule = schedule(book, c)?;
        let paid = repaid(book, c, Some(as_of));
        let outstanding = c.principal - repaid(book, c, None);
        let mut cumulative = Decimal::ZERO;
        let mut expected = Decimal::ZERO;
        let mut oldest_unpaid = None;
        let mut next = None;
        for i in &schedule {
            cumulative += i.principal.0;
            if i.date <= as_of {
                expected = cumulative;
                if cumulative > paid && oldest_unpaid.is_none() {
                    oldest_unpaid = Some(i.date);
                }
            } else if next.is_none() && cumulative > paid {
                next = Some(i.clone());
            }
        }
        let arrears = if c.closed {
            Decimal::ZERO
        } else {
            (expected - paid).max(Decimal::ZERO)
        };
        let days = oldest_unpaid
            .filter(|_| !arrears.is_zero())
            .map(|d| (as_of - d).num_days())
            .unwrap_or(0);
        let bucket = match days {
            0 => "current",
            1..=30 => "overdue_1_30",
            31..=90 => "overdue_31_90",
            _ => "overdue_91_plus",
        };
        let totals = match c.side {
            Side::Receivable => &mut receivable,
            Side::Payable => &mut payable,
        };
        if !c.closed {
            totals.outstanding.0 += outstanding;
            totals.arrears.0 += arrears;
            let slot = match bucket {
                "current" => &mut totals.current,
                "overdue_1_30" => &mut totals.overdue_1_30,
                "overdue_31_90" => &mut totals.overdue_31_90,
                _ => &mut totals.overdue_91_plus,
            };
            slot.0 += if bucket == "current" {
                outstanding
            } else {
                arrears
            };
            if bucket != "current" {
                totals.current.0 += outstanding - arrears;
            }
        }
        claims.push(ClaimStatus {
            id: c.id,
            side: c.side,
            title: c.title.clone(),
            partner: c.partner.and_then(|p| {
                book.partners
                    .iter()
                    .find(|x| x.id == p)
                    .map(|x| x.name.clone())
            }),
            account: book
                .account(c.account)
                .map(|a| a.name.clone())
                .unwrap_or_default(),
            principal: D(c.principal),
            repaid: D(paid),
            outstanding: D(outstanding),
            arrears: D(arrears),
            days_overdue: days,
            bucket,
            next,
            closed: c.closed,
            schedule,
        });
    }
    Ok(ClaimReport {
        as_of,
        claims,
        receivable,
        payable,
    })
}
