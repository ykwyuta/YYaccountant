//! ライフプラン：家族の年齢、収入・支出の流れ（変動率つき）、ライフイベントから年ごとの
//! キャッシュフロー表と金融資産残高を作る。FPの提案に使う比較（現状と提案）、6つの係数、
//! 教育費の目安、ローン返済額、帳簿からの現状の取り込みも提供する。
use crate::model::{Book, Category, text};
use crate::num::{self, D, Decimal};
use crate::reports;
use crate::{Result, invalid};
use chrono::{Datelike, Months, NaiveDate};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LifePlan {
    #[serde(default)]
    pub id: i64,
    pub name: String,
    #[serde(default)]
    pub memo: String,
    /// 表の最初の年。金額はこの年の価格で入力し、変動率で将来値にする。
    pub start_year: i32,
    /// 表の年数（1〜100）
    pub years: u32,
    /// 最初の年の初めの金融資産（預貯金・投資など）
    #[serde(default, with = "num::dec")]
    pub savings: Decimal,
    /// 金融資産の運用利回り（年%）
    #[serde(default, with = "num::dec")]
    pub return_percent: Decimal,
    #[serde(default)]
    pub members: Vec<PlanMember>,
    #[serde(default)]
    pub flows: Vec<PlanFlow>,
    #[serde(default)]
    pub events: Vec<PlanEvent>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanMember {
    pub name: String,
    #[serde(default)]
    pub relation: String,
    pub birth_year: i32,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FlowKind {
    Income,
    #[default]
    Expense,
}
impl FlowKind {
    pub const ALL: [Self; 2] = [Self::Income, Self::Expense];
    pub fn label(self) -> &'static str {
        match self {
            Self::Income => "収入",
            Self::Expense => "支出",
        }
    }
}
/// 毎年の収入・支出（給与の手取り、生活費、住宅ローン、教育費、年金など）。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanFlow {
    pub name: String,
    pub kind: FlowKind,
    /// 年額（最初の年の価格）
    #[serde(with = "num::dec")]
    pub amount: Decimal,
    pub start_year: i32,
    pub end_year: i32,
    /// 変動率（年%）。物価上昇や昇給を表す。負も可。
    #[serde(default, with = "num::dec")]
    pub growth_percent: Decimal,
}
/// 1回だけの収入・支出（車の買い替え、住宅購入の頭金、退職金、相続以外の一時金など）。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanEvent {
    pub year: i32,
    pub name: String,
    pub kind: FlowKind,
    #[serde(with = "num::dec")]
    pub amount: Decimal,
    #[serde(default, with = "num::dec")]
    pub growth_percent: Decimal,
}

fn rate_ok(d: Decimal, what: &str) -> Result<()> {
    if d <= -num::hundred() || d > num::hundred() || d.scale() > 4 {
        return Err(invalid(format!(
            "{what}は−100%より大きく100%以下、小数4桁以内にしてください"
        )));
    }
    Ok(())
}
fn year_ok(y: i32, what: &str) -> Result<()> {
    if !(1900..=2999).contains(&y) {
        return Err(invalid(format!("{what}は1900〜2999年にしてください")));
    }
    Ok(())
}
pub(crate) fn validate(book: &Book, p: &LifePlan) -> Result<()> {
    text(&p.name, "ライフプラン名", 200, true)?;
    text(&p.memo, "メモ", 4000, false)?;
    year_ok(p.start_year, "最初の年")?;
    if !(1..=100).contains(&p.years) {
        return Err(invalid("年数は1〜100年にしてください"));
    }
    book.money(p.savings, "金融資産")?;
    rate_ok(p.return_percent, "運用利回り")?;
    if p.members.len() > 20 || p.flows.len() > 200 || p.events.len() > 500 {
        return Err(invalid(
            "家族は20人、収支は200件、イベントは500件以内にしてください",
        ));
    }
    for m in &p.members {
        text(&m.name, "家族の名前", 100, true)?;
        text(&m.relation, "続柄", 100, false)?;
        year_ok(m.birth_year, "生まれ年")?;
    }
    for f in &p.flows {
        text(&f.name, "収支の名前", 200, true)?;
        book.money(f.amount, "年額")?;
        if f.amount.is_sign_negative() {
            return Err(invalid(
                "年額は0以上にし、収入か支出かは種類で選んでください",
            ));
        }
        year_ok(f.start_year, "開始年")?;
        year_ok(f.end_year, "終了年")?;
        if f.end_year < f.start_year {
            return Err(invalid(format!(
                "{}：終了年は開始年以降にしてください",
                f.name
            )));
        }
        rate_ok(f.growth_percent, "変動率")?;
    }
    for e in &p.events {
        text(&e.name, "イベントの名前", 200, true)?;
        year_ok(e.year, "イベントの年")?;
        book.money(e.amount, "イベントの金額")?;
        if e.amount.is_sign_negative() {
            return Err(invalid(
                "金額は0以上にし、収入か支出かは種類で選んでください",
            ));
        }
        rate_ok(e.growth_percent, "変動率")?;
    }
    Ok(())
}

