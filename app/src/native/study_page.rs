//! The problem-set page (問題集): what the list, detail and summary show, the answer form
//! for each problem format, and the texts shown after grading. Window-independent like
//! `sheets.rs`; grading and every number stay in `yy_accountant::study`.
use super::form::Field;
use super::records::View;
use super::sheets::{Align, Table};
use yy_accountant::Category;
use yy_accountant::study::{
    self, Answer, AnswerLine, Attempt, Format, Grade, Key, Problem, Scenario,
};
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
use Align::Left as L;

/// Journal answers have this many rows (every key in the problem set fits).
pub(super) const JOURNAL_ROWS: usize = 4;
/// Labels of the journal rows; form labels are static and fill values by label.
pub(super) const JOURNAL_LABELS: [[&str; 3]; JOURNAL_ROWS] = [
    ["1行目の科目", "1行目の借方", "1行目の貸方"],
    ["2行目の科目", "2行目の借方", "2行目の貸方"],
    ["3行目の科目", "3行目の借方", "3行目の貸方"],
    ["4行目の科目", "4行目の借方", "4行目の貸方"],
];
/// Characters per detail row; the list view does not wrap text.
const WIDTH: usize = 46;

/// Scenarios for the view selector: (index, title).
pub(super) fn choices() -> Vec<(i64, String)> {
    study::scenarios_all()
        .iter()
        .enumerate()
        .map(|(i, s)| (i as i64, format!("{}（{}問）", s.title, s.problems.len())))
        .collect()
}
pub(super) fn scenario(chosen: Option<i64>) -> &'static Scenario {
    let all = study::scenarios_all();
    &all[chosen.unwrap_or(0).clamp(0, all.len() as i64 - 1) as usize]
}
/// List rows carry the problem's position in its scenario plus one.
pub(super) fn problem(chosen: Option<i64>, row: Option<i64>) -> Option<&'static Problem> {
    let index = usize::try_from(row? - 1).ok()?;
    scenario(chosen).problems.get(index)
}
fn level(n: u8) -> &'static str {
    match n {
        1 => "基本",
        2 => "標準",
        _ => "応用",
    }
}
/// Splits text into rows of at most `width` characters, preferring to break after 。
pub(super) fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut out = vec![];
    for paragraph in text.lines() {
        let chars: Vec<char> = paragraph.chars().collect();
        let mut start = 0;
        while chars.len() - start > width {
            let window = &chars[start..start + width];
            let cut = window
                .iter()
                .rposition(|c| *c == '。' || *c == '、')
                .filter(|p| *p >= width / 2)
                .map_or(width, |p| p + 1);
            out.push(chars[start..start + cut].iter().collect());
            start += cut;
        }
        out.push(chars[start..].iter().collect());
    }
    out
}
fn status(attempts: &[Attempt], id: &str) -> &'static str {
    match attempts.iter().rev().find(|a| a.problem == id) {
        None => "",
        Some(a) if a.correct => "正解",
        Some(_) => "不正解",
    }
}

