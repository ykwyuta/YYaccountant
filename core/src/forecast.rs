//! Expense and income forecasting from monthly history plus known schedules.
//!
//! Baseline: the method (moving average, linear trend or seasonal) with the smallest
//! one-step-ahead backtest error, or the method requested. Entries posted from plans or
//! claim settlements are excluded from the history and added back as scheduled amounts,
//! so recurring items are not counted twice.
use crate::claims;
use crate::model::{Book, Category, Side};
use crate::num::{self, D, Decimal};
use crate::plans;
use crate::reports::{month_key, monthly, months};
use crate::{Result, invalid};
use chrono::{Datelike, Months, NaiveDate};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Method {
    #[default]
    Auto,
    MovingAverage,
    Linear,
    Seasonal,
}
impl Method {
    pub fn label(self) -> &'static str {
        match self {
            Self::Auto => "自動選択",
            Self::MovingAverage => "移動平均",
            Self::Linear => "線形トレンド",
            Self::Seasonal => "季節変動",
        }
    }
}

fn predict(method: Method, series: &[Decimal], h: usize) -> Option<Decimal> {
    let n = series.len();
    if n == 0 {
        return None;
    }
    let value = match method {
        Method::MovingAverage | Method::Auto => num::mean(&series[n - n.min(6)..]),
        Method::Linear => {
            if n < 3 {
                return None;
            }
            let (a, b) = num::regression(series);
            a + b * Decimal::from(n - 1 + h)
        }
        Method::Seasonal => {
            if n < 13 {
                return None;
            }
            let last: Decimal = series[n - 12..].iter().sum();
            let prev = &series[n.saturating_sub(24)..n - 12];
            let prev_mean = num::mean(prev);
            let ratio = if prev_mean.is_zero() {
                Decimal::ONE
            } else {
                (last / Decimal::from(12) / prev_mean).clamp(Decimal::new(5, 1), Decimal::TWO)
            };
            series[n - 12 + (h - 1) % 12] * ratio
        }
    };
    Some(value.max(Decimal::ZERO))
}
/// One-step-ahead errors over the last up to 6 months.
fn backtest(method: Method, series: &[Decimal]) -> Vec<Decimal> {
    let n = series.len();
    (n.saturating_sub(6).max(3)..n)
        .filter_map(|t| predict(method, &series[..t], 1).map(|p| series[t] - p))
        .collect()
}

