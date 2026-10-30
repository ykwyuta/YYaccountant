use serde_json::json;
use std::collections::BTreeSet;
use yy_accountant::study::{self, Answer, AnswerLine, Key, Topic};
use yy_accountant::{Store, num};

fn number(id: &str) -> String {
    let (_, p) = study::find(id).unwrap();
    match &p.key {
        Key::Number { value, .. } => num::text(*value),
        _ => panic!("{id} is not a number problem"),
    }
}
fn line(account: &str, debit: &str, credit: &str) -> AnswerLine {
    AnswerLine {
        account: account.into(),
        debit: debit.into(),
        credit: credit.into(),
    }
}

#[test]
fn scenarios_cover_three_topics_with_unique_ids_and_complete_texts() {
    let all = study::scenarios_all();
    assert_eq!(all.len(), 4);
    let mut ids = BTreeSet::new();
    let mut topics = BTreeSet::new();
    for s in all {
        assert!(s.problems.len() >= 10, "{}", s.id);
        assert!(!s.story.is_empty() && !s.facts.is_empty());
        for p in &s.problems {
            assert!(ids.insert(p.id.clone()), "duplicate {}", p.id);
            topics.insert(p.topic);
            assert!(p.explanation.chars().count() > 40, "{}", p.id);
        }
    }
    assert_eq!(topics, BTreeSet::from(Topic::ALL));
    assert!(ids.len() >= 50);
}

/// Answer keys come from the engines; these are the values worked out by hand.
#[test]
fn answer_keys_match_hand_calculations() {
    for (id, expected) in [
        // 4〜6月の預金: 320,000 + 576,000 − 216,000 − 14,940 − 16,600 − 121,680 − 41,180 − 20,000 − 30,000
        ("aoki-06", "435600"),
        // (576,000 − 447,750) ÷ 576,000 = 22.265…%
        ("aoki-07", "22.3"),
        // 別表第五: 2,370,000 → 2,368,000 × 70% − 80,000
        ("aoki-09", "1577600"),
        // 140,000 − 1,577,600 × 5%
        ("aoki-10", "61120"),
        // 課税所得 120,000 × 5% × 1.021 = 6,126、源泉 25,600 との差
        ("aoki-12", "19474"),
        // 360,000 × 47.575416（年金終価係数 3%・30年）
        ("aoki-13", "17127150"),
        // 1,500,000 × 0.196040（減債基金係数 1%・5年）
        ("aoki-14", "294060"),
        ("aoki-15", "2046"),
        // 352,000 ÷ 4 × 6/12
        ("sato-02", "44000"),
        ("sato-07", "296307"),
        // 1,650,000 − (330,000 + 52,800 + 77,760 + 10,560 + 25,000 + 44,000)
        ("sato-08", "1109880"),
        // 3,720,000 + (1,109,880 − 650,000)
        ("sato-09", "4179880"),
        // 元利均等 40,000,000・月利 1%/12・420回
        ("yamamoto-01", "112914"),
        ("yamamoto-02", "7423880"),
        // 1,100,000 − 740,000（令和8年分の給与所得控除の最低額）
        ("yamamoto-05", "360000"),
        ("yamamoto-06", "380000"),
        // min(残高, 3,000万) × 0.7% = 210,000 のうち所得税 133,600、残り 76,400 を住民税
        ("yamamoto-08", "133600"),
        ("yamamoto-09", "76400"),
        // (15kg × 240 + 25kg × 264) ÷ 40kg
        ("komugi-02", "255"),
        // 1,200,000 ÷ 8 ÷ 1,500 + 90
        ("komugi-04", "190"),
        // 78.125 + 27 + 7.6 + 156 = 268.725、× 1.1 = 295.5975
        ("komugi-05", "296"),
        ("komugi-06", "34.3"),
        // 39 × 255 + 1.8 × 1,800 − 105.125 × 120
        ("komugi-07", "570"),
        // 13,185 + 912 + 19,500 + 3,360
        ("komugi-08", "36957"),
        // 1,200,000 ÷ 8 × 9/12
        ("komugi-11", "112500"),
    ] {
        assert_eq!(number(id), expected, "{id}");
    }
    let (s, p) = study::find("sato-04").unwrap();
    assert_eq!(
        s.expected(p),
        "借方 売掛金 296,307、事業主貸 33,693 ／ 貸方 売上高 330,000"
    );
    let (s, p) = study::find("komugi-09").unwrap();
    // 4/10: 10 × 240 + 0.5 × 1,800 = 3,300、4/20: 13,185
    assert_eq!(s.expected(p), "借方 材料費 16,485 ／ 貸方 原材料 16,485");
}

