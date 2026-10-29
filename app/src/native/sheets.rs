//! What the native views show, as plain rows of text. These functions only read the
//! core model, keep every amount in `Decimal`, and are tested without any window.
use chrono::{Datelike, NaiveDate};
use yy_accountant::{
    Account, Book, BookKind, Category, Entry, Line, QuickKind, Role, chart,
    num::{self, Decimal},
    reports,
};
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Align {
    Left,
    Right,
}
#[derive(Debug, Default)]
pub(super) struct Table {
    /// Heading, width in 96-DPI pixels and alignment.
    pub columns: Vec<(&'static str, i32, Align)>,
    /// Record ID (entry, line index or account; 0 for totals) and cells.
    pub rows: Vec<(i64, Vec<String>)>,
}
impl Table {
    pub(super) fn new(columns: &[(&'static str, i32, Align)]) -> Self {
        Self {
            columns: columns.to_vec(),
            rows: vec![],
        }
    }
    pub(super) fn push(&mut self, id: i64, cells: Vec<String>) {
        self.rows.push((id, cells));
    }
}
use Align::{Left as L, Right as R};

/// Amount with the book's fraction digits and thousands separators (e.g. `-1,234,567`).
pub(super) fn money(book: &Book, d: Decimal) -> String {
    let fixed = num::fixed(d, book.minor_digits);
    let (sign, digits) = match fixed.strip_prefix('-') {
        Some(rest) => ("-", rest),
        None => ("", fixed.as_str()),
    };
    let (int, frac) = match digits.split_once('.') {
        Some((i, f)) => (i, Some(f)),
        None => (digits, None),
    };
    let mut grouped = String::new();
    for (i, c) in int.chars().enumerate() {
        if i > 0 && (int.len() - i) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(c);
    }
    let sign = if grouped.chars().all(|c| c == '0' || c == ',') {
        ""
    } else {
        sign
    };
    match frac {
        Some(f) => format!("{sign}{grouped}.{f}"),
        None => format!("{sign}{grouped}"),
    }
}
/// Blank for zero, so debit/credit columns read like a paper journal.
pub(super) fn side(book: &Book, d: Decimal) -> String {
    if d.is_zero() {
        String::new()
    } else {
        money(book, d)
    }
}
/// Empty input counts as zero; everything else goes through the core parser
/// (full-width digits, separators and 円 are accepted, exponents are rejected).
pub(super) fn amount(text: &str) -> Result<Decimal> {
    if text.trim().is_empty() {
        Ok(Decimal::ZERO)
    } else {
        Ok(num::parse(text)?)
    }
}
pub(super) fn label(a: &Account) -> String {
    format!("{} {}", a.code, a.name)
}
pub(super) fn name(book: &Book, id: i64) -> String {
    book.account(id)
        .map(|a| a.name.clone())
        .unwrap_or_else(|_| format!("#{id}"))
}
pub(super) fn partner(book: &Book, id: Option<i64>) -> String {
    id.and_then(|p| book.partners.iter().find(|x| x.id == p))
        .map(|p| p.name.clone())
        .unwrap_or_default()
}
pub(super) fn kind_labels(kind: QuickKind) -> (&'static str, &'static str) {
    match kind {
        QuickKind::Expense => ("費用の科目", "支払方法"),
        QuickKind::Income => ("収入の科目", "入金先"),
        QuickKind::Transfer => ("移動先", "移動元"),
    }
}
/// Accounts offered for quick input; the same choices as the Web UI.
pub(super) fn quick_accounts(book: &Book, kind: QuickKind, via: bool) -> Vec<(i64, String)> {
    let money_side = |a: &Account| {
        matches!(
            a.category,
            Category::Asset | Category::Liability | Category::Equity
        )
    };
    let mut list: Vec<&Account> = book
        .accounts
        .iter()
        .filter(|a| !a.archived)
        .filter(|a| match (via, kind) {
            (false, QuickKind::Expense) => {
                a.category == Category::Expense || (a.category == Category::Asset && !a.payment)
            }
            (false, QuickKind::Income) => a.category == Category::Income,
            (false, QuickKind::Transfer) | (true, QuickKind::Transfer) => money_side(a),
            (true, _) => {
                a.payment || matches!(a.role, Some(Role::OwnerContribution | Role::OwnerDraw))
            }
        })
        .collect();
    // Expense accounts first; assets bought with money (e.g. 投資) follow them.
    list.sort_by_key(|a| (a.category != Category::Expense, a.code.clone()));
    list.into_iter().map(|a| (a.id, label(a))).collect()
}
pub(super) fn all_accounts(book: &Book) -> Vec<(i64, String)> {
    book.accounts
        .iter()
        .filter(|a| !a.archived)
        .map(|a| (a.id, label(a)))
        .collect()
}
pub(super) fn partners(book: &Book) -> Vec<(i64, String)> {
    std::iter::once((0, "（なし）".to_string()))
        .chain(
            book.partners
                .iter()
                .filter(|p| !p.archived)
                .map(|p| (p.id, p.name.clone())),
        )
        .collect()
}
/// First day of the fiscal year that contains `day`.
pub(super) fn fiscal_start(book: &Book, day: NaiveDate) -> NaiveDate {
    let month = book.fiscal_start_month.clamp(1, 12);
    let year = if day.month() >= month {
        day.year()
    } else {
        day.year() - 1
    };
    NaiveDate::from_ymd_opt(year, month, 1).unwrap_or(day)
}
/// Account guide and the distinctions from similar accounts, as plain text.
pub(super) fn guide(book: &Book, account: i64) -> String {
    let Ok(a) = book.account(account) else {
        return String::new();
    };
    let mut out = format!(
        "{} の使い方\r\n{}\r\n",
        label(a),
        if a.guide.is_empty() {
            "説明は登録されていません。"
        } else {
            &a.guide
        }
    );
    for d in chart::guide(book, "", Some(account)).distinctions {
        out += &format!("\r\n■ {}\r\n{}\r\n", d.title, d.explanation);
        for (example, _, chosen) in &d.examples {
            out += &format!("・{example} → {chosen}\r\n");
        }
    }
    out.replace("\r\n", "\n").replace('\n', "\r\n")
}
/// Accounts that match a free text such as 「ガソリン」, best first.
pub(super) fn suggestions(book: &Book, query: &str) -> Vec<(i64, String)> {
    if query.trim().is_empty() {
        return vec![];
    }
    chart::guide(book, query, None)
        .suggestions
        .into_iter()
        .take(8)
        .map(|s| {
            let words = s.matched.join("・");
            (
                s.account,
                if words.is_empty() {
                    format!("{} {}", s.code, s.name)
                } else {
                    format!("{} {}（{words}）", s.code, s.name)
                },
            )
        })
        .collect()
}
/// Names on one side of an entry: one name, or the first name and a count.
fn names(book: &Book, e: &Entry, debit: bool) -> String {
    let ids: Vec<i64> = e
        .lines
        .iter()
        .filter(|l| {
            if debit {
                !l.debit.is_zero()
            } else {
                !l.credit.is_zero()
            }
        })
        .map(|l| l.account)
        .collect();
    match ids.as_slice() {
        [] => String::new(),
        [one] => name(book, *one),
        [first, rest @ ..] => format!("{} ほか{}", name(book, *first), rest.len()),
    }
}
pub(super) fn journal(book: &Book, filter: &reports::EntryFilter) -> (Table, usize) {
    let page = reports::entries(book, filter);
    let mut t = Table::new(&[
        ("日付", 96, L),
        ("No.", 56, R),
        ("摘要", 220, L),
        ("取引先", 120, L),
        ("借方", 150, L),
        ("貸方", 150, L),
        ("金額", 110, R),
        ("状態", 64, L),
    ]);
    for e in &page.entries {
        t.push(
            e.id,
            vec![
                e.date.to_string(),
                e.id.to_string(),
                e.description.clone(),
                partner(book, e.partner),
                names(book, e, true),
                names(book, e, false),
                money(book, e.total()),
                if e.void {
                    "取消".into()
                } else if e.source.is_empty() {
                    String::new()
                } else {
                    "自動".into()
                },
            ],
        );
    }
    (t, page.total)
}
pub(super) fn lines(book: &Book, lines: &[Line]) -> Table {
    let mut t = Table::new(&[
        ("勘定科目", 200, L),
        ("借方", 120, R),
        ("貸方", 120, R),
        ("メモ", 220, L),
    ]);
    for (i, l) in lines.iter().enumerate() {
        t.push(
            i as i64,
            vec![
                book.account(l.account).map(label).unwrap_or_default(),
                side(book, l.debit),
                side(book, l.credit),
                l.memo.clone(),
            ],
        );
    }
    t
}
pub(super) fn totals(lines: &[Line]) -> (Decimal, Decimal) {
    lines
        .iter()
        .fold((Decimal::ZERO, Decimal::ZERO), |(d, c), l| {
            (d + l.debit, c + l.credit)
        })
}
pub(super) fn totals_text(book: &Book, lines: &[Line]) -> String {
    let (d, c) = totals(lines);
    let state = if lines.len() < 2 {
        "明細を2行以上入れてください"
    } else if d != c {
        "貸借が一致しません"
    } else if d.is_zero() {
        "金額を入れてください"
    } else {
        "貸借一致"
    };
    format!(
        "借方合計 {} ／ 貸方合計 {} ／ 差額 {} … {state}",
        money(book, d),
        money(book, c),
        money(book, d - c)
    )
}

pub(super) const REPORTS: [&str; 4] = ["試算表", "損益計算書", "貸借対照表", "総勘定元帳"];
/// Report table and a one-line summary. `from` is optional only for the trial balance.
pub(super) fn report(
    book: &Book,
    kind: usize,
    from: Option<NaiveDate>,
    to: NaiveDate,
    account: Option<i64>,
) -> Result<(Table, String)> {
    let m = |d: Decimal| money(book, d);
    Ok(match kind {
        0 => {
            let tb = reports::trial_balance(book, from, to);
            let mut t = Table::new(&[
                ("コード", 64, L),
                ("勘定科目", 200, L),
                ("区分", 64, L),
                ("期首残高", 120, R),
                ("借方", 120, R),
                ("貸方", 120, R),
                ("残高", 120, R),
            ]);
            for r in &tb.rows {
                t.push(
                    r.account,
                    vec![
                        r.code.clone(),
                        r.name.clone(),
                        r.category.label().into(),
                        m(r.opening.0),
                        m(r.debit.0),
                        m(r.credit.0),
                        m(r.closing.0),
                    ],
                );
            }
            t.push(
                0,
                vec![
                    String::new(),
                    "合計".into(),
                    String::new(),
                    String::new(),
                    m(tb.total_debit.0),
                    m(tb.total_credit.0),
                    String::new(),
                ],
            );
            let ok = tb.total_debit.0 == tb.total_credit.0;
            (
                t,
                format!(
                    "{}〜{to} 借方合計 {} ／ 貸方合計 {} … {}",
                    from.map(|d| d.to_string()).unwrap_or_default(),
                    m(tb.total_debit.0),
                    m(tb.total_credit.0),
                    if ok { "一致" } else { "不一致" }
                ),
            )
        }
        1 => {
            let from = from.ok_or("損益計算書には開始日が必要です")?;
            let is = reports::income_statement(book, from, to);
            let mut t = Table::new(&[
                ("区分", 80, L),
                ("コード", 64, L),
                ("勘定科目", 220, L),
                ("金額", 140, R),
            ]);
            for (title, rows, total) in [
                ("収益", &is.income, is.total_income.0),
                ("費用", &is.expense, is.total_expense.0),
            ] {
                for r in rows {
                    t.push(
                        r.account,
                        vec![title.into(), r.code.clone(), r.name.clone(), m(r.amount.0)],
                    );
                }
                t.push(
                    0,
                    vec![
                        title.into(),
                        String::new(),
                        format!("{title}合計"),
                        m(total),
                    ],
                );
            }
            t.push(
                0,
                vec![
                    String::new(),
                    String::new(),
                    "当期純損益".into(),
                    m(is.net_income.0),
                ],
            );
            (
                t,
                format!(
                    "{from}〜{to} 収益 {} − 費用 {} ＝ {}",
                    m(is.total_income.0),
                    m(is.total_expense.0),
                    m(is.net_income.0)
                ),
            )
        }
        2 => {
            let bs = reports::balance_sheet(book, to);
            let mut t = Table::new(&[
                ("区分", 80, L),
                ("コード", 64, L),
                ("勘定科目", 220, L),
                ("金額", 140, R),
            ]);
            for (title, rows) in [
                ("資産", &bs.assets),
                ("負債", &bs.liabilities),
                ("純資産", &bs.equity),
            ] {
                for r in rows {
                    t.push(
                        r.account,
                        vec![title.into(), r.code.clone(), r.name.clone(), m(r.amount.0)],
                    );
                }
            }
            for (title, value) in [
                ("繰越損益（未振替）", bs.retained_earnings.0),
                ("資産合計", bs.total_assets.0),
                ("負債合計", bs.total_liabilities.0),
                ("純資産合計", bs.total_equity.0),
                (
                    "負債・純資産合計",
                    bs.total_liabilities.0 + bs.total_equity.0,
                ),
            ] {
                t.push(
                    0,
                    vec![String::new(), String::new(), title.into(), m(value)],
                );
            }
            (
                t,
                format!(
                    "{to}現在 資産 {} ／ 負債・純資産 {} … {}",
                    m(bs.total_assets.0),
                    m(bs.total_liabilities.0 + bs.total_equity.0),
                    if bs.balanced { "一致" } else { "不一致" }
                ),
            )
        }
        _ => {
            let from = from.ok_or("総勘定元帳には開始日が必要です")?;
            let account = account.ok_or("総勘定元帳の勘定科目を選んでください")?;
            let l = reports::ledger(book, account, from, to)?;
            let mut t = Table::new(&[
                ("日付", 96, L),
                ("No.", 56, R),
                ("摘要", 200, L),
                ("相手科目", 140, L),
                ("借方", 120, R),
                ("貸方", 120, R),
                ("残高", 120, R),
            ]);
            t.push(
                0,
                vec![
                    from.to_string(),
                    String::new(),
                    "前期繰越".into(),
                    String::new(),
                    String::new(),
                    String::new(),
                    m(l.opening.0),
                ],
            );
            for r in &l.rows {
                t.push(
                    r.entry,
                    vec![
                        r.date.to_string(),
                        r.entry.to_string(),
                        r.description.clone(),
                        r.counter.clone(),
                        side(book, r.debit.0),
                        side(book, r.credit.0),
                        m(r.balance.0),
                    ],
                );
            }
            (
                t,
                format!(
                    "{} {from}〜{to} 前期繰越 {} → 残高 {}",
                    l.name,
                    m(l.opening.0),
                    m(l.closing.0)
                ),
            )
        }
    })
}
pub(super) fn book_kinds() -> Vec<(i64, String)> {
    BookKind::ALL
        .iter()
        .enumerate()
        .map(|(i, k)| (i as i64, k.label().to_string()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use yy_accountant::{Change, NewBook, QuickEntry, Store};
    fn d(s: &str) -> Decimal {
        num::parse(s).unwrap()
    }
    fn book(kind: BookKind, digits: u32) -> (tempfile::TempDir, Store, Book) {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(&dir.path().join("t.sqlite3"), false).unwrap();
        let book = store
            .create(
                &NewBook {
                    name: "テスト".into(),
                    kind,
                    currency: if digits == 0 { "JPY" } else { "USD" }.into(),
                    minor_digits: digits,
                    rounding: Default::default(),
                    fiscal_start_month: 4,
                },
                "test",
            )
            .unwrap();
        (dir, store, book)
    }
    fn id(book: &Book, name: &str) -> i64 {
        book.accounts.iter().find(|a| a.name == name).unwrap().id
    }

    #[test]
    fn money_groups_digits_and_keeps_fraction_digits() {
        let (_dir, _store, jpy) = book(BookKind::Household, 0);
        let (_dir2, _store2, usd) = book(BookKind::Household, 2);
        assert_eq!(money(&jpy, d("1234567")), "1,234,567");
        assert_eq!(money(&jpy, d("-1000")), "-1,000");
        assert_eq!(money(&jpy, d("999")), "999");
        assert_eq!(money(&jpy, Decimal::ZERO), "0");
        assert_eq!(money(&usd, d("1234.5")), "1,234.50");
        assert_eq!(money(&usd, d("-0.004")), "0.00");
        assert_eq!(amount("").unwrap(), Decimal::ZERO);
        assert_eq!(amount("１，９８０円").unwrap(), d("1980"));
        assert!(amount("1e3").is_err());
    }

    #[test]
    fn quick_choices_match_the_web_ui() {
        let (_dir, _store, b) = book(BookKind::SoleProprietor, 0);
        let names = |v: Vec<(i64, String)>| -> Vec<String> { v.into_iter().map(|x| x.1).collect() };
        let expense = names(quick_accounts(&b, QuickKind::Expense, false));
        assert!(
            expense[0].starts_with('5') || expense[0].starts_with('6'),
            "{expense:?}"
        );
        assert!(expense.iter().any(|n| n.ends_with("消耗品費")));
        assert!(!expense.iter().any(|n| n.ends_with("普通預金")));
        let via = names(quick_accounts(&b, QuickKind::Expense, true));
        assert!(via.iter().any(|n| n.ends_with("普通預金")));
        assert!(via.iter().any(|n| n.ends_with("事業主借")));
        assert!(!via.iter().any(|n| n.ends_with("消耗品費")));
        let income = names(quick_accounts(&b, QuickKind::Income, false));
        assert!(income.iter().all(|n| !n.ends_with("現金")));
        assert_eq!(
            fiscal_start(&b, "2026-03-31".parse().unwrap()),
            "2025-04-01".parse().unwrap()
        );
        assert_eq!(
            fiscal_start(&b, "2026-04-01".parse().unwrap()),
            "2026-04-01".parse().unwrap()
        );
        let gas = suggestions(&b, "ガソリン");
        assert!(!gas.is_empty());
        assert!(guide(&b, gas[0].0).contains("の使い方"));
        assert!(suggestions(&b, " ").is_empty());
    }

    #[test]
    fn tables_follow_the_books() {
        let (_dir, mut store, b) = book(BookKind::Household, 0);
        let food = id(&b, "食費");
        let cash = id(&b, "現金");
        let bank = id(&b, "普通預金");
        let opening = id(&b, "開始残高（純資産）");
        let mut b = store
            .apply(
                b.id,
                b.revision,
                &Change::SaveEntry {
                    entry: Entry {
                        id: 0,
                        date: "2026-04-01".parse().unwrap(),
                        description: "開始".into(),
                        partner: None,
                        source: String::new(),
                        void: false,
                        lines: vec![
                            Line {
                                account: bank,
                                debit: d("100000"),
                                ..Default::default()
                            },
                            Line {
                                account: cash,
                                debit: d("20000"),
                                ..Default::default()
                            },
                            Line {
                                account: opening,
                                credit: d("120000"),
                                ..Default::default()
                            },
                        ],
                    },
                },
                "test",
            )
            .unwrap()
            .book;
        b = store
            .apply(
                b.id,
                b.revision,
                &Change::QuickEntry {
                    quick: QuickEntry {
                        kind: QuickKind::Expense,
                        date: "2026-04-02".parse().unwrap(),
                        amount: d("1980"),
                        account: food,
                        via: cash,
                        description: "スーパー".into(),
                        partner: None,
                        business_percent: None,
                        memo: String::new(),
                    },
                },
                "test",
            )
            .unwrap()
            .book;
        let (t, total) = journal(&b, &Default::default());
        assert_eq!(total, 2);
        assert_eq!(t.rows[0].1[2], "スーパー");
        assert_eq!(t.rows[0].1[4], "食費");
        assert_eq!(t.rows[1].1[4], "普通預金 ほか1");
        assert_eq!(t.rows[1].1[6], "120,000");
        let opening_lines = &b.entries.iter().find(|e| e.id == 1).unwrap().lines;
        assert_eq!(totals(opening_lines), (d("120000"), d("120000")));
        assert!(totals_text(&b, opening_lines).ends_with("貸借一致"));
        assert!(totals_text(&b, &opening_lines[..1]).ends_with("明細を2行以上入れてください"));
        assert!(totals_text(&b, &opening_lines[..2]).ends_with("貸借が一致しません"));
        assert_eq!(lines(&b, opening_lines).rows[2].1[2], "120,000");

        let to: NaiveDate = "2026-04-30".parse().unwrap();
        let from = Some(fiscal_start(&b, to));
        let (tb, summary) = report(&b, 0, from, to, None).unwrap();
        assert!(summary.ends_with("一致"), "{summary}");
        let last = &tb.rows.last().unwrap().1;
        assert_eq!((last[4].as_str(), last[5].as_str()), ("121,980", "121,980"));
        let (is, summary) = report(&b, 1, from, to, None).unwrap();
        assert!(summary.ends_with("＝ -1,980"), "{summary}");
        assert!(
            is.rows
                .iter()
                .any(|r| r.1[2] == "食費" && r.1[3] == "1,980")
        );
        let (_, summary) = report(&b, 2, None, to, None).unwrap();
        assert!(
            summary.ends_with("一致") && summary.contains("118,020"),
            "{summary}"
        );
        let (ledger, summary) = report(&b, 3, from, to, Some(cash)).unwrap();
        assert_eq!(ledger.rows.len(), 3);
        assert!(summary.ends_with("残高 18,020"), "{summary}");
        assert!(report(&b, 3, from, to, None).is_err());
        assert!(report(&b, 1, None, to, None).is_err());
    }
}
