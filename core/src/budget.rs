//! Budget versus actual by fiscal month.
use crate::cost::fiscal_range;
use crate::model::{Book, Category};
use crate::num::{self, D, Decimal};
use crate::reports::{monthly, months};
use crate::{Result, invalid};
use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
pub struct BudgetRow {
    pub account: i64,
    pub code: String,
    pub name: String,
    pub category: Category,
    pub budget: Vec<D>,
    pub actual: Vec<D>,
    /// actual - budget (positive means over budget for expenses, ahead of plan for income)
    pub variance: Vec<D>,
    pub total_budget: D,
    pub total_actual: D,
    pub total_variance: D,
    /// Actual as a percentage of budget, 0.1% precision.
    pub achievement: Option<D>,
    pub unbudgeted: bool,
}
#[derive(Clone, Debug, Serialize)]
pub struct BudgetTotals {
    pub budget: Vec<D>,
    pub actual: Vec<D>,
    pub total_budget: D,
    pub total_actual: D,
}
#[derive(Clone, Debug, Serialize)]
pub struct BudgetReport {
    pub budget: i64,
    pub name: String,
    pub fiscal_year: i32,
    pub months: Vec<String>,
    pub rows: Vec<BudgetRow>,
    pub income: BudgetTotals,
    pub expense: BudgetTotals,
    pub net_budget: D,
    pub net_actual: D,
}

fn totals(rows: &[BudgetRow], category: Category) -> BudgetTotals {
    let mut budget = vec![Decimal::ZERO; 12];
    let mut actual = vec![Decimal::ZERO; 12];
    for r in rows.iter().filter(|r| r.category == category) {
        for i in 0..12 {
            budget[i] += r.budget[i].0;
            actual[i] += r.actual[i].0;
        }
    }
    BudgetTotals {
        total_budget: D(budget.iter().sum()),
        total_actual: D(actual.iter().sum()),
        budget: budget.into_iter().map(D).collect(),
        actual: actual.into_iter().map(D).collect(),
    }
}

pub fn report(book: &Book, id: i64) -> Result<BudgetReport> {
    let b = book
        .budgets
        .iter()
        .find(|b| b.id == id)
        .ok_or_else(|| invalid("予算がありません"))?;
    let (from, to) = fiscal_range(book, b.fiscal_year)?;
    let keys = months(from, to);
    let data = monthly(book, from, to, false);
    let mut rows = vec![];
    let mut ordered: Vec<_> = book
        .accounts
        .iter()
        .filter(|a| matches!(a.category, Category::Income | Category::Expense))
        .collect();
    ordered.sort_by(|a, b| (a.category, &a.code).cmp(&(b.category, &b.code)));
    for a in ordered {
        let line = b.lines.iter().find(|l| l.account == a.id);
        let actual: Vec<Decimal> = keys
            .iter()
            .map(|k| {
                data.get(&a.id)
                    .and_then(|m| m.get(k))
                    .copied()
                    .unwrap_or_default()
            })
            .collect();
        if line.is_none() && actual.iter().all(|v| v.is_zero()) {
            continue;
        }
        let budget: Vec<Decimal> = line.map_or(vec![Decimal::ZERO; 12], |l| l.amounts.clone());
        let tb: Decimal = budget.iter().sum();
        let ta: Decimal = actual.iter().sum();
        rows.push(BudgetRow {
            account: a.id,
            code: a.code.clone(),
            name: a.name.clone(),
            category: a.category,
            variance: actual.iter().zip(&budget).map(|(x, y)| D(x - y)).collect(),
            budget: budget.into_iter().map(D).collect(),
            actual: actual.into_iter().map(D).collect(),
            total_budget: D(tb),
            total_actual: D(ta),
            total_variance: D(ta - tb),
            achievement: (!tb.is_zero()).then(|| D((ta / tb * num::hundred()).round_dp(1))),
            unbudgeted: line.is_none(),
        });
    }
    let income = totals(&rows, Category::Income);
    let expense = totals(&rows, Category::Expense);
    Ok(BudgetReport {
        budget: b.id,
        name: b.name.clone(),
        fiscal_year: b.fiscal_year,
        months: keys,
        net_budget: D(income.total_budget.0 - expense.total_budget.0),
        net_actual: D(income.total_actual.0 - expense.total_actual.0),
        rows,
        income,
        expense,
    })
}