/// 最初の年の価格を `elapsed` 年後の価格にする。
fn grow(book: &Book, amount: Decimal, percent: Decimal, elapsed: i32) -> Result<Decimal> {
    if percent.is_zero() || elapsed <= 0 {
        return Ok(amount);
    }
    let f = num::powi(Decimal::ONE + percent / num::hundred(), elapsed as u32)?;
    Ok(book.round(amount * f))
}

#[derive(Clone, Debug, Serialize)]
pub struct CashFlowItem {
    pub name: String,
    pub kind: FlowKind,
    /// イベントなら true（列ではなく年ごとの注記として示す）
    pub event: bool,
}
#[derive(Clone, Debug, Serialize)]
pub struct CashFlowRow {
    pub year: i32,
    /// 家族の年末時点の年齢（まだ生まれていなければ None）
    pub ages: Vec<Option<i32>>,
    /// 収支の項目ごとの金額（`flows` の順）
    pub values: Vec<D>,
    /// その年のイベント（名前、金額。支出は負）
    pub events: Vec<(String, D)>,
    pub income: D,
    pub expense: D,
    /// 年間収支（収入 − 支出）
    pub balance: D,
    /// 前年末の金融資産に利回りを掛けた運用益（残高が負なら0）
    pub investment: D,
    /// 年末の金融資産
    pub savings: D,
}
#[derive(Clone, Debug, Serialize)]
pub struct CashFlow {
    pub plan: i64,
    pub name: String,
    pub members: Vec<String>,
    pub flows: Vec<CashFlowItem>,
    pub rows: Vec<CashFlowRow>,
    pub total_income: D,
    pub total_expense: D,
    pub final_savings: D,
    pub min_savings: D,
    pub min_year: i32,
    /// 年末の金融資産がマイナスになる年
    pub shortfall_years: Vec<i32>,
    pub notes: Vec<String>,
}