/// Main list: the scenario's problems (or only those to review); detail: the selected
/// problem's text, options and hint; summary: progress.
pub(super) fn view(
    chosen: Option<i64>,
    review_only: bool,
    selected: Option<i64>,
    attempts: &[Attempt],
) -> View {
    let s = scenario(chosen);
    let progress = study::progress(attempts);
    let mut main = Table::new(&[
        ("問題", 100, L),
        ("分野", 100, L),
        ("レベル", 60, L),
        ("題名", 280, L),
        ("状態", 70, L),
    ]);
    for (i, p) in s.problems.iter().enumerate() {
        if review_only && !progress.review.contains(&p.id) {
            continue;
        }
        main.push(
            i as i64 + 1,
            vec![
                p.id.clone(),
                p.topic.label().into(),
                level(p.level).into(),
                p.title.clone(),
                status(attempts, &p.id).into(),
            ],
        );
    }
    let mut detail = Table::new(&[("内容", 760, L)]);
    match problem(chosen, selected) {
        Some(p) => {
            for row in wrap(&p.question, WIDTH) {
                detail.push(0, vec![row]);
            }
            match &p.view().format {
                Format::Choice { options } => {
                    for (i, o) in options.iter().enumerate() {
                        for row in wrap(&format!("{}. {o}", i + 1), WIDTH) {
                            detail.push(0, vec![row]);
                        }
                    }
                }
                Format::Number { unit, digits } => detail.push(
                    0,
                    vec![if *digits > 0 {
                        format!("（数値で答えます。単位: {unit}、小数第{digits}位まで）")
                    } else {
                        format!("（数値で答えます。単位: {unit}）")
                    }],
                ),
                Format::Journal => detail.push(
                    0,
                    vec!["（仕訳で答えます。科目と借方・貸方の金額を行ごとに入れます）".into()],
                ),
            }
            detail.push(0, vec![format!("関係する画面: {}", p.screen)]);
        }
        None => {
            for row in s.story.iter().flat_map(|p| wrap(p, WIDTH)) {
                detail.push(0, vec![row]);
            }
        }
    }
    let score = |x: Option<&study::Score>| {
        x.map_or_else(
            || "0 / 0".to_string(),
            |x| format!("{} / {}", x.solved, x.problems),
        )
    };
    let topics: Vec<String> = study::Topic::ALL
        .iter()
        .map(|t| format!("{} {}", t.label(), score(progress.topics.get(t.label()))))
        .collect();
    let summary = format!(
        "このシナリオ {}　全体 {}（{}）　復習 {}問 — 問題を選んで「答える...」",
        score(progress.scenarios.get(s.id)),
        score(Some(&progress.total)),
        topics.join("・"),
        progress.review.len()
    );
    View {
        main,
        detail,
        summary,
    }
}

