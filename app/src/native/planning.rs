//! The tax and life plan pages: the calculation and the cash-flow table, the forms that edit
//! a year's tax inputs or a plan's items, and the [`Change`] each form produces.
//! Window-independent like `records.rs`; amounts are parsed and shown as decimals.
use super::form::Field;
use super::records::{Act, Context, Prepared, View};
use super::sheets::{Align, Table, money};
use chrono::Datelike;
use yy_accountant::{
    Book, Change, LifePlan, Target, TaxReturn,
    lifeplan::{self, FlowKind, PlanEvent, PlanFlow, PlanMember, School},
    num::{self, Decimal},
    tax::{self, Dependent, Disability, Relation, SideKind},
};
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
use Align::{Left as L, Right as R};

/// Parts of a year's input, edited one form at a time.
pub(super) const TAX_PARTS: [&str; 7] = [
    "収入（給与）",
    "副業",
    "配偶者",
    "保険料",
    "医療費・ふるさと納税",
    "住宅ローン・住民税",
    "名前・メモ",
];
/// Items that can be added to a plan.
pub(super) const PLAN_PARTS: [&str; 5] = [
    "家族",
    "毎年の収支",
    "イベント（1回だけの収支）",
    "教育費（子の進路から）",
    "ローン返済額（借入額・金利・年数から）",
];
const BLUE: [i64; 4] = [0, 100_000, 550_000, 650_000];
const BLUE_LABELS: [&str; 4] = ["なし", "10万円", "55万円", "65万円"];
const NO_YES: [&str; 2] = ["いいえ", "はい"];
const SCHOOLS: [&str; 3] = ["含めない", "公立（大学は国公立）", "私立"];
/// Detail rows of a plan: kind × this + index + 1.
const ITEM: i64 = 100_000;

fn text(label: &'static str, value: impl Into<String>) -> Field {
    Field::Text(label, value.into())
}
fn choice(label: &'static str, options: &[&str], selected: usize) -> Field {
    Field::Choice(
        label,
        options.iter().map(|o| o.to_string()).collect(),
        selected,
    )
}
fn amount_text(v: Decimal) -> String {
    if v.is_zero() {
        String::new()
    } else {
        num::text(v)
    }
}
fn amount(v: &str) -> Result<Decimal> {
    if v.trim().is_empty() {
        Ok(Decimal::ZERO)
    } else {
        Ok(num::parse(v)?)
    }
}
fn required(v: &str, what: &str) -> Result<Decimal> {
    if v.trim().is_empty() {
        return Err(format!("{what}を入れてください").into());
    }
    Ok(num::parse(v).map_err(|e| format!("{what}: {e}"))?)
}
fn year(v: &str, what: &str) -> Result<i32> {
    v.trim()
        .parse()
        .map_err(|_| format!("{what}は西暦の年（例: 1985）で入れてください").into())
}
fn index(v: &str) -> usize {
    v.parse().unwrap_or(0)
}
fn position<T: PartialEq + Copy>(all: &[T], value: T) -> usize {
    all.iter().position(|x| *x == value).unwrap_or(0)
}
fn labels<T: Copy>(all: &[T], label: impl Fn(T) -> &'static str) -> Vec<String> {
    all.iter().map(|x| label(*x).to_string()).collect()
}

pub(super) fn tax_return(book: &Book, cx: &Context) -> Result<TaxReturn> {
    let id = cx
        .chosen
        .ok_or("年分がありません。「年分を追加...」で作ります")?;
    Ok(book
        .tax_returns
        .iter()
        .find(|t| t.id == id)
        .cloned()
        .ok_or("年分が見つかりません")?)
}
pub(super) fn life_plan(book: &Book, cx: &Context) -> Result<LifePlan> {
    let id = cx
        .chosen
        .ok_or("プランがありません。「新しいプラン...」で作ります")?;
    Ok(book
        .life_plans
        .iter()
        .find(|p| p.id == id)
        .cloned()
        .ok_or("プランが見つかりません")?)
}