pub fn cash_flow(book: &Book, p: &LifePlan) -> Result<CashFlow> {
    validate(book, p)?;
    let rate = p.return_percent / num::hundred();
    // Flows entirely outside the table (a school stage already finished) get no column.
    let last = p.start_year + p.years as i32 - 1;
    let flows: Vec<&PlanFlow> = p
        .flows
        .iter()
        .filter(|f| f.end_year >= p.start_year && f.start_year <= last)
        .collect();
    let mut savings = p.savings;
    let mut rows = vec![];
    let (mut total_income, mut total_expense) = (Decimal::ZERO, Decimal::ZERO);
    let mut min: Option<(Decimal, i32)> = None;
    let mut shortfall = vec![];
    for i in 0..p.years as i32 {
        let year = p.start_year + i;
        let mut income = Decimal::ZERO;
        let mut expense = Decimal::ZERO;
        let mut values = vec![];
        for f in &flows {
            let v = if (f.start_year..=f.end_year).contains(&year) {
                grow(book, f.amount, f.growth_percent, i)?
            } else {
                Decimal::ZERO
            };
            match f.kind {
                FlowKind::Income => income += v,
                FlowKind::Expense => expense += v,
            }
            values.push(D(v));
        }
        let mut events = vec![];
        for e in p.events.iter().filter(|e| e.year == year) {
            let v = grow(book, e.amount, e.growth_percent, i)?;
            match e.kind {
                FlowKind::Income => {
                    income += v;
                    events.push((e.name.clone(), D(v)));
                }
                FlowKind::Expense => {
                    expense += v;
                    events.push((e.name.clone(), D(-v)));
                }
            }
        }
        let investment = if savings.is_sign_positive() {
            book.round(savings * rate)
        } else {
            Decimal::ZERO
        };
        let balance = income - expense;
        savings += investment + balance;
        if savings.abs() >= num::limit() {
            return Err(invalid("金融資産が大きすぎて計算できません"));
        }
        if min.is_none_or(|(m, _)| savings < m) {
            min = Some((savings, year));
        }
        if savings.is_sign_negative() && !savings.is_zero() {
            shortfall.push(year);
        }
        total_income += income;
        total_expense += expense;
        rows.push(CashFlowRow {
            year,
            ages: p
                .members
                .iter()
                .map(|m| (year >= m.birth_year).then_some(year - m.birth_year))
                .collect(),
            values,
            events,
            income: D(income),
            expense: D(expense),
            balance: D(balance),
            investment: D(investment),
            savings: D(savings),
        });
    }
    let min = min.unwrap_or((p.savings, p.start_year));
    let mut notes = vec![];
    if let (Some(first), Some(last)) = (shortfall.first(), shortfall.last()) {
        notes.push(format!(
            "{first}年から金融資産がマイナスになります（最後は{last}年）。支出の見直し、収入の確保、運用や繰上返済の検討が必要です。"
        ));
    }
    notes.push("金額は最初の年の価格で入力し、変動率で毎年増減させています。年齢は各年の年末時点です。税金・社会保険料は手取りで入力する前提です。".into());
    Ok(CashFlow {
        plan: p.id,
        name: p.name.clone(),
        members: p.members.iter().map(|m| m.name.clone()).collect(),
        flows: flows
            .iter()
            .map(|f| CashFlowItem {
                name: f.name.clone(),
                kind: f.kind,
                event: false,
            })
            .collect(),
        rows,
        total_income: D(total_income),
        total_expense: D(total_expense),
        final_savings: D(savings),
        min_savings: D(min.0),
        min_year: min.1,
        shortfall_years: shortfall,
        notes,
    })
}

/// キャッシュフロー表のCSV（UTF-8 BOM、CRLF）。提案資料に表計算ソフトで使う。
pub fn csv(book: &Book, c: &CashFlow) -> String {
    let money = |d: &D| num::fixed(d.0, book.minor_digits);
    let mut head = vec!["年".to_string()];
    head.extend(c.members.iter().map(|m| format!("{m}（歳）")));
    head.extend(
        c.flows
            .iter()
            .map(|f| format!("{}（{}）", f.name, f.kind.label())),
    );
    head.extend(
        [
            "イベント",
            "収入合計",
            "支出合計",
            "年間収支",
            "運用益",
            "金融資産残高",
        ]
        .map(String::from),
    );
    let line = |cells: Vec<String>| {
        cells
            .iter()
            .map(|c| reports::csv_cell(c))
            .collect::<Vec<_>>()
            .join(",")
            + "\r\n"
    };
    let mut out = String::from("\u{feff}") + &line(head);
    for r in &c.rows {
        let mut cells = vec![r.year.to_string()];
        cells.extend(
            r.ages
                .iter()
                .map(|a| a.map(|a| a.to_string()).unwrap_or_default()),
        );
        cells.extend(r.values.iter().map(money));
        cells.push(
            r.events
                .iter()
                .map(|(n, v)| format!("{n} {}", money(v)))
                .collect::<Vec<_>>()
                .join(" / "),
        );
        cells.extend([&r.income, &r.expense, &r.balance, &r.investment, &r.savings].map(money));
        out += &line(cells);
    }
    out
}

