//! The record pages (accounts, claims, budgets, plans, costs, statistics, history): what
//! each list shows, which forms its buttons open and the [`Change`] a form produces.
//! Window-independent like `sheets.rs`; every amount is parsed and shown as a decimal.
use super::form::Field;
use super::sheets::{self, Align, Table, money, side};
use chrono::{Datelike, NaiveDate};
use serde_json::Value;
use yy_accountant::{
    Account, Book, Budget, BudgetLine, Category, Change, Claim, Equipment, EquipmentUse, Frequency,
    Material, MaterialMove, MoveKind, Partner, Plan, Product, ProductMaterial, Production,
    Repayment, Role, Settlement, Side, Target, budget, chart, claims, cost, forecast,
    num::{self, Decimal},
    plans, reports,
};
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
use Align::{Left as L, Right as R};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Section {
    Accounts,
    Partners,
    Claims,
    Budget,
    Plans,
    Materials,
    Moves,
    Equipment,
    Products,
    Productions,
    Monthly,
    AccountStats,
    Forecast,
    Audit,
}
use Section::*;
/// Pages after the four bookkeeping pages, and the views each one switches between.
pub(super) const GROUPS: [(&str, &[Section]); 7] = [
    ("勘定科目・取引先", &[Accounts, Partners]),
    ("債権・債務", &[Claims]),
    ("予算", &[Budget]),
    ("計画", &[Plans]),
    (
        "原価管理",
        &[Materials, Moves, Equipment, Products, Productions],
    ),
    ("統計・予測", &[Monthly, AccountStats, Forecast]),
    ("変更履歴", &[Audit]),
];
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Act {
    New,
    Edit,
    Remove,
    First,
    Second,
}
/// Which shared controls a view uses, and their captions.
#[derive(Default)]
pub(super) struct Spec {
    pub from: Option<&'static str>,
    pub to: bool,
    pub flag: Option<&'static str>,
    pub new: Option<&'static str>,
    pub edit: Option<&'static str>,
    pub remove: Option<&'static str>,
    pub first: Option<&'static str>,
    pub second: Option<&'static str>,
    pub detail: bool,
}
impl Section {
    pub(super) fn title(self) -> &'static str {
        match self {
            Accounts => "勘定科目",
            Partners => "取引先",
            Claims => "債権・債務",
            Budget => "予算",
            Plans => "計画",
            Materials => "原材料と在庫",
            Moves => "仕入・棚卸調整",
            Equipment => "設備と減価償却",
            Products => "製品と標準原価",
            Productions => "製造実績と原価差異",
            Monthly => "月別の収支",
            AccountStats => "科目別の統計",
            Forecast => "支出・収入の予測",
            Audit => "変更履歴",
        }
    }
    pub(super) fn spec(self) -> Spec {
        let crud = |new, edit, remove| Spec {
            new: Some(new),
            edit: Some(edit),
            remove: Some(remove),
            ..Default::default()
        };
        match self {
            Accounts => Spec {
                detail: true,
                ..crud("科目を追加...", "編集...", "削除")
            },
            Partners => crud("取引先を追加...", "編集...", "削除"),
            Claims => Spec {
                from: Some("基準日"),
                flag: Some("完了も表示"),
                first: Some("回収・返済..."),
                detail: true,
                ..crud("登録...", "編集...", "削除")
            },
            Budget => Spec {
                first: Some("科目の予算..."),
                second: Some("科目の予算を外す"),
                ..crud("新しい予算...", "予算の設定...", "予算を削除")
            },
            Plans => Spec {
                from: Some("予定の期間"),
                to: true,
                first: Some("選んだ予定を計上"),
                detail: true,
                ..crud("計画を追加...", "編集...", "削除")
            },
            Materials => Spec {
                from: Some("基準日"),
                detail: true,
                ..crud("原材料を追加...", "編集...", "削除")
            },
            Moves => crud("仕入・調整を追加...", "編集...", "削除"),
            Equipment => Spec {
                from: Some("基準日"),
                first: Some("選んだ年度の償却を計上"),
                detail: true,
                ..crud("設備を追加...", "編集...", "削除")
            },
            Products => Spec {
                first: Some("材料を設定..."),
                second: Some("設備時間を設定..."),
                detail: true,
                ..crud("製品を追加...", "編集...", "削除")
            },
            Productions => Spec {
                detail: true,
                ..crud("製造実績を追加...", "編集...", "削除")
            },
            Monthly | AccountStats => Spec {
                from: Some("期間"),
                to: true,
                ..Default::default()
            },
            Forecast => Spec {
                from: Some("基準日"),
                detail: true,
                ..Default::default()
            },
            Audit => Spec::default(),
        }
    }
    /// Default dates for the period controls.
    pub(super) fn dates(self, today: NaiveDate) -> (NaiveDate, NaiveDate) {
        match self {
            Plans => (today, add_months(today, 3)),
            Monthly | AccountStats => (
                add_months(month_start(today), -11),
                add_months(month_start(today), 1)
                    .pred_opt()
                    .unwrap_or(today),
            ),
            _ => (today, today),
        }
    }
}
fn month_start(d: NaiveDate) -> NaiveDate {
    d.with_day(1).unwrap_or(d)
}
fn add_months(d: NaiveDate, n: i32) -> NaiveDate {
    let months = chrono::Months::new(n.unsigned_abs());
    if n >= 0 {
        d.checked_add_months(months)
    } else {
        d.checked_sub_months(months)
    }
    .unwrap_or(d)
}

/// Inputs from the shared controls.
pub(super) struct Context {
    pub today: NaiveDate,
    pub from: Option<NaiveDate>,
    pub to: Option<NaiveDate>,
    pub flag: bool,
    /// Budget chosen in the view selector of the budget page.
    pub budget: Option<i64>,
    pub main: Option<i64>,
    pub detail: Option<i64>,
}
pub(super) struct View {
    pub main: Table,
    pub detail: Table,
    pub summary: String,
}

/// Detail rows of the plan calendar carry both the plan and the date.
pub(super) fn occurrence_key(plan: i64, date: NaiveDate) -> i64 {
    plan * 1_000_000 + i64::from(date.num_days_from_ce())
}
fn occurrence(key: i64) -> Option<(i64, NaiveDate)> {
    Some((
        key / 1_000_000,
        NaiveDate::from_num_days_from_ce_opt((key % 1_000_000) as i32)?,
    ))
}
fn yes_no(v: bool) -> String {
    if v { "はい" } else { "" }.into()
}
fn cut(s: &str, n: usize) -> String {
    let line = s.lines().next().unwrap_or("");
    if line.chars().count() > n {
        line.chars().take(n).collect::<String>() + "…"
    } else {
        line.into()
    }
}
fn d(book: &Book, v: Decimal) -> String {
    money(book, v)
}
/// Quantities, rates and unit costs: as entered, with separators only on the integer part.
fn q(v: Decimal) -> String {
    let t = num::text(v);
    let (sign, rest) = t.strip_prefix('-').map_or(("", t.as_str()), |r| ("-", r));
    let (int, frac) = rest
        .split_once('.')
        .map_or((rest, None), |(i, f)| (i, Some(f)));
    let mut g = String::new();
    for (i, c) in int.chars().enumerate() {
        if i > 0 && (int.len() - i) % 3 == 0 {
            g.push(',');
        }
        g.push(c);
    }
    match frac {
        Some(f) => format!("{sign}{g}.{f}"),
        None => format!("{sign}{g}"),
    }
}
fn pct(v: Option<Decimal>) -> String {
    v.map(|v| format!("{}%", num::text(v))).unwrap_or_default()
}