pub(super) fn tax_view(book: &Book, cx: &Context) -> Result<View> {
    let mut main = Table::new(&[
        ("区分", 72, L),
        ("項目", 240, L),
        ("所得税", 110, R),
        ("住民税", 110, R),
        ("説明", 460, L),
    ]);
    let mut detail = Table::new(&[
        ("親族", 120, L),
        ("続柄", 110, L),
        ("年齢（年末）", 90, R),
        ("合計所得", 110, R),
        ("同居", 56, L),
        ("障害", 100, L),
    ]);
    if cx.chosen.is_none() {
        return Ok(View {
            main,
            detail,
            summary: format!(
                "年分を追加すると、給与・副業・家族・保険・医療費・ふるさと納税・住宅ローンから所得税と住民税を試算します（対応: {}年分）",
                tax::YEARS.map(|y| y.to_string()).join("・")
            ),
        });
    }
    let t = tax_return(book, cx)?;
    let r = tax::calculate(book, &t)?;
    let cell = |v: Option<num::D>| v.map(|v| money(book, v.0)).unwrap_or_default();
    for l in &r.lines {
        main.push(
            0,
            vec![
                l.section.into(),
                l.label.clone(),
                cell(l.income_tax),
                cell(l.resident_tax),
                l.note.clone(),
            ],
        );
    }
    for n in &r.notes {
        main.push(
            0,
            vec![
                "注意".into(),
                String::new(),
                String::new(),
                String::new(),
                n.clone(),
            ],
        );
    }
    for (i, d) in t.dependents.iter().enumerate() {
        detail.push(
            i as i64 + 1,
            vec![
                d.name.clone(),
                d.relation.label().into(),
                (t.year - d.birth_year).to_string(),
                money(book, d.income),
                if d.cohabiting { "同居" } else { "" }.into(),
                if d.disability == Disability::None {
                    String::new()
                } else {
                    d.disability.label().into()
                },
            ],
        );
    }
    let due = r.balance_due.0;
    let summary = format!(
        "{}　所得税: {} {}　住民税 {}　手取りの目安 {}　ふるさと納税の上限の目安 {}　確定申告: {}",
        t.name,
        if due.is_sign_negative() {
            "還付"
        } else {
            "納付"
        },
        money(book, due.abs()),
        money(book, r.resident_tax.0),
        money(book, r.take_home.0),
        money(book, r.donation_limit.0),
        if r.filing_required {
            "必要"
        } else {
            "注意を参照"
        }
    );
    Ok(View {
        main,
        detail,
        summary,
    })
}

pub(super) fn plan_view(book: &Book, cx: &Context) -> Result<View> {
    let mut detail = Table::new(&[
        ("種類", 72, L),
        ("名前", 220, L),
        ("金額・生まれ年", 120, R),
        ("年・期間", 120, L),
        ("変動率", 72, R),
    ]);
    if cx.chosen.is_none() {
        return Ok(View {
            main: Table::new(&[("年", 64, R)]),
            detail,
            summary: "プランがありません。「新しいプラン...」で、帳簿の直近12か月の収支と預金残高（税額計算があればその手取り）から作れます".into(),
        });
    }
    let p = life_plan(book, cx)?;
    let c = lifeplan::cash_flow(book, &p)?;
    let mut columns = vec![("年".to_string(), 56, R)];
    columns.extend(c.members.iter().map(|m| (m.clone(), 56, R)));
    columns.extend(c.flows.iter().map(|f| (f.name.clone(), 110, R)));
    for (name, width) in [
        ("イベント", 220),
        ("収入", 110),
        ("支出", 110),
        ("年間収支", 110),
        ("運用益", 90),
        ("金融資産", 120),
    ] {
        columns.push((
            name.to_string(),
            width,
            if name == "イベント" { L } else { R },
        ));
    }
    let mut main = Table::with(columns);
    for r in &c.rows {
        let mut cells = vec![r.year.to_string()];
        cells.extend(
            r.ages
                .iter()
                .map(|a| a.map(|a| a.to_string()).unwrap_or_default()),
        );
        cells.extend(r.values.iter().map(|v| {
            if v.0.is_zero() {
                String::new()
            } else {
                money(book, v.0)
            }
        }));
        cells.push(
            r.events
                .iter()
                .map(|(n, v)| format!("{n} {}", money(book, v.0)))
                .collect::<Vec<_>>()
                .join(" / "),
        );
        for v in [&r.income, &r.expense, &r.balance, &r.investment, &r.savings] {
            cells.push(money(book, v.0));
        }
        main.push(i64::from(r.year), cells);
    }
    for (i, m) in p.members.iter().enumerate() {
        detail.push(
            ITEM + i as i64 + 1,
            vec![
                "家族".into(),
                if m.relation.is_empty() {
                    m.name.clone()
                } else {
                    format!("{}（{}）", m.name, m.relation)
                },
                m.birth_year.to_string(),
                String::new(),
                String::new(),
            ],
        );
    }
    for (i, f) in p.flows.iter().enumerate() {
        detail.push(
            2 * ITEM + i as i64 + 1,
            vec![
                f.kind.label().into(),
                f.name.clone(),
                money(book, f.amount),
                format!("{}〜{}年", f.start_year, f.end_year),
                format!("{}%", num::text(f.growth_percent)),
            ],
        );
    }
    for (i, e) in p.events.iter().enumerate() {
        detail.push(
            3 * ITEM + i as i64 + 1,
            vec![
                format!("イベント・{}", e.kind.label()),
                e.name.clone(),
                money(book, e.amount),
                format!("{}年", e.year),
                format!("{}%", num::text(e.growth_percent)),
            ],
        );
    }
    let mut summary = format!(
        "{}年末の金融資産 {}　最少 {}（{}年）",
        p.start_year + p.years as i32 - 1,
        money(book, c.final_savings.0),
        money(book, c.min_savings.0),
        c.min_year
    );
    if let (Some(a), Some(b)) = (c.shortfall_years.first(), c.shortfall_years.last()) {
        summary += &format!("　資金不足 {a}〜{b}年");
    }
    for other in book.life_plans.iter().filter(|o| o.id != p.id).take(2) {
        let cmp = lifeplan::compare(book, other, &p)?;
        summary += &format!(
            "　「{}」との差 {}",
            other.name,
            money(book, cmp.final_difference.0)
        );
    }
    Ok(View {
        main,
        detail,
        summary,
    })
}