#[derive(Clone, Debug, Serialize)]
pub struct ComparisonRow {
    pub year: i32,
    pub base: Option<D>,
    pub proposal: Option<D>,
    /// 提案 − 現状
    pub difference: Option<D>,
}
#[derive(Clone, Debug, Serialize)]
pub struct Comparison {
    pub base: String,
    pub proposal: String,
    pub rows: Vec<ComparisonRow>,
    pub final_difference: D,
    pub base_min: D,
    pub proposal_min: D,
    pub base_shortfall: Vec<i32>,
    pub proposal_shortfall: Vec<i32>,
}
/// 現状と提案のライフプランの年末金融資産を年ごとに並べる。
pub fn compare(book: &Book, base: &LifePlan, proposal: &LifePlan) -> Result<Comparison> {
    let a = cash_flow(book, base)?;
    let b = cash_flow(book, proposal)?;
    let first = base.start_year.min(proposal.start_year);
    let last =
        (base.start_year + base.years as i32).max(proposal.start_year + proposal.years as i32);
    let find = |c: &CashFlow, y: i32| c.rows.iter().find(|r| r.year == y).map(|r| r.savings);
    let rows = (first..last)
        .map(|year| {
            let (x, y) = (find(&a, year), find(&b, year));
            ComparisonRow {
                year,
                base: x,
                proposal: y,
                difference: x.zip(y).map(|(x, y)| D(y.0 - x.0)),
            }
        })
        .collect();
    Ok(Comparison {
        base: a.name,
        proposal: b.name,
        rows,
        final_difference: D(b.final_savings.0 - a.final_savings.0),
        base_min: a.min_savings,
        proposal_min: b.min_savings,
        base_shortfall: a.shortfall_years,
        proposal_shortfall: b.shortfall_years,
    })
}

/// 6つの係数（利率は年%）。小数6桁に丸める。
#[derive(Clone, Debug, Serialize)]
pub struct Coefficients {
    pub rate: D,
    pub years: u32,
    /// 終価係数：今の元本が n 年後にいくらになるか
    pub future_value: D,
    /// 現価係数：n 年後の目標額に今いくら必要か
    pub present_value: D,
    /// 年金終価係数：毎年の積立が n 年後にいくらになるか
    pub annuity_future: D,
    /// 減債基金係数：n 年後の目標額のために毎年いくら積み立てるか
    pub sinking_fund: D,
    /// 資本回収係数：今の元本を n 年で取り崩すと毎年いくら受け取れるか（ローンの年返済額）
    pub capital_recovery: D,
    /// 年金現価係数：毎年の受取を n 年続けるには今いくら必要か
    pub annuity_present: D,
}
pub fn coefficients(rate_percent: Decimal, years: u32) -> Result<Coefficients> {
    rate_ok(rate_percent, "利率")?;
    if rate_percent.is_sign_negative() {
        return Err(invalid("利率は0以上にしてください"));
    }
    if !(1..=100).contains(&years) {
        return Err(invalid("年数は1〜100年にしてください"));
    }
    let r = rate_percent / num::hundred();
    let n = Decimal::from(years);
    let f = num::powi(Decimal::ONE + r, years)?;
    let round = |d: Decimal| D(d.round_dp(6).normalize());
    let (af, sf, cr, ap) = if r.is_zero() {
        (n, Decimal::ONE / n, Decimal::ONE / n, n)
    } else {
        (
            (f - Decimal::ONE) / r,
            r / (f - Decimal::ONE),
            r * f / (f - Decimal::ONE),
            (f - Decimal::ONE) / (r * f),
        )
    };
    Ok(Coefficients {
        rate: D(rate_percent),
        years,
        future_value: round(f),
        present_value: round(Decimal::ONE / f),
        annuity_future: round(af),
        sinking_fund: round(sf),
        capital_recovery: round(cr),
        annuity_present: round(ap),
    })
}

