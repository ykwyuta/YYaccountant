//! 学習用の問題集：具体的な人物・家族のシナリオで、会計（記帳・帳票・原価）、税務（所得税・
//! 住民税）、ライフプラン（係数・ローン・キャッシュフロー）を練習する。
//!
//! シナリオごとに帳簿をメモリー上で組み立て、数値問題の正解は帳票・税額・ライフプラン・原価の
//! 計算エンジンで求める（解説の数値も同じ値から作る）。主な値は `tests/study.rs` で手計算と
//! 照合する。`docs/user/study.md` はここから生成する（`tests/docs.rs`）。
use crate::change::Change;
use crate::lifeplan::{self, LifePlan, PlanEvent, PlanFlow, PlanMember};
use crate::model::{Book, BookKind, Category};
use crate::num::{self, Decimal, Rounding};
use crate::store::NewBook;
use crate::tax::{self, Dependent, Relation, SideKind, TaxResult, TaxReturn};
use crate::{Result, claims, cost, invalid, reports};
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::sync::OnceLock;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Topic {
    Accounting,
    Tax,
    LifePlan,
}
impl Topic {
    pub const ALL: [Self; 3] = [Self::Accounting, Self::Tax, Self::LifePlan];
    pub fn label(self) -> &'static str {
        match self {
            Self::Accounting => "会計",
            Self::Tax => "税務",
            Self::LifePlan => "ライフプラン",
        }
    }
}

/// 正解（学習者に答える前は見せない）。
#[derive(Clone, Debug)]
pub enum Key {
    /// 選択肢（本文、その選択肢についての解説）と正解の番号（0から）
    Choice {
        options: Vec<(String, String)>,
        correct: usize,
    },
    /// 仕訳：科目コードごとの借方・貸方
    Journal(Vec<(String, Decimal, Decimal)>),
    /// 数値：小数 `digits` 桁に四捨五入して比べる
    Number {
        value: Decimal,
        digits: u32,
        unit: &'static str,
    },
}

#[derive(Clone, Debug)]
pub struct Problem {
    pub id: String,
    pub topic: Topic,
    /// 1 基本、2 標準、3 応用
    pub level: u8,
    pub title: String,
    pub question: String,
    pub hint: String,
    /// 関係する画面（Web UIの表示名）
    pub screen: &'static str,
    pub key: Key,
    pub explanation: String,
}

/// 学習者に見せる問題（正解と解説を含まない）。
#[derive(Clone, Debug, Serialize)]
pub struct ProblemView {
    pub id: String,
    pub topic: Topic,
    pub level: u8,
    pub title: String,
    pub question: String,
    pub hint: String,
    pub screen: &'static str,
    pub format: Format,
}
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Format {
    Choice { options: Vec<String> },
    Journal,
    Number { unit: &'static str, digits: u32 },
}

pub struct Scenario {
    pub id: &'static str,
    pub title: &'static str,
    pub subtitle: &'static str,
    pub kind: BookKind,
    pub story: Vec<String>,
    /// 前提（項目、内容）
    pub facts: Vec<(String, String)>,
    pub problems: Vec<Problem>,
    /// シナリオの帳簿（取引・債権債務・計画・税額・ライフプランを記録済み）
    pub book: Book,
    /// 帳簿を作った変更の列（練習用の帳簿を作るときに同じ順で適用する）
    pub changes: Vec<Change>,
}
#[derive(Clone, Debug, Serialize)]
pub struct AccountView {
    pub code: String,
    pub name: String,
    pub category: Category,
}
#[derive(Clone, Debug, Serialize)]
pub struct ScenarioView {
    pub id: &'static str,
    pub title: &'static str,
    pub subtitle: &'static str,
    pub kind: BookKind,
    pub story: Vec<String>,
    pub facts: Vec<(String, String)>,
    /// 仕訳問題で選べる科目
    pub accounts: Vec<AccountView>,
    pub problems: Vec<ProblemView>,
}
impl Problem {
    pub fn view(&self) -> ProblemView {
        ProblemView {
            id: self.id.clone(),
            topic: self.topic,
            level: self.level,
            title: self.title.clone(),
            question: self.question.clone(),
            hint: self.hint.clone(),
            screen: self.screen,
            format: match &self.key {
                Key::Choice { options, .. } => Format::Choice {
                    options: options.iter().map(|o| o.0.clone()).collect(),
                },
                Key::Journal(_) => Format::Journal,
                Key::Number { unit, digits, .. } => Format::Number {
                    unit,
                    digits: *digits,
                },
            },
        }
    }
}
impl Scenario {
    pub fn view(&self) -> ScenarioView {
        ScenarioView {
            id: self.id,
            title: self.title,
            subtitle: self.subtitle,
            kind: self.kind,
            story: self.story.clone(),
            facts: self.facts.clone(),
            accounts: self
                .book
                .accounts
                .iter()
                .filter(|a| !a.archived)
                .map(|a| AccountView {
                    code: a.code.clone(),
                    name: a.name.clone(),
                    category: a.category,
                })
                .collect(),
            problems: self.problems.iter().map(Problem::view).collect(),
        }
    }
    fn account_name(&self, code: &str) -> String {
        self.book
            .accounts
            .iter()
            .find(|a| a.code == code)
            .map_or_else(|| code.to_string(), |a| a.name.clone())
    }
    /// 正解の表示（仕訳は「借方 … ／ 貸方 …」）。
    pub fn expected(&self, p: &Problem) -> String {
        match &p.key {
            Key::Choice { options, correct } => options[*correct].0.clone(),
            Key::Journal(lines) => {
                let side = |debit: bool| {
                    lines
                        .iter()
                        .filter(|l| {
                            if debit {
                                !l.1.is_zero()
                            } else {
                                !l.2.is_zero()
                            }
                        })
                        .map(|l| {
                            format!(
                                "{} {}",
                                self.account_name(&l.0),
                                num::grouped(if debit { l.1 } else { l.2 })
                            )
                        })
                        .collect::<Vec<_>>()
                        .join("、")
                };
                format!("借方 {} ／ 貸方 {}", side(true), side(false))
            }
            Key::Number { value, unit, .. } if *unit == "年" => {
                format!("{}{unit}", num::text(*value))
            }
            Key::Number { value, unit, .. } => format!("{}{unit}", num::grouped(*value)),
        }
    }
}

// ---------- Answers and grading ----------

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Answer {
    /// 選んだ選択肢の番号（0から）
    Choice(usize),
    Journal(Vec<AnswerLine>),
    /// 数値（"1,234" や全角数字も可）
    Number(String),
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnswerLine {
    /// 科目コード
    pub account: String,
    #[serde(default)]
    pub debit: String,
    #[serde(default)]
    pub credit: String,
}
#[derive(Clone, Debug, Serialize)]
pub struct Grade {
    pub problem: String,
    pub correct: bool,
    /// 正解の表示
    pub expected: String,
    /// 解答に合わせた指摘（どこが違うか）
    pub feedback: Vec<String>,
    pub explanation: String,
    /// 選択肢ごとの解説（選択問題のみ）
    pub reasons: Vec<String>,
}

fn amount(text: &str) -> Result<Decimal> {
    if text.trim().is_empty() {
        Ok(Decimal::ZERO)
    } else {
        num::parse(text)
    }
}

pub fn grade(problem_id: &str, answer: &Answer) -> Result<Grade> {
    let (s, p) = find(problem_id).ok_or_else(|| invalid("問題がありません"))?;
    let mut feedback = vec![];
    let mut reasons = vec![];
    let correct = match (&p.key, answer) {
        (Key::Choice { options, correct }, Answer::Choice(i)) => {
            if *i >= options.len() {
                return Err(invalid("選択肢の番号が範囲外です"));
            }
            reasons = options.iter().map(|o| o.1.clone()).collect();
            if i != correct {
                feedback.push(format!("「{}」：{}", options[*i].0, options[*i].1));
            }
            i == correct
        }
        (
            Key::Number {
                value,
                digits,
                unit,
            },
            Answer::Number(text),
        ) => {
            let given = num::parse(text)?;
            let rounded = Rounding::HalfUp.round(given, *digits);
            if rounded != *value {
                feedback.push(format!(
                    "あなたの答えは {}{unit} です。",
                    num::grouped(given)
                ));
                let gap = (rounded - *value).abs();
                if !value.is_zero() && gap * num::hundred() <= value.abs() {
                    feedback.push(
                        "差は1%以内です。端数処理（切り捨て・四捨五入の位置）や、途中で丸めていないかを確かめましょう。"
                            .into(),
                    );
                } else if (rounded + *value).is_zero() {
                    feedback.push(
                        "符号が逆です。納める額と戻る額のどちらを聞いているか確かめましょう。"
                            .into(),
                    );
                }
            }
            rounded == *value
        }
        (Key::Journal(expected), Answer::Journal(lines)) => {
            let mut given: BTreeMap<String, Decimal> = BTreeMap::new();
            let mut debit = Decimal::ZERO;
            let mut credit = Decimal::ZERO;
            for l in lines {
                let code = l.account.trim();
                if code.is_empty() {
                    continue;
                }
                if !s.book.accounts.iter().any(|a| a.code == code) {
                    return Err(invalid(format!(
                        "科目コード {code} はこのシナリオにありません"
                    )));
                }
                let (d, c) = (amount(&l.debit)?, amount(&l.credit)?);
                if d.is_sign_negative() || c.is_sign_negative() {
                    return Err(invalid("金額は0以上で入力してください"));
                }
                debit += d;
                credit += c;
                *given.entry(code.to_string()).or_default() += d - c;
            }
            if given.is_empty() {
                return Err(invalid("仕訳の行を入力してください"));
            }
            if debit != credit {
                feedback.push(format!(
                    "借方合計 {} と貸方合計 {} が一致していません。仕訳は左右の合計が必ず一致します。",
                    num::grouped(debit),
                    num::grouped(credit)
                ));
            }
            let mut want: BTreeMap<String, Decimal> = BTreeMap::new();
            for (code, d, c) in expected {
                *want.entry(code.clone()).or_default() += d - c;
            }
            for (code, w) in &want {
                let name = s.account_name(code);
                match given.get(code) {
                    None => feedback.push(format!("「{name}」が入っていません。")),
                    Some(g) if g == w => {}
                    Some(g) if *g == -*w => {
                        feedback.push(format!("「{name}」の借方と貸方が逆です。"))
                    }
                    Some(g) if g.is_sign_negative() != w.is_sign_negative() => {
                        feedback.push(format!(
                            "「{name}」は{}に書きます。",
                            if w.is_sign_negative() {
                                "貸方"
                            } else {
                                "借方"
                            }
                        ))
                    }
                    Some(_) => feedback.push(format!("「{name}」の金額が違います。")),
                }
            }
            for (code, g) in &given {
                if !want.contains_key(code) && !g.is_zero() {
                    feedback.push(format!(
                        "「{}」はこの取引では使いません。",
                        s.account_name(code)
                    ));
                }
            }
            feedback.is_empty()
        }
        _ => return Err(invalid("この問題の解答の形式が違います")),
    };
    Ok(Grade {
        problem: p.id.clone(),
        correct,
        expected: s.expected(p),
        feedback,
        explanation: p.explanation.clone(),
        reasons,
    })
}

/// 記録された解答。
#[derive(Clone, Debug, Serialize)]
pub struct Attempt {
    pub id: i64,
    pub problem: String,
    pub at: String,
    pub correct: bool,
    pub answer: Value,
}
#[derive(Clone, Debug, Default, Serialize)]
pub struct ProblemProgress {
    pub attempts: usize,
    /// 一度でも正解した
    pub solved: bool,
    /// 最後の解答が正解
    pub last_correct: bool,
}
#[derive(Clone, Debug, Default, Serialize)]
pub struct Score {
    pub problems: usize,
    pub solved: usize,
}
#[derive(Clone, Debug, Serialize)]
pub struct Progress {
    pub problems: BTreeMap<String, ProblemProgress>,
    /// シナリオごと・分野ごとの解けた問題数
    pub scenarios: BTreeMap<&'static str, Score>,
    pub topics: BTreeMap<&'static str, Score>,
    pub total: Score,
    /// 最後に間違えたままの問題（復習用）
    pub review: Vec<String>,
}
pub fn progress(attempts: &[Attempt]) -> Progress {
    let mut problems: BTreeMap<String, ProblemProgress> = BTreeMap::new();
    for a in attempts {
        if find(&a.problem).is_none() {
            continue;
        }
        let p = problems.entry(a.problem.clone()).or_default();
        p.attempts += 1;
        p.solved |= a.correct;
        p.last_correct = a.correct;
    }
    let mut scenarios = BTreeMap::new();
    let mut topics: BTreeMap<&'static str, Score> = BTreeMap::new();
    let mut total = Score::default();
    let mut review = vec![];
    for s in scenarios_all() {
        let score: &mut Score = scenarios.entry(s.id).or_default();
        for p in &s.problems {
            let solved = problems.get(&p.id).is_some_and(|x| x.solved);
            let t = topics.entry(p.topic.label()).or_default();
            for x in [&mut *score, t, &mut total] {
                x.problems += 1;
                x.solved += usize::from(solved);
            }
            if problems.get(&p.id).is_some_and(|x| !x.last_correct) {
                review.push(p.id.clone());
            }
        }
    }
    Progress {
        problems,
        scenarios,
        topics,
        total,
        review,
    }
}

static SCENARIOS: OnceLock<Vec<Scenario>> = OnceLock::new();
/// すべてのシナリオ（初回に組み立てる）。
pub fn scenarios_all() -> &'static [Scenario] {
    SCENARIOS.get_or_init(|| {
        [aoki, sato, yamamoto, komugi]
            .iter()
            .map(|f| f().unwrap_or_else(|e| panic!("study scenario failed to build: {e}")))
            .collect()
    })
}
pub fn scenario(id: &str) -> Option<&'static Scenario> {
    scenarios_all().iter().find(|s| s.id == id)
}
pub fn find(problem_id: &str) -> Option<(&'static Scenario, &'static Problem)> {
    scenarios_all().iter().find_map(|s| {
        s.problems
            .iter()
            .find(|p| p.id == problem_id)
            .map(|p| (s, p))
    })
}
/// 練習用の帳簿の作成依頼と、作成後に順に適用する変更。
pub fn practice(id: &str) -> Result<(NewBook, &'static [Change])> {
    let s = scenario(id).ok_or_else(|| invalid("シナリオがありません"))?;
    Ok((
        NewBook {
            name: format!("学習：{}", s.title),
            kind: s.kind,
            currency: "JPY".into(),
            minor_digits: 0,
            rounding: Rounding::HalfUp,
            fiscal_start_month: 1,
        },
        &s.changes,
    ))
}

