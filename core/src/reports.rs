//! Financial statements, ledgers, journal listing, monthly statistics and CSV.
use crate::model::{Account, Book, Category, Entry};
use crate::num::{self, D, Decimal};
use crate::{Result, invalid};
use chrono::{Datelike, NaiveDate};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

/// Line amount in the account's normal direction (positive increases the account).
pub fn signed(a: &Account, debit: Decimal, credit: Decimal) -> Decimal {
    if a.category.debit_normal() {
        debit - credit
    } else {
        credit - debit
    }
}
pub fn month_key(d: NaiveDate) -> String {
    format!("{:04}-{:02}", d.year(), d.month())
}
pub fn month_start(key: &str) -> Result<NaiveDate> {
    NaiveDate::parse_from_str(&format!("{key}-01"), "%Y-%m-%d")
        .map_err(|_| invalid("月はYYYY-MM形式で指定してください"))
}
/// Consecutive month keys from `from` to `to` inclusive (max 600).
pub fn months(from: NaiveDate, to: NaiveDate) -> Vec<String> {
    let mut out = vec![];
    let mut d = from.with_day(1).unwrap_or(from);
    while d <= to && out.len() < 600 {
        out.push(month_key(d));
        d = d
            .checked_add_months(chrono::Months::new(1))
            .unwrap_or(NaiveDate::MAX);
    }
    out
}
fn live(book: &Book) -> impl Iterator<Item = &Entry> {
    book.entries.iter().filter(|e| !e.void)
}