pub(super) fn view(book: &Book, s: Section, cx: &Context, audit: &[Value]) -> Result<View> {
    let mut main;
    let mut detail = Table::default();
    let summary;
    match s {
        Accounts => {
            main = Table::new(&[
                ("コード", 64, L),
                ("勘定科目", 180, L),
                ("区分", 64, L),
                ("役割", 110, L),
                ("支払手段", 72, L),
                ("使用停止", 72, L),
                ("説明", 420, L),
            ]);
            let mut list: Vec<&Account> = book.accounts.iter().collect();
            list.sort_by(|a, b| (a.category, &a.code).cmp(&(b.category, &b.code)));
            for a in list {
                main.push(
                    a.id,
                    vec![
                        a.code.clone(),
                        a.name.clone(),
                        a.category.label().into(),
                        a.role.map(|r| r.label().to_string()).unwrap_or_default(),
                        yes_no(a.payment),
                        yes_no(a.archived),
                        cut(&a.guide, 60),
                    ],
                );
            }
            detail = Table::new(&[("使い分け", 200, L), ("例", 300, L), ("選ぶ科目", 180, L)]);
            if let Some(id) = cx.main {
                for dv in chart::guide(book, "", Some(id)).distinctions {
                    for (example, _, chosen) in &dv.examples {
                        detail.push(0, vec![dv.title.into(), example.clone(), chosen.clone()]);
                    }
                }
            }
            summary = match cx.main.and_then(|id| book.account(id).ok()) {
                Some(a) => format!("{}: {}", sheets::label(a), cut(&a.guide, 200)),
                None => format!(
                    "{}科目。科目を選ぶと使い分けの例を表示します",
                    book.accounts.len()
                ),
            };
        }
        Partners => {
            main = Table::new(&[
                ("取引先", 220, L),
                ("メモ", 320, L),
                ("仕訳", 64, R),
                ("使用停止", 72, L),
            ]);
            for p in &book.partners {
                let used = book
                    .entries
                    .iter()
                    .filter(|e| !e.void && e.partner == Some(p.id))
                    .count();
                main.push(
                    p.id,
                    vec![
                        p.name.clone(),
                        cut(&p.note, 60),
                        used.to_string(),
                        yes_no(p.archived),
                    ],
                );
            }
            summary = format!("{}件", book.partners.len());
        }
        Claims => {
            let as_of = cx.from.unwrap_or(cx.today);
            let r = claims::report(book, as_of, cx.flag)?;
            main = Table::new(&[
                ("区分", 56, L),
                ("件名", 200, L),
                ("取引先", 120, L),
                ("科目", 120, L),
                ("元本", 110, R),
                ("回収・返済済", 110, R),
                ("残高", 110, R),
                ("延滞額", 100, R),
                ("延滞", 90, L),
                ("次回", 170, L),
                ("状態", 56, L),
            ]);
            for c in &r.claims {
                main.push(
                    c.id,
                    vec![
                        c.side.label().into(),
                        c.title.clone(),
                        c.partner.clone().unwrap_or_default(),
                        c.account.clone(),
                        d(book, c.principal.0),
                        d(book, c.repaid.0),
                        d(book, c.outstanding.0),
                        side(book, c.arrears.0),
                        if c.days_overdue > 0 {
                            format!("{}日（{}）", c.days_overdue, c.bucket)
                        } else {
                            String::new()
                        },
                        c.next
                            .as_ref()
                            .map(|n| format!("{} {}", n.date, d(book, n.payment.0)))
                            .unwrap_or_default(),
                        if c.closed { "完了" } else { "" }.into(),
                    ],
                );
            }
            detail = Table::new(&[
                ("回", 48, R),
                ("期日", 100, L),
                ("元本", 110, R),
                ("利息", 100, R),
                ("支払額", 110, R),
                ("残高", 110, R),
            ]);
            if let Some(c) = r.claims.iter().find(|c| Some(c.id) == cx.main) {
                for i in &c.schedule {
                    detail.push(
                        i64::from(i.number),
                        vec![
                            i.number.to_string(),
                            i.date.to_string(),
                            d(book, i.principal.0),
                            d(book, i.interest.0),
                            d(book, i.payment.0),
                            d(book, i.remaining.0),
                        ],
                    );
                }
            }
            let aging = |t: &claims::AgingTotals| {
                format!(
                    "残高 {} 延滞 {}（1〜30日 {}・31〜90日 {}・91日以上 {}）",
                    d(book, t.outstanding.0),
                    d(book, t.arrears.0),
                    d(book, t.overdue_1_30.0),
                    d(book, t.overdue_31_90.0),
                    d(book, t.overdue_91_plus.0)
                )
            };
            summary = format!(
                "{as_of}現在　債権: {}　債務: {}",
                aging(&r.receivable),
                aging(&r.payable)
            );
        }
        Budget => {
            let Some(id) = cx
                .budget
                .filter(|id| book.budgets.iter().any(|b| b.id == *id))
            else {
                return Ok(View {
                    main: Table::new(&[("科目", 200, L)]),
                    detail,
                    summary: "予算がありません。「新しい予算」で年度の予算を作ります".into(),
                });
            };
            let r = budget::report(book, id)?;
            let mut columns = vec![
                ("科目", 160, L),
                ("予算", 110, R),
                ("実績", 110, R),
                ("差異", 110, R),
                ("達成率", 72, R),
            ];
            // Months in fiscal order, e.g. 4月…3月 for a year starting in April.
            for i in 0..12 {
                let month = (book.fiscal_start_month.clamp(1, 12) as usize - 1 + i) % 12;
                columns.push((MONTH_LABELS[month], 150, R));
            }
            main = Table::new(&columns);
            for row in &r.rows {
                let mut cells = vec![
                    format!(
                        "{}{}",
                        row.name,
                        if row.unbudgeted {
                            "（予算外）"
                        } else {
                            ""
                        }
                    ),
                    d(book, row.total_budget.0),
                    d(book, row.total_actual.0),
                    d(book, row.total_variance.0),
                    pct(row.achievement.map(|a| a.0)),
                ];
                for i in 0..12 {
                    cells.push(format!(
                        "{} / {}",
                        d(book, row.actual[i].0),
                        d(book, row.budget[i].0)
                    ));
                }
                main.push(row.account, cells);
            }
            summary = format!(
                "{}（{}年度 {}〜）月の欄は 実績 / 予算　収入 {} / {}　支出 {} / {}　収支 {} / {}",
                r.name,
                r.fiscal_year,
                r.months.first().cloned().unwrap_or_default(),
                d(book, r.income.total_actual.0),
                d(book, r.income.total_budget.0),
                d(book, r.expense.total_actual.0),
                d(book, r.expense.total_budget.0),
                d(book, r.net_actual.0),
                d(book, r.net_budget.0)
            );
        }
        Plans => {
            let from = cx.from.unwrap_or(cx.today);
            let to = cx.to.unwrap_or(from);
            if to < from || (to - from).num_days() > 3660 {
                return Err("予定の期間は10年以内で、終了を開始以降にしてください".into());
            }
            main = Table::new(&[
                ("計画", 180, L),
                ("借方", 140, L),
                ("貸方", 140, L),
                ("金額", 110, R),
                ("頻度", 90, L),
                ("開始", 100, L),
                ("終了", 100, L),
                ("取引先", 120, L),
                ("有効", 56, L),
            ]);
            for p in &book.plans {
                main.push(
                    p.id,
                    vec![
                        p.name.clone(),
                        sheets::name(book, p.debit),
                        sheets::name(book, p.credit),
                        d(book, p.amount),
                        if p.interval > 1 && p.frequency != Frequency::Once {
                            format!("{}（{}回ごと）", p.frequency.label(), p.interval)
                        } else {
                            p.frequency.label().into()
                        },
                        p.start.to_string(),
                        p.end.map(|e| e.to_string()).unwrap_or_default(),
                        sheets::partner(book, p.partner),
                        yes_no(p.active),
                    ],
                );
            }
            detail = Table::new(&[
                ("予定日", 100, L),
                ("計画", 180, L),
                ("借方", 140, L),
                ("貸方", 140, L),
                ("金額", 110, R),
                ("状態", 72, L),
            ]);
            let calendar = plans::calendar(book, from, to);
            let waiting = calendar.iter().filter(|o| !o.posted).count();
            for o in calendar
                .iter()
                .filter(|o| cx.main.is_none_or(|p| p == o.plan))
            {
                detail.push(
                    occurrence_key(o.plan, o.date),
                    vec![
                        o.date.to_string(),
                        o.name.clone(),
                        sheets::name(book, o.debit),
                        sheets::name(book, o.credit),
                        d(book, o.amount.0),
                        if o.posted { "計上済" } else { "未計上" }.into(),
                    ],
                );
            }
            summary = format!(
                "{from}〜{to} の予定 {}件（未計上 {waiting}件）。計画を選ぶとその予定だけを表示します",
                calendar.len()
            );
        }
        Materials | Moves | Equipment | Products | Productions => {
            let as_of = cx.from.unwrap_or(cx.today);
            let r = cost::report(book, as_of)?;
            match s {
                Materials => {
                    main = Table::new(&[
                        ("原材料", 180, L),
                        ("単位", 56, L),
                        ("在庫数量", 100, R),
                        ("平均単価", 100, R),
                        ("在庫金額", 110, R),
                        ("標準単価", 100, R),
                        ("注意", 260, L),
                    ]);
                    for st in &r.stock {
                        main.push(
                            st.material,
                            vec![
                                st.name.clone(),
                                st.unit.clone(),
                                q(st.quantity.0),
                                q(st.average_cost.0),
                                d(book, st.value.0),
                                q(st.standard_cost.0),
                                st.warnings.join("・"),
                            ],
                        );
                    }
                    detail = moves(book, cx.main);
                    summary = format!(
                        "{as_of}現在の在庫金額 {}（移動平均法）",
                        d(book, r.inventory_value.0)
                    );
                }
                Moves => {
                    main = moves(book, None);
                    summary = format!(
                        "{}件。払出しは製造実績から計算します",
                        book.material_moves.len()
                    );
                }
                Equipment => {
                    main = Table::new(&[
                        ("設備", 180, L),
                        ("取得日", 100, L),
                        ("取得価額", 110, R),
                        ("耐用年数", 72, R),
                        ("年償却額", 110, R),
                        ("帳簿価額", 110, R),
                        ("時間当たり費用", 120, R),
                    ]);
                    for e in &r.equipment {
                        let src = book.equipment.iter().find(|x| x.id == e.equipment);
                        main.push(
                            e.equipment,
                            vec![
                                e.name.clone(),
                                src.map(|x| x.acquired.to_string()).unwrap_or_default(),
                                src.map(|x| d(book, x.cost)).unwrap_or_default(),
                                src.map(|x| x.life_years.to_string()).unwrap_or_default(),
                                d(book, e.annual_depreciation.0),
                                d(book, e.book_value.0),
                                q(e.hourly_rate.0),
                            ],
                        );
                    }
                    detail = Table::new(&[
                        ("年度", 72, R),
                        ("償却額", 110, R),
                        ("累計額", 110, R),
                        ("帳簿価額", 110, R),
                        ("計上", 72, L),
                    ]);
                    if let Some(e) = r.equipment.iter().find(|e| Some(e.equipment) == cx.main) {
                        for row in &e.schedule {
                            detail.push(
                                i64::from(row.fiscal_year),
                                vec![
                                    row.fiscal_year.to_string(),
                                    d(book, row.amount.0),
                                    d(book, row.accumulated.0),
                                    d(book, row.book_value.0),
                                    if row.posted { "計上済" } else { "" }.into(),
                                ],
                            );
                        }
                    }
                    summary = "定額法・月割。年度を選んで「償却を計上」すると減価償却費の仕訳を作ります（二重計上は防止）".into();
                }
                Products => {
                    main = Table::new(&[
                        ("製品", 180, L),
                        ("単位", 56, L),
                        ("材料費", 100, R),
                        ("設備費", 100, R),
                        ("労務費", 100, R),
                        ("間接費", 100, R),
                        ("標準原価", 110, R),
                    ]);
                    for p in &r.products {
                        let b = &p.standard;
                        main.push(
                            p.product,
                            vec![
                                p.name.clone(),
                                p.unit.clone(),
                                d(book, b.materials.0),
                                d(book, b.equipment.0),
                                d(book, b.labor.0),
                                d(book, b.overhead.0),
                                d(book, b.total.0),
                            ],
                        );
                    }
                    detail = breakdown(
                        r.products
                            .iter()
                            .find(|p| Some(p.product) == cx.main)
                            .map(|p| &p.standard),
                    );
                    summary = "1単位の標準原価。材料はロス率込み、設備は時間当たり費用×時間。材料・設備時間は数量0で構成から外れます".into();
                }
                _ => {
                    main = Table::new(&[
                        ("日付", 100, L),
                        ("製品", 160, L),
                        ("数量", 90, R),
                        ("実際原価", 110, R),
                        ("単位原価", 100, R),
                        ("標準原価", 110, R),
                        ("差異", 100, R),
                        ("うち材料", 100, R),
                        ("うち設備", 100, R),
                        ("うち労務", 100, R),
                    ]);
                    for p in &r.productions {
                        main.push(
                            p.production,
                            vec![
                                p.date.to_string(),
                                p.name.clone(),
                                q(p.quantity.0),
                                d(book, p.actual.total.0),
                                q(p.actual_unit_cost.0),
                                d(book, p.standard_total.0),
                                d(book, p.variance.0),
                                d(book, p.variance_materials.0),
                                d(book, p.variance_equipment.0),
                                d(book, p.variance_labor.0),
                            ],
                        );
                    }
                    detail = breakdown(
                        r.productions
                            .iter()
                            .find(|p| Some(p.production) == cx.main)
                            .map(|p| &p.actual),
                    );
                    summary = "差異は実際原価−標準原価（正は標準より高い）。材料を指定しない実績は製品の構成で消費を計算します".into();
                }
            }
        }
        Monthly | AccountStats => {
            let (from, to) = (cx.from.unwrap_or(cx.today), cx.to.unwrap_or(cx.today));
            let st = reports::statistics(book, from, to)?;
            if s == Monthly {
                main = Table::new(&[
                    ("月", 90, L),
                    ("収入", 120, R),
                    ("支出", 120, R),
                    ("収支", 120, R),
                    ("貯蓄率", 80, R),
                ]);
                for (i, m) in st.months.iter().enumerate() {
                    main.push(
                        i as i64,
                        vec![
                            m.month.clone(),
                            d(book, m.income.0),
                            d(book, m.expense.0),
                            d(book, m.net.0),
                            pct(m.savings_rate.map(|r| r.0)),
                        ],
                    );
                }
            } else {
                main = Table::new(&[
                    ("区分", 56, L),
                    ("科目", 160, L),
                    ("合計", 110, R),
                    ("月平均", 100, R),
                    ("中央値", 100, R),
                    ("標準偏差", 100, R),
                    ("最小", 100, R),
                    ("最大", 100, R),
                    ("構成比", 72, R),
                    ("変動係数", 80, R),
                    ("発生月数", 72, R),
                ]);
                for a in &st.accounts {
                    main.push(
                        a.account,
                        vec![
                            a.category.label().into(),
                            a.name.clone(),
                            d(book, a.total.0),
                            d(book, a.mean.0),
                            d(book, a.median.0),
                            d(book, a.stdev.0),
                            d(book, a.min.0),
                            d(book, a.max.0),
                            pct(Some(a.share.0)),
                            pct(a.variability.map(|v| v.0)),
                            a.active_months.to_string(),
                        ],
                    );
                }
            }
            summary = format!(
                "{from}〜{to}　収入 {}（月平均 {}）　支出 {}（月平均 {}）　貯蓄率 {}",
                d(book, st.total_income.0),
                d(book, st.average_income.0),
                d(book, st.total_expense.0),
                d(book, st.average_expense.0),
                pct(st.savings_rate.map(|r| r.0))
            );
        }
        Forecast => {
            let as_of = cx.from.unwrap_or(cx.today);
            let f = forecast::forecast(
                book,
                &forecast::ForecastRequest {
                    as_of,
                    months: 6,
                    history_months: 24,
                    method: forecast::Method::Auto,
                },
            )?;
            main = Table::new(&[
                ("月", 90, L),
                ("収入", 120, R),
                ("支出", 120, R),
                ("収支", 120, R),
                ("支出の幅（約95%）", 200, L),
                ("予定の支払", 110, R),
                ("予定の受取", 110, R),
            ]);
            for (i, t) in f.totals.iter().enumerate() {
                main.push(
                    i as i64,
                    vec![
                        t.month.clone(),
                        d(book, t.income.0),
                        d(book, t.expense.0),
                        d(book, t.net.0),
                        format!(
                            "{} 〜 {}",
                            d(book, t.expense_low.0),
                            d(book, t.expense_high.0)
                        ),
                        side(book, t.planned_payments.0),
                        side(book, t.planned_receipts.0),
                    ],
                );
            }
            let mut columns = vec![("科目", 160, L), ("方法", 100, L), ("月平均", 100, R)];
            const NEXT: [&str; 6] = [
                "1か月後",
                "2か月後",
                "3か月後",
                "4か月後",
                "5か月後",
                "6か月後",
            ];
            for m in NEXT.iter().take(f.months.len()) {
                columns.push((*m, 110, R));
            }
            detail = Table::new(&columns);
            for a in &f.accounts {
                let mut cells = vec![a.name.clone(), a.method.label().into(), d(book, a.mean.0)];
                cells.extend(a.months.iter().map(|m| d(book, m.total.0)));
                detail.push(a.account, cells);
            }
            summary = format!(
                "{as_of}基準・過去24か月から科目ごとに方法を選んだ予測（計画と返済予定を含む）。{}",
                f.notes.join(" ")
            );
        }
        Audit => {
            main = Table::new(&[
                ("日時", 170, L),
                ("版", 56, R),
                ("操作者", 160, L),
                ("操作", 140, L),
                ("内容", 520, L),
            ]);
            for a in audit.iter().rev() {
                let change = &a["change"];
                let operation = change["operation"].as_str().unwrap_or("").to_string();
                let mut body = change.clone();
                if let Some(o) = body.as_object_mut() {
                    o.remove("operation");
                }
                main.push(
                    a["id"].as_i64().unwrap_or(0),
                    vec![
                        a["at"]
                            .as_str()
                            .unwrap_or("")
                            .replace('T', " ")
                            .chars()
                            .take(19)
                            .collect(),
                        a["revision"].to_string(),
                        a["actor"].as_str().unwrap_or("").into(),
                        operation,
                        cut(&body.to_string(), 160),
                    ],
                );
            }
            summary = format!(
                "{}件（新しい順）。変更は取り消さず、この記録に残ります",
                audit.len()
            );
        }
    }
    Ok(View {
        main,
        detail,
        summary,
    })
}
fn moves(book: &Book, material: Option<i64>) -> Table {
    let mut t = Table::new(&[
        ("日付", 100, L),
        ("原材料", 160, L),
        ("種類", 80, L),
        ("数量", 100, R),
        ("単価", 100, R),
        ("メモ", 260, L),
    ]);
    let mut list: Vec<&MaterialMove> = book
        .material_moves
        .iter()
        .filter(|m| material.is_none_or(|id| m.material == id))
        .collect();
    list.sort_by_key(|m| std::cmp::Reverse((m.date, m.id)));
    for m in list {
        t.push(
            m.id,
            vec![
                m.date.to_string(),
                book.materials
                    .iter()
                    .find(|x| x.id == m.material)
                    .map(|x| x.name.clone())
                    .unwrap_or_default(),
                move_label(m.kind).into(),
                q(m.quantity),
                m.unit_cost.map(q).unwrap_or_default(),
                m.memo.clone(),
            ],
        );
    }
    t
}
fn move_label(k: MoveKind) -> &'static str {
    match k {
        MoveKind::Purchase => "仕入",
        MoveKind::Adjust => "棚卸調整",
    }
}
/// Cost lines; materials carry their ID, equipment the negative ID.
fn breakdown(b: Option<&cost::CostBreakdown>) -> Table {
    let mut t = Table::new(&[
        ("種類", 72, L),
        ("名前", 180, L),
        ("数量・時間", 100, R),
        ("単価", 100, R),
        ("金額", 110, R),
    ]);
    for l in b.map(|b| b.lines.as_slice()).unwrap_or_default() {
        let (kind, id) = match l.kind {
            "material" => ("材料", l.id),
            "equipment" => ("設備", -l.id),
            other => (other, 0),
        };
        t.push(
            id,
            vec![
                kind.into(),
                l.name.clone(),
                q(l.quantity.0),
                q(l.unit_cost.0),
                q(l.amount.0),
            ],
        );
    }
    if let Some(b) = b {
        for (title, v) in [
            ("労務費", b.labor.0),
            ("間接費", b.overhead.0),
            ("合計", b.total.0),
        ] {
            t.push(
                0,
                vec![
                    String::new(),
                    title.into(),
                    String::new(),
                    String::new(),
                    q(v),
                ],
            );
        }
    }
    t
}