// ---------- Building scenarios ----------

fn d(s: &str) -> NaiveDate {
    s.parse().expect("scenario date")
}
fn yen(n: i64) -> Decimal {
    Decimal::from(n)
}
fn g(d: Decimal) -> String {
    num::grouped(d)
}

struct Build {
    book: Book,
    changes: Vec<Change>,
    problems: Vec<Problem>,
    prefix: &'static str,
}
impl Build {
    fn new(prefix: &'static str, name: &str, kind: BookKind) -> Self {
        let request = NewBook {
            name: name.into(),
            kind,
            currency: "JPY".into(),
            minor_digits: 0,
            rounding: Rounding::HalfUp,
            fiscal_start_month: 1,
        };
        Self {
            book: Book::blank(1, &request),
            changes: vec![],
            problems: vec![],
            prefix,
        }
    }
    fn apply(&mut self, change: Value) -> Result<i64> {
        let c: Change = serde_json::from_value(change.clone())?;
        let id = self
            .book
            .apply(&c)
            .map_err(|e| invalid(format!("{e}: {change}")))?;
        self.book.validate()?;
        self.changes.push(c);
        Ok(id.unwrap_or(0))
    }
    fn id(&self, code: &str) -> i64 {
        self.book
            .accounts
            .iter()
            .find(|a| a.code == code)
            .map(|a| a.id)
            .unwrap_or_else(|| panic!("scenario account {code}"))
    }
    fn quick(
        &mut self,
        kind: &str,
        date: &str,
        amount: i64,
        account: &str,
        via: &str,
        text: &str,
    ) -> Result<i64> {
        let (a, v) = (self.id(account), self.id(via));
        self.apply(json!({"operation":"quick_entry","quick":{"kind":kind,"date":date,"amount":amount.to_string(),"account":a,"via":v,"description":text}}))
    }
    fn entry(&mut self, date: &str, text: &str, lines: &[(&str, i64, i64)]) -> Result<i64> {
        let lines: Vec<Value> = lines
            .iter()
            .map(|(c, dr, cr)| json!({"account":self.id(c),"debit":dr.to_string(),"credit":cr.to_string()}))
            .collect();
        self.apply(json!({"operation":"save_entry","entry":{"date":date,"description":text,"lines":lines}}))
    }
    fn push(
        &mut self,
        topic: Topic,
        level: u8,
        title: &str,
        question: String,
        key: Key,
    ) -> &mut Problem {
        let id = format!("{}-{:02}", self.prefix, self.problems.len() + 1);
        self.problems.push(Problem {
            id,
            topic,
            level,
            title: title.into(),
            question,
            hint: String::new(),
            screen: "",
            key,
            explanation: String::new(),
        });
        self.problems.last_mut().unwrap()
    }
    fn finish(
        self,
        id: &'static str,
        title: &'static str,
        subtitle: &'static str,
        story: &[&str],
        facts: &[(&str, String)],
    ) -> Result<Scenario> {
        for p in &self.problems {
            if p.explanation.is_empty() || p.hint.is_empty() || p.screen.is_empty() {
                return Err(invalid(format!(
                    "{}: hint, screen and explanation are required",
                    p.id
                )));
            }
            if let Key::Journal(lines) = &p.key {
                let dr: Decimal = lines.iter().map(|l| l.1).sum();
                let cr: Decimal = lines.iter().map(|l| l.2).sum();
                if dr != cr
                    || lines
                        .iter()
                        .any(|l| !self.book.accounts.iter().any(|a| a.code == l.0))
                {
                    return Err(invalid(format!(
                        "{}: unbalanced or unknown account in key",
                        p.id
                    )));
                }
            }
        }
        Ok(Scenario {
            id,
            title,
            subtitle,
            kind: self.book.kind,
            story: story.iter().map(|s| s.to_string()).collect(),
            facts: facts
                .iter()
                .map(|(k, v)| (k.to_string(), v.clone()))
                .collect(),
            problems: self.problems,
            book: self.book,
            changes: self.changes,
        })
    }
}
impl Problem {
    fn hint(&mut self, text: &str) -> &mut Self {
        self.hint = text.into();
        self
    }
    fn screen(&mut self, screen: &'static str) -> &mut Self {
        self.screen = screen;
        self
    }
    fn explain(&mut self, text: String) -> &mut Self {
        self.explanation = text;
        self
    }
}
fn choice(options: &[(&str, &str)], correct: usize) -> Key {
    Key::Choice {
        options: options
            .iter()
            .map(|(a, b)| (a.to_string(), b.to_string()))
            .collect(),
        correct,
    }
}
fn journal(lines: &[(&str, Decimal, Decimal)]) -> Key {
    Key::Journal(
        lines
            .iter()
            .map(|(c, a, b)| (c.to_string(), *a, *b))
            .collect(),
    )
}
fn number(value: Decimal, digits: u32, unit: &'static str) -> Key {
    Key::Number {
        value: Rounding::HalfUp.round(value, digits),
        digits,
        unit,
    }
}
const Z: Decimal = Decimal::ZERO;

fn it(r: &TaxResult, label: &str) -> Decimal {
    r.lines
        .iter()
        .find(|l| l.label == label)
        .and_then(|l| l.income_tax)
        .map(|v| v.0)
        .unwrap_or_else(|| panic!("tax line {label}"))
}
fn rt(r: &TaxResult, label: &str) -> Decimal {
    r.lines
        .iter()
        .find(|l| l.label == label)
        .and_then(|l| l.resident_tax)
        .map(|v| v.0)
        .unwrap_or(Decimal::ZERO)
}

// ---------- Scenario 1: 新社会人 ----------