// ----- tax forms -----
fn dependent_fields(d: Option<&Dependent>, edit: bool) -> Vec<Field> {
    vec![
        text(
            "名前（例: 長男、父）",
            d.map(|d| d.name.clone()).unwrap_or_default(),
        ),
        Field::Choice(
            "続柄",
            labels(&Relation::ALL, Relation::label),
            d.map_or(0, |d| position(&Relation::ALL, d.relation)),
        ),
        text(
            "生まれ年",
            d.map(|d| d.birth_year.to_string()).unwrap_or_default(),
        ),
        text(
            "合計所得金額",
            d.map(|d| amount_text(d.income)).unwrap_or_default(),
        ),
        choice(
            "同居している",
            &NO_YES,
            d.map_or(1, |d| usize::from(d.cohabiting)),
        ),
        Field::Choice(
            "障害",
            labels(&Disability::ALL, Disability::label),
            d.map_or(0, |d| position(&Disability::ALL, d.disability)),
        ),
    ]
    .into_iter()
    .chain(edit.then(|| choice("外す", &NO_YES, 0)))
    .collect()
}
fn dependent(v: &[String]) -> Result<Dependent> {
    Ok(Dependent {
        name: v[0].trim().into(),
        relation: Relation::ALL[index(&v[1]).min(2)],
        birth_year: year(&v[2], "生まれ年")?,
        income: amount(&v[3])?,
        cohabiting: index(&v[4]) == 1,
        disability: Disability::ALL[index(&v[5]).min(2)],
    })
}