#[test]
fn grading_explains_what_is_wrong() {
    let g = study::grade(
        "aoki-02",
        &Answer::Journal(vec![
            line("111", "192,000", ""),
            line("401", "", "１９２０００"),
        ]),
    )
    .unwrap();
    assert!(g.correct && g.feedback.is_empty());
    let g = study::grade(
        "aoki-02",
        &Answer::Journal(vec![line("401", "192000", ""), line("111", "", "192000")]),
    )
    .unwrap();
    assert!(!g.correct);
    assert!(
        g.feedback
            .iter()
            .any(|f| f.contains("普通預金") && f.contains("逆"))
    );
    let g = study::grade(
        "aoki-03",
        &Answer::Journal(vec![line("501", "121680", ""), line("111", "", "121680")]),
    )
    .unwrap();
    assert!(
        g.feedback
            .iter()
            .any(|f| f.contains("クレジットカード") && f.contains("入っていません"))
    );
    assert!(
        g.feedback
            .iter()
            .any(|f| f.contains("食費") && f.contains("使いません"))
    );
    let g = study::grade(
        "sato-04",
        &Answer::Journal(vec![
            line("121", "330000", ""),
            line("401", "", "330000"),
            line("191", "33693", ""),
        ]),
    )
    .unwrap();
    assert!(g.feedback.iter().any(|f| f.contains("一致していません")));
    assert!(
        g.feedback
            .iter()
            .any(|f| f.contains("売掛金") && f.contains("金額"))
    );
    assert!(study::grade("aoki-02", &Answer::Journal(vec![line("999", "1", "")])).is_err());

    let g = study::grade("aoki-01", &Answer::Choice(0)).unwrap();
    assert!(!g.correct && g.reasons.len() == 4 && g.feedback[0].contains("日用品"));
    assert!(study::grade("aoki-01", &Answer::Choice(2)).unwrap().correct);
    assert!(study::grade("aoki-01", &Answer::Choice(9)).is_err());

    assert!(
        study::grade("aoki-06", &Answer::Number("435,600円".into()))
            .unwrap()
            .correct
    );
    assert!(
        study::grade("aoki-07", &Answer::Number("22.26".into()))
            .unwrap()
            .correct
    );
    let near = study::grade("aoki-09", &Answer::Number("1579000".into())).unwrap();
    assert!(!near.correct && near.feedback.iter().any(|f| f.contains("1%以内")));
    let sign = study::grade("aoki-12", &Answer::Number("-19474".into())).unwrap();
    assert!(sign.feedback.iter().any(|f| f.contains("符号")));
    assert!(study::grade("aoki-06", &Answer::Choice(0)).is_err());
    assert!(study::grade("nope-01", &Answer::Choice(0)).is_err());
}

#[test]
fn practice_books_replay_into_sqlite_and_progress_is_recorded() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("study.sqlite3");
    let mut store = Store::open(&path, false).unwrap();
    for s in study::scenarios_all() {
        let (request, changes) = study::practice(s.id).unwrap();
        let mut book = store.create(&request, "learner").unwrap();
        for c in changes {
            book = store
                .apply(book.id, book.revision, c, "learner")
                .unwrap()
                .book;
        }
        // Same IDs and records as the in-memory scenario the problems were computed from.
        assert_eq!(book.entries, s.book.entries, "{}", s.id);
        assert_eq!(book.tax_returns, s.book.tax_returns);
        assert_eq!(book.life_plans, s.book.life_plans);
        assert_eq!(book.claims, s.book.claims);
    }
    store
        .record_attempt("hina", "aoki-01", &json!({"choice":0}), false)
        .unwrap();
    store
        .record_attempt("hina", "aoki-01", &json!({"choice":2}), true)
        .unwrap();
    store
        .record_attempt("hina", "sato-02", &json!({"number":"88000"}), false)
        .unwrap();
    store
        .record_attempt("ken", "sato-02", &json!({"number":"44000"}), true)
        .unwrap();
    let p = study::progress(&store.attempts("hina").unwrap());
    assert_eq!(p.problems["aoki-01"].attempts, 2);
    assert!(p.problems["aoki-01"].solved && p.problems["aoki-01"].last_correct);
    assert_eq!(p.total.solved, 1);
    assert_eq!(p.scenarios["aoki"].solved, 1);
    assert_eq!(p.review, vec!["sato-02".to_string()]);
    assert_eq!(
        study::progress(&store.attempts("ken").unwrap()).topics["会計"].solved,
        1
    );
    drop(store);
    let mut ro = Store::open(&path, true).unwrap();
    assert_eq!(ro.attempts("hina").unwrap().len(), 3);
    assert!(
        ro.record_attempt("hina", "aoki-01", &json!({}), true)
            .is_err()
    );
}

#[test]
fn version_2_files_gain_the_study_table() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("old.sqlite3");
    Store::open(&path, false).unwrap();
    {
        let c = rusqlite::Connection::open(&path).unwrap();
        c.execute_batch("DROP TABLE study_attempts; PRAGMA user_version=2;")
            .unwrap();
    }
    // Read-only opening of an old file works and reports no attempts.
    assert!(
        Store::open(&path, true)
            .unwrap()
            .attempts("x")
            .unwrap()
            .is_empty()
    );
    let mut store = Store::open(&path, false).unwrap();
    store
        .record_attempt("x", "aoki-01", &json!({"choice":2}), true)
        .unwrap();
    let c = rusqlite::Connection::open(&path).unwrap();
    let v: i64 = c
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .unwrap();
    assert_eq!(v, 3);
}

#[test]
fn problem_views_hide_answers() {
    for s in study::scenarios_all() {
        let text = serde_json::to_string(&s.view()).unwrap();
        for p in &s.problems {
            assert!(
                !text.contains(&p.explanation),
                "{} leaks its explanation",
                p.id
            );
        }
        assert!(!text.contains("正解です"));
    }
}