/// Accounts offered for journal answers: (position + 1, label); 0 leaves the row empty.
pub(super) fn accounts(s: &Scenario) -> Vec<(i64, String)> {
    let category = |c: Category| c.label();
    std::iter::once((0, "（使わない）".to_string()))
        .chain(s.view().accounts.iter().enumerate().map(|(i, a)| {
            (
                i as i64 + 1,
                format!("{}｜{} {}", category(a.category), a.code, a.name),
            )
        }))
        .collect()
}
/// The answer form: its title and fields.
pub(super) fn form(s: &Scenario, p: &Problem) -> (String, Vec<Field>) {
    let title = format!("{} {}", p.id, p.title);
    let fields = match p.view().format {
        Format::Choice { options } => vec![Field::Choice(
            "答え",
            options
                .iter()
                .enumerate()
                .map(|(i, o)| format!("{}. {o}", i + 1))
                .collect(),
            0,
        )],
        Format::Number { unit, .. } => vec![Field::Text(
            match unit {
                "円" => "答え（円）",
                "%" => "答え（%）",
                "年" => "答え（年）",
                _ => "答え",
            },
            String::new(),
        )],
        Format::Journal => {
            let items = accounts(s);
            JOURNAL_LABELS
                .iter()
                .flat_map(|[account, debit, credit]| {
                    [
                        Field::Pick(account, items.clone(), 0),
                        Field::Text(debit, String::new()),
                        Field::Text(credit, String::new()),
                    ]
                })
                .collect()
        }
    };
    (title, fields)
}
/// Converts the form values back into an answer for `study::grade`.
pub(super) fn answer(s: &Scenario, p: &Problem, values: &[String]) -> Result<Answer> {
    Ok(match p.view().format {
        Format::Choice { .. } => Answer::Choice(values.first().ok_or("答えがありません")?.parse()?),
        Format::Number { .. } => {
            let v = values.first().map(|v| v.trim()).unwrap_or("");
            if v.is_empty() {
                return Err("答えを入力してください".into());
            }
            Answer::Number(v.into())
        }
        Format::Journal => {
            let accounts = s.view().accounts;
            let mut lines = vec![];
            for row in values.chunks(3) {
                let [account, debit, credit] = row else {
                    return Err("仕訳の入力が足りません".into());
                };
                let index: usize = account.parse().unwrap_or(0);
                let (debit, credit) = (debit.trim(), credit.trim());
                if index == 0 {
                    if !debit.is_empty() || !credit.is_empty() {
                        return Err("金額を入れた行の科目を選んでください".into());
                    }
                    continue;
                }
                let a = accounts.get(index - 1).ok_or("科目が見つかりません")?;
                lines.push(AnswerLine {
                    account: a.code.clone(),
                    debit: debit.into(),
                    credit: credit.into(),
                });
            }
            if lines.is_empty() {
                return Err("仕訳の行を入力してください".into());
            }
            Answer::Journal(lines)
        }
    })
}
/// Shown after grading: verdict, what was wrong, the expected answer and the explanation.
pub(super) fn result_text(p: &Problem, g: &Grade) -> String {
    let mut out = vec![if g.correct {
        "○ 正解です".to_string()
    } else {
        "× 不正解です".to_string()
    }];
    for f in &g.feedback {
        out.push(format!("・{f}"));
    }
    out.push(String::new());
    out.push(format!("正解：{}", g.expected));
    out.push(String::new());
    out.push(g.explanation.clone());
    if let Key::Choice { options, .. } = &p.key {
        out.push(String::new());
        out.push("選択肢ごとの解説".into());
        for (i, (option, reason)) in options.iter().zip(&g.reasons).enumerate() {
            out.push(format!("{}. {}：{reason}", i + 1, option.0));
        }
    }
    out.join("\r\n")
}
/// The scenario's story, facts and problem list.
pub(super) fn scenario_text(s: &Scenario) -> String {
    let mut out = vec![
        format!("■ {}", s.title),
        s.subtitle.to_string(),
        String::new(),
    ];
    out.extend(s.story.iter().cloned());
    out.push(String::new());
    out.push("■ 前提".into());
    for (k, v) in &s.facts {
        out.push(format!("・{k}：{v}"));
    }
    out.push(String::new());
    out.push("■ 問題".into());
    for p in &s.problems {
        out.push(format!(
            "{} {}（{}・{}）",
            p.id,
            p.title,
            p.topic.label(),
            level(p.level)
        ));
    }
    out.push(String::new());
    out.push("「練習用の帳簿を作る」で、このシナリオの取引・債権債務・計画・税額・ライフプランを入れた帳簿を作り、ふだんの画面で確かめられます。".into());
    out.join("\r\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    fn values(fields: &[Field], filled: &[(&str, &str)]) -> Vec<String> {
        fields
            .iter()
            .map(|f| {
                let (label, items) = match f {
                    Field::Text(l, _) => (*l, None),
                    Field::Choice(l, o, _) => (
                        *l,
                        Some(
                            o.iter()
                                .enumerate()
                                .map(|(i, t)| (i as i64, t.clone()))
                                .collect::<Vec<_>>(),
                        ),
                    ),
                    Field::Pick(l, items, _) => (*l, Some(items.clone())),
                };
                let wanted = filled.iter().find(|(k, _)| *k == label).map(|(_, v)| *v);
                match (wanted, items) {
                    (None, None) => String::new(),
                    (None, Some(_)) => "0".into(),
                    (Some(v), None) => v.into(),
                    // Like the smoke test: the first option containing the value.
                    (Some(v), Some(items)) => items
                        .iter()
                        .find(|(_, t)| t.contains(v))
                        .map(|(id, _)| id.to_string())
                        .unwrap(),
                }
            })
            .collect()
    }
    fn grade(id: &str, filled: &[(&str, &str)]) -> Grade {
        let (s, p) = study::find(id).unwrap();
        let (_, fields) = form(s, p);
        let a = answer(s, p, &values(&fields, filled)).unwrap();
        study::grade(id, &a).unwrap()
    }
    #[test]
    fn forms_round_trip_to_graded_answers() {
        let g = grade(
            "sato-04",
            &[
                ("1行目の科目", "121 売掛金"),
                ("1行目の借方", "296,307"),
                ("2行目の科目", "191 事業主貸"),
                ("2行目の借方", "33693"),
                ("3行目の科目", "401 売上高"),
                ("3行目の貸方", "３３００００"),
            ],
        );
        assert!(g.correct, "{:?}", g.feedback);
        let g = grade(
            "aoki-02",
            &[
                ("1行目の科目", "401 給与収入"),
                ("1行目の借方", "192000"),
                ("2行目の科目", "111 普通預金"),
                ("2行目の貸方", "192000"),
            ],
        );
        assert!(!g.correct && g.feedback.iter().any(|f| f.contains("逆")));
        assert!(grade("aoki-01", &[("答え", "特別支出")]).correct);
        assert!(grade("aoki-06", &[("答え（円）", "435,600")]).correct);
        assert!(grade("aoki-07", &[("答え（%）", "22.3")]).correct);
        assert!(grade("aoki-15", &[("答え（年）", "2046")]).correct);
        let (s, p) = study::find("aoki-02").unwrap();
        let (_, fields) = form(s, p);
        assert_eq!(fields.len(), JOURNAL_ROWS * 3);
        let mut v = values(&fields, &[("1行目の借方", "100")]);
        assert!(answer(s, p, &v).is_err(), "amount without an account");
        v[1].clear();
        assert!(answer(s, p, &v).is_err(), "no lines");
    }
    #[test]
    fn every_journal_key_fits_the_form_and_every_number_has_a_label() {
        for s in study::scenarios_all() {
            for p in &s.problems {
                if let Key::Journal(lines) = &p.key {
                    assert!(lines.len() <= JOURNAL_ROWS, "{}", p.id);
                }
                if let Format::Number { .. } = p.view().format {
                    let (_, f) = form(s, p);
                    assert!(
                        matches!(&f[0], Field::Text(l, _) if l.starts_with("答え（")),
                        "{}",
                        p.id
                    );
                }
            }
        }
    }
    #[test]
    fn view_lists_problems_progress_and_review() {
        let attempts = vec![
            Attempt {
                id: 1,
                problem: "aoki-01".into(),
                at: String::new(),
                correct: false,
                answer: serde_json::json!({}),
            },
            Attempt {
                id: 2,
                problem: "aoki-02".into(),
                at: String::new(),
                correct: true,
                answer: serde_json::json!({}),
            },
        ];
        let v = view(Some(0), false, Some(1), &attempts);
        assert_eq!(v.main.rows.len(), 16);
        assert_eq!(v.main.rows[0].1[4], "不正解");
        assert_eq!(v.main.rows[1].1[4], "正解");
        assert!(
            v.detail
                .rows
                .iter()
                .any(|r| r.1[0].starts_with("3. 特別支出"))
        );
        assert!(
            v.detail
                .rows
                .iter()
                .all(|r| r.1[0].chars().count() <= WIDTH)
        );
        assert!(v.summary.contains("このシナリオ 1 / 16") && v.summary.contains("復習 1問"));
        let review = view(Some(0), true, None, &attempts);
        assert_eq!(review.main.rows.len(), 1);
        assert_eq!(review.main.rows[0].0, 1);
        assert_eq!(view(Some(3), false, None, &[]).main.rows.len(), 11);
        assert_eq!(choices().len(), 4);
        let g = grade("aoki-01", &[("答え", "日用品")]);
        let text = result_text(study::find("aoki-01").unwrap().1, &g);
        assert!(
            text.starts_with("× 不正解")
                && text.contains("正解：特別支出")
                && text.contains("4. 雑費：")
        );
        assert!(scenario_text(scenario(Some(1))).contains("sato-14"));
    }
}