#[derive(Clone, Debug, Serialize)]
pub struct BalanceRow {
    pub account: i64,
    pub code: String,
    pub name: String,
    pub category: Category,
    pub opening: D,
    pub debit: D,
    pub credit: D,
    /// Closing balance in the normal direction.
    pub closing: D,
}
#[derive(Clone, Debug, Serialize)]
pub struct TrialBalance {
    pub from: Option<NaiveDate>,
    pub to: NaiveDate,
    pub rows: Vec<BalanceRow>,
    pub total_debit: D,
    pub total_credit: D,
}
pub fn trial_balance(book: &Book, from: Option<NaiveDate>, to: NaiveDate) -> TrialBalance {
    let accounts = book.account_map();
    let mut sums: BTreeMap<i64, (Decimal, Decimal, Decimal)> = BTreeMap::new();
    for e in live(book).filter(|e| e.date <= to) {
        for l in &e.lines {
            let s = sums.entry(l.account).or_default();
            if from.is_some_and(|f| e.date < f) {
                s.0 += signed(accounts[&l.account], l.debit, l.credit);
            } else {
                s.1 += l.debit;
                s.2 += l.credit;
            }
        }
    }
    let mut rows = vec![];
    let mut total_debit = Decimal::ZERO;
    let mut total_credit = Decimal::ZERO;
    let mut ordered: Vec<&Account> = book.accounts.iter().collect();
    ordered.sort_by(|a, b| (a.category, &a.code).cmp(&(b.category, &b.code)));
    for a in ordered {
        let (opening, debit, credit) = sums.get(&a.id).copied().unwrap_or_default();
        if a.archived && opening.is_zero() && debit.is_zero() && credit.is_zero() {
            continue;
        }
        total_debit += debit;
        total_credit += credit;
        rows.push(BalanceRow {
            account: a.id,
            code: a.code.clone(),
            name: a.name.clone(),
            category: a.category,
            opening: D(opening),
            debit: D(debit),
            credit: D(credit),
            closing: D(opening + signed(a, debit, credit)),
        });
    }
    TrialBalance {
        from,
        to,
        rows,
        total_debit: D(total_debit),
        total_credit: D(total_credit),
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct AmountRow {
    pub account: i64,
    pub code: String,
    pub name: String,
    pub amount: D,
}
#[derive(Clone, Debug, Serialize)]
pub struct IncomeStatement {
    pub from: NaiveDate,
    pub to: NaiveDate,
    pub income: Vec<AmountRow>,
    pub expense: Vec<AmountRow>,
    pub total_income: D,
    pub total_expense: D,
    pub net_income: D,
}
fn rows_of(tb: &TrialBalance, category: Category, closing: bool) -> (Vec<AmountRow>, Decimal) {
    let rows: Vec<AmountRow> = tb
        .rows
        .iter()
        .filter(|r| r.category == category)
        .map(|r| AmountRow {
            account: r.account,
            code: r.code.clone(),
            name: r.name.clone(),
            amount: if closing {
                r.closing
            } else {
                D(r.closing.0 - r.opening.0)
            },
        })
        .filter(|r| !r.amount.0.is_zero())
        .collect();
    let total = rows.iter().map(|r| r.amount.0).sum();
    (rows, total)
}
pub fn income_statement(book: &Book, from: NaiveDate, to: NaiveDate) -> IncomeStatement {
    let tb = trial_balance(book, Some(from), to);
    let (income, ti) = rows_of(&tb, Category::Income, false);
    let (expense, te) = rows_of(&tb, Category::Expense, false);
    IncomeStatement {
        from,
        to,
        income,
        expense,
        total_income: D(ti),
        total_expense: D(te),
        net_income: D(ti - te),
    }
}
#[derive(Clone, Debug, Serialize)]
pub struct BalanceSheet {
    pub as_of: NaiveDate,
    pub assets: Vec<AmountRow>,
    pub liabilities: Vec<AmountRow>,
    pub equity: Vec<AmountRow>,
    /// Cumulative income minus expense through the date (not yet closed into equity).
    pub retained_earnings: D,
    pub total_assets: D,
    pub total_liabilities: D,
    pub total_equity: D,
    pub balanced: bool,
}
pub fn balance_sheet(book: &Book, as_of: NaiveDate) -> BalanceSheet {
    let tb = trial_balance(book, None, as_of);
    let (assets, ta) = rows_of(&tb, Category::Asset, true);
    let (liabilities, tl) = rows_of(&tb, Category::Liability, true);
    let (equity, tq) = rows_of(&tb, Category::Equity, true);
    let (_, ti) = rows_of(&tb, Category::Income, true);
    let (_, te) = rows_of(&tb, Category::Expense, true);
    let retained = ti - te;
    BalanceSheet {
        as_of,
        assets,
        liabilities,
        equity,
        retained_earnings: D(retained),
        total_assets: D(ta),
        total_liabilities: D(tl),
        total_equity: D(tq + retained),
        balanced: ta == tl + tq + retained,
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct LedgerRow {
    pub date: NaiveDate,
    pub entry: i64,
    pub description: String,
    pub partner: Option<String>,
    pub counter: String,
    pub memo: String,
    pub debit: D,
    pub credit: D,
    pub balance: D,
}
#[derive(Clone, Debug, Serialize)]
pub struct Ledger {
    pub account: i64,
    pub name: String,
    pub opening: D,
    pub rows: Vec<LedgerRow>,
    pub closing: D,
}
pub fn ledger(book: &Book, account: i64, from: NaiveDate, to: NaiveDate) -> Result<Ledger> {
    let a = book.account(account)?;
    let accounts = book.account_map();
    let mut entries: Vec<&Entry> = live(book).filter(|e| e.date <= to).collect();
    entries.sort_by_key(|e| (e.date, e.id));
    let mut balance = Decimal::ZERO;
    let mut opening = Decimal::ZERO;
    let mut rows = vec![];
    for e in entries {
        for l in e.lines.iter().filter(|l| l.account == account) {
            balance += signed(a, l.debit, l.credit);
            if e.date < from {
                opening = balance;
                continue;
            }
            let counter: Vec<String> = e
                .lines
                .iter()
                .filter(|o| o.account != account && (o.debit.is_zero() != l.debit.is_zero()))
                .map(|o| accounts[&o.account].name.clone())
                .collect();
            rows.push(LedgerRow {
                date: e.date,
                entry: e.id,
                description: e.description.clone(),
                partner: partner_name(book, e.partner),
                counter: if counter.len() > 1 {
                    "諸口".into()
                } else {
                    counter.join("")
                },
                memo: l.memo.clone(),
                debit: D(l.debit),
                credit: D(l.credit),
                balance: D(balance),
            });
        }
    }
    Ok(Ledger {
        account,
        name: a.name.clone(),
        opening: D(opening),
        rows,
        closing: D(balance),
    })
}
fn partner_name(book: &Book, id: Option<i64>) -> Option<String> {
    id.and_then(|p| {
        book.partners
            .iter()
            .find(|x| x.id == p)
            .map(|x| x.name.clone())
    })
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct EntryFilter {
    pub from: Option<NaiveDate>,
    pub to: Option<NaiveDate>,
    pub account: Option<i64>,
    pub partner: Option<i64>,
    pub query: Option<String>,
    pub include_void: bool,
    pub offset: usize,
    pub limit: Option<usize>,
}
#[derive(Clone, Debug, Serialize)]
pub struct EntryPage {
    pub revision: i64,
    pub total: usize,
    pub entries: Vec<Entry>,
}
pub fn entries(book: &Book, f: &EntryFilter) -> EntryPage {
    let q = f.query.as_deref().unwrap_or("").to_lowercase();
    let accounts = book.account_map();
    let mut list: Vec<&Entry> = book
        .entries
        .iter()
        .filter(|e| f.include_void || !e.void)
        .filter(|e| f.from.is_none_or(|d| e.date >= d) && f.to.is_none_or(|d| e.date <= d))
        .filter(|e| {
            f.account
                .is_none_or(|a| e.lines.iter().any(|l| l.account == a))
        })
        .filter(|e| f.partner.is_none_or(|p| e.partner == Some(p)))
        .filter(|e| {
            q.is_empty()
                || e.description.to_lowercase().contains(&q)
                || partner_name(book, e.partner).is_some_and(|p| p.to_lowercase().contains(&q))
                || e.lines.iter().any(|l| {
                    l.memo.to_lowercase().contains(&q)
                        || accounts
                            .get(&l.account)
                            .is_some_and(|a| a.name.to_lowercase().contains(&q))
                        || num::text(l.debit.max(l.credit)) == q
                })
        })
        .collect();
    list.sort_by_key(|e| std::cmp::Reverse((e.date, e.id)));
    EntryPage {
        revision: book.revision,
        total: list.len(),
        entries: list
            .into_iter()
            .skip(f.offset)
            .take(f.limit.unwrap_or(100).clamp(1, 500))
            .cloned()
            .collect(),
    }
}

/// Per-account monthly totals in the normal direction for income and expense accounts.
/// When `exclude_plans` is set, entries posted from plans are skipped (used by forecasting).
pub fn monthly(
    book: &Book,
    from: NaiveDate,
    to: NaiveDate,
    exclude_plans: bool,
) -> BTreeMap<i64, BTreeMap<String, Decimal>> {
    let accounts = book.account_map();
    let mut out: BTreeMap<i64, BTreeMap<String, Decimal>> = BTreeMap::new();
    for e in live(book).filter(|e| e.date >= from && e.date <= to) {
        if exclude_plans && (e.source.starts_with("plan:") || e.source.starts_with("settle:")) {
            continue;
        }
        let key = month_key(e.date);
        for l in &e.lines {
            let a = accounts[&l.account];
            if matches!(a.category, Category::Income | Category::Expense) {
                *out.entry(a.id).or_default().entry(key.clone()).or_default() +=
                    signed(a, l.debit, l.credit);
            }
        }
    }
    out
}

#[derive(Clone, Debug, Serialize)]
pub struct MonthSummary {
    pub month: String,
    pub income: D,
    pub expense: D,
    pub net: D,
    /// (income - expense) / income in percent, rounded to 0.1%.
    pub savings_rate: Option<D>,
}
#[derive(Clone, Debug, Serialize)]
pub struct AccountStatistics {
    pub account: i64,
    pub code: String,
    pub name: String,
    pub category: Category,
    pub total: D,
    pub mean: D,
    pub median: D,
    pub stdev: D,
    pub min: D,
    pub max: D,
    /// Months with a nonzero amount.
    pub active_months: usize,
    /// Share of the category total in percent.
    pub share: D,
    /// Coefficient of variation (stdev/mean) in percent; high values mean irregular spending.
    pub variability: Option<D>,
    pub series: Vec<D>,
}
#[derive(Clone, Debug, Serialize)]
pub struct Statistics {
    pub from: NaiveDate,
    pub to: NaiveDate,
    pub months: Vec<MonthSummary>,
    pub accounts: Vec<AccountStatistics>,
    pub total_income: D,
    pub total_expense: D,
    pub average_income: D,
    pub average_expense: D,
    pub savings_rate: Option<D>,
}
fn rate(numerator: Decimal, denominator: Decimal) -> Option<D> {
    (!denominator.is_zero()).then(|| D((numerator / denominator * num::hundred()).round_dp(1)))
}
pub fn statistics(book: &Book, from: NaiveDate, to: NaiveDate) -> Result<Statistics> {
    if to < from {
        return Err(invalid("期間の終了日は開始日以降にしてください"));
    }
    let keys = months(from, to);
    if keys.len() > 600 {
        return Err(invalid("統計期間は600か月以内にしてください"));
    }
    let data = monthly(book, from, to, false);
    let mut by_month: HashMap<String, (Decimal, Decimal)> = HashMap::new();
    let mut accounts = vec![];
    let mut category_totals: HashMap<Category, Decimal> = HashMap::new();
    for a in &book.accounts {
        let Some(m) = data.get(&a.id) else { continue };
        let series: Vec<Decimal> = keys
            .iter()
            .map(|k| m.get(k).copied().unwrap_or_default())
            .collect();
        for (k, v) in keys.iter().zip(&series) {
            let slot = by_month.entry(k.clone()).or_default();
            if a.category == Category::Income {
                slot.0 += v;
            } else {
                slot.1 += v;
            }
        }
        let total: Decimal = series.iter().sum();
        *category_totals.entry(a.category).or_default() += total;
        let mean = num::mean(&series);
        let stdev = num::stdev(&series);
        accounts.push(AccountStatistics {
            account: a.id,
            code: a.code.clone(),
            name: a.name.clone(),
            category: a.category,
            total: D(total),
            mean: D(book.round(mean)),
            median: D(book.round(num::median(&series))),
            stdev: D(book.round(stdev)),
            min: D(series.iter().copied().min().unwrap_or_default()),
            max: D(series.iter().copied().max().unwrap_or_default()),
            active_months: series.iter().filter(|v| !v.is_zero()).count(),
            share: D::default(),
            variability: rate(stdev, mean),
            series: series.into_iter().map(D).collect(),
        });
    }
    for s in &mut accounts {
        s.share = rate(
            s.total.0,
            category_totals
                .get(&s.category)
                .copied()
                .unwrap_or_default(),
        )
        .unwrap_or_default();
    }
    accounts.sort_by(|a, b| {
        (a.category, std::cmp::Reverse(a.total)).cmp(&(b.category, std::cmp::Reverse(b.total)))
    });
    let months: Vec<MonthSummary> = keys
        .iter()
        .map(|k| {
            let (i, e) = by_month.get(k).copied().unwrap_or_default();
            MonthSummary {
                month: k.clone(),
                income: D(i),
                expense: D(e),
                net: D(i - e),
                savings_rate: rate(i - e, i),
            }
        })
        .collect();
    let ti: Decimal = months.iter().map(|m| m.income.0).sum();
    let te: Decimal = months.iter().map(|m| m.expense.0).sum();
    let n = Decimal::from(keys.len().max(1));
    Ok(Statistics {
        from,
        to,
        total_income: D(ti),
        total_expense: D(te),
        average_income: D(book.round(ti / n)),
        average_expense: D(book.round(te / n)),
        savings_rate: rate(ti - te, ti),
        months,
        accounts,
    })
}

pub(crate) fn csv_cell(s: &str) -> String {
    let s = if s.starts_with(['=', '+', '-', '@']) && num::parse(s).is_err() {
        format!("'{s}")
    } else {
        s.to_string()
    };
    if s.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s
    }
}
/// Journal lines as UTF-8 CSV with BOM and CRLF.
pub fn journal_csv(book: &Book, from: Option<NaiveDate>, to: Option<NaiveDate>) -> String {
    let accounts = book.account_map();
    let mut out = String::from(
        "\u{feff}日付,仕訳番号,摘要,取引先,科目コード,勘定科目,借方金額,貸方金額,明細メモ,債権債務,取消\r\n",
    );
    let mut list: Vec<&Entry> = book
        .entries
        .iter()
        .filter(|e| from.is_none_or(|d| e.date >= d) && to.is_none_or(|d| e.date <= d))
        .collect();
    list.sort_by_key(|e| (e.date, e.id));
    for e in list {
        for l in &e.lines {
            let a = accounts[&l.account];
            let fields = [
                e.date.to_string(),
                e.id.to_string(),
                e.description.clone(),
                partner_name(book, e.partner).unwrap_or_default(),
                a.code.clone(),
                a.name.clone(),
                if l.debit.is_zero() {
                    String::new()
                } else {
                    num::fixed(l.debit, book.minor_digits)
                },
                if l.credit.is_zero() {
                    String::new()
                } else {
                    num::fixed(l.credit, book.minor_digits)
                },
                l.memo.clone(),
                l.claim.map(|c| c.to_string()).unwrap_or_default(),
                if e.void {
                    "取消".into()
                } else {
                    String::new()
                },
            ];
            out.push_str(
                &fields
                    .iter()
                    .map(|f| csv_cell(f))
                    .collect::<Vec<_>>()
                    .join(","),
            );
            out.push_str("\r\n");
        }
    }
    out
}