#[derive(Clone, Debug, Serialize)]
pub struct LoanPayment {
    pub monthly: D,
    pub annual: D,
    pub total: D,
    pub interest: D,
}
/// 元利均等返済（毎月）の返済額。
pub fn loan_payment(
    book: &Book,
    principal: Decimal,
    rate_percent: Decimal,
    years: u32,
) -> Result<LoanPayment> {
    book.money(principal, "借入額")?;
    rate_ok(rate_percent, "金利")?;
    if principal <= Decimal::ZERO || rate_percent.is_sign_negative() || !(1..=50).contains(&years) {
        return Err(invalid(
            "借入額は正、金利は0以上、期間は1〜50年にしてください",
        ));
    }
    let n = years * 12;
    let r = rate_percent / num::hundred() / Decimal::from(12);
    let monthly = if r.is_zero() {
        book.round(principal / Decimal::from(n))
    } else {
        let f = num::powi(Decimal::ONE + r, n)?;
        book.round(principal * r * f / (f - Decimal::ONE))
    };
    let total = monthly * Decimal::from(n);
    Ok(LoanPayment {
        monthly: D(monthly),
        annual: D(monthly * Decimal::from(12)),
        total: D(total),
        interest: D(total - principal),
    })
}

/// 教育費の段階
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    Kindergarten,
    Elementary,
    Junior,
    High,
    University,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum School {
    /// 通わない・計上しない
    None,
    #[default]
    Public,
    Private,
}
#[derive(Clone, Copy, Debug, Serialize)]
pub struct EducationCost {
    pub stage: Stage,
    pub label: &'static str,
    /// 年末時点の年齢の範囲
    pub first_age: i32,
    pub last_age: i32,
    pub public: i64,
    pub private: i64,
    /// 入学時の一時金（入学金など）
    pub public_entry: i64,
    pub private_entry: i64,
}
/// 教育費の目安（年額、円）。幼稚園〜高校は文部科学省「令和5年度子供の学習費調査」の学習費総額、
/// 大学は国立大学の標準額と文部科学省「私立大学等の令和5年度入学者に係る学生納付金等調査」の平均。
/// 自治体・学校で大きく違うので、実際の金額で上書きして使う。
pub const EDUCATION: [EducationCost; 5] = [
    EducationCost {
        stage: Stage::Kindergarten,
        label: "幼稚園",
        first_age: 4,
        last_age: 6,
        public: 184_646,
        private: 347_338,
        public_entry: 0,
        private_entry: 0,
    },
    EducationCost {
        stage: Stage::Elementary,
        label: "小学校",
        first_age: 7,
        last_age: 12,
        public: 336_265,
        private: 1_828_112,
        public_entry: 0,
        private_entry: 0,
    },
    EducationCost {
        stage: Stage::Junior,
        label: "中学校",
        first_age: 13,
        last_age: 15,
        public: 542_475,
        private: 1_560_359,
        public_entry: 0,
        private_entry: 0,
    },
    EducationCost {
        stage: Stage::High,
        label: "高校",
        first_age: 16,
        last_age: 18,
        public: 597_752,
        private: 1_029_671,
        public_entry: 0,
        private_entry: 0,
    },
    EducationCost {
        stage: Stage::University,
        label: "大学",
        first_age: 19,
        last_age: 22,
        public: 535_800,
        private: 1_124_476,
        public_entry: 282_000,
        private_entry: 240_806,
    },
];
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EducationRequest {
    pub child: String,
    pub birth_year: i32,
    /// 幼稚園・小学校・中学校・高校・大学の順
    pub schools: [School; 5],
    /// 教育費の上昇率（年%）
    #[serde(default, with = "num::dec")]
    pub growth_percent: Decimal,
}
/// 子の生まれ年と進路から教育費の収支（年額）と入学金のイベントを作る。
pub fn education(r: &EducationRequest) -> Result<(Vec<PlanFlow>, Vec<PlanEvent>)> {
    text(&r.child, "子の名前", 100, true)?;
    year_ok(r.birth_year, "生まれ年")?;
    rate_ok(r.growth_percent, "上昇率")?;
    let mut flows = vec![];
    let mut events = vec![];
    for (c, school) in EDUCATION.iter().zip(r.schools) {
        let (amount, entry, kind) = match school {
            School::None => continue,
            School::Public => (c.public, c.public_entry, "公立"),
            School::Private => (c.private, c.private_entry, "私立"),
        };
        let kind = if c.stage == Stage::University && school == School::Public {
            "国公立"
        } else {
            kind
        };
        flows.push(PlanFlow {
            name: format!("{}の{}（{}）", r.child, c.label, kind),
            kind: FlowKind::Expense,
            amount: Decimal::from(amount),
            start_year: r.birth_year + c.first_age,
            end_year: r.birth_year + c.last_age,
            growth_percent: r.growth_percent,
        });
        if entry > 0 {
            events.push(PlanEvent {
                year: r.birth_year + c.first_age,
                name: format!("{}の{}入学金", r.child, c.label),
                kind: FlowKind::Expense,
                amount: Decimal::from(entry),
                growth_percent: r.growth_percent,
            });
        }
    }
    Ok((flows, events))
}