pub(super) fn tax_prepare(book: &Book, act: Act, cx: &Context) -> Result<Prepared> {
    Ok(match act {
        Act::New => {
            let mut fields = vec![
                Field::Choice(
                    "年分",
                    tax::YEARS.iter().map(|y| format!("{y}年分")).collect(),
                    tax::YEARS.len() - 1,
                ),
                text("名前（空欄なら「2026年分」）", ""),
            ];
            let previous = book.tax_returns.last();
            fields.push(Field::Choice(
                "前の入力を引き継ぐ",
                std::iter::once("引き継がない".to_string())
                    .chain(previous.map(|t| format!("「{}」を引き継ぐ", t.name)))
                    .collect(),
                usize::from(previous.is_some()),
            ));
            Prepared::Form("年分を追加".into(), fields)
        }
        Act::Edit => {
            let t = tax_return(book, cx)?;
            let Some(part) = cx.part else {
                return Ok(Prepared::Choose(
                    format!("{}の入力を編集", t.name),
                    "編集する入力",
                    TAX_PARTS.iter().map(|s| s.to_string()).collect(),
                ));
            };
            let m = |v: Decimal| amount_text(v);
            let fields = match part {
                0 => vec![
                    text("給与の支払金額（源泉徴収票）", m(t.salary)),
                    text("源泉徴収税額", m(t.salary_withheld)),
                    text("社会保険料（家族分を含む）", m(t.social_insurance)),
                    text("iDeCoなどの掛金", m(t.mutual_aid)),
                    Field::Choice(
                        "本人の障害",
                        labels(&Disability::ALL, Disability::label),
                        position(&Disability::ALL, t.self_disability),
                    ),
                ],
                1 => vec![
                    Field::Choice(
                        "副業の種類",
                        labels(&SideKind::ALL, SideKind::label),
                        position(&SideKind::ALL, t.side_kind),
                    ),
                    text("副業の収入", m(t.side_revenue)),
                    text("副業の経費", m(t.side_expenses)),
                    text("副業の源泉徴収税額", m(t.side_withheld)),
                    choice(
                        "青色申告特別控除",
                        &BLUE_LABELS,
                        BLUE.iter()
                            .position(|b| Decimal::from(*b) == t.blue_deduction)
                            .unwrap_or(0),
                    ),
                ],
                2 => vec![
                    choice("配偶者", &["いない", "いる"], usize::from(t.has_spouse)),
                    text("配偶者の合計所得金額", m(t.spouse_income)),
                    text(
                        "配偶者の生まれ年",
                        t.spouse_birth_year
                            .map(|y| y.to_string())
                            .unwrap_or_default(),
                    ),
                    Field::Choice(
                        "配偶者の障害",
                        labels(&Disability::ALL, Disability::label),
                        position(&Disability::ALL, t.spouse_disability),
                    ),
                ],
                3 => vec![
                    text("一般生命保険料（新）", m(t.life_general_new)),
                    text("一般生命保険料（旧）", m(t.life_general_old)),
                    text("介護医療保険料", m(t.life_care)),
                    text("個人年金保険料（新）", m(t.life_pension_new)),
                    text("個人年金保険料（旧）", m(t.life_pension_old)),
                    text("地震保険料", m(t.earthquake)),
                ],
                4 => vec![
                    text("医療費（家族分の合計）", m(t.medical_paid)),
                    text("保険などで補塡された額", m(t.medical_covered)),
                    text("セルフメディケーション対象", m(t.self_medication)),
                    text("ふるさと納税の寄附額", m(t.donation)),
                ],
                5 => vec![
                    text("住宅ローンの年末残高", m(t.housing_balance)),
                    text("借入限度額", m(t.housing_limit)),
                    text("控除率（%）", num::text(t.housing_rate)),
                    text("住民税からの控除上限", num::text(t.housing_resident_cap)),
                    text("住民税の均等割", num::text(t.per_capita)),
                ],
                _ => vec![text("名前", t.name.clone()), text("メモ", t.memo.clone())],
            };
            Prepared::Form(
                format!("{}：{}", t.name, TAX_PARTS[part.min(TAX_PARTS.len() - 1)]),
                fields,
            )
        }
        Act::Remove => {
            let t = tax_return(book, cx)?;
            Prepared::Confirm(
                format!("「{}」を削除しますか？", t.name),
                Change::Remove {
                    target: Target::TaxReturn,
                    id: t.id,
                },
            )
        }
        Act::First => {
            let t = tax_return(book, cx)?;
            Prepared::Form(
                format!("{}：親族を追加", t.name),
                dependent_fields(None, false),
            )
        }
        Act::Second => {
            let t = tax_return(book, cx)?;
            let i = cx.detail.ok_or("下の一覧で親族を選んでください")? as usize - 1;
            let d = t.dependents.get(i).ok_or("選んだ親族が見つかりません")?;
            Prepared::Form(
                format!("{}：{}", t.name, d.name),
                dependent_fields(Some(d), true),
            )
        }
    })
}