// ----- forms -----
/// What a button does: open a form, or apply a change after confirmation.
pub(super) enum Prepared {
    Form(String, Vec<Field>),
    Confirm(String, Change),
    Apply(Change),
}
fn pick(label: &'static str, items: Vec<(i64, String)>, selected: Option<i64>) -> Field {
    Field::Pick(label, items, selected.unwrap_or(0))
}
fn none_and(items: Vec<(i64, String)>, none: &str) -> Vec<(i64, String)> {
    std::iter::once((0, none.to_string()))
        .chain(items)
        .collect()
}
fn accounts_where(book: &Book, f: impl Fn(&Account) -> bool) -> Vec<(i64, String)> {
    book.accounts
        .iter()
        .filter(|a| !a.archived && f(a))
        .map(|a| (a.id, sheets::label(a)))
        .collect()
}
fn choice(label: &'static str, options: &[&str], selected: usize) -> Field {
    Field::Choice(
        label,
        options.iter().map(|o| o.to_string()).collect(),
        selected,
    )
}
fn text(label: &'static str, value: impl Into<String>) -> Field {
    Field::Text(label, value.into())
}
fn dec_text(v: Decimal) -> String {
    if v.is_zero() {
        String::new()
    } else {
        num::text(v)
    }
}
fn selected<T>(list: &[T], id: Option<i64>, get: impl Fn(&T) -> i64) -> Result<&T> {
    let id = id.ok_or("一覧で対象を選んでください")?;
    list.iter()
        .find(|x| get(x) == id)
        .ok_or_else(|| "選んだ項目が見つかりません".into())
}