fn aoki() -> Result<Scenario> {
    let mut b = Build::new("aoki", "青木陽菜さんの家計", BookKind::Household);
    let loan = b.id("211");
    let opening = b.id("301");
    let scholarship = b.apply(json!({"operation":"save_claim","opening":opening,"claim":{"side":"payable","title":"奨学金（無利子）","account":loan,"principal":"2400000","issued":"2026-03-31","due":"2026-10-27","annual_rate":"0","repayment":"equal_principal","installments":240,"memo":"10月から毎月27日に10,000円"}}))?;
    b.entry(
        "2026-03-31",
        "記帳開始時の残高",
        &[("101", 15_000, 0), ("111", 320_000, 0), ("301", 0, 335_000)],
    )?;
    let (rent, bank) = (b.id("521"), b.id("111"));
    let rent_plan = b.apply(json!({"operation":"save_plan","plan":{"name":"家賃","debit":rent,"credit":bank,"amount":"72000","start":"2026-04-27","frequency":"monthly"}}))?;
    let nisa = b.id("131");
    let nisa_plan = b.apply(json!({"operation":"save_plan","plan":{"name":"つみたて投資枠（NISA）","debit":nisa,"credit":bank,"amount":"30000","start":"2026-06-12","frequency":"monthly"}}))?;
    // 4月
    b.quick(
        "expense",
        "2026-04-02",
        89_800,
        "641",
        "201",
        "冷蔵庫・洗濯機・電子レンジ",
    )?;
    b.quick(
        "expense",
        "2026-04-03",
        6_280,
        "511",
        "201",
        "日用品のまとめ買い",
    )?;
    for (day, v) in [("05", 5_420), ("12", 4_980), ("19", 6_130), ("26", 5_870)] {
        b.quick(
            "expense",
            &format!("2026-04-{day}"),
            v,
            "501",
            "201",
            "スーパー",
        )?;
    }
    b.quick("expense", "2026-04-18", 3_200, "502", "201", "同期とランチ")?;
    // 5月
    b.quick("transfer", "2026-05-06", 20_000, "101", "111", "ATM引出し")?;
    b.quick("expense", "2026-05-05", 3_300, "591", "201", "書籍")?;
    for (day, v) in [
        ("03", 6_240),
        ("10", 5_560),
        ("17", 6_880),
        ("24", 5_120),
        ("31", 6_600),
    ] {
        b.quick(
            "expense",
            &format!("2026-05-{day}"),
            v,
            "501",
            "201",
            "スーパー",
        )?;
    }
    b.quick("expense", "2026-05-16", 4_500, "502", "201", "外食")?;
    b.quick(
        "expense",
        "2026-05-20",
        2_980,
        "511",
        "201",
        "ドラッグストア",
    )?;
    b.quick(
        "expense",
        "2026-05-23",
        6_000,
        "611",
        "101",
        "友人の誕生日プレゼント",
    )?;
    // 6月
    for (day, v) in [("07", 5_900), ("14", 6_450), ("21", 5_780), ("28", 6_270)] {
        b.quick(
            "expense",
            &format!("2026-06-{day}"),
            v,
            "501",
            "201",
            "スーパー",
        )?;
    }
    b.quick("expense", "2026-06-13", 3_800, "502", "201", "外食")?;
    b.quick(
        "expense",
        "2026-06-18",
        3_150,
        "511",
        "201",
        "ドラッグストア",
    )?;
    for (month, utility) in [("04", 4_800), ("05", 6_200), ("06", 5_600)] {
        b.quick(
            "income",
            &format!("2026-{month}-25"),
            192_000,
            "401",
            "111",
            "給与（手取り）",
        )?;
        b.apply(json!({"operation":"post_plan","plan":rent_plan,"occurrence":format!("2026-{month}-27")}))?;
        b.quick(
            "expense",
            &format!("2026-{month}-28"),
            4_980,
            "541",
            "111",
            "スマホ",
        )?;
        b.quick(
            "expense",
            &format!("2026-{month}-29"),
            utility,
            "531",
            "111",
            "電気・ガス",
        )?;
    }
    let april_card = 89_800 + 6_280 + 22_400 + 3_200;
    let may_card = 3_300 + 30_400 + 4_500 + 2_980;
    b.quick(
        "transfer",
        "2026-05-10",
        april_card,
        "201",
        "111",
        "カード引落し（4月分）",
    )?;
    b.quick(
        "transfer",
        "2026-06-10",
        may_card,
        "201",
        "111",
        "カード引落し（5月分）",
    )?;
    b.apply(json!({"operation":"post_plan","plan":nisa_plan,"occurrence":"2026-06-12"}))?;

    let mut tr = TaxReturn::new(2026);
    tr.name = "2026年分（入社1年目）".into();
    tr.salary = yen(2_370_000);
    tr.salary_withheld = yen(25_600);
    tr.social_insurance = yen(355_500);
    tr.medical_paid = yen(140_000);
    b.apply(json!({"operation":"save_tax_return","tax_return":tr}))?;
    let r = tax::calculate(&b.book, &tr)?;
    let salary_income = it(&r, "給与所得（給与所得控除後）");
    let medical = it(&r, "医療費控除");
    let refund = -r.balance_due.0;

    let bs = reports::balance_sheet(&b.book, d("2026-06-30"));
    let bank_balance = bs
        .assets
        .iter()
        .find(|a| a.code == "111")
        .map(|a| a.amount.0)
        .unwrap_or_default();
    let st = reports::statistics(&b.book, d("2026-04-01"), d("2026-06-30"))?;
    let rate = st.savings_rate.map(|r| r.0).unwrap_or_default();
    let schedule = claims::schedule(
        &b.book,
        b.book.claims.iter().find(|c| c.id == scholarship).unwrap(),
    )?;
    let last_year = schedule.last().map(|i| i.date).unwrap();
    let af = lifeplan::coefficients(yen(3), 30)?;
    let sf = lifeplan::coefficients(Decimal::ONE, 5)?;

    b.push(Topic::Accounting, 1, "家電を買った", "4月2日、引越しにあわせて冷蔵庫・洗濯機・電子レンジを家電量販店で89,800円で買い、クレジットカードで払いました。費用の科目はどれが適切ですか。".into(),
        choice(&[("日用品", "日用品は洗剤やティッシュなど短期間で使い切る物です。何年も使う家電は分けておくと、毎月の生活費の傾向が読みやすくなります。"),
                 ("住居費", "住居費は家賃・管理費など住まいそのものの費用です。家具・家電の購入は含めません。"),
                 ("特別支出", "正解です。年に数回の大きな支出・1年以上使う家具や家電は特別支出に分けます。統計や予測で毎月の生活費と混ざらなくなります。"),
                 ("雑費", "雑費は他に当てはまらない少額の支出です。金額が大きく内容がはっきりしている支出には使いません。")], 2))
        .hint("「何年も使う大きな買い物」は毎月の生活費と分けて記録すると、予算や予測が狂いません。")
        .screen("かんたん入力")
        .explain("家計簿では、毎月くり返す生活費と、年に数回の大きな支出を分けるのがコツです。この帳簿では「特別支出」がその受け皿です。支払いはカードなので、仕訳は「借方 特別支出 89,800 ／ 貸方 クレジットカード 89,800」になります。".into());
    b.push(Topic::Accounting, 1, "初任給の仕訳", "4月25日、初任給の手取り192,000円が普通預金に振り込まれました。この取引の仕訳を入力してください。".into(),
        journal(&[("111", yen(192_000), Z), ("401", Z, yen(192_000))]))
        .hint("お金が入った口座（普通預金）が借方、お金の出どころ（給与収入）が貸方です。")
        .screen("仕訳帳")
        .explain("増えた資産（普通預金）を借方、その原因である収益（給与収入）を貸方に書きます。家計簿では手取り額で記録するのがいちばん簡単です。額面で記録して税金・社会保険料を費用にすると、負担額の推移も分かります。".into());
    b.push(Topic::Accounting, 2, "カードの引落し", format!("5月10日、4月にクレジットカードで使った {} 円が普通預金から引き落とされました。この取引の仕訳を入力してください。", g(yen(april_card))),
        journal(&[("201", yen(april_card), Z), ("111", Z, yen(april_card))]))
        .hint("食費や家電の支出は、カードで買った日にもう記録してあります。引落しで減るのは「カードの未払い残高」です。")
        .screen("かんたん入力（振替）")
        .explain("カードで買った時点で「費用／クレジットカード（負債）」と記録しているので、引落し日は負債を減らして預金を減らす振替になります。ここで食費などの費用を書くと、同じ支出を2回数えてしまいます（二重計上）。かんたん入力では種類を「振替」、移動先をクレジットカード、移動元を普通預金にします。".into());
    b.push(Topic::Accounting, 1, "友人へのプレゼント", "5月23日、友人の誕生日プレゼントを現金6,000円で買いました。費用の科目はどれが適切ですか。".into(),
        choice(&[("外食費", "外食費は自分や家族の食事です。"),
                 ("交際費", "正解です。他人のための支出（贈り物・ご祝儀・会食）は交際費です。"),
                 ("特別支出", "特別支出は自分の家の家電や家具など大きな買い物です。"),
                 ("衣服・美容", "自分が身に着ける物ではありません。")], 1))
        .hint("「誰のための支出か」で考えます。")
        .screen("勘定科目・取引先（使い分けガイド）")
        .explain("「食費・外食費・交際費」の使い分けと同じく、他人のための支出は交際費にまとめると、付き合いにかかるお金が見えるようになります。仕訳は「借方 交際費 6,000 ／ 貸方 現金 6,000」です。".into());
    b.push(Topic::Accounting, 1, "NISAで積立", "6月12日、NISAのつみたて投資枠で投資信託を30,000円分、普通預金から買い付けました。この取引の仕訳を入力してください。".into(),
        journal(&[("131", yen(30_000), Z), ("111", Z, yen(30_000))]))
        .hint("お金は使ってなくなったのではなく、預金から投資信託という別の資産に移っただけです。")
        .screen("計画")
        .explain("投資信託の購入は支出ではなく、資産から資産への振替です（借方 投資・証券口座 ／ 貸方 普通預金）。支出にすると貯蓄しているのに家計が赤字に見えてしまいます。毎月の積立は「計画」に登録して計上すると、予測でも予定として扱われます。".into());
    b.push(Topic::Accounting, 2, "6月末の預金残高", "シナリオの帳簿（4〜6月の取引を記録済み）で、2026年6月30日時点の普通預金の残高はいくらですか。".into(),
        number(bank_balance, 0, "円"))
        .hint("帳票の貸借対照表を6月30日で表示するか、総勘定元帳で普通預金を見ます。")
        .screen("帳票")
        .explain(format!("開始時320,000円に、給与3回分576,000円が入り、家賃（72,000円×3）、スマホ（4,980円×3）、電気・ガス（16,600円）、カード引落し2回（{}円・{}円）、ATM引出し20,000円、NISA 30,000円が出ていきます。6月にカードで使った31,350円はまだ引き落とされていないので、預金は減っていません（7月10日に減ります）。残高は {} 円です。", g(yen(april_card)), g(yen(may_card)), g(bank_balance)));
    b.push(Topic::Accounting, 2, "貯蓄率", "4〜6月の3か月間の貯蓄率（（収入−支出）÷収入）は何%ですか。小数第1位まで答えてください。".into(),
        number(rate, 1, "%"))
        .hint("統計・予測の画面で期間を4月1日〜6月30日にすると、収入・支出の合計と貯蓄率が表示されます。")
        .screen("統計・予測")
        .explain(format!("収入は給与576,000円、支出は家電89,800円を含めて{}円です。（{}−{}）÷{} ＝ {}% になります。カード引落し・ATM引出し・NISAは振替なので支出に入りません。家電を除いた毎月の生活費で見ると貯蓄率はさらに高くなります。", g(st.total_expense.0), g(st.total_income.0), g(st.total_expense.0), g(st.total_income.0), num::text(rate)));
    b.push(Topic::Accounting, 2, "奨学金の返済", "10月27日から奨学金の返済が始まり、毎月10,000円が普通預金から引き落とされます。借方に書く科目はどれですか。".into(),
        choice(&[("教育費", "教育費は今の学費や教材です。昔借りたお金の返済は費用ではありません。"),
                 ("ローン・借入金", "正解です。借りたお金（負債）を返すので、負債を減らす借方に書きます。"),
                 ("支払利息", "無利子の奨学金なので利息はありません。利息がある借入では、利息の部分だけが支払利息です。"),
                 ("開始残高（純資産）", "開始残高は記帳を始めたときに一度だけ使う科目です。")], 1))
        .hint("借りたときに「負債」として記録した残高を、返済のたびに減らしていきます。")
        .screen("債権・債務")
        .explain("仕訳は「借方 ローン・借入金 10,000 ／ 貸方 普通預金 10,000」です。シナリオでは奨学金を債権・債務に登録してあるので、返済予定表で毎月の元本と残高を確認でき、回収・返済ボタンで同じ仕訳を作れます。".into());
    b.push(Topic::Tax, 1, "給与所得", "2026年分の給与の支払金額（額面）は2,370,000円です（4〜12月の給与2,070,000円と冬の賞与300,000円）。給与所得控除後の金額（給与所得）はいくらですか。".into(),
        number(salary_income, 0, "円"))
        .hint("給与には経費の代わりに「給与所得控除」があります。給与収入が660万円未満のときは、国税庁の「給与所得控除後の給与等の金額の表」（別表第五）で求めます。収入を4,000円単位に切り捨ててから計算するのがポイントです。")
        .screen("税金")
        .explain(format!("給与収入190万円超360万円以下の給与所得控除は「収入×30%＋8万円」、つまり給与所得は「収入×70%−8万円」です。660万円未満は別表第五に従い、収入を4,000円単位に切り捨てた 2,368,000円で計算します。2,368,000×70%−80,000 ＝ {} 円です。2,370,000円のまま計算した1,579,000円とは1,400円ずれるので注意しましょう。税金はこの「所得」から計算が始まります。", g(salary_income)));
    b.push(Topic::Tax, 2, "医療費控除", "親知らずの抜歯と矯正の検査で、2026年に医療費を140,000円払いました（保険金などの補塡はありません）。医療費控除の額はいくらですか。".into(),
        number(medical, 0, "円"))
        .hint("医療費控除は「医療費−10万円」とは限りません。総所得金額が200万円未満なら、10万円の代わりに所得の5%を引きます。")
        .screen("税金")
        .explain(format!("引く額は「10万円」と「総所得金額等の5%」の少ない方です。所得は {} 円なので5%は {} 円で、10万円より少なくなります。140,000−{} ＝ {} 円が医療費控除です。所得が少ない1年目は、10万円以下の医療費でも控除が受けられることがあります。", g(salary_income), g(salary_income * yen(5) / num::hundred()), g(salary_income * yen(5) / num::hundred()), g(medical)));
    b.push(Topic::Tax, 1, "医療費控除の手続き", "陽菜さんが医療費控除を受けるには、どうすればよいですか。".into(),
        choice(&[("会社の年末調整で申し出る", "年末調整で受けられるのは生命保険料控除などで、医療費控除は対象外です。"),
                 ("翌年に確定申告をする", "正解です。医療費控除は確定申告で受けます（明細書を付け、領収書は5年間保存）。"),
                 ("何もしなくても自動で反映される", "医療費の情報は自動では反映されません。自分で申告します。"),
                 ("住民税の申告だけをする", "住民税だけの申告では所得税の還付は受けられません。確定申告をすれば住民税にも反映されます。")], 1))
        .hint("年末調整で扱える控除と、扱えない控除があります。")
        .screen("税金")
        .explain("会社員でも、医療費控除・ふるさと納税（6自治体以上やワンストップ特例を使わない場合）・住宅ローン控除の1年目は確定申告が必要です。確定申告をすると、その内容が住民税にも自動で反映されます。".into());
    b.push(Topic::Tax, 2, "還付される額", "給与の源泉徴収税額（年末調整後）は25,600円、社会保険料は355,500円です。医療費控除を受けて確定申告をすると、所得税はいくら戻りますか（還付額を正の数で）。".into(),
        number(refund, 0, "円"))
        .hint("税金の画面で、給与・源泉徴収税額・社会保険料・医療費を入れて計算します。")
        .screen("税金")
        .explain(format!("医療費控除で課税所得が減ると、所得税（復興特別所得税を含む）が {} 円になります。すでに源泉徴収された25,600円との差 {} 円が還付されます。住民税も翌年度に軽くなります（住民税の年税額の見込み {} 円）。", g(r.income_tax.0), g(refund), g(r.resident_tax.0)));
    b.push(Topic::LifePlan, 1, "積立の将来値", format!("NISAで毎月30,000円（年360,000円）を、年3%で運用しながら30年間積み立てると、30年後にいくらになりますか（年1回の積立とみなし、年金終価係数 {} を使います）。円未満は四捨五入。", num::text(af.annuity_future.0)),
        number(af.annuity_future.0 * yen(360_000), 0, "円"))
        .hint("毎年の積立額 × 年金終価係数 で求めます。")
        .screen("ライフプラン（FP電卓）")
        .explain(format!("360,000 × {} ＝ {} 円です。積み立てた元本は1,080万円なので、差額が運用で増えた分です（実際の運用成果は変動し、元本割れもありえます）。", num::text(af.annuity_future.0), g((af.annuity_future.0 * yen(360_000)).round())));
    b.push(Topic::LifePlan, 2, "目標額のための積立", format!("5年後に留学費用1,500,000円を用意したいと考えています。年1%で運用できるとすると、毎年いくら積み立てればよいですか（減債基金係数 {} を使います）。円未満は四捨五入。", num::text(sf.sinking_fund.0)),
        number(sf.sinking_fund.0 * yen(1_500_000), 0, "円"))
        .hint("目標額 × 減債基金係数 ＝ 毎年の積立額 です。")
        .screen("ライフプラン（FP電卓）")
        .explain(format!("1,500,000 × {} ＝ {} 円です。利回りが0%なら毎年300,000円必要なので、運用でその差が小さくなっています。月にすると約{}円です。", num::text(sf.sinking_fund.0), g((sf.sinking_fund.0 * yen(1_500_000)).round()), g((sf.sinking_fund.0 * yen(1_500_000) / yen(12)).round())));
    b.push(Topic::LifePlan, 1, "奨学金の完済", "奨学金2,400,000円を2026年10月から毎月10,000円ずつ返すと、最後の返済は西暦何年ですか。".into(),
        number(Decimal::from(chrono::Datelike::year(&last_year)), 0, "年"))
        .hint("返済回数は 2,400,000÷10,000 ＝ 240回（20年）です。債権・債務の返済予定表でも確かめられます。")
        .screen("債権・債務")
        .explain(format!("240回目の返済日は {} です。陽菜さんは43歳まで返済が続くので、ライフプランの支出に年120,000円を入れておきます。繰上げ返済をすると早く終わりますが、無利子の奨学金は利息が減らないので、預金や投資を優先する考え方もあります。", last_year));
    b.push(Topic::LifePlan, 1, "先取り貯蓄", "手取り192,000円から毎月30,000円を確実に貯めたいと考えています。いちばん続けやすい方法はどれですか。".into(),
        choice(&[("月末に残ったお金を貯金する", "残った分だけを貯める方法は、使い過ぎた月に貯金がゼロになりがちです。"),
                 ("給料日に自動で積立口座や投資に移す", "正解です。先に貯める分を移してから残りで暮らす「先取り貯蓄」は、意志に頼らず続けられます。"),
                 ("ボーナスでまとめて貯める", "ボーナスは金額や支給が変わることがあります。毎月の仕組みと組み合わせるのが安全です。"),
                 ("クレジットカードの利用限度額を下げる", "使い過ぎの予防にはなりますが、貯める仕組みにはなりません。")], 1))
        .hint("意志の力に頼らない仕組みはどれでしょう。")
        .screen("計画")
        .explain("給料日の直後に自動で積立・振替する先取り貯蓄は、家計管理の基本です。このアプリでは「計画」に毎月の積立を登録し、予定日に計上すると、予算・予測にも反映されます。".into());
    let facts = [
        ("人物", "青木陽菜さん（2003年生まれ、23歳）。2026年4月に入社し、都内で一人暮らし".to_string()),
        ("3月31日の残高", "普通預金 320,000円、現金 15,000円".into()),
        ("奨学金", format!("無利子 2,400,000円。2026年10月から毎月27日に10,000円×240回（最終 {}）", last_year)),
        ("給与", "毎月25日、額面230,000円・手取り192,000円。12月に賞与（額面300,000円）".into()),
        ("固定費", "家賃72,000円（27日・口座振替）、スマホ4,980円、電気・ガス（月4,800〜6,200円）".into()),
        ("支払方法", "買い物はクレジットカード（月末締め・翌月10日引落し）".into()),
        ("2026年分の税金", "給与2,370,000円、社会保険料355,500円、源泉徴収税額（年末調整後）25,600円、医療費140,000円".into()),
    ];
    b.finish("aoki", "青木陽菜さんの1年目", "新社会人の家計簿・はじめての確定申告・積立の計画", &[
        "陽菜さんは2026年4月に社会人になり、初めての一人暮らしを始めました。引越しで家電をそろえ、給料日・家賃・カードの引落しが毎月めぐってきます。",
        "「お金が貯まっているのか分からない」という不安から家計簿を始めることにしました。年末には歯の治療費がかさみ、医療費控除で税金が戻るのかも気になっています。",
        "奨学金の返済が10月から始まり、NISAでの積立や数年後の留学も考えています。",
    ], &facts,)
}