pub(super) fn tax_finish(book: &Book, act: Act, cx: &Context, v: &[String]) -> Result<Change> {
    let mut t = match act {
        Act::New => {
            let year = tax::YEARS[index(&v[0]).min(tax::YEARS.len() - 1)];
            let mut t = match book.tax_returns.last().filter(|_| index(&v[2]) == 1) {
                Some(previous) => previous.clone(),
                None => TaxReturn::new(year),
            };
            t.id = 0;
            t.year = year;
            t.name = v[1].trim().into();
            return Ok(Change::SaveTaxReturn {
                tax_return: Box::new(t),
            });
        }
        _ => tax_return(book, cx)?,
    };
    match act {
        Act::Edit => match cx.part.unwrap_or(0) {
            0 => {
                t.salary = amount(&v[0])?;
                t.salary_withheld = amount(&v[1])?;
                t.social_insurance = amount(&v[2])?;
                t.mutual_aid = amount(&v[3])?;
                t.self_disability = Disability::ALL[index(&v[4]).min(2)];
            }
            1 => {
                t.side_kind = SideKind::ALL[index(&v[0]).min(SideKind::ALL.len() - 1)];
                t.side_revenue = amount(&v[1])?;
                t.side_expenses = amount(&v[2])?;
                t.side_withheld = amount(&v[3])?;
                t.blue_deduction = Decimal::from(BLUE[index(&v[4]).min(3)]);
            }
            2 => {
                t.has_spouse = index(&v[0]) == 1;
                t.spouse_income = amount(&v[1])?;
                t.spouse_birth_year = if v[2].trim().is_empty() {
                    None
                } else {
                    Some(year(&v[2], "配偶者の生まれ年")?)
                };
                t.spouse_disability = Disability::ALL[index(&v[3]).min(2)];
            }
            3 => {
                t.life_general_new = amount(&v[0])?;
                t.life_general_old = amount(&v[1])?;
                t.life_care = amount(&v[2])?;
                t.life_pension_new = amount(&v[3])?;
                t.life_pension_old = amount(&v[4])?;
                t.earthquake = amount(&v[5])?;
            }
            4 => {
                t.medical_paid = amount(&v[0])?;
                t.medical_covered = amount(&v[1])?;
                t.self_medication = amount(&v[2])?;
                t.donation = amount(&v[3])?;
            }
            5 => {
                t.housing_balance = amount(&v[0])?;
                t.housing_limit = amount(&v[1])?;
                t.housing_rate = amount(&v[2])?;
                t.housing_resident_cap = amount(&v[3])?;
                t.per_capita = amount(&v[4])?;
            }
            _ => {
                t.name = v[0].trim().into();
                t.memo = v[1].trim().into();
            }
        },
        Act::First => t.dependents.push(dependent(v)?),
        Act::Second => {
            let i = cx.detail.ok_or("下の一覧で親族を選んでください")? as usize - 1;
            if i >= t.dependents.len() {
                return Err("選んだ親族が見つかりません".into());
            }
            if index(&v[6]) == 1 {
                t.dependents.remove(i);
            } else {
                t.dependents[i] = dependent(v)?;
            }
        }
        _ => {}
    }
    Ok(Change::SaveTaxReturn {
        tax_return: Box::new(t),
    })
}