pub(super) fn prepare(book: &Book, s: Section, act: Act, cx: &Context) -> Result<Prepared> {
    if act == Act::Remove {
        let (target, id) = match s {
            Accounts => (Target::Account, cx.main),
            Partners => (Target::Partner, cx.main),
            Claims => (Target::Claim, cx.main),
            Budget => (Target::Budget, cx.budget),
            Plans => (Target::Plan, cx.main),
            Materials => (Target::Material, cx.main),
            Moves => (Target::MaterialMove, cx.main),
            Equipment => (Target::Equipment, cx.main),
            Products => (Target::Product, cx.main),
            Productions => (Target::Production, cx.main),
            _ => return Err("この画面では削除できません".into()),
        };
        let id = id.ok_or("一覧で対象を選んでください")?;
        return Ok(Prepared::Confirm(
            format!(
                "選んだ{}を削除しますか？（使用中のものは削除できません。使わなくなった科目・取引先は「使用停止」にします）",
                s.title()
            ),
            Change::Remove { target, id },
        ));
    }
    let new = act == Act::New;
    let title = |what: &str| format!("{what}の{}", if new { "追加" } else { "編集" });
    let money_side =
        |a: &Account| matches!(a.category, Category::Asset | Category::Liability) && !a.payment;
    Ok(match (s, act) {
        (Accounts, Act::New | Act::Edit) => {
            let a = if new {
                None
            } else {
                Some(selected(&book.accounts, cx.main, |a| a.id)?)
            };
            let category = a.map(|a| a.category).unwrap_or(Category::Expense);
            let roles: Vec<(i64, String)> = Role::ALL
                .iter()
                .enumerate()
                .map(|(i, r)| (i as i64 + 1, r.label().to_string()))
                .collect();
            let role = a
                .and_then(|a| a.role)
                .and_then(|r| Role::ALL.iter().position(|x| *x == r));
            Prepared::Form(
                title("勘定科目"),
                vec![
                    text("コード", a.map(|a| a.code.clone()).unwrap_or_default()),
                    text("科目名", a.map(|a| a.name.clone()).unwrap_or_default()),
                    Field::Choice(
                        "区分",
                        Category::ALL
                            .iter()
                            .map(|c| c.label().to_string())
                            .collect(),
                        Category::ALL
                            .iter()
                            .position(|c| *c == category)
                            .unwrap_or(4),
                    ),
                    pick(
                        "役割（自動計上用）",
                        none_and(roles, "（なし）"),
                        role.map(|i| i as i64 + 1),
                    ),
                    choice(
                        "支払手段",
                        &["いいえ", "はい"],
                        usize::from(a.is_some_and(|a| a.payment)),
                    ),
                    text(
                        "使い分けの説明",
                        a.map(|a| a.guide.clone()).unwrap_or_default(),
                    ),
                    text(
                        "検索語（、区切り）",
                        a.map(|a| a.keywords.join("、")).unwrap_or_default(),
                    ),
                    choice(
                        "使用停止",
                        &["いいえ", "はい"],
                        usize::from(a.is_some_and(|a| a.archived)),
                    ),
                ],
            )
        }
        (Partners, Act::New | Act::Edit) => {
            let p = if new {
                None
            } else {
                Some(selected(&book.partners, cx.main, |p| p.id)?)
            };
            Prepared::Form(
                title("取引先"),
                vec![
                    text("名前", p.map(|p| p.name.clone()).unwrap_or_default()),
                    text("メモ", p.map(|p| p.note.clone()).unwrap_or_default()),
                    choice(
                        "使用停止",
                        &["いいえ", "はい"],
                        usize::from(p.is_some_and(|p| p.archived)),
                    ),
                ],
            )
        }
        (Claims, Act::New | Act::Edit) => {
            let c = if new {
                None
            } else {
                Some(selected(&book.claims, cx.main, |c| c.id)?)
            };
            let mut fields = vec![
                choice(
                    "区分",
                    &["債権（受け取る）", "債務（支払う）"],
                    usize::from(c.is_some_and(|c| c.side == Side::Payable)),
                ),
                text("件名", c.map(|c| c.title.clone()).unwrap_or_default()),
                pick("取引先", sheets::partners(book), c.and_then(|c| c.partner)),
                pick(
                    "科目（売掛金・貸付金・借入金など）",
                    accounts_where(book, money_side),
                    c.map(|c| c.account),
                ),
                text(
                    "元本",
                    c.map(|c| num::text(c.principal)).unwrap_or_default(),
                ),
                text(
                    "発生日",
                    c.map(|c| c.issued).unwrap_or(cx.today).to_string(),
                ),
                text(
                    "期日・初回返済日（空欄可）",
                    c.and_then(|c| c.due)
                        .map(|d| d.to_string())
                        .unwrap_or_default(),
                ),
                text(
                    "年利率（%）",
                    c.map(|c| dec_text(c.annual_rate)).unwrap_or_default(),
                ),
                Field::Choice(
                    "返済方法",
                    Repayment::ALL
                        .iter()
                        .map(|r| r.label().to_string())
                        .collect(),
                    c.and_then(|c| Repayment::ALL.iter().position(|r| *r == c.repayment))
                        .unwrap_or(0),
                ),
                text(
                    "回数（月払い）",
                    c.map(|c| c.installments.to_string())
                        .unwrap_or_else(|| "1".into()),
                ),
                choice(
                    "完了",
                    &["いいえ", "はい"],
                    usize::from(c.is_some_and(|c| c.closed)),
                ),
                text("メモ", c.map(|c| c.memo.clone()).unwrap_or_default()),
            ];
            if new {
                fields.push(pick(
                    "発生の仕訳の相手科目",
                    none_and(accounts_where(book, |_| true), "（仕訳を作らない）"),
                    None,
                ));
            }
            Prepared::Form(title("債権・債務"), fields)
        }
        (Claims, Act::First) => {
            let c = selected(&book.claims, cx.main, |c| c.id)?;
            // The first installment not yet covered by what has been settled.
            let settled = c.principal - claims::outstanding(book, c);
            let mut cumulative = Decimal::ZERO;
            let next = claims::schedule(book, c)?.into_iter().find(|i| {
                cumulative += i.principal.0;
                cumulative > settled
            });
            let bank = book.role(Role::Bank).ok().map(|a| a.id);
            Prepared::Form(
                format!(
                    "{}の{}",
                    c.title,
                    if c.side == Side::Receivable {
                        "回収"
                    } else {
                        "返済"
                    }
                ),
                vec![
                    text("日付", cx.today.to_string()),
                    text(
                        "元本",
                        next.as_ref()
                            .map(|n| num::text(n.principal.0))
                            .unwrap_or_default(),
                    ),
                    text(
                        "利息",
                        next.as_ref()
                            .map(|n| dec_text(n.interest.0))
                            .unwrap_or_default(),
                    ),
                    pick("入出金の口座", accounts_where(book, |a| a.payment), bank),
                    text("摘要", String::new()),
                ],
            )
        }
        (Budget, Act::New | Act::Edit) => {
            let b = if new {
                None
            } else {
                Some(selected(&book.budgets, cx.budget, |b| b.id)?)
            };
            let year = cost::fiscal_year_of(book, cx.today);
            Prepared::Form(
                if new {
                    "新しい予算".into()
                } else {
                    "予算の設定".into()
                },
                vec![
                    text(
                        "名前",
                        b.map(|b| b.name.clone())
                            .unwrap_or_else(|| format!("{year}年度予算")),
                    ),
                    text(
                        "年度（開始する年）",
                        b.map(|b| b.fiscal_year).unwrap_or(year).to_string(),
                    ),
                    text("メモ", b.map(|b| b.memo.clone()).unwrap_or_default()),
                ],
            )
        }
        (Budget, Act::First) => {
            let b = selected(&book.budgets, cx.budget, |b| b.id)?;
            let line = cx
                .main
                .and_then(|a| b.lines.iter().find(|l| l.account == a));
            let (start, _) = cost::fiscal_range(book, b.fiscal_year)?;
            let mut fields = vec![
                pick(
                    "科目",
                    accounts_where(book, |a| {
                        matches!(a.category, Category::Income | Category::Expense)
                    }),
                    cx.main,
                ),
                text("毎月同額（入れると各月より優先）", String::new()),
            ];
            for i in 0..12 {
                let month = add_months(start, i).month();
                fields.push(Field::Text(
                    MONTH_LABELS[month as usize - 1],
                    line.map(|l| dec_text(l.amounts[i as usize]))
                        .unwrap_or_default(),
                ));
            }
            Prepared::Form(format!("{}の科目の予算", b.name), fields)
        }
        (Budget, Act::Second) => {
            let mut b = selected(&book.budgets, cx.budget, |b| b.id)?.clone();
            let account = cx.main.ok_or("一覧で科目を選んでください")?;
            let before = b.lines.len();
            b.lines.retain(|l| l.account != account);
            if b.lines.len() == before {
                return Err("この科目には予算がありません".into());
            }
            Prepared::Confirm(
                format!("{}の予算を外しますか？", sheets::name(book, account)),
                Change::SaveBudget { budget: b },
            )
        }
        (Plans, Act::New | Act::Edit) => {
            let p = if new {
                None
            } else {
                Some(selected(&book.plans, cx.main, |p| p.id)?)
            };
            let all = accounts_where(book, |_| true);
            Prepared::Form(
                title("計画"),
                vec![
                    text("名前", p.map(|p| p.name.clone()).unwrap_or_default()),
                    pick("借方科目", all.clone(), p.map(|p| p.debit)),
                    pick("貸方科目", all, p.map(|p| p.credit)),
                    text("金額", p.map(|p| num::text(p.amount)).unwrap_or_default()),
                    text("開始日", p.map(|p| p.start).unwrap_or(cx.today).to_string()),
                    text(
                        "終了日（空欄可）",
                        p.and_then(|p| p.end)
                            .map(|d| d.to_string())
                            .unwrap_or_default(),
                    ),
                    Field::Choice(
                        "頻度",
                        Frequency::ALL
                            .iter()
                            .map(|f| f.label().to_string())
                            .collect(),
                        p.and_then(|p| Frequency::ALL.iter().position(|f| *f == p.frequency))
                            .unwrap_or(2),
                    ),
                    text(
                        "間隔（何回ごと）",
                        p.map(|p| p.interval).unwrap_or(1).to_string(),
                    ),
                    pick("取引先", sheets::partners(book), p.and_then(|p| p.partner)),
                    choice(
                        "有効",
                        &["はい", "いいえ"],
                        usize::from(p.is_some_and(|p| !p.active)),
                    ),
                    text("メモ", p.map(|p| p.memo.clone()).unwrap_or_default()),
                ],
            )
        }
        (Plans, Act::First) => {
            let (plan, date) = cx
                .detail
                .and_then(occurrence)
                .ok_or("下の一覧で計上する予定を選んでください")?;
            Prepared::Apply(Change::PostPlan {
                plan,
                occurrence: date,
                date: None,
                amount: None,
            })
        }
        (Materials, Act::New | Act::Edit) => {
            let m = if new {
                None
            } else {
                Some(selected(&book.materials, cx.main, |m| m.id)?)
            };
            Prepared::Form(
                title("原材料"),
                vec![
                    text("名前", m.map(|m| m.name.clone()).unwrap_or_default()),
                    text(
                        "単位（kg・個など）",
                        m.map(|m| m.unit.clone()).unwrap_or_default(),
                    ),
                    text(
                        "標準単価",
                        m.map(|m| dec_text(m.standard_cost)).unwrap_or_default(),
                    ),
                    text("メモ", m.map(|m| m.memo.clone()).unwrap_or_default()),
                    choice(
                        "使用停止",
                        &["いいえ", "はい"],
                        usize::from(m.is_some_and(|m| m.archived)),
                    ),
                ],
            )
        }
        (Moves, Act::New | Act::Edit) => {
            let m = if new {
                None
            } else {
                Some(selected(&book.material_moves, cx.main, |m| m.id)?)
            };
            Prepared::Form(
                title("仕入・棚卸調整"),
                vec![
                    pick(
                        "原材料",
                        book.materials
                            .iter()
                            .filter(|x| !x.archived || m.is_some_and(|m| m.material == x.id))
                            .map(|x| (x.id, format!("{}（{}）", x.name, x.unit)))
                            .collect(),
                        m.map(|m| m.material),
                    ),
                    text("日付", m.map(|m| m.date).unwrap_or(cx.today).to_string()),
                    choice(
                        "種類",
                        &["仕入", "棚卸調整"],
                        usize::from(m.is_some_and(|m| m.kind == MoveKind::Adjust)),
                    ),
                    text(
                        "数量（調整は減少を負で）",
                        m.map(|m| num::text(m.quantity)).unwrap_or_default(),
                    ),
                    text(
                        "単価（調整は空欄可）",
                        m.and_then(|m| m.unit_cost)
                            .map(num::text)
                            .unwrap_or_default(),
                    ),
                    text("メモ", m.map(|m| m.memo.clone()).unwrap_or_default()),
                ],
            )
        }
        (Equipment, Act::New | Act::Edit) => {
            let e = if new {
                None
            } else {
                Some(selected(&book.equipment, cx.main, |e| e.id)?)
            };
            Prepared::Form(
                title("設備"),
                vec![
                    text("名前", e.map(|e| e.name.clone()).unwrap_or_default()),
                    text(
                        "取得日",
                        e.map(|e| e.acquired).unwrap_or(cx.today).to_string(),
                    ),
                    text("取得価額", e.map(|e| num::text(e.cost)).unwrap_or_default()),
                    text(
                        "残存価額",
                        e.map(|e| dec_text(e.salvage)).unwrap_or_default(),
                    ),
                    text(
                        "耐用年数",
                        e.map(|e| e.life_years.to_string()).unwrap_or_default(),
                    ),
                    text(
                        "年間稼働時間",
                        e.map(|e| num::text(e.annual_hours)).unwrap_or_default(),
                    ),
                    text(
                        "時間当たり運転費",
                        e.map(|e| dec_text(e.running_cost_per_hour))
                            .unwrap_or_default(),
                    ),
                    text("メモ", e.map(|e| e.memo.clone()).unwrap_or_default()),
                    choice(
                        "使用停止",
                        &["いいえ", "はい"],
                        usize::from(e.is_some_and(|e| e.archived)),
                    ),
                ],
            )
        }
        (Equipment, Act::First) => {
            let e = selected(&book.equipment, cx.main, |e| e.id)?;
            let year = cx.detail.ok_or("下の償却予定で年度を選んでください")? as i32;
            let (_, end) = cost::fiscal_range(book, year)?;
            Prepared::Confirm(
                format!(
                    "{}の{year}年度の減価償却を{end}付けで計上しますか？",
                    e.name
                ),
                Change::PostDepreciation {
                    equipment: e.id,
                    fiscal_year: year,
                    date: end,
                },
            )
        }
        (Products, Act::New | Act::Edit) => {
            let p = if new {
                None
            } else {
                Some(selected(&book.products, cx.main, |p| p.id)?)
            };
            Prepared::Form(
                title("製品"),
                vec![
                    text("名前", p.map(|p| p.name.clone()).unwrap_or_default()),
                    text("単位", p.map(|p| p.unit.clone()).unwrap_or_default()),
                    text(
                        "1単位の作業時間",
                        p.map(|p| dec_text(p.labor_hours)).unwrap_or_default(),
                    ),
                    text(
                        "時間当たり賃率",
                        p.map(|p| dec_text(p.labor_rate)).unwrap_or_default(),
                    ),
                    text(
                        "間接費配賦率（%）",
                        p.map(|p| dec_text(p.overhead_percent)).unwrap_or_default(),
                    ),
                    text("メモ", p.map(|p| p.memo.clone()).unwrap_or_default()),
                    choice(
                        "使用停止",
                        &["いいえ", "はい"],
                        usize::from(p.is_some_and(|p| p.archived)),
                    ),
                ],
            )
        }
        (Products, Act::First) => {
            let p = selected(&book.products, cx.main, |p| p.id)?;
            let current = cx
                .detail
                .filter(|id| *id > 0)
                .and_then(|id| p.materials.iter().find(|m| m.material == id));
            Prepared::Form(
                format!("{}の材料（1単位あたり）", p.name),
                vec![
                    pick(
                        "原材料",
                        book.materials
                            .iter()
                            .filter(|m| !m.archived)
                            .map(|m| (m.id, format!("{}（{}）", m.name, m.unit)))
                            .collect(),
                        current.map(|m| m.material),
                    ),
                    text(
                        "数量（0で外す）",
                        current.map(|m| num::text(m.quantity)).unwrap_or_default(),
                    ),
                    text(
                        "ロス率（%）",
                        current
                            .map(|m| dec_text(m.loss_percent))
                            .unwrap_or_default(),
                    ),
                ],
            )
        }
        (Products, Act::Second) => {
            let p = selected(&book.products, cx.main, |p| p.id)?;
            let current = cx
                .detail
                .filter(|id| *id < 0)
                .and_then(|id| p.equipment.iter().find(|e| e.equipment == -id));
            Prepared::Form(
                format!("{}の設備時間（1単位あたり）", p.name),
                vec![
                    pick(
                        "設備",
                        book.equipment
                            .iter()
                            .filter(|e| !e.archived)
                            .map(|e| (e.id, e.name.clone()))
                            .collect(),
                        current.map(|e| e.equipment),
                    ),
                    text(
                        "時間（0で外す）",
                        current.map(|e| num::text(e.hours)).unwrap_or_default(),
                    ),
                ],
            )
        }
        (Productions, Act::New | Act::Edit) => {
            let p = if new {
                None
            } else {
                Some(selected(&book.productions, cx.main, |p| p.id)?)
            };
            Prepared::Form(
                title("製造実績"),
                vec![
                    pick(
                        "製品",
                        book.products
                            .iter()
                            .filter(|x| !x.archived || p.is_some_and(|p| p.product == x.id))
                            .map(|x| (x.id, format!("{}（{}）", x.name, x.unit)))
                            .collect(),
                        p.map(|p| p.product),
                    ),
                    text("日付", p.map(|p| p.date).unwrap_or(cx.today).to_string()),
                    text("数量", p.map(|p| num::text(p.quantity)).unwrap_or_default()),
                    text(
                        "作業時間（合計）",
                        p.map(|p| dec_text(p.labor_hours)).unwrap_or_default(),
                    ),
                    text("メモ", p.map(|p| p.memo.clone()).unwrap_or_default()),
                ],
            )
        }
        _ => return Err("この画面にはない操作です".into()),
    })
}
const MONTH_LABELS: [&str; 12] = [
    "1月", "2月", "3月", "4月", "5月", "6月", "7月", "8月", "9月", "10月", "11月", "12月",
];