/// 帳簿から取り込む現状：直近12か月の収入・支出と、基準日の金融資産（現金・預金など支払手段の資産）。
#[derive(Clone, Debug, Serialize)]
pub struct Baseline {
    pub from: NaiveDate,
    pub to: NaiveDate,
    pub annual_income: D,
    pub annual_expense: D,
    /// 現金・預金など（支払手段の資産科目）の残高
    pub cash: D,
    /// 資産合計 − 負債合計
    pub net_assets: D,
    pub months_with_data: usize,
}
pub fn baseline(book: &Book, as_of: NaiveDate) -> Result<Baseline> {
    let from = as_of
        .checked_sub_months(Months::new(12))
        .and_then(|d| d.succ_opt())
        .ok_or_else(|| invalid("基準日が範囲外です"))?;
    let stats = reports::statistics(book, from, as_of)?;
    let bs = reports::balance_sheet(book, as_of);
    let tb = reports::trial_balance(book, None, as_of);
    let cash = tb
        .rows
        .iter()
        .filter(|r| {
            book.account(r.account)
                .is_ok_and(|a| a.payment && a.category == Category::Asset)
        })
        .map(|r| r.closing.0)
        .sum();
    let months = stats
        .months
        .iter()
        .filter(|m| !(m.income.0.is_zero() && m.expense.0.is_zero()))
        .count();
    Ok(Baseline {
        from,
        to: as_of,
        annual_income: stats.total_income,
        annual_expense: stats.total_expense,
        cash: D(cash),
        net_assets: D(bs.total_assets.0 - bs.total_liabilities.0),
        months_with_data: months,
    })
}