// ----- life plan forms -----
fn flow_fields(f: Option<&PlanFlow>, p: &LifePlan, edit: bool) -> Vec<Field> {
    let mut fields = vec![
        text("名前", f.map(|f| f.name.clone()).unwrap_or_default()),
        choice(
            "種類",
            &["収入", "支出"],
            f.map_or(1, |f| position(&FlowKind::ALL, f.kind)),
        ),
        text(
            "年額（今の価格）",
            f.map(|f| num::text(f.amount)).unwrap_or_default(),
        ),
        text(
            "開始年",
            f.map_or(p.start_year, |f| f.start_year).to_string(),
        ),
        text(
            "終了年",
            f.map_or(p.start_year + p.years as i32 - 1, |f| f.end_year)
                .to_string(),
        ),
        text(
            "変動率（年%）",
            f.map(|f| num::text(f.growth_percent))
                .unwrap_or_else(|| "0".into()),
        ),
    ];
    if edit {
        fields.push(choice("外す", &NO_YES, 0));
    }
    fields
}
fn flow(v: &[String]) -> Result<PlanFlow> {
    Ok(PlanFlow {
        name: v[0].trim().into(),
        kind: FlowKind::ALL[index(&v[1]).min(1)],
        amount: required(&v[2], "年額")?,
        start_year: year(&v[3], "開始年")?,
        end_year: year(&v[4], "終了年")?,
        growth_percent: amount(&v[5])?,
    })
}
fn event_fields(e: Option<&PlanEvent>, p: &LifePlan, edit: bool) -> Vec<Field> {
    let mut fields = vec![
        text("名前", e.map(|e| e.name.clone()).unwrap_or_default()),
        choice(
            "種類",
            &["収入", "支出"],
            e.map_or(1, |e| position(&FlowKind::ALL, e.kind)),
        ),
        text(
            "金額（今の価格）",
            e.map(|e| num::text(e.amount)).unwrap_or_default(),
        ),
        text("年", e.map_or(p.start_year, |e| e.year).to_string()),
        text(
            "変動率（年%）",
            e.map(|e| num::text(e.growth_percent))
                .unwrap_or_else(|| "0".into()),
        ),
    ];
    if edit {
        fields.push(choice("外す", &NO_YES, 0));
    }
    fields
}
fn event(v: &[String]) -> Result<PlanEvent> {
    Ok(PlanEvent {
        name: v[0].trim().into(),
        kind: FlowKind::ALL[index(&v[1]).min(1)],
        amount: required(&v[2], "金額")?,
        year: year(&v[3], "年")?,
        growth_percent: amount(&v[4])?,
    })
}
fn member_fields(m: Option<&PlanMember>, edit: bool) -> Vec<Field> {
    let mut fields = vec![
        text("名前", m.map(|m| m.name.clone()).unwrap_or_default()),
        text(
            "続柄（本人・配偶者・長男など）",
            m.map(|m| m.relation.clone()).unwrap_or_default(),
        ),
        text(
            "生まれ年",
            m.map(|m| m.birth_year.to_string()).unwrap_or_default(),
        ),
    ];
    if edit {
        fields.push(choice("外す", &NO_YES, 0));
    }
    fields
}
fn member(v: &[String]) -> Result<PlanMember> {
    Ok(PlanMember {
        name: v[0].trim().into(),
        relation: v[1].trim().into(),
        birth_year: year(&v[2], "生まれ年")?,
    })
}
fn item(cx: &Context) -> Result<(i64, usize)> {
    let id = cx.detail.ok_or("下の一覧で項目を選んでください")?;
    Ok((id / ITEM, (id % ITEM - 1) as usize))
}