// ---------- Scenario 2: 会社員＋副業 ----------

fn sato() -> Result<Scenario> {
    let mut b = Build::new("sato", "佐藤デザイン（副業）", BookKind::SoleProprietor);
    b.entry(
        "2026-01-05",
        "開業時の元入金（個人の口座から）",
        &[("111", 300_000, 0), ("301", 0, 300_000)],
    )?;
    let client = b.apply(json!({"operation":"save_partner","partner":{"name":"株式会社アトリエA","note":"報酬から源泉徴収あり。月末締め翌月末払い"}}))?;
    for m in 1..=12 {
        b.quick(
            "expense",
            &format!("2026-{m:02}-05"),
            6_480,
            "528",
            "111",
            "デザインソフト利用料",
        )?;
        b.quick(
            "expense",
            &format!("2026-{m:02}-25"),
            27_500,
            "526",
            "311",
            "家賃110,000円のうち仕事部屋分25%（個人の口座から）",
        )?;
        b.quick(
            "expense",
            &format!("2026-{m:02}-23"),
            4_400,
            "515",
            "311",
            "スマホ・光回線8,800円のうち事業分50%（個人のカード）",
        )?;
    }
    for m in [2, 3, 4, 5, 6, 8, 9, 11] {
        b.quick(
            "expense",
            &format!("2026-{m:02}-15"),
            1_320,
            "530",
            "311",
            "打合せのカフェ代（個人の財布）",
        )?;
    }
    b.entry(
        "2026-07-10",
        "デザイン用パソコン",
        &[("164", 352_000, 0), ("111", 0, 352_000)],
    )?;
    let pc = b.apply(json!({"operation":"save_equipment","equipment":{"name":"デザイン用パソコン","acquired":"2026-07-10","cost":"352000","salvage":"0","life_years":4,"annual_hours":"1200","memo":"法定耐用年数4年（パソコン）"}}))?;
    b.quick(
        "expense",
        "2026-09-12",
        25_000,
        "532",
        "111",
        "オンライン講座",
    )?;
    let (recv, draw, sales, bank) = (b.id("121"), b.id("191"), b.id("401"), b.id("111"));
    for (bill, paid) in [
        ("2026-03-31", Some("2026-04-30")),
        ("2026-06-30", Some("2026-07-31")),
        ("2026-09-30", Some("2026-10-30")),
        ("2026-12-28", None),
    ] {
        b.apply(json!({"operation":"save_entry","entry":{"date":bill,"description":"ロゴ・Webデザイン制作（源泉徴収あり）","partner":client,"lines":[
            {"account":recv,"debit":"296307"},{"account":draw,"debit":"33693","memo":"源泉徴収された所得税"},{"account":sales,"credit":"330000"}]}}))?;
        if let Some(paid) = paid {
            b.apply(json!({"operation":"save_entry","entry":{"date":paid,"description":"アトリエA 入金","partner":client,"lines":[{"account":bank,"debit":"296307"},{"account":recv,"credit":"296307"}]}}))?;
        }
    }
    for date in ["2026-05-20", "2026-08-20", "2026-11-20"] {
        b.quick(
            "income",
            date,
            110_000,
            "401",
            "111",
            "個人のお客様の名刺・チラシ",
        )?;
    }
    b.apply(json!({"operation":"post_depreciation","equipment":pc,"fiscal_year":2026,"date":"2026-12-31"}))?;

    let pl = reports::income_statement(&b.book, d("2026-01-01"), d("2026-12-31"));
    let profit = pl.net_income.0;
    let bs = reports::balance_sheet(&b.book, d("2026-12-31"));
    let receivable = bs
        .assets
        .iter()
        .find(|a| a.code == "121")
        .map(|a| a.amount.0)
        .unwrap_or_default();
    let depreciation = cost::depreciation(
        &b.book,
        b.book.equipment.iter().find(|e| e.id == pc).unwrap(),
        2026,
    )?;

    let mut tr = TaxReturn::new(2026);
    tr.name = "2026年分（青色申告）".into();
    tr.salary = yen(5_200_000);
    tr.salary_withheld = yen(128_000);
    tr.social_insurance = yen(780_000);
    tr.mutual_aid = yen(276_000);
    tr.side_kind = SideKind::BlueBusiness;
    tr.side_revenue = pl.total_income.0;
    tr.side_expenses = pl.total_expense.0;
    tr.side_withheld = yen(134_772);
    tr.blue_deduction = yen(650_000);
    b.apply(json!({"operation":"save_tax_return","tax_return":tr}))?;
    let blue = tax::calculate(&b.book, &tr)?;
    let mut misc = tr.clone();
    misc.side_kind = SideKind::Miscellaneous;
    misc.blue_deduction = Z;
    let misc_r = tax::calculate(&b.book, &misc)?;
    let mut no_ideco = tr.clone();
    no_ideco.mutual_aid = Z;
    let no_ideco_r = tax::calculate(&b.book, &no_ideco)?;
    let total = |r: &TaxResult| r.income_tax.0 + r.resident_tax.0;
    let blue_saving = total(&misc_r) - total(&blue);
    let ideco_saving = total(&no_ideco_r) - total(&blue);
    let balance = blue.balance_due.0;
    let salary_income = it(&blue, "給与所得（給与所得控除後）");

    b.push(Topic::Accounting, 1, "パソコンの購入", "7月10日、デザイン用のパソコンを352,000円で買い、事業用の普通預金から払いました。借方の科目はどれですか（佐藤さんは青色申告者です）。".into(),
        choice(&[("消耗品費", "10万円未満（または使用期間1年未満）なら消耗品費ですが、352,000円なので固定資産になります。"),
                 ("工具器具備品", "正解です。10万円以上で1年以上使うパソコンは固定資産として計上し、耐用年数（パソコンは4年）にわたって減価償却します。"),
                 ("減価償却費", "減価償却費は決算で「その年の分」を費用にするときに使います。買ったときの科目ではありません。"),
                 ("事業主貸", "事業のために買ったので事業主貸（私的な支出）ではありません。")], 1))
        .hint("取得価額10万円・20万円・30万円という区切りがあります。青色申告者の少額減価償却資産の特例は30万円未満が対象です。")
        .screen("勘定科目・取引先（使い分けガイド）")
        .explain("仕訳は「借方 工具器具備品 352,000 ／ 貸方 普通預金 352,000」です。10万円未満なら消耗品費、20万円未満なら一括償却資産（3年均等）、青色申告者は30万円未満なら少額減価償却資産の特例で全額をその年の経費にできます（年300万円まで。上限や期限は税制改正で変わるので確認してください）。352,000円はどれにも当たらないので、通常の減価償却をします。".into());
    b.push(Topic::Accounting, 2, "減価償却費", "このパソコン（352,000円、耐用年数4年、定額法、7月10日から使用）の2026年分の減価償却費はいくらですか。".into(),
        number(depreciation, 0, "円"))
        .hint("定額法では、1年分は 取得価額÷耐用年数。使い始めた年は月数で按分します（7月〜12月の6か月。月の途中でも1か月とします）。")
        .screen("原価管理（設備）")
        .explain(format!("1年分は 352,000÷4 ＝ 88,000円（定額法の償却率0.250と同じ）。2026年は7〜12月の6か月なので 88,000×6/12 ＝ {} 円です。仕訳は「借方 減価償却費 ／ 貸方 減価償却累計額」。原価管理の設備画面の「仕訳を計上」で作れます。", g(depreciation)));
    b.push(Topic::Accounting, 2, "家賃の家事按分", "自宅マンションの家賃110,000円は、個人の口座から払っています。仕事部屋は床面積の25%です。事業の帳簿に記録する毎月の仕訳を入力してください。".into(),
        journal(&[("526", yen(27_500), Z), ("311", Z, yen(27_500))]))
        .hint("事業の帳簿には事業分だけを記録します。事業用ではないお金（個人の口座）で払ったときの相手科目は何でしょう。")
        .screen("仕訳帳")
        .explain("事業分 110,000×25% ＝ 27,500円を地代家賃にし、個人のお金で払ったので貸方は事業主借です。もし事業用口座から全額を払っていたなら、「借方 地代家賃 27,500・事業主貸 82,500 ／ 貸方 普通預金 110,000」となります（かんたん入力の事業割合25%で自動で分かれます）。按分の割合は床面積や使用時間など説明できる基準で決めます。".into());
    b.push(Topic::Accounting, 3, "源泉徴収のある売上", "3月31日、株式会社アトリエAにデザイン料330,000円を請求しました。法人から個人へのデザイン料は10.21%が源泉徴収され、入金は翌月末に296,307円になります。請求時の仕訳を入力してください。".into(),
        journal(&[("121", yen(296_307), Z), ("191", yen(33_693), Z), ("401", Z, yen(330_000))]))
        .hint("売上は源泉徴収前の330,000円です。差し引かれる33,693円は、佐藤さん個人の所得税の前払いです。")
        .screen("仕訳帳")
        .explain("330,000×10.21% ＝ 33,693円（1円未満切り捨て）が源泉徴収されます。売上高は総額の330,000円、受け取る予定の296,307円が売掛金、源泉所得税は個人の所得税の前払いなので事業主貸（または仮払税金）にします。確定申告で所得税から差し引かれ、払い過ぎなら還付されます。".into());
    b.push(Topic::Accounting, 1, "打合せのカフェ代", "取引先との打合せで入ったカフェの代金1,320円を、個人の財布から払いました。正しい仕訳はどれですか。".into(),
        choice(&[("借方 会議費 ／ 貸方 事業主借", "正解です。打合せのための飲食は会議費、個人のお金で払ったので事業主借です。"),
                 ("借方 接待交際費 ／ 貸方 現金", "もてなしが目的ではなく打合せなので会議費が適切です。また事業用の現金からは払っていません。"),
                 ("借方 会議費 ／ 貸方 事業主貸", "事業主貸は事業のお金を私的に使ったときの科目です。向きが逆です。"),
                 ("記録しない", "事業のための支出なので、個人のお金で払っても経費として記録できます。")], 0))
        .hint("お金がどこから出たか（事業用か個人か）と、何のための支出か、の2つを決めます。")
        .screen("勘定科目・取引先（使い分けガイド）")
        .explain("「事業主貸と事業主借」「接待交際費・会議費・福利厚生費」の使い分けの組み合わせです。会議費は相手・人数・目的をメモに残しておくと説明しやすくなります。".into());
    b.push(Topic::Accounting, 1, "経費にならない税金", "会社員としての住民税を、うっかり事業用の普通預金から払いました。借方の科目はどれですか。".into(),
        choice(&[("租税公課", "租税公課は事業税や事業用の固定資産税・印紙税など事業の税金です。住民税は個人の税金なので経費になりません。"),
                 ("事業主貸", "正解です。事業のお金を個人のために使ったので事業主貸です。"),
                 ("未払金", "支払いが済んでいるので未払金ではありません。"),
                 ("事業主借", "事業主借は個人のお金を事業に入れたときです。")], 1))
        .hint("所得税・住民税・国民年金・国民健康保険は事業の経費になりません。")
        .screen("勘定科目・取引先（使い分けガイド）")
        .explain("仕訳は「借方 事業主貸 ／ 貸方 普通預金」です。事業主貸・事業主借は年末に元入金と相殺されるので、私的な出入りをこまめに記録しても困りません。".into());
    b.push(Topic::Accounting, 2, "年末の売掛金", "シナリオの帳簿で、2026年12月31日時点の売掛金の残高はいくらですか。".into(),
        number(receivable, 0, "円"))
        .hint("12月28日に請求した分は、入金が翌年1月末です。貸借対照表で確かめます。")
        .screen("帳票")
        .explain(format!("3月・6月・9月の請求分は入金済みで、12月28日の請求分 {} 円が残っています。売上は請求した（仕事を終えた）年に計上するので、入金が翌年でも2026年分の売上です（発生主義）。", g(receivable)));
    b.push(Topic::Accounting, 2, "事業の所得", "シナリオの帳簿の2026年の損益計算書で、収益合計から費用合計を引いた額（青色申告特別控除の前の所得）はいくらですか。".into(),
        number(profit, 0, "円"))
        .hint("帳票の損益計算書を2026年1月1日〜12月31日で表示します。")
        .screen("帳票")
        .explain(format!("収益は売上 {} 円、費用は地代家賃330,000円・通信費52,800円・支払手数料77,760円・会議費10,560円・研修費25,000円・減価償却費{}円の合計 {} 円で、差し引き {} 円です。パソコン本体の352,000円は費用ではなく、減価償却費の分だけが費用になります。", g(pl.total_income.0), g(depreciation), g(pl.total_expense.0), g(profit)));
    b.push(Topic::Tax, 2, "合計所得金額", format!("給与の支払金額は5,200,000円です。副業を事業所得（青色申告、65万円控除）として申告するとき、合計所得金額はいくらですか（事業の所得は前の問題の {} 円です）。", g(profit)),
        number(blue.total_income.0, 0, "円"))
        .hint("給与所得（5,200,000×20%＋44万円の給与所得控除を引く）と、事業所得（所得−65万円）を合計します。")
        .screen("税金")
        .explain(format!("給与所得は 5,200,000−（5,200,000×20%＋440,000）＝ {} 円、事業所得は {}−650,000 ＝ {} 円で、合計 {} 円です。合計所得金額は基礎控除や配偶者控除の判定に使う大事な数字です。", g(salary_income), g(profit), g(profit - yen(650_000)), g(blue.total_income.0)));
    b.push(Topic::Tax, 2, if balance.is_sign_negative() { "確定申告で戻る額" } else { "確定申告で納める額" },
        format!("社会保険料780,000円、iDeCo 276,000円、給与の源泉徴収税額128,000円、アトリエAからの報酬の源泉徴収税額134,772円です。確定申告で所得税は{}（正の数で答えてください）。", if balance.is_sign_negative() { "いくら還付されますか" } else { "いくら納めますか" }),
        number(balance.abs(), 0, "円"))
        .hint("税金の画面で、給与・副業（事業所得・青色）・源泉徴収税額・社会保険料・小規模企業共済等掛金を入れて計算します。")
        .screen("税金")
        .explain(format!("所得税及び復興特別所得税は {} 円、源泉徴収税額は 128,000＋134,772 ＝ 262,772 円です。差の {} 円が{}。報酬から天引きされた源泉所得税は、申告で精算する前払いだと分かります。住民税の見込みは {} 円です。", g(blue.income_tax.0), g(balance.abs()), if balance.is_sign_negative() { "還付されます" } else { "納める額です" }, g(blue.resident_tax.0)));
    b.push(Topic::Tax, 3, "青色申告の効果", "同じ副業を、帳簿を簡単にして雑所得（業務）として申告した場合と比べると、青色申告（65万円控除）の方が所得税と住民税の合計はいくら少なくなりますか。".into(),
        number(blue_saving, 0, "円"))
        .hint("税金の画面で副業の種類だけを「雑所得（業務）」に変えて計算し、所得税＋住民税の合計を比べます。")
        .screen("税金")
        .explain(format!("雑所得では所得税 {} 円・住民税 {} 円、青色申告では所得税 {} 円・住民税 {} 円です。65万円の控除で課税所得が減り、合計 {} 円少なくなります。65万円控除には、事業所得であること、複式簿記で記帳し貸借対照表を添付すること、e-Taxで申告する（または優良な電子帳簿保存をする）ことが必要です。", g(misc_r.income_tax.0), g(misc_r.resident_tax.0), g(blue.income_tax.0), g(blue.resident_tax.0), g(blue_saving)));
    b.push(Topic::Tax, 2, "iDeCoの節税効果", "iDeCoの掛金276,000円（月23,000円）によって、所得税と住民税の合計はいくら少なくなっていますか。".into(),
        number(ideco_saving, 0, "円"))
        .hint("小規模企業共済等掛金を0にした計算と比べます。掛金は全額が所得控除になります。")
        .screen("税金")
        .explain(format!("掛金を0にすると税金の合計は {} 円、掛金276,000円では {} 円で、差は {} 円です。このケースでは所得税の税率10%（復興特別所得税を含めて10.21%）と住民税10%を合わせた約20%分、掛金に対して税金が減ります（276,000×20.21% ≒ 55,780）。iDeCoは原則60歳まで引き出せないので、生活防衛資金を確保した上で使います。", g(total(&no_ideco_r)), g(total(&blue)), g(ideco_saving)));
    b.push(Topic::Tax, 1, "青色申告特別控除の条件", "青色申告特別控除の65万円を受けるための条件として正しいものはどれですか。".into(),
        choice(&[("白色申告で売上を記録しておけばよい", "白色申告には青色申告特別控除はありません。"),
                 ("複式簿記で記帳し、貸借対照表を添付してe-Taxで期限内に申告する", "正解です。または優良な電子帳簿保存でも65万円になります。紙で提出すると55万円、簡易な記帳なら10万円です。"),
                 ("雑所得として申告する", "雑所得には青色申告特別控除はありません。事業所得（または不動産所得）が対象です。"),
                 ("副業の所得が20万円以下である", "所得の額は条件ではありません。")], 1))
        .hint("このアプリの帳簿は複式簿記で、貸借対照表も作れます。")
        .screen("税金")
        .explain("青色申告をするには、開業から2か月以内（その年の1月から開業していれば3月15日まで）に「青色申告承認申請書」を出しておく必要があります。副業が事業といえる規模・継続性があるかは、帳簿の記録や収入の状況で判断されます。".into());
    b.push(Topic::Tax, 1, "副業20万円の目安", "会社員の副業について正しいものはどれですか。".into(),
        choice(&[("副業の所得が20万円以下なら、所得税の確定申告も住民税の申告もいらない", "所得税の確定申告が不要でも、住民税の申告は必要です。"),
                 ("副業の所得が20万円以下でも、住民税の申告は必要", "正解です。また医療費控除などで確定申告をするときは、20万円以下の副業の所得も含めて申告します。"),
                 ("副業の収入（売上）が20万円を超えたら必ず確定申告が必要", "判定は収入ではなく所得（収入−経費）で行います。"),
                 ("源泉徴収されていれば申告しなくてよい", "源泉徴収は前払いです。税額との差を精算するため、条件に当たれば申告が必要です。")], 1))
        .hint("「20万円」は所得税の確定申告をしなくてよい特例の基準です。")
        .screen("税金")
        .explain("給与所得者で、給与以外の所得が20万円以下なら所得税の確定申告は不要とされています。ただし住民税にはこの特例がないので、市区町村への申告が必要です。佐藤さんは所得が20万円を超えるので確定申告が必要です。".into());
    let facts = [
        ("人物", "佐藤健さん（1992年生まれ、34歳）。会社員として働きながら、週末にWebデザインを請け負う".to_string()),
        ("開業", "2026年1月に開業届と青色申告承認申請書を提出。事業用口座に個人の口座から300,000円を入れた".into()),
        ("会社の給与（源泉徴収票）", "支払金額5,200,000円、源泉徴収税額128,000円、社会保険料780,000円。iDeCo 月23,000円".into()),
        ("売上", "株式会社アトリエA：1件330,000円（源泉徴収10.21%）×4件（3・6・9・12月に請求、翌月末入金）／個人のお客様：110,000円×3件（源泉徴収なし）".into()),
        ("自宅", "家賃110,000円（個人の口座）。仕事部屋は床面積の25%。スマホ・回線8,800円は事業50%".into()),
        ("その他の経費", "デザインソフト6,480円/月（事業用口座）、打合せのカフェ代1,320円×8回（個人の財布）、オンライン講座25,000円".into()),
        ("設備", "デザイン用パソコン352,000円（7月10日、事業用口座、耐用年数4年）".into()),
    ];
    b.finish("sato", "佐藤健さんの副業開業", "副業の記帳・家事按分・減価償却・青色申告とiDeCo", &[
        "佐藤さんは会社員として働きながら、週末にロゴやWebサイトのデザインを請け負っています。2026年に副業の収入が増えたので、開業届と青色申告の申請を出し、事業用の口座で帳簿をつけることにしました。",
        "自宅の一室を仕事部屋にしているので、家賃や通信費のどこまでを経費にできるのか、法人の取引先から天引きされる源泉徴収税がどうなるのか、パソコンは経費にできるのかを知りたがっています。",
        "年が明けたら初めての確定申告です。青色申告にした効果や、続けているiDeCoでどれくらい税金が減っているのかも確かめます。",
    ], &facts,)
}

