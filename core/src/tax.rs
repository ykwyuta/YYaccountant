//! 個人の所得税・住民税の試算：会社員の給与と副業、配偶者と扶養親族（子・同居の老親）、
//! 社会保険料・iDeCo、生命保険料・地震保険料、医療費、ふるさと納税、住宅ローン控除。
//!
//! 金額基準は年分ごとの表（[`Rules`]）で持ち、表のない年分は計算しない。税制は毎年のように
//! 変わるので、結果は申告前に国税庁・市区町村の資料で確かめる前提の試算として示す。
//! 相続・贈与、譲渡所得、配当・株式、ひとり親・寡婦・勤労学生、外国税額などは扱わない。
use crate::model::{Book, date_ok, text};
use crate::num::{self, D, Decimal};
use crate::{Result, invalid};
use serde::{Deserialize, Serialize};

/// 年分の入力。帳簿に年分ごとに保存する。金額はすべて円。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaxReturn {
    #[serde(default)]
    pub id: i64,
    /// 年分（暦年）
    pub year: i32,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub memo: String,
    /// 給与の支払金額（源泉徴収票の「支払金額」）
    #[serde(default, with = "num::dec")]
    pub salary: Decimal,
    /// 給与の源泉徴収税額（年末調整後）
    #[serde(default, with = "num::dec")]
    pub salary_withheld: Decimal,
    /// 社会保険料（給与天引き分＋自分で払った家族分の国民年金・国民健康保険など）
    #[serde(default, with = "num::dec")]
    pub social_insurance: Decimal,
    #[serde(default)]
    pub side_kind: SideKind,
    /// 副業の収入（売上・報酬、副業が給与なら支払金額）
    #[serde(default, with = "num::dec")]
    pub side_revenue: Decimal,
    /// 副業の必要経費
    #[serde(default, with = "num::dec")]
    pub side_expenses: Decimal,
    /// 副業の報酬から源泉徴収された税額
    #[serde(default, with = "num::dec")]
    pub side_withheld: Decimal,
    /// 青色申告特別控除の額（10万・55万・65万円）
    #[serde(default, with = "num::dec")]
    pub blue_deduction: Decimal,
    #[serde(default)]
    pub self_disability: Disability,
    #[serde(default)]
    pub has_spouse: bool,
    /// 配偶者の合計所得金額（給与だけなら給与所得控除後の金額）
    #[serde(default, with = "num::dec")]
    pub spouse_income: Decimal,
    #[serde(default)]
    pub spouse_birth_year: Option<i32>,
    #[serde(default)]
    pub spouse_disability: Disability,
    #[serde(default)]
    pub dependents: Vec<Dependent>,
    /// 小規模企業共済等掛金（iDeCoなど）
    #[serde(default, with = "num::dec")]
    pub mutual_aid: Decimal,
    /// 一般生命保険料（新制度：2012年以後の契約）
    #[serde(default, with = "num::dec")]
    pub life_general_new: Decimal,
    /// 一般生命保険料（旧制度：2011年以前の契約）
    #[serde(default, with = "num::dec")]
    pub life_general_old: Decimal,
    /// 介護医療保険料
    #[serde(default, with = "num::dec")]
    pub life_care: Decimal,
    /// 個人年金保険料（新制度）
    #[serde(default, with = "num::dec")]
    pub life_pension_new: Decimal,
    /// 個人年金保険料（旧制度）
    #[serde(default, with = "num::dec")]
    pub life_pension_old: Decimal,
    /// 地震保険料
    #[serde(default, with = "num::dec")]
    pub earthquake: Decimal,
    /// 支払った医療費（家族分を含む）
    #[serde(default, with = "num::dec")]
    pub medical_paid: Decimal,
    /// 保険金などで補塡された額
    #[serde(default, with = "num::dec")]
    pub medical_covered: Decimal,
    /// セルフメディケーション税制の対象医薬品の購入額（医療費控除と有利な方を使う）
    #[serde(default, with = "num::dec")]
    pub self_medication: Decimal,
    /// ふるさと納税の寄附額の合計
    #[serde(default, with = "num::dec")]
    pub donation: Decimal,
    /// 住宅ローンの年末残高
    #[serde(default, with = "num::dec")]
    pub housing_balance: Decimal,
    /// 借入限度額（入居年と住宅の種類で決まる。証明書や申告書で確認）
    #[serde(default, with = "num::dec")]
    pub housing_limit: Decimal,
    /// 控除率（%）。2022年以後の入居は0.7
    #[serde(default, with = "num::dec")]
    pub housing_rate: Decimal,
    /// 住宅ローン控除のうち住民税から引ける上限（2022年以後の入居は97,500円）
    #[serde(default, with = "num::dec")]
    pub housing_resident_cap: Decimal,
    /// 住民税の均等割（森林環境税を含む。市区町村で異なる）
    #[serde(default, with = "num::dec")]
    pub per_capita: Decimal,
}
impl TaxReturn {
    /// A new return with the usual defaults (mortgage rate 0.7%, resident cap 97,500, per capita 5,000).
    pub fn new(year: i32) -> Self {
        let z = Decimal::ZERO;
        Self {
            id: 0,
            year,
            name: format!("{year}年分"),
            memo: String::new(),
            salary: z,
            salary_withheld: z,
            social_insurance: z,
            side_kind: SideKind::None,
            side_revenue: z,
            side_expenses: z,
            side_withheld: z,
            blue_deduction: z,
            self_disability: Disability::None,
            has_spouse: false,
            spouse_income: z,
            spouse_birth_year: None,
            spouse_disability: Disability::None,
            dependents: vec![],
            mutual_aid: z,
            life_general_new: z,
            life_general_old: z,
            life_care: z,
            life_pension_new: z,
            life_pension_old: z,
            earthquake: z,
            medical_paid: z,
            medical_covered: z,
            self_medication: z,
            donation: z,
            housing_balance: z,
            housing_limit: z,
            housing_rate: yen(7) / Decimal::TEN,
            housing_resident_cap: yen(97_500),
            per_capita: yen(5_000),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SideKind {
    /// 副業なし
    #[default]
    None,
    /// 雑所得（業務）：会社員の副業の多く
    Miscellaneous,
    /// 事業所得（白色申告）
    Business,
    /// 事業所得（青色申告）
    BlueBusiness,
    /// 2か所目の給与（アルバイトなど）
    Salary,
}
impl SideKind {
    pub const ALL: [Self; 5] = [
        Self::None,
        Self::Miscellaneous,
        Self::Business,
        Self::BlueBusiness,
        Self::Salary,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::None => "副業なし",
            Self::Miscellaneous => "雑所得（業務）",
            Self::Business => "事業所得（白色）",
            Self::BlueBusiness => "事業所得（青色）",
            Self::Salary => "給与（2か所目）",
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Disability {
    #[default]
    None,
    /// 障害者
    Normal,
    /// 特別障害者
    Special,
}
impl Disability {
    pub const ALL: [Self; 3] = [Self::None, Self::Normal, Self::Special];
    pub fn label(self) -> &'static str {
        match self {
            Self::None => "なし",
            Self::Normal => "障害者",
            Self::Special => "特別障害者",
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Relation {
    /// 子・孫
    #[default]
    Child,
    /// 父母・祖父母（本人または配偶者の直系尊属）
    Parent,
    /// その他の親族
    Other,
}
impl Relation {
    pub const ALL: [Self; 3] = [Self::Child, Self::Parent, Self::Other];
    pub fn label(self) -> &'static str {
        match self {
            Self::Child => "子・孫",
            Self::Parent => "父母・祖父母",
            Self::Other => "その他の親族",
        }
    }
}
/// 生計を一にする親族（配偶者以外）。16歳未満の子も入れる（生命保険料控除の特例などに使う）。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Dependent {
    pub name: String,
    #[serde(default)]
    pub relation: Relation,
    pub birth_year: i32,
    /// 合計所得金額（公的年金だけなら年金収入から公的年金等控除を引いた額）
    #[serde(default, with = "num::dec")]
    pub income: Decimal,
    /// 同居している
    #[serde(default)]
    pub cohabiting: bool,
    #[serde(default)]
    pub disability: Disability,
}

fn yen(n: i64) -> Decimal {
    Decimal::from(n)
}
fn man(n: i64) -> Decimal {
    yen(n * 10_000)
}
fn floor_to(d: Decimal, unit: i64) -> Decimal {
    (d / yen(unit)).floor() * yen(unit)
}
fn ceil(d: Decimal) -> Decimal {
    d.ceil()
}
fn pos(d: Decimal) -> Decimal {
    d.max(Decimal::ZERO)
}
fn percent(d: Decimal, p: i64) -> Decimal {
    d * yen(p) / num::hundred()
}

/// 年分ごとの金額基準。
pub struct Rules {
    pub year: i32,
    /// 給与所得控除の最低額と、それが適用される給与収入の上限
    employment_min: i64,
    employment_min_until: i64,
    /// 基礎控除（所得税）: (合計所得金額の上限, 控除額)
    basic: &'static [(i64, i64)],
    /// 扶養親族・同一生計配偶者の合計所得金額の上限
    dependent_limit: i64,
    /// 23歳未満の扶養親族がいるときの一般生命保険料控除の上限6万円
    life_child_special: bool,
    pub notes: &'static [&'static str],
}
const BASIC_2025: &[(i64, i64)] = &[
    (1_320_000, 950_000),
    (3_360_000, 880_000),
    (4_890_000, 680_000),
    (6_550_000, 630_000),
    (23_500_000, 580_000),
    (24_000_000, 480_000),
    (24_500_000, 320_000),
    (25_000_000, 160_000),
];
const BASIC_2026: &[(i64, i64)] = &[
    (4_890_000, 1_040_000),
    (6_550_000, 670_000),
    (23_500_000, 620_000),
    (24_000_000, 480_000),
    (24_500_000, 320_000),
    (25_000_000, 160_000),
];
pub const YEARS: [i32; 2] = [2025, 2026];
pub fn rules(year: i32) -> Result<Rules> {
    match year {
        2025 => Ok(Rules {
            year,
            employment_min: 650_000,
            employment_min_until: 1_900_000,
            basic: BASIC_2025,
            dependent_limit: 580_000,
            life_child_special: false,
            notes: &[
                "令和7年度税制改正（基礎控除の特例、給与所得控除の最低65万円、扶養親族の所得要件58万円、特定親族特別控除）を反映しています。",
            ],
        }),
        2026 => Ok(Rules {
            year,
            employment_min: 740_000,
            employment_min_until: 2_200_000,
            basic: BASIC_2026,
            dependent_limit: 620_000,
            life_child_special: true,
            notes: &[
                "令和8年度税制改正大綱（2025年12月）に基づく令和8年分の金額です（基礎控除は本則62万円＋特例、給与所得控除は最低74万円、扶養親族の所得要件62万円）。成立した法令と国税庁の資料で確かめてください。",
                "23歳未満の扶養親族がいるときの一般生命保険料控除の上限6万円（令和8年分の特例）を反映しています。",
            ],
        }),
        _ => Err(invalid(format!(
            "{year}年分の税制はまだ対応していません（対応: {}年分）",
            YEARS.map(|y| y.to_string()).join("・")
        ))),
    }
}
impl Rules {
    /// 給与所得（給与所得控除後の金額）。660万円未満は法定の表に合わせて4,000円単位で計算する。
    pub fn salary_income(&self, salary: Decimal) -> Decimal {
        if salary <= yen(self.employment_min_until) {
            return pos(salary - yen(self.employment_min));
        }
        if salary < yen(6_600_000) {
            let a = floor_to(salary / yen(4), 1000);
            return if salary <= yen(3_600_000) {
                a * yen(28) / yen(10) - yen(80_000)
            } else {
                a * yen(32) / yen(10) - yen(440_000)
            };
        }
        if salary <= yen(8_500_000) {
            salary - ceil(percent(salary, 10)) - yen(1_100_000)
        } else {
            salary - yen(1_950_000)
        }
    }
    fn basic(&self, total: Decimal) -> Decimal {
        self.basic
            .iter()
            .find(|(limit, _)| total <= yen(*limit))
            .map_or(Decimal::ZERO, |(_, d)| yen(*d))
    }
}

/// 所得税の速算表
fn income_tax(taxable: Decimal) -> Decimal {
    const TABLE: [(i64, i64, i64); 7] = [
        (1_950_000, 5, 0),
        (3_300_000, 10, 97_500),
        (6_950_000, 20, 427_500),
        (9_000_000, 23, 636_000),
        (18_000_000, 33, 1_536_000),
        (40_000_000, 40, 2_796_000),
        (i64::MAX, 45, 4_796_000),
    ];
    let (_, rate, minus) = TABLE
        .iter()
        .find(|(limit, _, _)| taxable <= yen(*limit))
        .unwrap();
    pos(percent(taxable, *rate) - yen(*minus)).floor()
}
/// ふるさと納税の特例分の割合（90% − 所得税率×1.021）。課税所得から人的控除差を引いた額で決まる。
fn special_ratio(base: Decimal) -> Decimal {
    let r = |s: &str| num::parse(s).unwrap();
    if base.is_sign_negative() {
        return r("0.9");
    }
    for (limit, ratio) in [
        (1_950_000, "0.84895"),
        (3_300_000, "0.7979"),
        (6_950_000, "0.6958"),
        (9_000_000, "0.66517"),
        (18_000_000, "0.56307"),
        (40_000_000, "0.4916"),
    ] {
        if base <= yen(limit) {
            return r(ratio);
        }
    }
    r("0.44055")
}

/// 生命保険料控除の1区分。`scale` は所得税の新制度（2万・4万・8万）に対する倍率。
fn life_new(x: Decimal, resident: bool, special: bool) -> Decimal {
    let (a, b, c, cap) = match (resident, special) {
        (false, false) => (20_000, 40_000, 80_000, 40_000),
        (false, true) => (30_000, 60_000, 120_000, 60_000),
        (true, _) => (12_000, 32_000, 56_000, 28_000),
    };
    ceil(if x <= yen(a) {
        x
    } else if x <= yen(b) {
        x / yen(2) + yen(a / 2)
    } else if x <= yen(c) {
        x / yen(4) + yen(c / 4)
    } else {
        yen(cap)
    })
}
fn life_old(x: Decimal, resident: bool) -> Decimal {
    let (a, b, c, cap) = if resident {
        (15_000, 40_000, 70_000, 35_000)
    } else {
        (25_000, 50_000, 100_000, 50_000)
    };
    ceil(if x <= yen(a) {
        x
    } else if x <= yen(b) {
        x / yen(2) + yen(a / 2)
    } else if x <= yen(c) {
        x / yen(4) + yen(cap / 2)
    } else {
        yen(cap)
    })
}
/// 新旧両方あるときは「旧だけ」と「新旧合計（上限あり）」の有利な方。
fn life_category(new: Decimal, old: Decimal, resident: bool, special: bool) -> Decimal {
    let n = life_new(new, resident, special);
    if old.is_zero() {
        return n;
    }
    let o = life_old(old, resident);
    if new.is_zero() {
        return o;
    }
    let cap = match (resident, special) {
        (true, _) => 28_000,
        (false, true) => 60_000,
        (false, false) => 40_000,
    };
    o.max((n + o).min(yen(cap)))
}

#[derive(Clone, Debug, Serialize)]
pub struct TaxLine {
    pub section: &'static str,
    pub label: String,
    pub income_tax: Option<D>,
    pub resident_tax: Option<D>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub note: String,
}
#[derive(Clone, Debug, Serialize)]
pub struct TaxResult {
    pub year: i32,
    /// 項目ごとの計算（所得税・住民税を並べる）
    pub lines: Vec<TaxLine>,
    /// 合計所得金額
    pub total_income: D,
    pub taxable_income: D,
    pub taxable_resident: D,
    /// 所得税及び復興特別所得税
    pub income_tax: D,
    /// 源泉徴収税額の合計
    pub withheld: D,
    /// 確定申告で納める額（正）または還付される額（負）
    pub balance_due: D,
    /// 住民税（翌年6月から納める年税額）
    pub resident_tax: D,
    /// 年間の手取り（可処分所得）の目安：収入 − 経費 − 社会保険料 − 所得税 − 住民税
    pub take_home: D,
    /// ふるさと納税の自己負担が2,000円で済む寄附額の目安
    pub donation_limit: D,
    /// 寄附で減った税額（所得税＋住民税）と実質の自己負担
    pub donation_saving: D,
    pub donation_cost: D,
    pub filing_required: bool,
    pub notes: Vec<String>,
}

pub fn validate(book: &Book, t: &TaxReturn) -> Result<()> {
    if book.currency != "JPY" {
        return Err(invalid("税額計算は通貨がJPYの帳簿で使えます"));
    }
    rules(t.year)?;
    text(&t.name, "名前", 200, false)?;
    text(&t.memo, "メモ", 4000, false)?;
    let amounts = [
        (t.salary, "給与の支払金額"),
        (t.salary_withheld, "源泉徴収税額"),
        (t.social_insurance, "社会保険料"),
        (t.side_revenue, "副業の収入"),
        (t.side_expenses, "副業の経費"),
        (t.side_withheld, "副業の源泉徴収税額"),
        (t.blue_deduction, "青色申告特別控除"),
        (t.spouse_income, "配偶者の所得"),
        (t.mutual_aid, "小規模企業共済等掛金"),
        (t.life_general_new, "一般生命保険料（新）"),
        (t.life_general_old, "一般生命保険料（旧）"),
        (t.life_care, "介護医療保険料"),
        (t.life_pension_new, "個人年金保険料（新）"),
        (t.life_pension_old, "個人年金保険料（旧）"),
        (t.earthquake, "地震保険料"),
        (t.medical_paid, "医療費"),
        (t.medical_covered, "補塡額"),
        (t.self_medication, "セルフメディケーション"),
        (t.donation, "寄附額"),
        (t.housing_balance, "住宅ローンの年末残高"),
        (t.housing_limit, "借入限度額"),
        (t.housing_resident_cap, "住民税からの控除上限"),
        (t.per_capita, "均等割"),
    ];
    for (d, what) in amounts {
        book.money(d, what)?;
        if d.is_sign_negative() {
            return Err(invalid(format!("{what}は0以上にしてください")));
        }
        if !d.fract().is_zero() {
            return Err(invalid(format!("{what}は円単位で入力してください")));
        }
    }
    if t.housing_rate.is_sign_negative() || t.housing_rate > yen(10) || t.housing_rate.scale() > 4 {
        return Err(invalid("住宅ローンの控除率は0〜10%にしてください"));
    }
    if ![0, 100_000, 550_000, 650_000]
        .iter()
        .any(|v| t.blue_deduction == yen(*v))
    {
        return Err(invalid(
            "青色申告特別控除は0・100000・550000・650000のいずれかにしてください",
        ));
    }
    let born = |y: i32| -> Result<()> {
        if !(1900..=t.year).contains(&y) {
            return Err(invalid("生まれ年は1900年から年分までにしてください"));
        }
        Ok(())
    };
    if let Some(y) = t.spouse_birth_year {
        born(y)?;
    }
    if t.dependents.len() > 30 {
        return Err(invalid("親族は30人以内にしてください"));
    }
    for d in &t.dependents {
        text(&d.name, "親族の名前", 100, true)?;
        born(d.birth_year)?;
        book.money(d.income, "親族の所得")?;
        if d.income.is_sign_negative() {
            return Err(invalid("親族の所得は0以上にしてください"));
        }
    }
    date_ok(chrono::NaiveDate::from_ymd_opt(t.year, 12, 31).unwrap())
}

/// 年分の所得税・住民税を計算する。
pub fn calculate(book: &Book, t: &TaxReturn) -> Result<TaxResult> {
    validate(book, t)?;
    let mut result = compute(t)?;
    if !t.donation.is_zero() {
        let mut without = t.clone();
        without.donation = Decimal::ZERO;
        let base = compute(&without)?;
        let saving = (base.income_tax.0 - result.income_tax.0)
            + (base.resident_tax.0 - result.resident_tax.0);
        result.donation_saving = D(saving);
        result.donation_cost = D(t.donation - saving);
        if t.donation <= result.donation_limit.0 && t.donation - saving > yen(2_000) {
            result.notes.push(format!(
                "ふるさと納税の実質負担が {} 円で、2,000円を超えています。{}",
                num::grouped(t.donation - saving),
                if t.housing_balance.is_zero() {
                    "所得税が少ないため、寄附金控除による所得税の軽減が小さくなっています。"
                } else {
                    "住宅ローン控除で所得税が少ないため、所得税分の軽減が受けられず、住民税で引ける住宅ローン控除も寄附で課税所得が減った分だけ小さくなります。"
                }
            ));
        }
        if t.donation > result.donation_limit.0 {
            result.notes.push(format!(
                "ふるさと納税が上限の目安 {} 円を超えています。超えた分は自己負担が増えます。",
                num::grouped(result.donation_limit.0)
            ));
        }
    }
    Ok(result)
}

struct Lines(Vec<TaxLine>);
impl Lines {
    fn push(
        &mut self,
        section: &'static str,
        label: &str,
        it: Option<Decimal>,
        rt: Option<Decimal>,
        note: &str,
    ) {
        self.0.push(TaxLine {
            section,
            label: label.to_string(),
            income_tax: it.map(D),
            resident_tax: rt.map(D),
            note: note.to_string(),
        });
    }
}
/// 所得控除：(名前, 所得税, 住民税, 説明)
type Deductions = Vec<(String, Decimal, Decimal, String)>;

fn compute(t: &TaxReturn) -> Result<TaxResult> {
    let r = rules(t.year)?;
    let z = Decimal::ZERO;
    let mut lines = Lines(vec![]);
    let age = |born: i32| t.year - born;
    let limit = yen(r.dependent_limit);

    // 所得
    let second_salary = if t.side_kind == SideKind::Salary {
        t.side_revenue
    } else {
        z
    };
    let salary = t.salary + second_salary;
    let under23 = t
        .dependents
        .iter()
        .any(|d| age(d.birth_year) < 23 && d.income <= limit);
    let special_family = t.self_disability == Disability::Special
        || (t.has_spouse && t.spouse_income <= limit && t.spouse_disability == Disability::Special)
        || t.dependents
            .iter()
            .any(|d| d.income <= limit && d.disability == Disability::Special);
    let adjustment = if salary > yen(8_500_000) && (under23 || special_family) {
        ceil(percent(salary.min(yen(10_000_000)) - yen(8_500_000), 10))
    } else {
        z
    };
    let salary_income = pos(r.salary_income(salary) - adjustment);
    if !salary.is_zero() {
        let mut note = format!("給与収入 {} 円", num::grouped(salary));
        if !adjustment.is_zero() {
            note += &format!("、所得金額調整控除 {} 円", num::grouped(adjustment));
        }
        lines.push(
            "所得",
            "給与所得（給与所得控除後）",
            Some(salary_income),
            Some(salary_income),
            &note,
        );
    }
    let profit = t.side_revenue - t.side_expenses;
    let side_income = match t.side_kind {
        SideKind::None | SideKind::Salary => z,
        SideKind::Miscellaneous => pos(profit),
        SideKind::Business => profit,
        SideKind::BlueBusiness => profit - t.blue_deduction.min(pos(profit)),
    };
    if !matches!(t.side_kind, SideKind::None | SideKind::Salary) {
        lines.push(
            "所得",
            &format!("副業の所得（{}）", t.side_kind.label()),
            Some(side_income),
            Some(side_income),
            match t.side_kind {
                SideKind::Miscellaneous if profit.is_sign_negative() => {
                    "雑所得の赤字は給与と通算できません"
                }
                SideKind::BlueBusiness => "収入 − 経費 − 青色申告特別控除",
                _ => "収入 − 経費",
            },
        );
    }
    let total = pos(salary_income + side_income);
    lines.push(
        "所得",
        "合計所得金額",
        Some(total),
        Some(total),
        "控除の判定（基礎控除・配偶者控除など）に使う金額",
    );

    // 所得控除
    let mut deductions: Deductions = vec![];
    let mut add = |label: &str, it: Decimal, rt: Decimal, note: &str| {
        if !(it.is_zero() && rt.is_zero()) {
            deductions.push((label.to_string(), it, rt, note.to_string()));
        }
    };
    add(
        "社会保険料控除",
        t.social_insurance,
        t.social_insurance,
        "支払った全額",
    );
    add(
        "小規模企業共済等掛金控除",
        t.mutual_aid,
        t.mutual_aid,
        "iDeCoなどの掛金の全額",
    );
    let special = r.life_child_special && under23;
    let life = |resident: bool| {
        let general = life_category(t.life_general_new, t.life_general_old, resident, special);
        let care = life_new(t.life_care, resident, false);
        let pension = life_category(t.life_pension_new, t.life_pension_old, resident, false);
        (general + care + pension).min(yen(if resident { 70_000 } else { 120_000 }))
    };
    add(
        "生命保険料控除",
        life(false),
        life(true),
        if special {
            "一般・介護医療・個人年金の合計（所得税12万・住民税7万まで。23歳未満の扶養親族がいるので所得税の一般は6万まで）"
        } else {
            "一般・介護医療・個人年金の合計（所得税12万・住民税7万まで）"
        },
    );
    add(
        "地震保険料控除",
        t.earthquake.min(yen(50_000)),
        ceil(t.earthquake / yen(2)).min(yen(25_000)),
        "所得税は5万円、住民税は半額で2.5万円まで",
    );
    let threshold = yen(100_000).min(percent(total, 5).floor());
    let medical = pos(t.medical_paid - t.medical_covered - threshold).min(yen(2_000_000));
    let otc = pos(t.self_medication - yen(12_000)).min(yen(88_000));
    let medical_used = !(medical.is_zero() && otc.is_zero());
    if otc > medical {
        add(
            "セルフメディケーション税制",
            otc,
            otc,
            "対象医薬品の購入額 − 1.2万円（8.8万円まで）。医療費控除より有利なのでこちらを選択",
        );
    } else {
        add(
            "医療費控除",
            medical,
            medical,
            &format!(
                "医療費 − 補塡額 − {} 円（10万円か所得の5%の少ない方）",
                num::grouped(threshold)
            ),
        );
    }
    add(
        "寄附金控除",
        pos(t.donation.min(percent(total, 40).floor()) - yen(2_000)),
        z,
        "ふるさと納税 − 2,000円（住民税は下の税額控除で減ります）",
    );
    let difference = personal(t, &r, total, &mut add);
    let basic_rt = if total <= man(2_400) {
        man(43)
    } else if total <= man(2_450) {
        man(29)
    } else if total <= man(2_500) {
        man(15)
    } else {
        z
    };
    add(
        "基礎控除",
        r.basic(total),
        basic_rt,
        "合計所得金額で決まる（住民税は43万円）",
    );
    let mut it_deductions = z;
    let mut rt_deductions = z;
    for (label, it, rt, note) in &deductions {
        it_deductions += *it;
        rt_deductions += *rt;
        lines.push("所得控除", label, Some(*it), Some(*rt), note);
    }
    lines.push(
        "所得控除",
        "所得控除の合計",
        Some(it_deductions),
        Some(rt_deductions),
        "",
    );
    let taxable = floor_to(pos(total - it_deductions), 1000);
    let taxable_rt = floor_to(pos(total - rt_deductions), 1000);
    lines.push(
        "税額",
        "課税所得金額",
        Some(taxable),
        Some(taxable_rt),
        "1,000円未満切り捨て",
    );

    // 所得税
    let computed = income_tax(taxable);
    lines.push(
        "所得税",
        "算出税額",
        Some(computed),
        None,
        "速算表（5〜45%）",
    );
    let housing = if !t.housing_balance.is_zero() && total <= man(2_000) {
        floor_to(
            t.housing_balance.min(t.housing_limit) * t.housing_rate / num::hundred(),
            100,
        )
    } else {
        z
    };
    let housing_it = housing.min(computed);
    if !housing.is_zero() {
        lines.push(
            "所得税",
            "住宅借入金等特別控除",
            Some(-housing_it),
            None,
            &format!(
                "年末残高と借入限度額の少ない方 × {}%（控除額 {} 円）",
                num::text(t.housing_rate),
                num::grouped(housing)
            ),
        );
    }
    let base_tax = computed - housing_it;
    let reconstruction = (base_tax * yen(21) / yen(1000)).floor();
    lines.push(
        "所得税",
        "復興特別所得税",
        Some(reconstruction),
        None,
        "基準所得税額 × 2.1%",
    );
    let income_total = base_tax + reconstruction;
    lines.push(
        "所得税",
        "所得税及び復興特別所得税",
        Some(income_total),
        None,
        "",
    );
    let withheld = t.salary_withheld + t.side_withheld;
    lines.push(
        "所得税",
        "源泉徴収税額",
        Some(-withheld),
        None,
        "給与（年末調整後）と副業の報酬から天引きされた額",
    );
    let diff = income_total - withheld;
    let balance_due = if diff.is_sign_positive() {
        floor_to(diff, 100)
    } else {
        diff
    };
    lines.push(
        "所得税",
        if balance_due.is_sign_negative() {
            "還付される税額"
        } else {
            "納める税額"
        },
        Some(balance_due.abs()),
        None,
        "納める額は100円未満切り捨て",
    );

    // 住民税
    let rate_part = percent(taxable_rt, 10);
    lines.push(
        "住民税",
        "所得割（税率10%）",
        None,
        Some(rate_part),
        "市町村民税6%＋道府県民税4%",
    );
    let adjust = if total <= man(2_500) {
        ceil(if taxable_rt <= man(200) {
            percent(difference.min(taxable_rt), 5)
        } else {
            percent(
                pos(difference - (taxable_rt - man(200))).max(yen(50_000)),
                5,
            )
        })
        .min(rate_part)
    } else {
        z
    };
    lines.push(
        "住民税",
        "調整控除",
        None,
        Some(-adjust),
        &format!(
            "所得税と住民税の人的控除の差（{} 円）による調整",
            num::grouped(difference)
        ),
    );
    let after_adjust = rate_part - adjust;
    let housing_rt = (housing - housing_it)
        .min(percent(taxable, 5).floor())
        .min(t.housing_resident_cap)
        .min(after_adjust);
    if !housing_rt.is_zero() {
        lines.push(
            "住民税",
            "住宅ローン控除（所得税から引ききれない分）",
            None,
            Some(-housing_rt),
            "所得税の課税所得の5%と上限額の少ない方まで",
        );
    }
    let after_housing = after_adjust - housing_rt;
    let ratio = special_ratio(taxable_rt - difference);
    let donation_basic = percent(
        pos(t.donation.min(percent(total, 30).floor()) - yen(2_000)),
        10,
    );
    let donation_special = (pos(t.donation - yen(2_000)) * ratio).min(percent(after_adjust, 20));
    let donation_credit = (donation_basic + donation_special)
        .floor()
        .min(after_housing);
    if !t.donation.is_zero() {
        lines.push(
            "住民税",
            "寄附金税額控除（ふるさと納税）",
            None,
            Some(-donation_credit),
            &format!(
                "基本分10% ＋ 特例分{}%（特例分は調整控除後の所得割の2割まで）",
                num::text(ratio * num::hundred())
            ),
        );
    }
    let income_part = floor_to(after_housing - donation_credit, 100);
    lines.push(
        "住民税",
        "所得割額",
        None,
        Some(income_part),
        "100円未満切り捨て",
    );
    lines.push(
        "住民税",
        "均等割",
        None,
        Some(t.per_capita),
        "森林環境税を含む。市区町村により異なります",
    );
    let resident = income_part + t.per_capita;
    lines.push("住民税", "住民税の年税額", None, Some(resident), "");

    let donation_limit = (percent(after_adjust, 20) / ratio).floor() + yen(2_000);
    let side_net = match t.side_kind {
        SideKind::None => z,
        _ => t.side_revenue - t.side_expenses,
    };
    let take_home = t.salary + side_net - t.social_insurance - income_total - resident;
    let mut notes: Vec<String> = r.notes.iter().map(|s| s.to_string()).collect();
    let mut filing = false;
    match t.side_kind {
        SideKind::None => {}
        SideKind::Salary => {
            filing = true;
            notes.push("2か所から給与を受けているので、原則として確定申告が必要です（2か所目の給与が20万円以下などの例外があります）。".into());
        }
        _ if side_income > yen(200_000) => {
            filing = true;
            notes.push("副業の所得が20万円を超えるので確定申告が必要です。".into());
        }
        _ => notes.push("副業の所得が20万円以下なので所得税の確定申告は不要な場合があります。ただし住民税の申告は必要で、医療費控除などで確定申告するときは副業の所得も含めます。".into()),
    }
    if salary > man(2_000) {
        filing = true;
        notes.push("給与が2,000万円を超えると年末調整されないので確定申告が必要です。".into());
    }
    if medical_used {
        notes.push("医療費控除・セルフメディケーション税制は年末調整では受けられません。確定申告してください（明細書の添付と領収書の保存が必要です）。".into());
    }
    if !t.donation.is_zero() {
        notes.push(if filing {
            "確定申告するので、ふるさと納税のワンストップ特例は使えません。寄附金控除として申告に含めてください（ここではその前提で計算しています）。".into()
        } else {
            "ふるさと納税は、寄附先が5自治体以内で確定申告をしないならワンストップ特例も使えます。税額の合計はほぼ同じです。".into()
        });
    }
    if !t.housing_balance.is_zero() {
        notes.push("住宅ローン控除は1年目は確定申告、2年目以後は年末調整で受けます。借入限度額・控除率・期間は入居年と住宅の種類で決まるので、証明書や税務署から届く申告書で確かめてください。".into());
        if total > man(2_000) {
            notes
                .push("合計所得金額が2,000万円を超える年は住宅ローン控除を受けられません。".into());
        }
    }
    notes.push("住民税は翌年6月から（給与天引きなら翌年6月〜翌々年5月の12回で）納めます。非課税の判定、市町村・道府県ごとの端数処理、調整控除のうち配偶者特別控除・特定親族特別控除の差は概算です。".into());
    Ok(TaxResult {
        year: t.year,
        lines: lines.0,
        total_income: D(total),
        taxable_income: D(taxable),
        taxable_resident: D(taxable_rt),
        income_tax: D(income_total),
        withheld: D(withheld),
        balance_due: D(balance_due),
        resident_tax: D(resident),
        take_home: D(take_home),
        donation_limit: D(donation_limit),
        donation_saving: D(z),
        donation_cost: D(z),
        filing_required: filing,
        notes,
    })
}

type Found<'a> = Option<(String, Decimal, Decimal, Decimal, &'a str)>;
fn add_disability(
    add: &mut impl FnMut(&str, Decimal, Decimal, &str),
    found: Found,
    difference: &mut Decimal,
) {
    if let Some((label, it, rt, df, kind)) = found {
        *difference += df;
        add(&label, it, rt, kind);
    }
}

/// 障害者・配偶者・扶養・特定親族の控除を加え、人的控除差（調整控除とふるさと納税の
/// 特例分の割合に使う）の合計を返す。基礎控除の差5万円を含む。
fn personal(
    t: &TaxReturn,
    r: &Rules,
    total: Decimal,
    add: &mut impl FnMut(&str, Decimal, Decimal, &str),
) -> Decimal {
    let limit = yen(r.dependent_limit);
    let age = |born: i32| t.year - born;
    let mut difference = man(5);
    // 障害者控除（所得税, 住民税, 人的控除差）
    let disability = |who: &str, d: Disability, cohabiting: bool| {
        let (it, rt, df) = match (d, cohabiting) {
            (Disability::None, _) => return None,
            (Disability::Normal, _) => (27, 26, 1),
            (Disability::Special, false) => (40, 30, 10),
            (Disability::Special, true) => (75, 53, 22),
        };
        let kind = if cohabiting && d == Disability::Special {
            "同居特別障害者"
        } else {
            d.label()
        };
        Some((
            format!("障害者控除（{who}）"),
            man(it),
            man(rt),
            man(df),
            kind,
        ))
    };
    add_disability(
        add,
        disability("本人", t.self_disability, false),
        &mut difference,
    );
    // 本人の合計所得金額の区分（900万・950万・1,000万円以下）
    let tier = [900, 950, 1_000].iter().position(|m| total <= man(*m));
    if t.has_spouse {
        let elderly = t.spouse_birth_year.is_some_and(|y| age(y) >= 70);
        if t.spouse_income <= limit {
            add_disability(
                add,
                disability("配偶者", t.spouse_disability, true),
                &mut difference,
            );
            if let Some(i) = tier {
                let (it, rt, df) = if elderly {
                    ([48, 32, 16][i], [38, 26, 13][i], [10, 6, 3][i])
                } else {
                    ([38, 26, 13][i], [33, 22, 11][i], [5, 4, 2][i])
                };
                difference += man(df);
                add(
                    if elderly {
                        "配偶者控除（老人）"
                    } else {
                        "配偶者控除"
                    },
                    man(it),
                    man(rt),
                    "配偶者の合計所得金額が扶養の範囲内",
                );
            }
        } else if let Some(i) = tier.filter(|_| t.spouse_income <= man(133)) {
            const BOUNDS: [i64; 9] = [95, 100, 105, 110, 115, 120, 125, 130, 133];
            const IT: [[i64; 9]; 3] = [
                [38, 36, 31, 26, 21, 16, 11, 6, 3],
                [26, 24, 21, 18, 14, 11, 8, 4, 2],
                [13, 12, 11, 9, 7, 6, 4, 2, 1],
            ];
            const RT: [[i64; 9]; 3] = [
                [33, 33, 31, 26, 21, 16, 11, 6, 3],
                [22, 22, 21, 18, 14, 11, 8, 4, 2],
                [11, 11, 11, 9, 7, 6, 4, 2, 1],
            ];
            let j = BOUNDS
                .iter()
                .position(|b| t.spouse_income <= man(*b))
                .unwrap_or(8);
            add(
                "配偶者特別控除",
                man(IT[i][j]),
                man(RT[i][j]),
                "配偶者の合計所得金額に応じて段階的に減る",
            );
        }
    }
    for d in &t.dependents {
        let a = age(d.birth_year);
        if d.income <= limit {
            add_disability(
                add,
                disability(&d.name, d.disability, d.cohabiting),
                &mut difference,
            );
            if a < 16 {
                continue;
            }
            let (kind, it, rt, df) = if (19..23).contains(&a) {
                ("特定扶養親族", 63, 45, 18)
            } else if a >= 70 && d.relation == Relation::Parent && d.cohabiting {
                ("同居老親等", 58, 45, 13)
            } else if a >= 70 {
                ("老人扶養親族", 48, 38, 10)
            } else {
                ("一般の扶養親族", 38, 33, 5)
            };
            difference += man(df);
            add(
                &format!("扶養控除（{}）", d.name),
                man(it),
                man(rt),
                &format!("{kind}・{a}歳"),
            );
        } else if (19..23).contains(&a) && d.income <= man(123) {
            const BOUNDS: [i64; 9] = [85, 90, 95, 100, 105, 110, 115, 120, 123];
            const IT: [i64; 9] = [63, 61, 51, 41, 31, 21, 11, 6, 3];
            const RT: [i64; 9] = [45, 45, 45, 41, 31, 21, 11, 6, 3];
            let j = BOUNDS.iter().position(|b| d.income <= man(*b)).unwrap_or(8);
            add(
                &format!("特定親族特別控除（{}）", d.name),
                man(IT[j]),
                man(RT[j]),
                &format!("19〜22歳で所得が扶養の範囲を超える親族・{a}歳"),
            );
        }
    }
    difference
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::BookKind;

    fn book() -> Book {
        Book {
            id: 1,
            name: "家計".into(),
            kind: BookKind::Household,
            currency: "JPY".into(),
            minor_digits: 0,
            rounding: Default::default(),
            fiscal_start_month: 1,
            revision: 1,
            next_ids: Default::default(),
            accounts: vec![],
            partners: vec![],
            entries: vec![],
            claims: vec![],
            budgets: vec![],
            plans: vec![],
            materials: vec![],
            material_moves: vec![],
            equipment: vec![],
            products: vec![],
            productions: vec![],
            tax_returns: vec![],
            life_plans: vec![],
        }
    }
    fn d(s: &str) -> Decimal {
        num::parse(s).unwrap()
    }

    #[test]
    fn salary_income_follows_the_statutory_table() {
        let r = rules(2026).unwrap();
        assert_eq!(r.salary_income(d("1780000")), d("1040000"));
        assert_eq!(r.salary_income(d("2200000")), d("1460000"));
        // 5,000,000 / 4 = 1,250,000 → ×3.2 − 440,000
        assert_eq!(r.salary_income(d("5000000")), d("3560000"));
        assert_eq!(r.salary_income(d("5003999")), d("3560000"));
        assert_eq!(r.salary_income(d("7000000")), d("5200000"));
        assert_eq!(r.salary_income(d("9000000")), d("7050000"));
        let r = rules(2025).unwrap();
        assert_eq!(r.salary_income(d("1900000")), d("1250000"));
        // 178万円までは所得税がかからない（令和8年分）
        let mut t = TaxReturn::new(2026);
        t.salary = d("1780000");
        assert!(calculate(&book(), &t).unwrap().income_tax.0.is_zero());
        assert!(rules(2024).is_err());
    }

    #[test]
    fn life_insurance_tables_and_the_child_special() {
        assert_eq!(life_new(d("100000"), false, false), d("40000"));
        assert_eq!(life_new(d("50000"), false, false), d("32500"));
        assert_eq!(life_new(d("100000"), false, true), d("55000"));
        assert_eq!(life_new(d("50000"), true, false), d("26500"));
        assert_eq!(life_old(d("80000"), false), d("45000"));
        assert_eq!(life_old(d("80000"), true), d("35000"));
        // 新旧両方：旧だけ45,000と新旧合計（上限4万）の有利な方
        assert_eq!(
            life_category(d("30000"), d("80000"), false, false),
            d("45000")
        );
    }

    /// 会社員（年収600万円）、副業の雑所得、妻（パート年収100万円）、
    /// 16歳の子、同居の父母（72歳・70歳、年金所得あり）、生命保険・地震保険、
    /// 住宅ローン、ふるさと納税。手計算した値と一致することを確かめる。
    #[test]
    fn household_with_side_job_family_mortgage_and_furusato() {
        let mut t = TaxReturn::new(2026);
        t.salary = d("6000000");
        t.salary_withheld = d("80000");
        t.social_insurance = d("870000");
        t.side_kind = SideKind::Miscellaneous;
        t.side_revenue = d("600000");
        t.side_expenses = d("200000");
        t.has_spouse = true;
        // パート年収100万円 → 給与所得 26万円
        t.spouse_income = d("260000");
        t.spouse_birth_year = Some(1988);
        t.dependents = vec![
            Dependent {
                name: "長男".into(),
                relation: Relation::Child,
                birth_year: 2010,
                income: d("0"),
                cohabiting: true,
                disability: Disability::None,
            },
            Dependent {
                name: "父".into(),
                relation: Relation::Parent,
                birth_year: 1954,
                income: d("400000"),
                cohabiting: true,
                disability: Disability::None,
            },
            Dependent {
                name: "母".into(),
                relation: Relation::Parent,
                birth_year: 1956,
                income: d("0"),
                cohabiting: true,
                disability: Disability::None,
            },
        ];
        t.life_general_new = d("100000");
        t.life_care = d("40000");
        t.earthquake = d("30000");
        t.medical_paid = d("250000");
        t.donation = d("50000");
        t.housing_balance = d("25000000");
        t.housing_limit = d("40000000");
        t.housing_rate = d("0.7");
        let r = calculate(&book(), &t).unwrap();
        // 給与所得 6,000,000/4=1,500,000 ×3.2−440,000 = 4,360,000、雑所得 400,000
        assert_eq!(r.total_income.0, d("4760000"));
        let it = |label: &str| {
            r.lines
                .iter()
                .find(|l| l.label == label)
                .and_then(|l| l.income_tax)
                .unwrap()
                .0
        };
        let rt = |label: &str| {
            r.lines
                .iter()
                .find(|l| l.label == label)
                .and_then(|l| l.resident_tax)
                .unwrap()
                .0
        };
        // 一般 新10万 → 特例の表で55,000、介護医療 4万 → 30,000
        assert_eq!(it("生命保険料控除"), d("85000"));
        assert_eq!(rt("生命保険料控除"), d("52000"));
        // 医療費 250,000 − 100,000
        assert_eq!(it("医療費控除"), d("150000"));
        assert_eq!(it("寄附金控除"), d("48000"));
        assert_eq!(it("扶養控除（父）"), d("580000"));
        assert_eq!(it("扶養控除（長男）"), d("380000"));
        assert_eq!(it("基礎控除"), d("1040000"));
        // 所得控除: 870,000+85,000+30,000+150,000+48,000+380,000(配偶者)
        //   +380,000+580,000+580,000+1,040,000 = 4,143,000
        assert_eq!(it("所得控除の合計"), d("4143000"));
        assert_eq!(r.taxable_income.0, d("617000"));
        // 算出税額 30,850、住宅ローン控除 175,000 のうち 30,850
        assert_eq!(it("算出税額"), d("30850"));
        assert_eq!(r.income_tax.0, Decimal::ZERO);
        assert_eq!(r.balance_due.0, d("-80000"));
        // 住民税の所得控除: 870,000+52,000+15,000+150,000+330,000+330,000+450,000+450,000+430,000
        assert_eq!(rt("所得控除の合計"), d("3077000"));
        assert_eq!(r.taxable_resident.0, d("1683000"));
        // 人的控除差: 基礎5+配偶者5+一般扶養5+同居老親13×2 = 41万 → 調整控除 20,500
        assert_eq!(rt("調整控除"), d("-20500"));
        // 住民税の住宅ローン控除: 残り144,150 → 課税所得617,000の5%＝30,850まで
        assert_eq!(
            rt("住宅ローン控除（所得税から引ききれない分）"),
            d("-30850")
        );
        // 寄附金税額控除: 基本分 4,800 ＋ 特例分 48,000×84.895%＝40,749.6 → 所得割の2割 29,560
        assert_eq!(rt("寄附金税額控除（ふるさと納税）"), d("-34360"));
        // 168,300 − 20,500 − 30,850 − 34,360 = 82,590 → 82,500 ＋ 均等割 5,000
        assert_eq!(r.resident_tax.0, d("87500"));
        assert_eq!(r.take_home.0, d("5442500"));
        assert!(r.filing_required);
        // 上限の目安: 29,560 / 0.84895 + 2,000 → 36,819。寄附5万円はそれを超える
        assert_eq!(r.donation_limit.0, d("36819"));
        // 寄附なしの住民税 119,500 との差 32,000（所得税はどちらも0）
        assert_eq!(r.donation_saving.0, d("32000"));
        assert_eq!(r.donation_cost.0, d("18000"));
        assert!(r.notes.iter().any(|n| n.contains("上限の目安")));
    }
}