// ----- values from forms -----
fn dec(v: &str, what: &str) -> Result<Decimal> {
    if v.trim().is_empty() {
        return Err(format!("{what}を入れてください").into());
    }
    Ok(num::parse(v).map_err(|e| format!("{what}: {e}"))?)
}
fn dec0(v: &str) -> Result<Decimal> {
    sheets::amount(v)
}
fn opt_dec(v: &str) -> Result<Option<Decimal>> {
    Ok(if v.trim().is_empty() {
        None
    } else {
        Some(num::parse(v)?)
    })
}
fn date(v: &str, what: &str) -> Result<NaiveDate> {
    v.trim()
        .replace('/', "-")
        .parse()
        .map_err(|_| format!("{what}は 2026-04-01 の形で入れてください").into())
}
fn opt_date(v: &str, what: &str) -> Result<Option<NaiveDate>> {
    if v.trim().is_empty() {
        Ok(None)
    } else {
        date(v, what).map(Some)
    }
}
fn id(v: &str) -> Option<i64> {
    v.parse::<i64>().ok().filter(|i| *i != 0)
}
fn index(v: &str) -> usize {
    v.parse().unwrap_or(0)
}
fn count(v: &str, what: &str) -> Result<u32> {
    v.trim()
        .parse()
        .map_err(|_| format!("{what}は整数で入れてください").into())
}