// ---------- Scenario 3: 子育て世帯の住宅購入とライフプラン ----------

fn yamamoto() -> Result<Scenario> {
    let mut b = Build::new("yamamoto", "山本家の家計", BookKind::Household);
    let (loan_acc, opening, bank) = (b.id("211"), b.id("301"), b.id("111"));
    b.entry(
        "2026-03-31",
        "住宅購入後の預金残高",
        &[("111", 4_200_000, 0), ("301", 0, 4_200_000)],
    )?;
    let loan = b.apply(json!({"operation":"save_claim","opening":opening,"claim":{"side":"payable","title":"住宅ローン（変動1.0%・35年）","account":loan_acc,"principal":"40000000","issued":"2026-03-20","due":"2026-04-27","annual_rate":"1.0","repayment":"equal_payment","installments":420}}))?;
    let claim = b.book.claims.iter().find(|c| c.id == loan).unwrap().clone();
    let schedule = claims::schedule(&b.book, &claim)?;
    for i in schedule.iter().take(9) {
        b.apply(json!({"operation":"settle_claim","settlement":{"claim":loan,"date":i.date,"principal":i.principal,"interest":i.interest,"via":bank}}))?;
    }
    let year_end = claims::report(&b.book, d("2026-12-31"), false)?
        .claims
        .iter()
        .find(|c| c.id == loan)
        .map(|c| c.outstanding.0)
        .unwrap_or_default();
    let payment = lifeplan::loan_payment(&b.book, yen(40_000_000), Decimal::ONE, 35)?;

    let wife_income = tax::rules(2026)?.salary_income(yen(1_100_000));
    let mut tr = TaxReturn::new(2026);
    tr.name = "2026年分（住宅ローン控除1年目）".into();
    tr.salary = yen(6_500_000);
    tr.salary_withheld = yen(175_000);
    tr.social_insurance = yen(975_000);
    tr.has_spouse = true;
    tr.spouse_income = wife_income;
    tr.spouse_birth_year = Some(1992);
    tr.dependents = vec![
        Dependent {
            name: "結衣".into(),
            relation: Relation::Child,
            birth_year: 2020,
            income: Z,
            cohabiting: true,
            disability: Default::default(),
        },
        Dependent {
            name: "蓮".into(),
            relation: Relation::Child,
            birth_year: 2023,
            income: Z,
            cohabiting: true,
            disability: Default::default(),
        },
    ];
    tr.life_general_new = yen(96_000);
    tr.housing_balance = year_end;
    tr.housing_limit = yen(30_000_000);
    b.apply(json!({"operation":"save_tax_return","tax_return":tr}))?;
    let r = tax::calculate(&b.book, &tr)?;
    let spouse = it(&r, "配偶者控除");
    let housing_it = -it(&r, "住宅借入金等特別控除");
    let housing_rt = -rt(&r, "住宅ローン控除（所得税から引ききれない分）");
    let housing_total = (year_end.min(yen(30_000_000)) * Decimal::new(7, 3)).floor();

    let annual_loan = payment.annual.0;
    let members = vec![
        PlanMember {
            name: "大輔".into(),
            relation: "夫".into(),
            birth_year: 1990,
        },
        PlanMember {
            name: "美咲".into(),
            relation: "妻".into(),
            birth_year: 1992,
        },
        PlanMember {
            name: "結衣".into(),
            relation: "長女".into(),
            birth_year: 2020,
        },
        PlanMember {
            name: "蓮".into(),
            relation: "長男".into(),
            birth_year: 2023,
        },
    ];
    let flow = |name: &str,
                kind: &str,
                amount: Decimal,
                from: i32,
                to: i32,
                growth: &str|
     -> Result<PlanFlow> {
        Ok(serde_json::from_value(
            json!({"name":name,"kind":kind,"amount":amount,"start_year":from,"end_year":to,"growth_percent":growth}),
        )?)
    };
    let event = |year: i32, name: &str, kind: &str, amount: Decimal| -> Result<PlanEvent> {
        Ok(serde_json::from_value(
            json!({"year":year,"name":name,"kind":kind,"amount":amount}),
        )?)
    };
    let mut flows = vec![
        flow("大輔の手取り", "income", yen(5_050_000), 2026, 2054, "1")?,
        flow("大輔の年金", "income", yen(1_900_000), 2055, 2060, "0")?,
        flow(
            "美咲のパート手取り",
            "income",
            yen(1_000_000),
            2026,
            2056,
            "0",
        )?,
        flow("美咲の年金", "income", yen(800_000), 2057, 2060, "0")?,
        flow("基本生活費", "expense", yen(3_300_000), 2026, 2060, "1")?,
        flow("住宅ローン", "expense", annual_loan, 2027, 2060, "0")?,
        flow(
            "管理費・修繕積立金・固定資産税",
            "expense",
            yen(480_000),
            2026,
            2060,
            "1",
        )?,
        flow("生命保険料", "expense", yen(96_000), 2026, 2045, "0")?,
    ];
    let mut events = vec![
        event(
            2026,
            "住宅ローン（4〜12月の9回）",
            "expense",
            payment.monthly.0 * yen(9),
        )?,
        event(2031, "車の買い替え", "expense", yen(3_000_000))?,
        event(2055, "大輔の退職金", "income", yen(18_000_000))?,
    ];
    for (child, born, schools) in [
        (
            "結衣",
            2020,
            ["none", "public", "public", "public", "private"],
        ),
        (
            "蓮",
            2023,
            ["public", "public", "public", "public", "private"],
        ),
    ] {
        let req: lifeplan::EducationRequest = serde_json::from_value(
            json!({"child":child,"birth_year":born,"schools":schools,"growth_percent":"1"}),
        )?;
        let (f, e) = lifeplan::education(&req)?;
        flows.extend(f);
        events.extend(e);
    }
    let base = LifePlan {
        id: 0,
        name: "現状（美咲さんはパートを続ける）".into(),
        memo: String::new(),
        start_year: 2026,
        years: 35,
        savings: yen(4_200_000),
        return_percent: Decimal::ONE,
        members: members.clone(),
        flows: flows.clone(),
        events: events.clone(),
    };
    let mut proposal = base.clone();
    proposal.name = "提案（美咲さんが2029年から正社員）".into();
    proposal.flows.retain(|f| !f.name.starts_with("美咲"));
    proposal.flows.extend([
        flow(
            "美咲のパート手取り",
            "income",
            yen(1_000_000),
            2026,
            2028,
            "0",
        )?,
        flow(
            "美咲の正社員の手取り",
            "income",
            yen(2_600_000),
            2029,
            2056,
            "1",
        )?,
        flow("美咲の年金", "income", yen(1_100_000), 2057, 2060, "0")?,
    ]);
    b.apply(json!({"operation":"save_life_plan","life_plan":base}))?;
    b.apply(json!({"operation":"save_life_plan","life_plan":proposal}))?;
    let base = b.book.life_plans[0].clone();
    let proposal = b.book.life_plans[1].clone();
    let cf = lifeplan::cash_flow(&b.book, &base)?;
    let cmp = lifeplan::compare(&b.book, &base, &proposal)?;
    let pf = lifeplan::cash_flow(&b.book, &proposal)?;
    let gain = pf.final_savings.0 - cf.final_savings.0;
    let sf = lifeplan::coefficients(Decimal::ONE, 12)?;
    let _ = cmp;

    b.push(Topic::LifePlan, 1, "住宅ローンの毎月返済額", "40,000,000円を金利1.0%（全期間同じと仮定）、35年の元利均等返済で借りると、毎月の返済額はいくらですか。".into(),
        number(payment.monthly.0, 0, "円"))
        .hint("ライフプランのFP電卓の「ローン返済額」で計算できます。月利は年利÷12、回数は420回です。")
        .screen("ライフプラン（FP電卓）")
        .explain(format!("毎月の返済額は 借入額×月利×(1＋月利)^回数÷((1＋月利)^回数−1) で求めます。月利 1.0%÷12、420回で {} 円、年 {} 円です。変動金利は将来上がることがあるので、金利が1%上がった場合の返済額も確かめておくと安心です。", g(payment.monthly.0), g(payment.annual.0)));
    b.push(Topic::LifePlan, 2, "利息の総額", "前の問題のローンを35年間返し続けると、支払う利息の総額はいくらですか。".into(),
        number(payment.interest.0, 0, "円"))
        .hint("毎月の返済額×420回 − 借入額 です。")
        .screen("ライフプラン（FP電卓）")
        .explain(format!("{}×420 ＝ {} 円を返済し、そのうち借入額を除いた {} 円が利息です。金利1.0%でも35年では大きな金額になります。", g(payment.monthly.0), g(payment.total.0), g(payment.interest.0)));
    let first = &schedule[0];
    b.push(Topic::Accounting, 2, "返済の仕訳", format!("4月27日、住宅ローンの第1回の返済 {} 円が普通預金から引き落とされました。返済予定表では元本 {} 円、利息 {} 円です。仕訳を入力してください。", g(first.payment.0), g(first.principal.0), g(first.interest.0)),
        journal(&[("211", first.principal.0, Z), ("661", first.interest.0, Z), ("111", Z, first.payment.0)]))
        .hint("返済額のうち、元本は借入（負債）の減少、利息だけが費用です。")
        .screen("債権・債務")
        .explain(format!("「借方 ローン・借入金 {} ・支払利息 {} ／ 貸方 普通預金 {}」です。返済額全部を住居費にすると、ローン残高が帳簿上減らず、毎月の費用も多く見えます。初回の利息は 40,000,000×1.0%÷12 ＝ {} 円です。債権・債務の「返済」ボタンで同じ仕訳を作れます。", g(first.principal.0), g(first.interest.0), g(first.payment.0), g(first.interest.0)));
    b.push(Topic::Accounting, 2, "年末のローン残高", "シナリオの帳簿で、9回返済した後の2026年12月31日時点の住宅ローン残高はいくらですか。".into(),
        number(year_end, 0, "円"))
        .hint("債権・債務の画面で基準日を12月31日にするか、貸借対照表のローン・借入金を見ます。")
        .screen("債権・債務")
        .explain(format!("40,000,000円から9回分の元本を返して {} 円です。この「年末残高」が住宅ローン控除の計算のもとになり、金融機関から届く残高証明書の金額と一致するはずです。", g(year_end)));
    b.push(Topic::Tax, 1, "妻の合計所得金額", "妻の美咲さんのパート収入（給与）は年1,100,000円です。美咲さんの合計所得金額はいくらですか（2026年分）。".into(),
        number(wife_income, 0, "円"))
        .hint("給与だけなら、給与所得控除後の金額が合計所得金額です。令和8年分の給与所得控除の最低額は74万円です。")
        .screen("税金")
        .explain(format!("1,100,000−740,000 ＝ {} 円です。令和8年分は合計所得金額62万円以下（給与だけなら年収136万円以下）なら配偶者控除の対象です。社会保険の扶養（年収130万円の目安など）は税金とは別の基準なので、働き方を決めるときは両方を確かめます。", g(wife_income)));
    b.push(Topic::Tax, 1, "配偶者控除", "大輔さん（給与6,500,000円）が受けられる配偶者控除の額（所得税）はいくらですか。".into(),
        number(spouse, 0, "円"))
        .hint("本人の合計所得金額が900万円以下で、配偶者の合計所得金額が扶養の範囲内のとき、70歳未満の配偶者なら38万円です。")
        .screen("税金")
        .explain(format!("所得税の配偶者控除は {} 円、住民税は33万円です。子ども2人（6歳・3歳）は16歳未満なので扶養控除はありません（代わりに児童手当があります）。", g(spouse)));
    b.push(Topic::Tax, 1, "16歳未満の子", "結衣さん（6歳）と蓮さん（3歳）について、大輔さんの所得税で受けられる扶養控除はいくらですか。".into(),
        choice(&[("0円（16歳未満は扶養控除の対象外）", "正解です。年少扶養親族（16歳未満）の扶養控除は、児童手当の創設にあわせて廃止されています。"),
                 ("1人38万円で合計76万円", "38万円の扶養控除は16歳以上の扶養親族が対象です。"),
                 ("1人63万円で合計126万円", "63万円は19歳以上23歳未満の特定扶養親族です。"),
                 ("1人48万円で合計96万円", "48万円は基礎控除の以前の額で、扶養控除ではありません。")], 0))
        .hint("扶養控除は、その年の12月31日の年齢で決まります。")
        .screen("税金")
        .explain("16歳未満の子は扶養控除の対象外ですが、扶養親族として申告書に書く欄はあり、住民税の非課税の判定などに使われます。税金の画面でも子どもを入力しておくと、生命保険料控除の特例などに反映されます。".into());
    b.push(Topic::Tax, 2, "住宅ローン控除（所得税）", format!("2026年に入居し、年末残高は {} 円、借入限度額は30,000,000円、控除率は0.7%とします。所得税から差し引かれる住宅ローン控除（住宅借入金等特別控除）の額はいくらですか。", g(year_end)),
        number(housing_it, 0, "円"))
        .hint("控除額は「年末残高と借入限度額の少ない方×0.7%」ですが、所得税から引けるのは所得税の額までです。")
        .screen("税金")
        .explain(format!("控除の枠は min({}, 30,000,000)×0.7% ＝ {} 円です。所得税（住宅ローン控除の前）が {} 円までしか引けないため、所得税から差し引くのは {} 円です。", g(year_end), g(housing_total), g(housing_it), g(housing_it)));
    b.push(Topic::Tax, 3, "住宅ローン控除（住民税）", "前の問題で所得税から引ききれなかった住宅ローン控除のうち、翌年度の住民税から差し引かれる額はいくらですか。".into(),
        number(housing_rt, 0, "円"))
        .hint("住民税から引けるのは「所得税の課税所得の5%」と「97,500円」の少ない方までです。")
        .screen("税金")
        .explain(format!("引ききれなかった {} 円のうち、住民税から {} 円が差し引かれます。それでも残る分は控除されません。借入限度額や控除率は入居年・住宅の省エネ性能・子育て世帯かどうかで変わるので、実際には借入金の年末残高証明書と国税庁の案内で確かめます。", g(housing_total - housing_it), g(housing_rt)));
    b.push(Topic::Tax, 1, "1年目の手続き", "住宅ローン控除を初めて受ける2026年分の手続きとして正しいものはどれですか。".into(),
        choice(&[("会社の年末調整で受ける", "1年目は年末調整では受けられません。"),
                 ("2027年に確定申告をし、2年目からは年末調整で受ける", "正解です。1年目は登記事項証明書・残高証明書などを添えて確定申告します。2年目以後は税務署から届く申告書で年末調整できます。"),
                 ("市区町村に住民税の申告だけをする", "所得税の控除なので確定申告が必要です。"),
                 ("申請しなくても自動で控除される", "申告しないと控除されません。")], 1))
        .hint("医療費控除と同じく、1年目は自分で申告します。")
        .screen("税金")
        .explain("山本家は2026年分を確定申告すると、源泉徴収された所得税から住宅ローン控除の分が還付され、住民税も翌年度から少なくなります。".into());
    b.push(Topic::LifePlan, 2, "大学資金の準備", format!("結衣さんが大学に入る2038年までの12年間で4,000,000円を準備したいと考えています。年1%で運用すると、毎年いくら積み立てればよいですか（減債基金係数 {} を使います）。円未満は四捨五入。", num::text(sf.sinking_fund.0)),
        number(sf.sinking_fund.0 * yen(4_000_000), 0, "円"))
        .hint("目標額×減債基金係数 ＝ 毎年の積立額。")
        .screen("ライフプラン（FP電卓）")
        .explain(format!("4,000,000×{} ＝ {} 円（月約{}円）です。児童手当を全額ためると、この積立のかなりの部分をまかなえます。", num::text(sf.sinking_fund.0), g((sf.sinking_fund.0 * yen(4_000_000)).round()), g((sf.sinking_fund.0 * yen(4_000_000) / yen(12)).round())));
    b.push(Topic::LifePlan, 2, "資産がいちばん減る年", "シナリオのライフプラン「現状」のキャッシュフロー表で、年末の金融資産がいちばん少なくなるのは西暦何年ですか。".into(),
        number(Decimal::from(cf.min_year), 0, "年"))
        .hint("ライフプランの画面で「現状」のキャッシュフロー表を表示し、年末の金融資産の列を見ます。")
        .screen("ライフプラン")
        .explain(format!("{} 年の年末に {} 円となり、いちばん少なくなります。2人の大学（私立）の入学金・学費が重なる時期に、住宅ローンと上がり続ける生活費が重なるためです。{}", cf.min_year, g(cf.min_savings.0), if cf.shortfall_years.is_empty() { "マイナスにはなりませんが、余裕は大きくありません。".to_string() } else { format!("マイナスになる年（{}）があり、計画の見直しが必要です。", cf.shortfall_years.iter().map(|y| y.to_string()).collect::<Vec<_>>().join("・")) }));
    b.push(Topic::LifePlan, 3, "働き方の提案を比べる", "美咲さんが2029年から正社員として働く提案（手取り年2,600,000円、年1%上昇、年金も年1,100,000円に増える）では、2060年末の金融資産は現状よりいくら多くなりますか。".into(),
        number(gain, 0, "円"))
        .hint("ライフプランで「現状」と「提案」を比較表示し、最後の年の差を見ます。")
        .screen("ライフプラン")
        .explain(format!("現状の2060年末は {} 円、提案では {} 円で、差は {} 円です。提案では資産がいちばん少ない年も {} 円になります。収入が増えると、税・社会保険料・保育などの支出も変わるので、手取りベースで比べることが大切です。", g(cf.final_savings.0), g(pf.final_savings.0), g(gain), g(pf.min_savings.0)));
    b.push(Topic::LifePlan, 1, "繰上げ返済の型", "住宅ローンを100万円繰上げ返済します。総返済額（利息）を減らす効果が大きいのはどちらですか。".into(),
        choice(&[("期間短縮型", "正解です。返済期間の後ろの方の元本を先に返すので、その期間の利息がまるごとなくなり、利息の軽減効果が大きくなります。"),
                 ("返済額軽減型", "毎月の返済額は下がりますが、返済期間は変わらないので、利息の軽減効果は期間短縮型より小さくなります。"),
                 ("どちらも同じ", "同じ金額を返しても、利息の減り方は違います。"),
                 ("繰上げ返済では利息は減らない", "繰上げ返済した元本には、その後の利息がかかりません。")], 0))
        .hint("どちらも元本を100万円減らしますが、その後の返済期間が違います。")
        .screen("ライフプラン")
        .explain("期間短縮型は利息を減らす効果が大きく、返済額軽減型は毎月の家計を楽にする効果があります。教育費のピーク前に手元資金を減らしすぎない、住宅ローン控除の対象期間中は残高が減ると控除も減る、といった点も合わせて判断します。".into());
    let facts = [
        ("家族", "大輔さん（1990年生、会社員）、美咲さん（1992年生、パート）、結衣さん（2020年生）、蓮さん（2023年生）".to_string()),
        ("住宅", "2026年3月に新築マンションを購入・入居。借入40,000,000円、変動金利1.0%、35年、元利均等、4月27日から毎月返済".into()),
        ("購入後の預金", "4,200,000円（2026年3月末）".into()),
        ("大輔さんの給与（2026年分）", "支払金額6,500,000円、社会保険料975,000円、源泉徴収税額175,000円、一般生命保険料（新）96,000円".into()),
        ("美咲さん", "パート収入（給与）年1,100,000円".into()),
        ("住宅ローン控除の前提", "借入限度額30,000,000円、控除率0.7%、住民税からの上限97,500円（実際の限度額は住宅の種類と入居年で決まります）".into()),
        ("ライフプランの前提", format!("2026〜2060年。手取り・生活費は年1%上昇、運用利回り年1%。教育費は文部科学省の調査をもとにした目安（大学は私立）。現状と、美咲さんが2029年から正社員になる提案。住宅ローンは年 {} 円", g(annual_loan))),
    ];
    b.finish("yamamoto", "山本家の住宅購入", "住宅ローン・住宅ローン控除・配偶者控除・教育費とキャッシュフロー表", &[
        "山本さん一家は2026年春に新築マンションを買い、4,000万円の住宅ローンを組みました。毎月の返済が家計にどう響くのか、住宅ローン控除でどれくらい税金が戻るのかを知りたいと考えています。",
        "子どもは6歳と3歳。これから教育費が増えていき、2人とも大学に進む可能性があります。美咲さんはパートを続けるか、正社員に戻るかを迷っています。",
        "FPと一緒にキャッシュフロー表を作り、現状と提案を比べて、お金が足りなくなる時期がないかを確かめます。",
    ], &facts,)
}

