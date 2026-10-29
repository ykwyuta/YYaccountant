//! Book model shared by the SQLite store, MCP and Web UI.
use crate::num::{self, Decimal, Rounding};
use crate::{Result, invalid};
use chrono::{Datelike, NaiveDate};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BookKind {
    /// 家計
    #[default]
    Household,
    /// 個人事業主
    SoleProprietor,
}
impl BookKind {
    pub const ALL: [Self; 2] = [Self::Household, Self::SoleProprietor];
    pub fn label(self) -> &'static str {
        match self {
            Self::Household => "家計",
            Self::SoleProprietor => "個人事業主",
        }
    }
}

#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum Category {
    Asset,
    Liability,
    Equity,
    Income,
    #[default]
    Expense,
}
impl Category {
    pub const ALL: [Self; 5] = [
        Self::Asset,
        Self::Liability,
        Self::Equity,
        Self::Income,
        Self::Expense,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Asset => "資産",
            Self::Liability => "負債",
            Self::Equity => "純資産",
            Self::Income => "収益",
            Self::Expense => "費用",
        }
    }
    /// True when the normal balance is on the debit side.
    pub fn debit_normal(self) -> bool {
        matches!(self, Self::Asset | Self::Expense)
    }
}

/// Fixed meaning used by automatic postings. At most one account per role.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Cash,
    Bank,
    Card,
    Receivable,
    Payable,
    Loan,
    Lending,
    InterestIncome,
    InterestExpense,
    OwnerDraw,
    OwnerContribution,
    OpeningEquity,
    Depreciation,
    AccumulatedDepreciation,
    Materials,
    Sales,
}
impl Role {
    pub const ALL: [Self; 16] = [
        Self::Cash,
        Self::Bank,
        Self::Card,
        Self::Receivable,
        Self::Payable,
        Self::Loan,
        Self::Lending,
        Self::InterestIncome,
        Self::InterestExpense,
        Self::OwnerDraw,
        Self::OwnerContribution,
        Self::OpeningEquity,
        Self::Depreciation,
        Self::AccumulatedDepreciation,
        Self::Materials,
        Self::Sales,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Cash => "現金",
            Self::Bank => "預金",
            Self::Card => "カード未払",
            Self::Receivable => "売掛・未収",
            Self::Payable => "買掛・未払",
            Self::Loan => "借入",
            Self::Lending => "貸付",
            Self::InterestIncome => "受取利息",
            Self::InterestExpense => "支払利息",
            Self::OwnerDraw => "事業主貸",
            Self::OwnerContribution => "事業主借",
            Self::OpeningEquity => "元入・開始残高",
            Self::Depreciation => "減価償却費",
            Self::AccumulatedDepreciation => "減価償却累計額",
            Self::Materials => "原材料",
            Self::Sales => "売上",
        }
    }
    pub fn category(self) -> Category {
        match self {
            Self::Cash
            | Self::Bank
            | Self::Receivable
            | Self::Lending
            | Self::Materials
            | Self::OwnerDraw
            | Self::AccumulatedDepreciation => Category::Asset,
            Self::Card | Self::Payable | Self::Loan => Category::Liability,
            Self::OwnerContribution | Self::OpeningEquity => Category::Equity,
            Self::InterestIncome | Self::Sales => Category::Income,
            Self::InterestExpense | Self::Depreciation => Category::Expense,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Account {
    #[serde(default)]
    pub id: i64,
    pub code: String,
    pub name: String,
    pub category: Category,
    #[serde(default)]
    pub role: Option<Role>,
    #[serde(default)]
    pub parent: Option<i64>,
    /// Usable as the money side of quick income/expense input (cash, bank, card, e-money).
    #[serde(default)]
    pub payment: bool,
    /// 使い分けの説明
    #[serde(default)]
    pub guide: String,
    /// Search words for the account finder.
    #[serde(default)]
    pub keywords: Vec<String>,
    #[serde(default)]
    pub archived: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Partner {
    #[serde(default)]
    pub id: i64,
    pub name: String,
    #[serde(default)]
    pub note: String,
    #[serde(default)]
    pub archived: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Line {
    pub account: i64,
    #[serde(default, with = "num::dec")]
    pub debit: Decimal,
    #[serde(default, with = "num::dec")]
    pub credit: Decimal,
    #[serde(default)]
    pub memo: String,
    /// Claim (receivable/payable) this line originates or settles.
    #[serde(default)]
    pub claim: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    #[serde(default)]
    pub id: i64,
    pub date: NaiveDate,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub partner: Option<i64>,
    /// Automatic posting key such as `plan:3:2026-11-25` or `depreciation:2:2026`; prevents double posting.
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub void: bool,
    pub lines: Vec<Line>,
}
impl Entry {
    pub fn plan(&self) -> Option<i64> {
        self.source
            .strip_prefix("plan:")
            .and_then(|s| s.split(':').next())
            .and_then(|s| s.parse().ok())
    }
    pub fn total(&self) -> Decimal {
        self.lines.iter().map(|l| l.debit).sum()
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Side {
    /// 債権（受け取る権利）
    #[default]
    Receivable,
    /// 債務（支払う義務）
    Payable,
}
impl Side {
    pub fn label(self) -> &'static str {
        match self {
            Self::Receivable => "債権",
            Self::Payable => "債務",
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Repayment {
    /// 一括
    #[default]
    Lump,
    /// 元利均等
    EqualPayment,
    /// 元金均等
    EqualPrincipal,
}
impl Repayment {
    pub const ALL: [Self; 3] = [Self::Lump, Self::EqualPayment, Self::EqualPrincipal];
    pub fn label(self) -> &'static str {
        match self {
            Self::Lump => "一括",
            Self::EqualPayment => "元利均等",
            Self::EqualPrincipal => "元金均等",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Claim {
    #[serde(default)]
    pub id: i64,
    pub side: Side,
    pub title: String,
    #[serde(default)]
    pub partner: Option<i64>,
    /// 売掛金・未収入金・貸付金（債権）または買掛金・未払金・借入金（債務）
    pub account: i64,
    #[serde(with = "num::dec")]
    pub principal: Decimal,
    pub issued: NaiveDate,
    /// Lump sum due date, or the first installment date.
    #[serde(default)]
    pub due: Option<NaiveDate>,
    /// Annual percentage, e.g. "1.5".
    #[serde(default, with = "num::dec")]
    pub annual_rate: Decimal,
    #[serde(default)]
    pub repayment: Repayment,
    /// Monthly installments; ignored for lump sums.
    #[serde(default)]
    pub installments: u32,
    #[serde(default)]
    pub closed: bool,
    #[serde(default)]
    pub memo: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BudgetLine {
    pub account: i64,
    /// Twelve monthly amounts from the fiscal year's first month.
    #[serde(with = "num::list")]
    pub amounts: Vec<Decimal>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Budget {
    #[serde(default)]
    pub id: i64,
    pub name: String,
    /// Calendar year in which the fiscal year starts.
    pub fiscal_year: i32,
    #[serde(default)]
    pub memo: String,
    #[serde(default)]
    pub lines: Vec<BudgetLine>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Frequency {
    /// 1回のみ
    Once,
    Weekly,
    #[default]
    Monthly,
    Yearly,
}
impl Frequency {
    pub const ALL: [Self; 4] = [Self::Once, Self::Weekly, Self::Monthly, Self::Yearly];
    pub fn label(self) -> &'static str {
        match self {
            Self::Once => "1回",
            Self::Weekly => "毎週",
            Self::Monthly => "毎月",
            Self::Yearly => "毎年",
        }
    }
}
/// 計画：定期的または予定された取引。計上するまで帳簿には影響しない。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Plan {
    #[serde(default)]
    pub id: i64,
    pub name: String,
    pub debit: i64,
    pub credit: i64,
    #[serde(with = "num::dec")]
    pub amount: Decimal,
    pub start: NaiveDate,
    #[serde(default)]
    pub end: Option<NaiveDate>,
    #[serde(default)]
    pub frequency: Frequency,
    #[serde(default = "one")]
    pub interval: u32,
    #[serde(default)]
    pub partner: Option<i64>,
    #[serde(default = "yes")]
    pub active: bool,
    #[serde(default)]
    pub memo: String,
}
fn one() -> u32 {
    1
}
fn yes() -> bool {
    true
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Material {
    #[serde(default)]
    pub id: i64,
    pub name: String,
    pub unit: String,
    /// 標準単価
    #[serde(default, with = "num::dec")]
    pub standard_cost: Decimal,
    #[serde(default)]
    pub archived: bool,
    #[serde(default)]
    pub memo: String,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MoveKind {
    /// 仕入（受入）
    #[default]
    Purchase,
    /// 棚卸調整（増減）
    Adjust,
}
/// Material receipts and stocktake adjustments. Consumption comes from productions.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MaterialMove {
    #[serde(default)]
    pub id: i64,
    pub material: i64,
    pub date: NaiveDate,
    #[serde(default)]
    pub kind: MoveKind,
    #[serde(with = "num::dec")]
    pub quantity: Decimal,
    /// Purchase unit price; optional for positive adjustments (moving average is used when absent).
    #[serde(default, with = "num::opt")]
    pub unit_cost: Option<Decimal>,
    #[serde(default)]
    pub memo: String,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Equipment {
    #[serde(default)]
    pub id: i64,
    pub name: String,
    pub acquired: NaiveDate,
    #[serde(with = "num::dec")]
    pub cost: Decimal,
    #[serde(default, with = "num::dec")]
    pub salvage: Decimal,
    pub life_years: u32,
    /// 年間想定稼働時間
    #[serde(with = "num::dec")]
    pub annual_hours: Decimal,
    /// 1時間あたりの電力・保守などの運転費
    #[serde(default, with = "num::dec")]
    pub running_cost_per_hour: Decimal,
    #[serde(default)]
    pub archived: bool,
    #[serde(default)]
    pub memo: String,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProductMaterial {
    pub material: i64,
    #[serde(with = "num::dec")]
    pub quantity: Decimal,
    /// 歩留まりロス率（%）
    #[serde(default, with = "num::dec")]
    pub loss_percent: Decimal,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EquipmentUse {
    pub equipment: i64,
    #[serde(with = "num::dec")]
    pub hours: Decimal,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Product {
    #[serde(default)]
    pub id: i64,
    pub name: String,
    pub unit: String,
    #[serde(default, with = "num::dec")]
    pub labor_hours: Decimal,
    #[serde(default, with = "num::dec")]
    pub labor_rate: Decimal,
    /// 製造間接費配賦率（直接費合計に対する%）
    #[serde(default, with = "num::dec")]
    pub overhead_percent: Decimal,
    #[serde(default)]
    pub archived: bool,
    #[serde(default)]
    pub memo: String,
    #[serde(default)]
    pub materials: Vec<ProductMaterial>,
    #[serde(default)]
    pub equipment: Vec<EquipmentUse>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MaterialUse {
    pub material: i64,
    #[serde(with = "num::dec")]
    pub quantity: Decimal,
}
/// 製造実績。材料消費は移動平均単価で払い出す。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Production {
    #[serde(default)]
    pub id: i64,
    pub product: i64,
    pub date: NaiveDate,
    #[serde(with = "num::dec")]
    pub quantity: Decimal,
    #[serde(default, with = "num::dec")]
    pub labor_hours: Decimal,
    #[serde(default)]
    pub memo: String,
    #[serde(default)]
    pub materials: Vec<MaterialUse>,
    #[serde(default)]
    pub equipment: Vec<EquipmentUse>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Book {
    pub id: i64,
    pub name: String,
    pub kind: BookKind,
    pub currency: String,
    /// Fraction digits of the currency (JPY: 0).
    pub minor_digits: u32,
    pub rounding: Rounding,
    /// First month of the fiscal year (1-12).
    pub fiscal_start_month: u32,
    pub revision: i64,
    #[serde(default)]
    pub next_ids: BTreeMap<String, i64>,
    #[serde(default)]
    pub accounts: Vec<Account>,
    #[serde(default)]
    pub partners: Vec<Partner>,
    #[serde(default)]
    pub entries: Vec<Entry>,
    #[serde(default)]
    pub claims: Vec<Claim>,
    #[serde(default)]
    pub budgets: Vec<Budget>,
    #[serde(default)]
    pub plans: Vec<Plan>,
    #[serde(default)]
    pub materials: Vec<Material>,
    #[serde(default)]
    pub material_moves: Vec<MaterialMove>,
    #[serde(default)]
    pub equipment: Vec<Equipment>,
    #[serde(default)]
    pub products: Vec<Product>,
    #[serde(default)]
    pub productions: Vec<Production>,
    /// 年分ごとの所得税・住民税の試算の入力
    #[serde(default)]
    pub tax_returns: Vec<crate::tax::TaxReturn>,
    /// ライフプラン（キャッシュフロー表）
    #[serde(default)]
    pub life_plans: Vec<crate::lifeplan::LifePlan>,
}

pub(crate) fn text(value: &str, what: &str, max: usize, required: bool) -> Result<()> {
    if (required && value.trim().is_empty())
        || value.len() > max
        || value
            .chars()
            .any(|c| c.is_control() && c != '\n' && c != '\t')
    {
        return Err(invalid(format!(
            "{what}は{}{max}バイト以内の文字列にしてください",
            if required { "1〜" } else { "" }
        )));
    }
    Ok(())
}
pub(crate) fn date_ok(d: NaiveDate) -> Result<()> {
    if !(1900..=2999).contains(&d.year()) {
        return Err(invalid("日付は1900〜2999年にしてください"));
    }
    Ok(())
}
fn bounded(d: Decimal, what: &str) -> Result<()> {
    if d.abs() >= num::limit() || d.scale() > 10 {
        return Err(invalid(format!(
            "{what}は絶対値10^15未満・小数10桁以内にしてください"
        )));
    }
    Ok(())
}
fn nonneg(d: Decimal, what: &str) -> Result<()> {
    bounded(d, what)?;
    if d.is_sign_negative() && !d.is_zero() {
        return Err(invalid(format!("{what}は0以上にしてください")));
    }
    Ok(())
}
fn unique_ids<'a>(ids: impl Iterator<Item = &'a i64>, what: &str) -> Result<BTreeSet<i64>> {
    let mut set = BTreeSet::new();
    for id in ids {
        if *id <= 0 || !set.insert(*id) {
            return Err(invalid(format!("{what}のIDが不正または重複しています")));
        }
    }
    Ok(set)
}

impl Book {
    pub fn account(&self, id: i64) -> Result<&Account> {
        self.accounts
            .iter()
            .find(|a| a.id == id)
            .ok_or_else(|| invalid(format!("勘定科目 {id} がありません")))
    }
    pub fn role(&self, role: Role) -> Result<&Account> {
        self.accounts
            .iter()
            .find(|a| a.role == Some(role) && !a.archived)
            .ok_or_else(|| {
                invalid(format!(
                    "役割「{}」の勘定科目がありません。勘定科目の役割を設定してください",
                    role.label()
                ))
            })
    }
    pub fn account_map(&self) -> HashMap<i64, &Account> {
        self.accounts.iter().map(|a| (a.id, a)).collect()
    }
    pub fn round(&self, d: Decimal) -> Decimal {
        self.rounding.round(d, self.minor_digits)
    }
    pub(crate) fn money(&self, d: Decimal, what: &str) -> Result<()> {
        bounded(d, what)?;
        if d.normalize().scale() > self.minor_digits {
            return Err(invalid(format!(
                "{what}は小数{}桁以内で入力してください",
                self.minor_digits
            )));
        }
        Ok(())
    }
    pub fn validate(&self) -> Result<()> {
        text(&self.name, "帳簿名", 200, true)?;
        if self.currency.len() != 3 || !self.currency.bytes().all(|b| b.is_ascii_uppercase()) {
            return Err(invalid("通貨はISO 4217の3文字（例: JPY）にしてください"));
        }
        if self.minor_digits > 4 || !(1..=12).contains(&self.fiscal_start_month) {
            return Err(invalid("小数桁は0〜4、期首月は1〜12にしてください"));
        }
        unique_ids(self.accounts.iter().map(|a| &a.id), "勘定科目")?;
        let mut codes = BTreeSet::new();
        let mut roles = BTreeSet::new();
        for a in &self.accounts {
            text(&a.code, "科目コード", 32, true)?;
            text(&a.name, "科目名", 100, true)?;
            text(&a.guide, "使い分けの説明", 8000, false)?;
            if a.keywords.len() > 100
                || a.keywords
                    .iter()
                    .any(|k| text(k, "検索語", 100, true).is_err())
            {
                return Err(invalid("検索語は100件以内・各100バイト以内にしてください"));
            }
            if !codes.insert(a.code.as_str()) {
                return Err(invalid(format!("科目コード {} が重複しています", a.code)));
            }
            if let Some(role) = a.role {
                if role.category() != a.category {
                    return Err(invalid(format!(
                        "{}：役割「{}」は{}の科目にだけ設定できます",
                        a.name,
                        role.label(),
                        role.category().label()
                    )));
                }
                if !a.archived && !roles.insert(role) {
                    return Err(invalid(format!(
                        "役割「{}」は1科目だけに設定してください",
                        role.label()
                    )));
                }
            }
            if a.payment && !matches!(a.category, Category::Asset | Category::Liability) {
                return Err(invalid("支払手段は資産または負債の科目だけに設定できます"));
            }
            if let Some(p) = a.parent {
                let parent = self.account(p)?;
                if parent.category != a.category {
                    return Err(invalid("親科目は同じ区分にしてください"));
                }
                let mut seen = BTreeSet::from([a.id]);
                let mut cur = Some(p);
                while let Some(c) = cur {
                    if !seen.insert(c) {
                        return Err(invalid("親科目が循環しています"));
                    }
                    cur = self.account(c)?.parent;
                }
            }
        }
        let partners = unique_ids(self.partners.iter().map(|p| &p.id), "取引先")?;
        for p in &self.partners {
            text(&p.name, "取引先名", 200, true)?;
            text(&p.note, "取引先メモ", 4000, false)?;
        }
        let partner_ok = |p: &Option<i64>| p.is_none_or(|p| partners.contains(&p));
        unique_ids(self.claims.iter().map(|c| &c.id), "債権債務")?;
        for c in &self.claims {
            text(&c.title, "債権債務の件名", 200, true)?;
            text(&c.memo, "メモ", 4000, false)?;
            let a = self.account(c.account)?;
            let expected = match c.side {
                Side::Receivable => Category::Asset,
                Side::Payable => Category::Liability,
            };
            if a.category != expected {
                return Err(invalid(format!(
                    "{}の科目は{}にしてください",
                    c.side.label(),
                    expected.label()
                )));
            }
            if !partner_ok(&c.partner) {
                return Err(invalid("取引先がありません"));
            }
            self.money(c.principal, "元本")?;
            if c.principal <= Decimal::ZERO {
                return Err(invalid("元本は正の金額にしてください"));
            }
            nonneg(c.annual_rate, "年利")?;
            if c.annual_rate > Decimal::ONE_HUNDRED {
                return Err(invalid("年利は100%以下にしてください"));
            }
            date_ok(c.issued)?;
            if c.due.is_some_and(|d| d < c.issued) {
                return Err(invalid("期日は発生日以降にしてください"));
            }
            if c.repayment != Repayment::Lump && !(1..=600).contains(&c.installments) {
                return Err(invalid("分割回数は1〜600回にしてください"));
            }
        }
        unique_ids(self.entries.iter().map(|e| &e.id), "仕訳")?;
        let mut sources = BTreeSet::new();
        for e in &self.entries {
            date_ok(e.date)?;
            text(&e.description, "摘要", 1000, false)?;
            text(&e.source, "自動計上キー", 200, false)?;
            if !partner_ok(&e.partner) {
                return Err(invalid("取引先がありません"));
            }
            if !e.void && !e.source.is_empty() && !sources.insert(e.source.as_str()) {
                return Err(invalid(format!("{} は計上済みです", e.source)));
            }
            if e.lines.len() < 2 || e.lines.len() > 200 {
                return Err(invalid(format!(
                    "仕訳{}：明細は2〜200行にしてください",
                    e.id
                )));
            }
            let mut debit = Decimal::ZERO;
            let mut credit = Decimal::ZERO;
            for l in &e.lines {
                self.account(l.account)?;
                text(&l.memo, "明細メモ", 1000, false)?;
                self.money(l.debit, "借方金額")?;
                self.money(l.credit, "貸方金額")?;
                if l.debit.is_sign_negative() || l.credit.is_sign_negative() {
                    return Err(invalid("金額は0以上にし、減少は反対側に記入してください"));
                }
                if l.debit.is_zero() == l.credit.is_zero() {
                    return Err(invalid(format!(
                        "仕訳{}：各明細は借方か貸方のどちらか一方に金額を入れてください",
                        e.id
                    )));
                }
                if let Some(c) = l.claim {
                    let claim = self
                        .claims
                        .iter()
                        .find(|x| x.id == c)
                        .ok_or_else(|| invalid("債権債務がありません"))?;
                    if claim.account != l.account {
                        return Err(invalid(format!(
                            "債権債務「{}」に紐づく明細は科目を一致させてください",
                            claim.title
                        )));
                    }
                }
                debit += l.debit;
                credit += l.credit;
            }
            if debit != credit {
                return Err(invalid(format!(
                    "仕訳{}：借方合計 {} と貸方合計 {} が一致しません",
                    e.id,
                    num::text(debit),
                    num::text(credit)
                )));
            }
        }
        unique_ids(self.budgets.iter().map(|b| &b.id), "予算")?;
        for b in &self.budgets {
            text(&b.name, "予算名", 200, true)?;
            text(&b.memo, "メモ", 4000, false)?;
            if !(1900..=2999).contains(&b.fiscal_year) {
                return Err(invalid("年度は1900〜2999にしてください"));
            }
            let mut seen = BTreeSet::new();
            for l in &b.lines {
                let a = self.account(l.account)?;
                if !matches!(a.category, Category::Income | Category::Expense) {
                    return Err(invalid("予算は収益または費用の科目に設定してください"));
                }
                if !seen.insert(l.account) {
                    return Err(invalid("予算の科目が重複しています"));
                }
                if l.amounts.len() != 12 {
                    return Err(invalid("予算は12か月分の金額が必要です"));
                }
                for m in &l.amounts {
                    self.money(*m, "予算額")?;
                }
            }
        }
        unique_ids(self.plans.iter().map(|p| &p.id), "計画")?;
        for p in &self.plans {
            text(&p.name, "計画名", 200, true)?;
            text(&p.memo, "メモ", 4000, false)?;
            self.account(p.debit)?;
            self.account(p.credit)?;
            if p.debit == p.credit {
                return Err(invalid("計画の借方と貸方は別の科目にしてください"));
            }
            self.money(p.amount, "計画金額")?;
            if p.amount <= Decimal::ZERO {
                return Err(invalid("計画金額は正の金額にしてください"));
            }
            date_ok(p.start)?;
            if p.end.is_some_and(|e| e < p.start) {
                return Err(invalid("終了日は開始日以降にしてください"));
            }
            if !(1..=120).contains(&p.interval) {
                return Err(invalid("間隔は1〜120にしてください"));
            }
            if !partner_ok(&p.partner) {
                return Err(invalid("取引先がありません"));
            }
        }
        let materials = unique_ids(self.materials.iter().map(|m| &m.id), "原材料")?;
        for m in &self.materials {
            text(&m.name, "原材料名", 200, true)?;
            text(&m.unit, "単位", 32, true)?;
            text(&m.memo, "メモ", 4000, false)?;
            nonneg(m.standard_cost, "標準単価")?;
        }
        unique_ids(self.material_moves.iter().map(|m| &m.id), "入出庫")?;
        for m in &self.material_moves {
            if !materials.contains(&m.material) {
                return Err(invalid("原材料がありません"));
            }
            date_ok(m.date)?;
            text(&m.memo, "メモ", 1000, false)?;
            bounded(m.quantity, "数量")?;
            if m.quantity.is_zero() {
                return Err(invalid("数量を入力してください"));
            }
            match m.kind {
                MoveKind::Purchase => {
                    if m.quantity.is_sign_negative() {
                        return Err(invalid("仕入数量は正の値にしてください（減少は棚卸調整）"));
                    }
                    let cost = m
                        .unit_cost
                        .ok_or_else(|| invalid("仕入単価を入力してください"))?;
                    nonneg(cost, "仕入単価")?;
                }
                MoveKind::Adjust => {
                    if let Some(c) = m.unit_cost {
                        nonneg(c, "単価")?;
                    }
                }
            }
        }
        let equipment = unique_ids(self.equipment.iter().map(|e| &e.id), "設備")?;
        for e in &self.equipment {
            text(&e.name, "設備名", 200, true)?;
            text(&e.memo, "メモ", 4000, false)?;
            date_ok(e.acquired)?;
            self.money(e.cost, "取得価額")?;
            self.money(e.salvage, "残存価額")?;
            nonneg(e.salvage, "残存価額")?;
            if e.cost <= Decimal::ZERO || e.salvage > e.cost {
                return Err(invalid(
                    "取得価額は正、残存価額は取得価額以下にしてください",
                ));
            }
            if !(1..=100).contains(&e.life_years) {
                return Err(invalid("耐用年数は1〜100年にしてください"));
            }
            nonneg(e.annual_hours, "年間稼働時間")?;
            if e.annual_hours.is_zero() {
                return Err(invalid("年間稼働時間は正の値にしてください"));
            }
            nonneg(e.running_cost_per_hour, "運転費")?;
        }
        let uses = |list: &[EquipmentUse]| -> Result<()> {
            let mut seen = BTreeSet::new();
            for u in list {
                if !equipment.contains(&u.equipment) || !seen.insert(u.equipment) {
                    return Err(invalid("設備がないか重複しています"));
                }
                nonneg(u.hours, "稼働時間")?;
            }
            Ok(())
        };
        let products = unique_ids(self.products.iter().map(|p| &p.id), "製品")?;
        for p in &self.products {
            text(&p.name, "製品名", 200, true)?;
            text(&p.unit, "単位", 32, true)?;
            text(&p.memo, "メモ", 4000, false)?;
            nonneg(p.labor_hours, "標準作業時間")?;
            nonneg(p.labor_rate, "賃率")?;
            nonneg(p.overhead_percent, "間接費配賦率")?;
            let mut seen = BTreeSet::new();
            for m in &p.materials {
                if !materials.contains(&m.material) || !seen.insert(m.material) {
                    return Err(invalid("原材料がないか重複しています"));
                }
                nonneg(m.quantity, "使用量")?;
                nonneg(m.loss_percent, "ロス率")?;
                if m.loss_percent >= Decimal::ONE_HUNDRED {
                    return Err(invalid("ロス率は100%未満にしてください"));
                }
            }
            uses(&p.equipment)?;
        }
        unique_ids(self.productions.iter().map(|p| &p.id), "製造実績")?;
        for p in &self.productions {
            if !products.contains(&p.product) {
                return Err(invalid("製品がありません"));
            }
            date_ok(p.date)?;
            text(&p.memo, "メモ", 1000, false)?;
            nonneg(p.quantity, "製造数量")?;
            if p.quantity.is_zero() {
                return Err(invalid("製造数量は正の値にしてください"));
            }
            nonneg(p.labor_hours, "作業時間")?;
            let mut seen = BTreeSet::new();
            for m in &p.materials {
                if !materials.contains(&m.material) || !seen.insert(m.material) {
                    return Err(invalid("原材料がないか重複しています"));
                }
                nonneg(m.quantity, "消費量")?;
            }
            uses(&p.equipment)?;
        }
        unique_ids(self.tax_returns.iter().map(|t| &t.id), "税額計算")?;
        for t in &self.tax_returns {
            crate::tax::validate(self, t)?;
        }
        unique_ids(self.life_plans.iter().map(|p| &p.id), "ライフプラン")?;
        for p in &self.life_plans {
            crate::lifeplan::validate(self, p)?;
        }
        Ok(())
    }
}