/// 帳簿の最初の年から使える新しいライフプランの雛形（帳簿の現状と年分の手取りを使う）。
pub fn draft(book: &Book, as_of: NaiveDate, take_home: Option<Decimal>) -> Result<LifePlan> {
    let b = baseline(book, as_of)?;
    let year = as_of.year() + 1;
    let mut flows = vec![];
    let income = take_home.unwrap_or(b.annual_income.0);
    if !income.is_zero() {
        flows.push(PlanFlow {
            name: if take_home.is_some() {
                "手取り収入（税額計算から）".into()
            } else {
                "収入（帳簿の直近12か月）".into()
            },
            kind: FlowKind::Income,
            amount: book.round(income),
            start_year: year,
            end_year: year + 19,
            growth_percent: Decimal::ZERO,
        });
    }
    if !b.annual_expense.0.is_zero() {
        flows.push(PlanFlow {
            name: "生活費（帳簿の直近12か月）".into(),
            kind: FlowKind::Expense,
            amount: book.round(b.annual_expense.0),
            start_year: year,
            end_year: year + 29,
            growth_percent: Decimal::ONE,
        });
    }
    Ok(LifePlan {
        id: 0,
        name: "現状".into(),
        memo: format!("{}〜{} の帳簿から作成", b.from, b.to),
        start_year: year,
        years: 30,
        savings: b.cash.0,
        return_percent: Decimal::ZERO,
        members: vec![],
        flows,
        events: vec![],
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn d(s: &str) -> Decimal {
        num::parse(s).unwrap()
    }
    fn book() -> Book {
        let mut b: Book = serde_json::from_value(serde_json::json!({
            "id":1,"name":"家計","kind":"household","currency":"JPY","minor_digits":0,
            "rounding":"half_up","fiscal_start_month":1,"revision":1
        }))
        .unwrap();
        b.accounts = crate::chart::accounts(crate::model::BookKind::Household);
        b
    }

    #[test]
    fn cash_flow_grows_amounts_and_finds_shortfalls() {
        let plan = LifePlan {
            id: 1,
            name: "現状".into(),
            memo: String::new(),
            start_year: 2027,
            years: 3,
            savings: d("1000000"),
            return_percent: d("1"),
            members: vec![PlanMember {
                name: "長男".into(),
                relation: "子".into(),
                birth_year: 2010,
            }],
            flows: vec![
                PlanFlow {
                    name: "手取り".into(),
                    kind: FlowKind::Income,
                    amount: d("5000000"),
                    start_year: 2027,
                    end_year: 2029,
                    growth_percent: d("0"),
                },
                PlanFlow {
                    name: "生活費".into(),
                    kind: FlowKind::Expense,
                    amount: d("4000000"),
                    start_year: 2027,
                    end_year: 2029,
                    growth_percent: d("2"),
                },
            ],
            events: vec![PlanEvent {
                year: 2029,
                name: "車の買い替え".into(),
                kind: FlowKind::Expense,
                amount: d("4000000"),
                growth_percent: d("0"),
            }],
        };
        let c = cash_flow(&book(), &plan).unwrap();
        // 2027: 運用益 10,000、収支 1,000,000 → 2,010,000
        assert_eq!(c.rows[0].savings.0, d("2010000"));
        assert_eq!(c.rows[0].ages, vec![Some(17)]);
        // 2028: 生活費 4,080,000、運用益 20,100 → 2,950,100
        assert_eq!(c.rows[1].values[1].0, d("4080000"));
        assert_eq!(c.rows[1].savings.0, d("2950100"));
        // 2029: 生活費 4,161,600 + 車 4,000,000、運用益 29,501 → −181,999
        assert_eq!(c.rows[2].savings.0, d("-181999"));
        assert_eq!(c.shortfall_years, vec![2029]);
        assert_eq!(c.min_year, 2029);
        let mut better = plan.clone();
        better.events.clear();
        let text = csv(&book(), &c);
        assert!(text.starts_with("\u{feff}年,長男（歳）,手取り（収入）,生活費（支出）,イベント"));
        assert!(text.contains(
            "2029,19,5000000,4161600,車の買い替え -4000000,5000000,8161600,-3161600,29501,-181999\r\n"
        ));
        let cmp = compare(&book(), &plan, &better).unwrap();
        assert_eq!(cmp.final_difference.0, d("4000000"));
    }

    #[test]
    fn coefficients_loans_and_education() {
        let c = coefficients(d("2"), 10).unwrap();
        assert_eq!(c.future_value.0, d("1.218994"));
        assert_eq!(c.present_value.0, d("0.820348"));
        assert_eq!(c.annuity_future.0, d("10.949721"));
        assert_eq!(c.capital_recovery.0, d("0.111327"));
        assert_eq!(c.annuity_present.0, d("8.982585"));
        let zero = coefficients(d("0"), 4).unwrap();
        assert_eq!(zero.sinking_fund.0, d("0.25"));
        let l = loan_payment(&book(), d("30000000"), d("1"), 35).unwrap();
        assert_eq!(l.monthly.0, d("84686"));
        let (flows, events) = education(&EducationRequest {
            child: "長男".into(),
            birth_year: 2010,
            schools: [
                School::None,
                School::Public,
                School::Public,
                School::Private,
                School::Private,
            ],
            growth_percent: d("0"),
        })
        .unwrap();
        assert_eq!(flows.len(), 4);
        assert_eq!((flows[2].start_year, flows[2].end_year), (2026, 2028));
        assert_eq!(events[0].year, 2029);
    }
}