// ---------- Scenario 4: パン工房の原価計算 ----------

fn komugi() -> Result<Scenario> {
    let mut b = Build::new("komugi", "パン工房こむぎ", BookKind::SoleProprietor);
    b.entry(
        "2026-04-01",
        "開業資金",
        &[("111", 2_000_000, 0), ("301", 0, 2_000_000)],
    )?;
    let flour = b.apply(json!({"operation":"save_material","material":{"name":"小麦粉（強力粉）","unit":"kg","standard_cost":"250"}}))?;
    let butter = b.apply(json!({"operation":"save_material","material":{"name":"発酵バター","unit":"kg","standard_cost":"1800"}}))?;
    let oven = b.apply(json!({"operation":"save_equipment","equipment":{"name":"デッキオーブン","acquired":"2026-04-01","cost":"1200000","salvage":"0","life_years":8,"annual_hours":"1500","running_cost_per_hour":"90"}}))?;
    b.entry(
        "2026-04-01",
        "デッキオーブンの購入",
        &[("162", 1_200_000, 0), ("111", 0, 1_200_000)],
    )?;
    let bread = b.apply(json!({"operation":"save_product","product":{"name":"食パン","unit":"斤","labor_hours":"0.12","labor_rate":"1300","overhead_percent":"10",
        "materials":[{"material":flour,"quantity":"0.3","loss_percent":"4"},{"material":butter,"quantity":"0.015"}],
        "equipment":[{"equipment":oven,"hours":"0.04"}]}}))?;
    let (mat, payable) = (b.id("132"), b.id("201"));
    for (date, material, qty, price) in [
        ("2026-04-01", flour, "25", "240"),
        ("2026-04-01", butter, "5", "1800"),
        ("2026-04-15", flour, "25", "264"),
    ] {
        b.apply(json!({"operation":"save_material_move","material_move":{"material":material,"date":date,"kind":"purchase","quantity":qty,"unit_cost":price}}))?;
        let total = num::parse(qty)? * num::parse(price)?;
        b.apply(json!({"operation":"save_entry","entry":{"date":date,"description":"原材料の掛仕入","lines":[{"account":mat,"debit":total},{"account":payable,"credit":total}]}}))?;
    }
    let first = b.apply(json!({"operation":"save_production","production":{"product":bread,"date":"2026-04-10","quantity":"30","materials":[{"material":flour,"quantity":"10"},{"material":butter,"quantity":"0.5"}]}}))?;
    let second = b.apply(json!({"operation":"save_production","production":{"product":bread,"date":"2026-04-20","quantity":"120","labor_hours":"15","materials":[{"material":flour,"quantity":"39"},{"material":butter,"quantity":"1.8"}]}}))?;
    let at15 = cost::report(&b.book, d("2026-04-15"))?;
    let avg = at15
        .stock
        .iter()
        .find(|s| s.material == flour)
        .map(|s| s.average_cost.0)
        .unwrap_or_default();
    let r = cost::report(&b.book, d("2026-04-30"))?;
    let rate = r
        .equipment
        .iter()
        .find(|e| e.equipment == oven)
        .map(|e| e.hourly_rate.0)
        .unwrap_or_default();
    let std = r
        .products
        .iter()
        .find(|p| p.product == bread)
        .map(|p| p.standard.total.0)
        .unwrap_or_default();
    let run = r
        .productions
        .iter()
        .find(|p| p.production == second)
        .unwrap();
    let run1 = r
        .productions
        .iter()
        .find(|p| p.production == first)
        .unwrap();
    let materials_used = run.actual.materials.0 + run1.actual.materials.0;
    let margin = (yen(450) - std) / yen(450) * num::hundred();
    let dep = cost::depreciation(
        &b.book,
        b.book.equipment.iter().find(|e| e.id == oven).unwrap(),
        2026,
    )?;

    b.push(Topic::Accounting, 1, "材料の掛仕入", "4月1日、製粉会社から小麦粉25kgを1kgあたり240円で仕入れ、代金は月末締め翌月払い（掛け）にしました。仕訳を入力してください。".into(),
        journal(&[("132", yen(6_000), Z), ("201", Z, yen(6_000))]))
        .hint("製造に使う材料は、使うまで「原材料」という資産です。仕入代金の後払いは何という負債でしょう。")
        .screen("仕訳帳")
        .explain("「借方 原材料 6,000 ／ 貸方 買掛金 6,000」です。商品・材料の仕入れの後払いは買掛金、備品や経費の後払いは未払金と使い分けます。使った分は、製造の実績に合わせて材料費に振り替えます。".into());
    b.push(Topic::Accounting, 2, "移動平均単価", "4月10日に小麦粉10kgを製造で使い、4月15日に25kgを1kgあたり264円で追加で仕入れました。4月15日の仕入後の小麦粉の移動平均単価（1kgあたり）はいくらですか。".into(),
        number(avg, 0, "円"))
        .hint("仕入のたびに（残っている在庫の金額＋仕入れた金額）÷（在庫の数量＋仕入れた数量）で平均単価を計算し直します。")
        .screen("原価管理（原材料）")
        .explain(format!("4月10日の払出し後の在庫は 15kg×240円 ＝ 3,600円。15日に 25kg×264円 ＝ 6,600円を受け入れると、(3,600＋6,600)÷(15＋25) ＝ {} 円になります。", g(avg)));
    b.push(Topic::Accounting, 1, "移動平均法の性質", "仕入単価が上がり続けているとき、移動平均法で払い出す単価は、いちばん新しい仕入単価と比べてどうなりますか。".into(),
        choice(&[("高くなる", "平均には過去の安い仕入れも含まれるので、最新の単価より高くはなりません。"),
                 ("低くなる", "正解です。過去の安い単価と平均されるので、最新の仕入単価より低くなります。原価が実態より低く見えることに注意が必要です。"),
                 ("いつも同じになる", "仕入単価が変わらないときだけ同じになります。"),
                 ("在庫数量によって必ず高くなる", "在庫数量は平均の重みを変えるだけです。")], 1))
        .hint("移動平均は、過去の仕入れと新しい仕入れの加重平均です。")
        .screen("原価管理（原材料）")
        .explain("値上がりが続く局面では、移動平均の原価は最新の仕入価格より低く出ます。販売価格を決めるときは、標準単価を最新の相場に合わせて見直すと安全です。".into());
    b.push(Topic::Accounting, 2, "設備の時間当たり費用", "デッキオーブン（取得価額1,200,000円、残存価額0円、耐用年数8年、年間稼働1,500時間、電気・ガス代など1時間90円）の時間当たり費用はいくらですか。".into(),
        number(rate, 0, "円"))
        .hint("（取得価額−残存価額）÷耐用年数÷年間稼働時間 ＋ 1時間の運転費。")
        .screen("原価管理（設備）")
        .explain(format!("1,200,000÷8÷1,500 ＝ 100円が1時間あたりの減価償却分、これに運転費90円を足して {} 円です。製品1個の設備費は、この時間当たり費用×使用時間で求めます。", g(rate)));
    b.push(Topic::Accounting, 2, "標準原価", "食パン1斤の標準は、小麦粉0.3kg（ロス率4%、標準単価250円）、バター0.015kg（1,800円）、オーブン0.04時間、作業0.12時間（賃率1,300円）、製造間接費は直接費の10%です。1斤の標準原価はいくらですか（円未満を四捨五入）。".into(),
        number(std, 0, "円"))
        .hint("ロスがあるので小麦粉は 0.3÷(1−0.04) kg 使います。材料・設備・労務を足し、その10%を間接費として加えます。")
        .screen("原価管理（製品原価）")
        .explain(format!("小麦粉 0.3÷0.96 ＝ 0.3125kg × 250 ＝ 78.125円、バター 0.015×1,800 ＝ 27円、オーブン 0.04×{} ＝ 7.6円、労務 0.12×1,300 ＝ 156円で直接費は268.725円。間接費は26.8725円で、標準原価は {} 円（約{}円）です。", g(rate), num::text(std), g(std.round())));
    b.push(Topic::Accounting, 2, "粗利率", "食パンを1斤450円で売るとき、標準原価に対する粗利率（（販売価格−原価）÷販売価格）は何%ですか。小数第1位まで。".into(),
        number(margin, 1, "%"))
        .hint("前の問題の標準原価（四捨五入する前の値）を使います。")
        .screen("原価管理（製品原価）")
        .explain(format!("(450−{})÷450 ＝ {}% です。ここから店舗の家賃・販売の人件費・包材・広告などをまかないます。原価が上がったときに粗利率がどこまで下がるかを試算しておくと、値上げの判断に役立ちます。", num::text(std), num::text(Rounding::HalfUp.round(margin, 1))));
    b.push(Topic::Accounting, 3, "材料の差異", "4月20日に食パン120斤を作りました。実際に使ったのは小麦粉39kg、バター1.8kgです（材料は移動平均単価で払い出します）。材料費の差異（実際−標準）はいくらですか。".into(),
        number(run.variance_materials.0, 0, "円"))
        .hint("標準の材料費は1斤あたりの標準材料費×120斤です。実際は払出時の移動平均単価×実際の使用量です。")
        .screen("原価管理（製造実績）")
        .explain(format!("実際の材料費は 小麦粉39×{}円＋バター1.8×1,800円 ＝ {} 円、標準は 105.125×120 ＝ {} 円で、差異は {} 円（不利差異）です。標準の小麦粉は 0.3125×120 ＝ 37.5kg なので、使用量が1.5kg多いことと、仕入単価の上昇が原因です。", g(avg), g(run.actual.materials.0), g(run.actual.materials.0 - run.variance_materials.0), g(run.variance_materials.0)));
    b.push(Topic::Accounting, 2, "実際原価", "4月20日の製造（120斤、作業時間15時間）の実際原価の合計はいくらですか。オーブンの使用時間は標準どおりとします。".into(),
        number(run.actual.total.0, 0, "円"))
        .hint("材料費＋設備費（0.04時間×120斤×時間当たり費用）＋労務費（15時間×1,300円）＋製造間接費（直接費の10%）。")
        .screen("原価管理（製造実績）")
        .explain(format!("材料 {} 円、設備 {} 円、労務 {} 円、間接費 {} 円で合計 {} 円、1斤あたり {} 円です。標準原価との差 {} 円のうち、労務の差異は {} 円（作業時間が標準14.4時間より長い）です。", g(run.actual.materials.0), g(run.actual.equipment.0), g(run.actual.labor.0), g(run.actual.overhead.0), g(run.actual.total.0), num::text(run.actual_unit_cost.0), g(run.variance.0), g(run.variance_labor.0)));
    b.push(Topic::Accounting, 2, "材料費への振替", "4月の2回の製造で使った原材料の金額（移動平均単価で計算）を、月末に原材料から材料費に振り替えます。仕訳を入力してください。".into(),
        journal(&[("502", materials_used, Z), ("132", Z, materials_used)]))
        .hint("製造実績の画面で、4月10日と4月20日の実際の材料費を合計します。")
        .screen("原価管理（製造実績）")
        .explain(format!("4月10日 {} 円＋4月20日 {} 円 ＝ {} 円を「借方 材料費 ／ 貸方 原材料」で振り替えます。原材料の残高は月末の在庫金額（{} 円）と一致します。", g(run1.actual.materials.0), g(run.actual.materials.0), g(materials_used), g(r.inventory_value.0)));
    b.push(Topic::Accounting, 1, "オーブンの科目", "製造に使う業務用デッキオーブンを1,200,000円で買いました。借方の科目はどれですか。".into(),
        choice(&[("消耗品費", "10万円以上で何年も使うので費用ではなく固定資産です。"),
                 ("工具器具備品", "パソコンや家具・工具は工具器具備品ですが、製造に使う機械は機械装置に分けます。"),
                 ("機械装置", "正解です。製造用の機械は機械装置として計上し、減価償却します。原価管理の設備にも登録して製品原価に配賦します。"),
                 ("修繕費", "修理ではなく新たな購入です。")], 2))
        .hint("製造用の機械か、事務用の備品かで分けます。")
        .screen("勘定科目・取引先（使い分けガイド）")
        .explain("「借方 機械装置 1,200,000 ／ 貸方 普通預金 1,200,000」。税法の耐用年数は設備の種類（業種）で決まるので、実際には耐用年数表で確かめます。".into());
    b.push(Topic::Tax, 2, "開業年の減価償却費", "このオーブン（定額法、耐用年数8年）を2026年4月1日から使い始めました。2026年分の減価償却費はいくらですか。".into(),
        number(dep, 0, "円"))
        .hint("1年分は1,200,000÷8。使い始めた年は月数（4〜12月の9か月）で按分します。")
        .screen("原価管理（設備）")
        .explain(format!("1,200,000÷8 ＝ 150,000円（定額法の償却率0.125と同じ）。9か月分で 150,000×9/12 ＝ {} 円です。減価償却費は事業所得の計算で経費になり、製品原価にも時間当たり費用として配賦されています（決算書では二重に計上しないよう、どちらで管理するかを決めておきます）。", g(dep)));
    let facts = [
        ("人物", "小麦さん（個人事業主）。2026年4月にパン工房こむぎを開業".to_string()),
        ("原材料", "小麦粉（標準単価250円/kg）、発酵バター（1,800円/kg）。4/1 小麦粉25kg@240円・バター5kg@1,800円、4/15 小麦粉25kg@264円（いずれも掛仕入）".into()),
        ("設備", "デッキオーブン 1,200,000円（4/1取得、耐用年数8年、年1,500時間、運転費90円/時間）".into()),
        ("食パン1斤の標準", "小麦粉0.3kg（ロス4%）、バター0.015kg、オーブン0.04時間、作業0.12時間（賃率1,300円）、製造間接費は直接費の10%".into()),
        ("製造実績", "4/10 30斤（小麦粉10kg・バター0.5kg）、4/20 120斤（小麦粉39kg・バター1.8kg・作業15時間）".into()),
        ("販売価格", "食パン1斤450円".into()),
    ];
    b.finish("komugi", "パン工房こむぎの原価計算", "材料の在庫評価・設備費・標準原価と差異・価格設定", &[
        "小麦さんは2026年4月にパン工房を開きました。看板商品の食パンを1斤450円で売るつもりですが、本当に利益が出る値段なのか自信がありません。",
        "小麦粉の値段は少しずつ上がっています。オーブンは高い買い物だったので、パン1斤あたりいくらの負担になるのかも知りたいところです。",
        "製品の「標準」を決め、実際の製造と比べて、どこで原価がふくらんでいるのかを見つけます。",
    ], &facts,)
}