/// The change a submitted form makes.
pub(super) fn finish(
    book: &Book,
    s: Section,
    act: Act,
    cx: &Context,
    v: &[String],
) -> Result<Change> {
    let edit = act == Act::Edit;
    let keep = |list_id: Option<i64>| if edit { list_id.unwrap_or(0) } else { 0 };
    Ok(match (s, act) {
        (Accounts, _) => {
            let old = book.accounts.iter().find(|a| edit && Some(a.id) == cx.main);
            Change::SaveAccount {
                account: Account {
                    id: keep(cx.main),
                    code: v[0].trim().into(),
                    name: v[1].trim().into(),
                    category: Category::ALL[index(&v[2]).min(4)],
                    role: id(&v[3]).and_then(|i| Role::ALL.get(i as usize - 1).copied()),
                    parent: old.and_then(|a| a.parent),
                    payment: index(&v[4]) == 1,
                    guide: v[5].trim().into(),
                    keywords: v[6]
                        .split(['、', ','])
                        .map(|k| k.trim().to_string())
                        .filter(|k| !k.is_empty())
                        .collect(),
                    archived: index(&v[7]) == 1,
                },
            }
        }
        (Partners, _) => Change::SavePartner {
            partner: Partner {
                id: keep(cx.main),
                name: v[0].trim().into(),
                note: v[1].trim().into(),
                archived: index(&v[2]) == 1,
            },
        },
        (Claims, Act::First) => Change::SettleClaim {
            settlement: Settlement {
                claim: cx.main.ok_or("債権・債務を選んでください")?,
                date: date(&v[0], "日付")?,
                principal: dec0(&v[1])?,
                interest: dec0(&v[2])?,
                via: id(&v[3]).ok_or("入出金の口座を選んでください")?,
                description: v[4].trim().into(),
            },
        },
        (Claims, _) => Change::SaveClaim {
            claim: Claim {
                id: keep(cx.main),
                side: if index(&v[0]) == 1 {
                    Side::Payable
                } else {
                    Side::Receivable
                },
                title: v[1].trim().into(),
                partner: id(&v[2]),
                account: id(&v[3]).ok_or("科目を選んでください")?,
                principal: dec(&v[4], "元本")?,
                issued: date(&v[5], "発生日")?,
                due: opt_date(&v[6], "期日")?,
                annual_rate: dec0(&v[7])?,
                repayment: Repayment::ALL[index(&v[8]).min(2)],
                installments: count(&v[9], "回数")?,
                closed: index(&v[10]) == 1,
                memo: v[11].trim().into(),
            },
            opening: v.get(12).and_then(|x| id(x)),
        },
        (Budget, Act::First) => {
            let mut b = selected(&book.budgets, cx.budget, |b| b.id)?.clone();
            let account = id(&v[0]).ok_or("科目を選んでください")?;
            let same = opt_dec(&v[1])?;
            let amounts = (0..12)
                .map(|i| match same {
                    Some(x) => Ok(x),
                    None => dec0(&v[2 + i]),
                })
                .collect::<Result<Vec<_>>>()?;
            match b.lines.iter_mut().find(|l| l.account == account) {
                Some(l) => l.amounts = amounts,
                None => b.lines.push(BudgetLine { account, amounts }),
            }
            Change::SaveBudget { budget: b }
        }
        (Budget, _) => {
            let old = book
                .budgets
                .iter()
                .find(|b| edit && Some(b.id) == cx.budget);
            Change::SaveBudget {
                budget: Budget {
                    id: old.map(|b| b.id).unwrap_or(0),
                    name: v[0].trim().into(),
                    fiscal_year: v[1]
                        .trim()
                        .parse()
                        .map_err(|_| "年度は西暦の整数で入れてください")?,
                    memo: v[2].trim().into(),
                    lines: old.map(|b| b.lines.clone()).unwrap_or_default(),
                },
            }
        }
        (Plans, _) => Change::SavePlan {
            plan: Plan {
                id: keep(cx.main),
                name: v[0].trim().into(),
                debit: id(&v[1]).ok_or("借方科目を選んでください")?,
                credit: id(&v[2]).ok_or("貸方科目を選んでください")?,
                amount: dec(&v[3], "金額")?,
                start: date(&v[4], "開始日")?,
                end: opt_date(&v[5], "終了日")?,
                frequency: Frequency::ALL[index(&v[6]).min(3)],
                interval: count(&v[7], "間隔")?,
                partner: id(&v[8]),
                active: index(&v[9]) == 0,
                memo: v[10].trim().into(),
            },
        },
        (Materials, _) => Change::SaveMaterial {
            material: Material {
                id: keep(cx.main),
                name: v[0].trim().into(),
                unit: v[1].trim().into(),
                standard_cost: dec0(&v[2])?,
                memo: v[3].trim().into(),
                archived: index(&v[4]) == 1,
            },
        },
        (Moves, _) => Change::SaveMaterialMove {
            material_move: MaterialMove {
                id: keep(cx.main),
                material: id(&v[0]).ok_or("原材料を選んでください")?,
                date: date(&v[1], "日付")?,
                kind: if index(&v[2]) == 1 {
                    MoveKind::Adjust
                } else {
                    MoveKind::Purchase
                },
                quantity: dec(&v[3], "数量")?,
                unit_cost: opt_dec(&v[4])?,
                memo: v[5].trim().into(),
            },
        },
        (Equipment, _) => Change::SaveEquipment {
            equipment: Equipment {
                id: keep(cx.main),
                name: v[0].trim().into(),
                acquired: date(&v[1], "取得日")?,
                cost: dec(&v[2], "取得価額")?,
                salvage: dec0(&v[3])?,
                life_years: count(&v[4], "耐用年数")?,
                annual_hours: dec(&v[5], "年間稼働時間")?,
                running_cost_per_hour: dec0(&v[6])?,
                memo: v[7].trim().into(),
                archived: index(&v[8]) == 1,
            },
        },
        (Products, Act::First | Act::Second) => {
            let mut p = selected(&book.products, cx.main, |p| p.id)?.clone();
            let target = id(&v[0]).ok_or("対象を選んでください")?;
            let quantity = dec0(&v[1])?;
            if act == Act::First {
                p.materials.retain(|m| m.material != target);
                if !quantity.is_zero() {
                    p.materials.push(ProductMaterial {
                        material: target,
                        quantity,
                        loss_percent: dec0(&v[2])?,
                    });
                }
            } else {
                p.equipment.retain(|e| e.equipment != target);
                if !quantity.is_zero() {
                    p.equipment.push(EquipmentUse {
                        equipment: target,
                        hours: quantity,
                    });
                }
            }
            Change::SaveProduct { product: p }
        }
        (Products, _) => {
            let old = book.products.iter().find(|p| edit && Some(p.id) == cx.main);
            Change::SaveProduct {
                product: Product {
                    id: keep(cx.main),
                    name: v[0].trim().into(),
                    unit: v[1].trim().into(),
                    labor_hours: dec0(&v[2])?,
                    labor_rate: dec0(&v[3])?,
                    overhead_percent: dec0(&v[4])?,
                    memo: v[5].trim().into(),
                    archived: index(&v[6]) == 1,
                    materials: old.map(|p| p.materials.clone()).unwrap_or_default(),
                    equipment: old.map(|p| p.equipment.clone()).unwrap_or_default(),
                },
            }
        }
        (Productions, _) => {
            let old = book
                .productions
                .iter()
                .find(|p| edit && Some(p.id) == cx.main);
            Change::SaveProduction {
                production: Production {
                    id: keep(cx.main),
                    product: id(&v[0]).ok_or("製品を選んでください")?,
                    date: date(&v[1], "日付")?,
                    quantity: dec(&v[2], "数量")?,
                    labor_hours: dec0(&v[3])?,
                    memo: v[4].trim().into(),
                    materials: old.map(|p| p.materials.clone()).unwrap_or_default(),
                    equipment: old.map(|p| p.equipment.clone()).unwrap_or_default(),
                },
            }
        }
        _ => return Err("この画面にはない操作です".into()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use yy_accountant::{BookKind, NewBook, Store};
    fn context(today: NaiveDate) -> Context {
        Context {
            today,
            from: Some(today),
            to: None,
            flag: false,
            budget: None,
            main: None,
            detail: None,
        }
    }
    fn values(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn forms_round_trip_through_the_core() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(&dir.path().join("r.sqlite3"), false).unwrap();
        let mut book = store
            .create(
                &NewBook {
                    name: "原価".into(),
                    kind: BookKind::SoleProprietor,
                    currency: "JPY".into(),
                    minor_digits: 0,
                    rounding: Default::default(),
                    fiscal_start_month: 1,
                },
                "test",
            )
            .unwrap();
        let today: NaiveDate = "2026-10-29".parse().unwrap();
        let mut apply = |book: &mut Book, change: Change| {
            *book = store
                .apply(book.id, book.revision, &change, "test")
                .unwrap()
                .book;
        };
        let mut cx = context(today);
        let id_of = |b: &Book, name: &str| b.accounts.iter().find(|a| a.name == name).unwrap().id;
        // Every form the buttons open has as many fields as `finish` reads.
        for s in GROUPS.iter().flat_map(|g| g.1.iter()) {
            if s.spec().new.is_some() && *s != Budget {
                let Ok(Prepared::Form(_, fields)) = prepare(&book, *s, Act::New, &cx) else {
                    panic!("{s:?}");
                };
                assert!(fields.len() >= 3, "{s:?}");
            }
        }
        let change = finish(
            &book,
            Equipment,
            Act::New,
            &cx,
            &values(&[
                "オーブン",
                "2026-07-01",
                "600,000",
                "",
                "5",
                "1000",
                "30",
                "",
                "いいえ",
            ]),
        )
        .unwrap();
        apply(&mut book, change);
        let oven = book.equipment[0].id;
        cx.main = Some(oven);
        let shown = view(&book, Equipment, &cx, &[]).unwrap();
        assert_eq!(shown.main.rows[0].1[6], "150");
        let first = shown.detail.rows[0].0;
        cx.detail = Some(first);
        let Prepared::Confirm(_, change) = prepare(&book, Equipment, Act::First, &cx).unwrap()
        else {
            panic!()
        };
        apply(&mut book, change);
        assert_eq!(book.entries.len(), 1);
        assert_eq!(book.entries[0].total(), num::parse("60000").unwrap());

        let change = finish(
            &book,
            Materials,
            Act::New,
            &cx,
            &values(&["小麦粉", "kg", "200", "", "0"]),
        )
        .unwrap();
        apply(&mut book, change);
        let flour = book.materials[0].id;
        let change = finish(
            &book,
            Moves,
            Act::New,
            &cx,
            &values(&[&flour.to_string(), "2026-10-01", "0", "10", "180", ""]),
        )
        .unwrap();
        apply(&mut book, change);
        let change = finish(
            &book,
            Products,
            Act::New,
            &cx,
            &values(&["食パン", "斤", "0.5", "1000", "10", "", "0"]),
        )
        .unwrap();
        apply(&mut book, change);
        cx.main = Some(book.products[0].id);
        let change = finish(
            &book,
            Products,
            Act::First,
            &cx,
            &values(&[&flour.to_string(), "0.5", "0"]),
        )
        .unwrap();
        apply(&mut book, change);
        let products = view(&book, Products, &cx, &[]).unwrap();
        // 材料 0.5kg×標準単価200円＝100、労務 0.5時間×1,000円＝500、間接費 10%＝60 → 660
        assert_eq!(products.main.rows[0].1[6], "660");
        assert!(products.detail.rows.iter().any(|r| r.0 == flour));

        let bank = id_of(&book, "普通預金");
        let rent = id_of(&book, "地代家賃");
        let change = finish(
            &book,
            Plans,
            Act::New,
            &cx,
            &values(&[
                "家賃",
                &rent.to_string(),
                &bank.to_string(),
                "80000",
                "2026-11-25",
                "",
                "2",
                "1",
                "0",
                "0",
                "",
            ]),
        )
        .unwrap();
        apply(&mut book, change);
        cx.main = None;
        cx.from = Some(today);
        cx.to = Some("2027-01-31".parse().unwrap());
        let plan = view(&book, Plans, &cx, &[]).unwrap();
        assert_eq!(plan.detail.rows.len(), 3);
        cx.detail = Some(plan.detail.rows[0].0);
        let Prepared::Apply(change) = prepare(&book, Plans, Act::First, &cx).unwrap() else {
            panic!()
        };
        apply(&mut book, change);
        assert!(view(&book, Plans, &cx, &[]).unwrap().detail.rows[0].1[5] == "計上済");

        let change = finish(
            &book,
            Budget,
            Act::New,
            &cx,
            &values(&["2026年度予算", "2026", ""]),
        )
        .unwrap();
        apply(&mut book, change);
        cx.budget = Some(book.budgets[0].id);
        let mut line = vec![rent.to_string(), "80,000".to_string()];
        line.extend(std::iter::repeat_n(String::new(), 12));
        let change = finish(&book, Budget, Act::First, &cx, &line).unwrap();
        apply(&mut book, change);
        let budget = view(&book, Budget, &cx, &[]).unwrap();
        let row = budget.main.rows.iter().find(|r| r.0 == rent).unwrap();
        assert_eq!(
            (row.1[1].as_str(), row.1[2].as_str()),
            ("960,000", "80,000")
        );

        let loan = id_of(&book, "借入金");
        let change = finish(
            &book,
            Claims,
            Act::New,
            &cx,
            &values(&[
                "1",
                "設備資金",
                "0",
                &loan.to_string(),
                "1200000",
                "2026-01-10",
                "2026-02-10",
                "2.4",
                "1",
                "12",
                "0",
                "",
                &bank.to_string(),
            ]),
        )
        .unwrap();
        apply(&mut book, change);
        cx.main = Some(book.claims[0].id);
        let claim = view(&book, Claims, &cx, &[]).unwrap();
        assert_eq!(claim.detail.rows.len(), 12);
        assert_eq!(claim.detail.rows[0].1[4], "101,305");
        assert!(view(&book, Monthly, &context(today), &[]).is_ok());
        assert!(view(&book, Forecast, &context(today), &[]).is_ok());
        assert!(prepare(&book, Partners, Act::Remove, &context(today)).is_err());
    }
}