#[derive(Clone, Debug, Serialize)]
pub struct ForecastMonth {
    pub month: String,
    pub baseline: D,
    pub scheduled: D,
    pub total: D,
    pub low: D,
    pub high: D,
}
#[derive(Clone, Debug, Serialize)]
pub struct AccountForecast {
    pub account: i64,
    pub code: String,
    pub name: String,
    pub category: Category,
    pub method: Method,
    pub history: Vec<D>,
    pub mean: D,
    pub stdev: D,
    /// Linear trend per month.
    pub trend: D,
    /// Mean absolute one-step-ahead error of the chosen method.
    pub error: Option<D>,
    pub months: Vec<ForecastMonth>,
}
#[derive(Clone, Debug, Serialize)]
pub struct ForecastTotal {
    pub month: String,
    pub income: D,
    pub expense: D,
    pub net: D,
    pub expense_low: D,
    pub expense_high: D,
    /// Scheduled cash outflows that are not expenses (loan principal, card payments in plans).
    pub planned_payments: D,
    pub planned_receipts: D,
}
#[derive(Clone, Debug, Serialize)]
pub struct Forecast {
    pub as_of: NaiveDate,
    pub history_months: Vec<String>,
    pub months: Vec<String>,
    pub accounts: Vec<AccountForecast>,
    pub totals: Vec<ForecastTotal>,
    pub notes: Vec<String>,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ForecastRequest {
    pub as_of: NaiveDate,
    #[serde(default = "default_months")]
    pub months: u32,
    #[serde(default = "default_history")]
    pub history_months: u32,
    #[serde(default)]
    pub method: Method,
}
fn default_months() -> u32 {
    6
}
fn default_history() -> u32 {
    24
}

pub fn forecast(book: &Book, r: &ForecastRequest) -> Result<Forecast> {
    if !(1..=36).contains(&r.months) || !(3..=120).contains(&r.history_months) {
        return Err(invalid("予測は1〜36か月、履歴は3〜120か月にしてください"));
    }
    let this_month = r
        .as_of
        .with_day(1)
        .ok_or_else(|| invalid("日付が不正です"))?;
    let history_start = this_month
        .checked_sub_months(Months::new(r.history_months))
        .ok_or_else(|| invalid("日付が不正です"))?;
    let history_end = this_month
        .pred_opt()
        .ok_or_else(|| invalid("日付が不正です"))?;
    let first = this_month
        .checked_add_months(Months::new(1))
        .ok_or_else(|| invalid("日付が不正です"))?;
    let last = first
        .checked_add_months(Months::new(r.months))
        .and_then(|d| d.pred_opt())
        .ok_or_else(|| invalid("日付が不正です"))?;
    // Trim leading months before the first entry so new books are not averaged with zeros.
    let first_entry = book
        .entries
        .iter()
        .filter(|e| !e.void)
        .map(|e| e.date)
        .min();
    let history_from = first_entry.map_or(history_start, |d| {
        d.with_day(1).unwrap_or(d).max(history_start)
    });
    let history_keys = if history_from <= history_end {
        months(history_from, history_end)
    } else {
        vec![]
    };
    let future_keys = months(first, last);
    let data = monthly(book, history_from, history_end, true);

    let mut scheduled: BTreeMap<(i64, String), Decimal> = BTreeMap::new();
    let mut payments: BTreeMap<String, (Decimal, Decimal)> = BTreeMap::new();
    let accounts = book.account_map();
    for o in plans::calendar(book, first, last) {
        let key = month_key(o.date);
        let (debit, credit) = (accounts[&o.debit], accounts[&o.credit]);
        if debit.category == Category::Expense {
            *scheduled.entry((debit.id, key.clone())).or_default() += o.amount.0;
        }
        if credit.category == Category::Income {
            *scheduled.entry((credit.id, key.clone())).or_default() += o.amount.0;
        }
        if debit.category != Category::Expense && credit.payment {
            payments.entry(key.clone()).or_default().0 += o.amount.0;
        }
        if credit.category != Category::Income && debit.payment {
            payments.entry(key).or_default().1 += o.amount.0;
        }
    }
    let mut notes = vec![];
    for c in book.claims.iter().filter(|c| !c.closed) {
        let paid = claims::repaid(book, c, None);
        let mut cumulative = Decimal::ZERO;
        for i in claims::schedule(book, c)? {
            cumulative += i.principal.0;
            if cumulative <= paid || i.date < first || i.date > last {
                continue;
            }
            let key = month_key(i.date);
            let slot = payments.entry(key.clone()).or_default();
            match c.side {
                Side::Payable => slot.0 += i.principal.0,
                Side::Receivable => slot.1 += i.principal.0,
            }
            if !i.interest.0.is_zero() {
                let role = match c.side {
                    Side::Payable => crate::model::Role::InterestExpense,
                    Side::Receivable => crate::model::Role::InterestIncome,
                };
                match book.role(role) {
                    Ok(a) => *scheduled.entry((a.id, key)).or_default() += i.interest.0,
                    Err(_) => {
                        notes.push(format!("「{}」の利息を計上する科目がありません", c.title))
                    }
                }
            }
        }
    }
    let z = Decimal::new(196, 2);
    let mut out = vec![];
    for a in book
        .accounts
        .iter()
        .filter(|a| matches!(a.category, Category::Income | Category::Expense))
    {
        let series: Vec<Decimal> = history_keys
            .iter()
            .map(|k| {
                data.get(&a.id)
                    .and_then(|m| m.get(k))
                    .copied()
                    .unwrap_or_default()
            })
            .collect();
        let has_schedule = scheduled.keys().any(|(id, _)| *id == a.id);
        if series.iter().all(|v| v.is_zero()) && !has_schedule {
            continue;
        }
        let candidates = [Method::MovingAverage, Method::Linear, Method::Seasonal];
        let errors = |m: Method| {
            let e = backtest(m, &series);
            (e.len() >= 2).then(|| {
                let abs: Vec<Decimal> = e.iter().map(|x| x.abs()).collect();
                let sq: Vec<Decimal> = e.iter().map(|x| x * x).collect();
                (num::mean(&abs), num::sqrt(num::mean(&sq)))
            })
        };
        let method = match r.method {
            Method::Auto => candidates
                .into_iter()
                .filter_map(|m| errors(m).map(|e| (e.0, m)))
                .min_by(|a, b| a.0.cmp(&b.0))
                .map_or(Method::MovingAverage, |x| x.1),
            m => m,
        };
        let err = errors(method);
        let rmse = err.map_or_else(|| num::stdev(&series), |e| e.1);
        let (_, slope) = num::regression(&series);
        let mut months_out = vec![];
        for (k, key) in future_keys.iter().enumerate() {
            // History ends the month before as_of; the as_of month itself is skipped as partial.
            let h = k + 2;
            let base = if series.is_empty() {
                Decimal::ZERO
            } else {
                predict(method, &series, h)
                    .or_else(|| predict(Method::MovingAverage, &series, h))
                    .unwrap_or_default()
            };
            let base = book.round(base);
            let sched = scheduled
                .get(&(a.id, key.clone()))
                .copied()
                .unwrap_or_default();
            let margin = book.round(rmse * z);
            months_out.push(ForecastMonth {
                month: key.clone(),
                baseline: D(base),
                scheduled: D(sched),
                total: D(base + sched),
                low: D((base - margin).max(Decimal::ZERO) + sched),
                high: D(base + margin + sched),
            });
        }
        out.push(AccountForecast {
            account: a.id,
            code: a.code.clone(),
            name: a.name.clone(),
            category: a.category,
            method,
            mean: D(book.round(num::mean(&series))),
            stdev: D(book.round(num::stdev(&series))),
            trend: D(book.round(slope)),
            error: err.map(|e| D(book.round(e.0))),
            history: series.into_iter().map(D).collect(),
            months: months_out,
        });
    }
    out.sort_by(|a, b| (a.category, &a.code).cmp(&(b.category, &b.code)));
    let totals = future_keys
        .iter()
        .enumerate()
        .map(|(i, key)| {
            let sum = |cat: Category, f: fn(&ForecastMonth) -> Decimal| -> Decimal {
                out.iter()
                    .filter(|a| a.category == cat)
                    .map(|a| f(&a.months[i]))
                    .sum()
            };
            let income = sum(Category::Income, |m| m.total.0);
            let expense = sum(Category::Expense, |m| m.total.0);
            let (pay, receive) = payments.get(key).copied().unwrap_or_default();
            ForecastTotal {
                month: key.clone(),
                income: D(income),
                expense: D(expense),
                net: D(income - expense),
                expense_low: D(sum(Category::Expense, |m| m.low.0)),
                expense_high: D(sum(Category::Expense, |m| m.high.0)),
                planned_payments: D(pay),
                planned_receipts: D(receive),
            }
        })
        .collect();
    if history_keys.len() < 6 {
        notes.push("履歴が6か月未満のため予測の幅が大きくなります".into());
    }
    if history_keys.len() < 13 {
        notes.push("季節変動の推定には13か月以上の履歴が必要です".into());
    }
    Ok(Forecast {
        as_of: r.as_of,
        history_months: history_keys,
        months: future_keys,
        accounts: out,
        totals,
        notes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn s(v: &[i64]) -> Vec<Decimal> {
        v.iter().map(|x| Decimal::from(*x)).collect()
    }
    #[test]
    fn methods_follow_level_trend_and_season() {
        assert_eq!(
            predict(Method::MovingAverage, &s(&[10, 20, 30]), 1),
            Some(Decimal::from(20))
        );
        assert_eq!(
            predict(Method::Linear, &s(&[10, 20, 30]), 2),
            Some(Decimal::from(50))
        );
        let mut season = s(&[1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 9]);
        season.extend(s(&[1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 9]));
        assert_eq!(
            predict(Method::Seasonal, &season, 12),
            Some(Decimal::from(9))
        );
        let trend = s(&[100, 110, 120, 130, 140, 150, 160, 170, 180]);
        let ma = backtest(Method::MovingAverage, &trend);
        let lin = backtest(Method::Linear, &trend);
        assert!(lin.iter().all(|e| e.is_zero()) && ma.iter().all(|e| !e.is_zero()));
    }
}