// ---------- Documents ----------

fn level(n: u8) -> &'static str {
    match n {
        1 => "基本",
        2 => "標準",
        _ => "応用",
    }
}
/// `docs/user/study.md`：問題と、末尾にまとめた解答・解説。
pub fn markdown() -> String {
    let mut out = String::from(
        "# 問題集：シナリオで学ぶ会計・税務・ライフプラン\n\n\
         <!-- core/src/study.rs から生成。直接編集せず `YYA_UPDATE_DOCS=1 cargo test -p yy-accountant --test docs` で更新します。 -->\n\n\
         具体的な人物・家族の1年を題材に、記帳（会計）、所得税・住民税（税務）、ライフプランの問題を解きます。\
         アプリの「問題集」（Web UIとWindowsのネイティブ画面）とMCPの `study_list`・`study_answer` で解くと自動で採点され、間違えた点の指摘と解説が表示されます。\
         各シナリオは「練習用の帳簿を作る」で帳簿として開けるので、問題で使う帳票・税額・ライフプランを実際の画面で確かめられます。\n\n\
         > 数値の正解は、アプリの計算（帳票・税額・ライフプラン・原価）と同じ方法で求めています。税額は令和8年（2026年）分の基準による試算です。税制は毎年のように変わるので、実際の申告では国税庁・市区町村の案内で確かめてください。\n",
    );
    for s in scenarios_all() {
        out += &format!("\n## {}\n\n_{}_\n\n", s.title, s.subtitle);
        for p in &s.story {
            out += &format!("{p}\n\n");
        }
        out += "| 前提 | 内容 |\n|---|---|\n";
        for (k, v) in &s.facts {
            out += &format!("| {k} | {} |\n", v.replace('|', "｜"));
        }
        for p in &s.problems {
            out += &format!(
                "\n### {} {}（{}・{}）\n\n{}\n",
                p.id,
                p.title,
                p.topic.label(),
                level(p.level),
                p.question
            );
            if let Key::Choice { options, .. } = &p.key {
                out += "\n";
                for (i, o) in options.iter().enumerate() {
                    out += &format!("{}. {}\n", i + 1, o.0);
                }
            }
            if matches!(p.key, Key::Journal(_)) {
                out += "\n（仕訳で答えます）\n";
            }
            out += &format!("\nヒント：{}（画面: {}）\n", p.hint, p.screen);
        }
    }
    out += "\n## 解答と解説\n";
    for s in scenarios_all() {
        out += &format!("\n### {}\n", s.title);
        for p in &s.problems {
            let answer = match &p.key {
                Key::Choice { correct, .. } => format!("{}. {}", correct + 1, s.expected(p)),
                _ => s.expected(p),
            };
            out += &format!(
                "\n**{} {}** 正解：{}\n\n{}\n",
                p.id, p.title, answer, p.explanation
            );
        }
    }
    out.truncate(out.trim_end().len());
    out.push('\n');
    out
}