pub(super) fn plan_prepare(book: &Book, act: Act, cx: &Context) -> Result<Prepared> {
    Ok(match act {
        Act::New => {
            let current = cx
                .chosen
                .and_then(|id| book.life_plans.iter().find(|p| p.id == id));
            let mut ways = vec![
                "空のプラン".to_string(),
                "帳簿から作成（直近12か月）".into(),
            ];
            if let Some(p) = current {
                ways.push(format!("「{}」を複製（提案づくり）", p.name));
            }
            // The period and return follow the plan on screen, so a copy compares year by year.
            let (start, years, rate) = current
                .map_or((cx.today.year() + 1, 30, Decimal::ZERO), |p| {
                    (p.start_year, p.years, p.return_percent)
                });
            let mut income = vec![(0, "帳簿の収入".to_string())];
            income.extend(
                book.tax_returns
                    .iter()
                    .map(|t| (t.id, format!("「{}」の手取り", t.name))),
            );
            let latest = book.tax_returns.last().map_or(0, |t| t.id);
            Prepared::Form(
                "新しいプラン".into(),
                vec![
                    text(
                        "名前",
                        if current.is_some() {
                            "提案"
                        } else {
                            "現状"
                        },
                    ),
                    Field::Choice("作り方", ways, if current.is_some() { 2 } else { 1 }),
                    Field::Pick("帳簿から作るときの収入", income, latest),
                    text("最初の年", start.to_string()),
                    text("年数", years.to_string()),
                    text("最初の金融資産（空欄なら預金残高）", ""),
                    text("運用利回り（年%）", num::text(rate)),
                ],
            )
        }
        Act::Edit => {
            let p = life_plan(book, cx)?;
            Prepared::Form(
                format!("「{}」の設定", p.name),
                vec![
                    text("名前", p.name.clone()),
                    text("最初の年", p.start_year.to_string()),
                    text("年数", p.years.to_string()),
                    text("最初の金融資産", num::text(p.savings)),
                    text("運用利回り（年%）", num::text(p.return_percent)),
                    text("メモ", p.memo.clone()),
                ],
            )
        }
        Act::Remove => {
            let p = life_plan(book, cx)?;
            Prepared::Confirm(
                format!("「{}」を削除しますか？", p.name),
                Change::Remove {
                    target: Target::LifePlan,
                    id: p.id,
                },
            )
        }
        Act::First => {
            let p = life_plan(book, cx)?;
            let Some(part) = cx.part else {
                return Ok(Prepared::Choose(
                    format!("「{}」に項目を追加", p.name),
                    "追加する項目",
                    PLAN_PARTS.iter().map(|s| s.to_string()).collect(),
                ));
            };
            let title = format!("「{}」：{}", p.name, PLAN_PARTS[part.min(4)]);
            let fields = match part {
                0 => member_fields(None, false),
                1 => flow_fields(None, &p, false),
                2 => event_fields(None, &p, false),
                3 => {
                    let child = p.members.iter().find(|m| {
                        ["子", "男", "女", "娘", "息子"]
                            .iter()
                            .any(|k| m.relation.contains(k) || m.name.contains(k))
                    });
                    let mut fields = vec![
                        text(
                            "子の名前",
                            child.map(|c| c.name.clone()).unwrap_or_default(),
                        ),
                        text(
                            "子の生まれ年",
                            child.map(|c| c.birth_year.to_string()).unwrap_or_default(),
                        ),
                    ];
                    for (i, stage) in lifeplan::EDUCATION.iter().enumerate() {
                        fields.push(choice(
                            ["幼稚園", "小学校", "中学校", "高校", "大学"][i],
                            &SCHOOLS,
                            usize::from(stage.stage != lifeplan::Stage::Kindergarten),
                        ));
                    }
                    fields.push(text("教育費の上昇率（年%）", "0"));
                    fields
                }
                _ => vec![
                    text("名前", "住宅ローン返済"),
                    text("借入額（残高）", ""),
                    text("金利（年%）", "1"),
                    text("返済期間（年）", "35"),
                    text("返済開始年", p.start_year.to_string()),
                ],
            };
            Prepared::Form(title, fields)
        }
        Act::Second => {
            let p = life_plan(book, cx)?;
            let (kind, i) = item(cx)?;
            let missing = || "選んだ項目が見つかりません";
            let (title, fields) = match kind {
                1 => {
                    let m = p.members.get(i).ok_or_else(missing)?;
                    (m.name.clone(), member_fields(Some(m), true))
                }
                2 => {
                    let f = p.flows.get(i).ok_or_else(missing)?;
                    (f.name.clone(), flow_fields(Some(f), &p, true))
                }
                _ => {
                    let e = p.events.get(i).ok_or_else(missing)?;
                    (e.name.clone(), event_fields(Some(e), &p, true))
                }
            };
            Prepared::Form(format!("「{}」：{title}", p.name), fields)
        }
    })
}

pub(super) fn plan_finish(book: &Book, act: Act, cx: &Context, v: &[String]) -> Result<Change> {
    let count = |s: &str, what: &str| -> Result<u32> {
        Ok(s.trim()
            .parse()
            .map_err(|_| format!("{what}は整数で入れてください"))?)
    };
    let mut p = match act {
        Act::New => {
            let way = index(&v[1]);
            let mut p = match way {
                2 => life_plan(book, cx)?,
                1 => {
                    let take_home = match v[2].parse::<i64>().ok().filter(|id| *id != 0) {
                        Some(id) => {
                            let t = book
                                .tax_returns
                                .iter()
                                .find(|t| t.id == id)
                                .ok_or("税額計算が見つかりません")?;
                            Some(tax::calculate(book, t)?.take_home.0)
                        }
                        None => None,
                    };
                    lifeplan::draft(book, cx.today, take_home)?
                }
                _ => LifePlan {
                    id: 0,
                    name: String::new(),
                    memo: String::new(),
                    start_year: 0,
                    years: 0,
                    savings: Decimal::ZERO,
                    return_percent: Decimal::ZERO,
                    members: vec![],
                    flows: vec![],
                    events: vec![],
                },
            };
            let start = year(&v[3], "最初の年")?;
            // A draft's flows start in its own first year; move them with the plan.
            if way == 1 {
                let shift = start - p.start_year;
                for f in &mut p.flows {
                    f.start_year += shift;
                    f.end_year += shift;
                }
            }
            p.id = 0;
            p.name = v[0].trim().into();
            p.start_year = start;
            p.years = count(&v[4], "年数")?;
            if !v[5].trim().is_empty() || way == 0 {
                p.savings = amount(&v[5])?;
            }
            p.return_percent = amount(&v[6])?;
            return Ok(Change::SaveLifePlan { life_plan: p });
        }
        _ => life_plan(book, cx)?,
    };
    match act {
        Act::Edit => {
            p.name = v[0].trim().into();
            p.start_year = year(&v[1], "最初の年")?;
            p.years = count(&v[2], "年数")?;
            p.savings = amount(&v[3])?;
            p.return_percent = amount(&v[4])?;
            p.memo = v[5].trim().into();
        }
        Act::First => match cx.part.unwrap_or(0) {
            0 => p.members.push(member(v)?),
            1 => p.flows.push(flow(v)?),
            2 => p.events.push(event(v)?),
            3 => {
                let schools = [2, 3, 4, 5, 6]
                    .map(|i| [School::None, School::Public, School::Private][index(&v[i]).min(2)]);
                let (flows, events) = lifeplan::education(&lifeplan::EducationRequest {
                    child: v[0].trim().into(),
                    birth_year: year(&v[1], "子の生まれ年")?,
                    schools,
                    growth_percent: amount(&v[7])?,
                })?;
                p.flows.extend(flows);
                p.events.extend(events);
            }
            _ => {
                let years = count(&v[3], "返済期間")?;
                let pay = lifeplan::loan_payment(
                    book,
                    required(&v[1], "借入額")?,
                    amount(&v[2])?,
                    years,
                )?;
                let start = year(&v[4], "返済開始年")?;
                p.flows.push(PlanFlow {
                    name: v[0].trim().into(),
                    kind: FlowKind::Expense,
                    amount: pay.annual.0,
                    start_year: start,
                    end_year: start + years as i32 - 1,
                    growth_percent: Decimal::ZERO,
                });
            }
        },
        Act::Second => {
            let (kind, i) = item(cx)?;
            let last = v.len() - 1;
            let drop = index(&v[last]) == 1;
            let missing = "選んだ項目が見つかりません";
            match kind {
                1 if i < p.members.len() => {
                    if drop {
                        p.members.remove(i);
                    } else {
                        p.members[i] = member(v)?;
                    }
                }
                2 if i < p.flows.len() => {
                    if drop {
                        p.flows.remove(i);
                    } else {
                        p.flows[i] = flow(v)?;
                    }
                }
                3 if i < p.events.len() => {
                    if drop {
                        p.events.remove(i);
                    } else {
                        p.events[i] = event(v)?;
                    }
                }
                _ => return Err(missing.into()),
            }
        }
        _ => {}
    }
    Ok(Change::SaveLifePlan { life_plan: p })
}

/// Text of the six coefficients for a message box.
pub(super) fn coefficients_text(rate: &str, years: &str) -> Result<String> {
    let c = lifeplan::coefficients(
        required(rate, "利率")?,
        years
            .trim()
            .parse()
            .map_err(|_| "年数は整数で入れてください")?,
    )?;
    let t = |d: num::D| num::text(d.0);
    Ok(format!(
        "利率 {}%・{} 年\n\n終価係数 {}（今の元本が何倍になるか）\n現価係数 {}（目標額に今いくら必要か）\n年金終価係数 {}（毎年の積立が何倍になるか）\n減債基金係数 {}（目標額のために毎年いくら積み立てるか）\n資本回収係数 {}（元本を取り崩すと毎年いくら受け取れるか・ローンの年返済額）\n年金現価係数 {}（毎年の受取に今いくら必要か）",
        num::text(c.rate.0),
        c.years,
        t(c.future_value),
        t(c.present_value),
        t(c.annuity_future),
        t(c.sinking_fund),
        t(c.capital_recovery),
        t(c.annuity_present)
    ))
}
