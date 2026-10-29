use serde_json::json;
use yy_accountant::num::{Decimal, parse};
use yy_accountant::*;

fn d(s: &str) -> chrono::NaiveDate {
    s.parse().unwrap()
}
fn n(s: &str) -> Decimal {
    parse(s).unwrap()
}
fn store() -> (tempfile::TempDir, Store) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("家計 帳簿.sqlite3"), false).unwrap();
    (dir, store)
}
fn change(v: serde_json::Value) -> Change {
    serde_json::from_value(v).unwrap()
}
fn code(book: &Book, code: &str) -> i64 {
    book.accounts.iter().find(|a| a.code == code).unwrap().id
}
fn apply(store: &mut Store, book: &mut Book, v: serde_json::Value) -> Option<i64> {
    let applied = store
        .apply(book.id, book.revision, &change(v), "test")
        .unwrap();
    *book = applied.book;
    applied.id
}
fn new_book(store: &mut Store, kind: &str) -> Book {
    store
        .create(
            &serde_json::from_value(json!({"name":"テスト","kind":kind})).unwrap(),
            "test",
        )
        .unwrap()
}

#[test]
fn quick_entries_are_balanced_decimal_and_survive_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("book.sqlite3");
    let mut store = Store::open(&path, false).unwrap();
    let mut book = store
        .create(
            &serde_json::from_value(json!({"name":"USD","minor_digits":2,"currency":"USD"}))
                .unwrap(),
            "test",
        )
        .unwrap();
    let food = code(&book, "501");
    let cash = code(&book, "101");
    for _ in 0..3 {
        apply(
            &mut store,
            &mut book,
            json!({"operation":"quick_entry","quick":{"kind":"expense","date":"2026-10-01","amount":"0.10","account":food,"via":cash,"description":"飴"}}),
        );
    }
    let tb = reports::trial_balance(&book, None, d("2026-12-31"));
    let food_row = tb.rows.iter().find(|r| r.account == food).unwrap();
    assert_eq!(food_row.closing.0, n("0.3"));
    assert_eq!(tb.total_debit, tb.total_credit);
    // Too many fraction digits for the currency, float JSON numbers and unbalanced entries are rejected.
    let r = store.apply(book.id, book.revision, &change(json!({"operation":"quick_entry","quick":{"kind":"expense","date":"2026-10-01","amount":"0.001","account":food,"via":cash}})), "t");
    assert!(r.is_err());
    assert!(serde_json::from_value::<Change>(json!({"operation":"quick_entry","quick":{"kind":"expense","date":"2026-10-01","amount":0.1,"account":food,"via":cash}})).is_err());
    let r = store.apply(book.id, book.revision, &change(json!({"operation":"save_entry","entry":{"date":"2026-10-02","lines":[{"account":food,"debit":"1"},{"account":cash,"credit":"2"}]}})), "t");
    assert!(r.unwrap_err().to_string().contains("一致しません"));
    drop(store);
    let mut reopened = Store::open(&path, true).unwrap();
    let again = reopened.get(book.id).unwrap();
    assert_eq!(again, book);
    assert!(
        reopened
            .apply(
                book.id,
                book.revision,
                &change(json!({"operation":"void_entry","id":1,"void":true})),
                "t"
            )
            .is_err()
    );
    // Decimals are TEXT in SQLite, never REAL.
    let c = rusqlite::Connection::open(&path).unwrap();
    let kinds: Vec<String> = c
        .prepare("SELECT DISTINCT typeof(debit) FROM entry_lines")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<std::result::Result<_, _>>()
        .unwrap();
    assert_eq!(kinds, vec!["text".to_string()]);
}

#[test]
fn revisions_conflict_and_audit_records_every_change() {
    let (_dir, mut store) = store();
    let mut book = new_book(&mut store, "household");
    let stale = book.revision;
    apply(
        &mut store,
        &mut book,
        json!({"operation":"save_partner","partner":{"name":"スーパー"}}),
    );
    let err = store
        .apply(
            book.id,
            stale,
            &change(json!({"operation":"save_partner","partner":{"name":"別"}})),
            "other",
        )
        .unwrap_err();
    assert!(matches!(err, Error::Conflict));
    let audit = store.audit(book.id, 0).unwrap();
    assert_eq!(audit.len(), 2);
    assert_eq!(audit[1]["change"]["operation"], "save_partner");
    assert_eq!(audit[1]["change"]["result_id"], 1);
    assert_eq!(store.get(book.id).unwrap().partners.len(), 1);
}

#[test]
fn void_keeps_history_and_ids_are_never_reused() {
    let (_dir, mut store) = store();
    let mut book = new_book(&mut store, "household");
    let (food, cash) = (code(&book, "501"), code(&book, "101"));
    let q = json!({"operation":"quick_entry","quick":{"kind":"expense","date":"2026-10-01","amount":"500","account":food,"via":cash}});
    let first = apply(&mut store, &mut book, q.clone()).unwrap();
    apply(
        &mut store,
        &mut book,
        json!({"operation":"void_entry","id":first,"void":true}),
    );
    let tb = reports::trial_balance(&book, None, d("2026-12-31"));
    assert!(tb.rows.iter().all(|r| r.closing.0.is_zero()));
    let second = apply(&mut store, &mut book, q).unwrap();
    assert_eq!(second, first + 1);
    let mid = apply(
        &mut store,
        &mut book,
        json!({"operation":"save_account","account":{"code":"699","name":"ペット","category":"expense"}}),
    )
    .unwrap();
    apply(
        &mut store,
        &mut book,
        json!({"operation":"remove","target":"account","id":mid}),
    );
    let next = apply(
        &mut store,
        &mut book,
        json!({"operation":"save_account","account":{"code":"699","name":"ペット","category":"expense"}}),
    )
    .unwrap();
    assert_eq!(next, mid + 1);
    // Accounts in use cannot be removed.
    assert!(
        store
            .apply(
                book.id,
                book.revision,
                &change(json!({"operation":"remove","target":"account","id":food})),
                "t"
            )
            .is_err()
    );
}

#[test]
fn business_ratio_splits_private_share_and_statements_balance() {
    let (_dir, mut store) = store();
    let mut book = new_book(&mut store, "sole_proprietor");
    let (rent, bank, sales, recv) = (
        code(&book, "526"),
        code(&book, "111"),
        code(&book, "401"),
        code(&book, "121"),
    );
    apply(
        &mut store,
        &mut book,
        json!({"operation":"quick_entry","quick":{"kind":"expense","date":"2026-01-25","amount":"100001","account":rent,"via":bank,"business_percent":"30"}}),
    );
    let e = book.entries.last().unwrap();
    assert_eq!(e.lines[0].debit, n("30000")); // 30000.3 rounded half-up
    assert_eq!(e.lines[1].debit, n("70001"));
    assert_eq!(e.lines[1].account, code(&book, "191"));
    apply(
        &mut store,
        &mut book,
        json!({"operation":"save_entry","entry":{"date":"2026-01-31","description":"制作代金","lines":[{"account":recv,"debit":"550000"},{"account":sales,"credit":"550000"}]}}),
    );
    let pl = reports::income_statement(&book, d("2026-01-01"), d("2026-12-31"));
    assert_eq!(pl.net_income.0, n("520000"));
    let bs = reports::balance_sheet(&book, d("2026-12-31"));
    assert!(bs.balanced);
    let ledger = reports::ledger(&book, bank, d("2026-01-01"), d("2026-12-31")).unwrap();
    assert_eq!(ledger.closing.0, n("-100001"));
    assert_eq!(ledger.rows[0].counter, "諸口");
    let csv = reports::journal_csv(&book, None, None);
    assert!(
        csv.starts_with('\u{feff}') && csv.contains("地代家賃,30000,") && csv.ends_with("\r\n")
    );
}

#[test]
fn claims_schedule_settle_and_age() {
    let (_dir, mut store) = store();
    let mut book = new_book(&mut store, "household");
    let (loan, bank) = (code(&book, "211"), code(&book, "111"));
    let id = apply(
        &mut store,
        &mut book,
        json!({"operation":"save_claim","opening":bank,"claim":{"side":"payable","title":"自動車ローン","account":loan,"principal":"1200000","issued":"2026-01-10","due":"2026-02-27","annual_rate":"2.4","repayment":"equal_payment","installments":12}}),
    )
    .unwrap();
    let claim = book.claims[0].clone();
    let schedule = claims::schedule(&book, &claim).unwrap();
    assert_eq!(schedule.len(), 12);
    let total: Decimal = schedule.iter().map(|i| i.principal.0).sum();
    assert_eq!(total, n("1200000"));
    assert_eq!(schedule[0].interest.0, n("2400"));
    assert_eq!(schedule[0].payment.0, n("101305"));
    assert_eq!(schedule[1].date, d("2026-03-27"));
    assert!(schedule.last().unwrap().remaining.0.is_zero());
    apply(
        &mut store,
        &mut book,
        json!({"operation":"settle_claim","settlement":{"claim":id,"date":"2026-02-27","principal":schedule[0].principal,"interest":schedule[0].interest,"via":bank}}),
    );
    let report = claims::report(&book, d("2026-05-01"), false).unwrap();
    let s = &report.claims[0];
    assert_eq!(s.repaid.0, schedule[0].principal.0);
    assert_eq!(
        s.arrears.0,
        schedule[1].principal.0 + schedule[2].principal.0
    );
    assert_eq!(s.days_overdue, 35);
    assert_eq!(s.bucket, "overdue_31_90");
    let interest = code(&book, "661");
    let tb = reports::trial_balance(&book, None, d("2026-12-31"));
    assert_eq!(
        tb.rows
            .iter()
            .find(|r| r.account == interest)
            .unwrap()
            .closing
            .0,
        n("2400")
    );
    let over = store.apply(book.id, book.revision, &change(json!({"operation":"settle_claim","settlement":{"claim":id,"date":"2026-03-27","principal":"9999999","via":bank}})), "t");
    assert!(over.is_err());
}

#[test]
fn budgets_compare_fiscal_months() {
    let (_dir, mut store) = store();
    let mut book = new_book(&mut store, "household");
    apply(
        &mut store,
        &mut book,
        json!({"operation":"update_book","name":"家計","currency":"JPY","minor_digits":0,"rounding":"half_up","fiscal_start_month":4}),
    );
    let (food, cash, salary) = (code(&book, "501"), code(&book, "101"), code(&book, "401"));
    let mut amounts = vec!["40000"; 12];
    amounts[0] = "50000";
    let id = apply(
        &mut store,
        &mut book,
        json!({"operation":"save_budget","budget":{"name":"2026年度","fiscal_year":2026,"lines":[{"account":food,"amounts":amounts}]}}),
    )
    .unwrap();
    apply(
        &mut store,
        &mut book,
        json!({"operation":"quick_entry","quick":{"kind":"expense","date":"2026-04-05","amount":"55000","account":food,"via":cash}}),
    );
    apply(
        &mut store,
        &mut book,
        json!({"operation":"quick_entry","quick":{"kind":"income","date":"2027-03-25","amount":"300000","account":salary,"via":cash}}),
    );
    let r = budget::report(&book, id).unwrap();
    assert_eq!(r.months[0], "2026-04");
    assert_eq!(r.months[11], "2027-03");
    let row = r.rows.iter().find(|r| r.account == food).unwrap();
    assert_eq!(row.variance[0].0, n("5000"));
    assert_eq!(row.total_budget.0, n("490000"));
    assert!(r.rows.iter().any(|r| r.account == salary && r.unbudgeted));
    assert_eq!(r.income.actual[11].0, n("300000"));
}

#[test]
fn plans_post_once_and_forecast_combines_history_and_schedule() {
    let (_dir, mut store) = store();
    let mut book = new_book(&mut store, "household");
    let (food, utility, bank, rent) = (
        code(&book, "501"),
        code(&book, "531"),
        code(&book, "111"),
        code(&book, "521"),
    );
    for (i, m) in (1..=12).enumerate() {
        apply(
            &mut store,
            &mut book,
            json!({"operation":"quick_entry","quick":{"kind":"expense","date":format!("2025-{m:02}-10"),"amount":(30000 + 1000 * i as i64).to_string(),"account":food,"via":bank}}),
        );
        apply(
            &mut store,
            &mut book,
            json!({"operation":"quick_entry","quick":{"kind":"expense","date":format!("2025-{m:02}-20"),"amount":if m == 1 || m == 2 {"15000"} else {"8000"},"account":utility,"via":bank}}),
        );
    }
    let plan = apply(
        &mut store,
        &mut book,
        json!({"operation":"save_plan","plan":{"name":"家賃","debit":rent,"credit":bank,"amount":"80000","start":"2025-11-27","frequency":"monthly"}}),
    )
    .unwrap();
    apply(
        &mut store,
        &mut book,
        json!({"operation":"post_plan","plan":plan,"occurrence":"2025-11-27"}),
    );
    let dup = store.apply(
        book.id,
        book.revision,
        &change(json!({"operation":"post_plan","plan":plan,"occurrence":"2025-11-27"})),
        "t",
    );
    assert!(dup.unwrap_err().to_string().contains("計上済み"));
    let not_planned = store.apply(
        book.id,
        book.revision,
        &change(json!({"operation":"post_plan","plan":plan,"occurrence":"2025-11-28"})),
        "t",
    );
    assert!(not_planned.is_err());
    let cal = plans::calendar(&book, d("2025-11-01"), d("2025-12-31"));
    assert_eq!(cal.len(), 2);
    assert!(cal[0].posted && !cal[1].posted);

    let f = forecast::forecast(
        &book,
        &serde_json::from_value(json!({"as_of":"2026-01-05","months":3,"history_months":12}))
            .unwrap(),
    )
    .unwrap();
    assert_eq!(f.months, vec!["2026-02", "2026-03", "2026-04"]);
    let food_f = f.accounts.iter().find(|a| a.account == food).unwrap();
    assert_eq!(food_f.method, forecast::Method::Linear);
    // History ends 2025-12 and January is skipped as the partial current month: 41000 + 2 * 1000.
    assert_eq!(food_f.months[0].baseline.0, n("43000"));
    assert_eq!(food_f.error.unwrap().0, Decimal::ZERO);
    let rent_f = f.accounts.iter().find(|a| a.account == rent).unwrap();
    // Plan-posted history is excluded from the baseline and re-added as scheduled amounts.
    assert!(rent_f.months[0].baseline.0.is_zero());
    assert_eq!(rent_f.months[0].scheduled.0, n("80000"));
    let total = &f.totals[0];
    assert_eq!(
        total.expense.0,
        f.accounts
            .iter()
            .filter(|a| a.category == Category::Expense)
            .map(|a| a.months[0].total.0)
            .sum::<Decimal>()
    );
    assert!(total.expense_low.0 <= total.expense.0 && total.expense.0 <= total.expense_high.0);

    let stats = reports::statistics(&book, d("2025-01-01"), d("2025-12-31")).unwrap();
    let u = stats
        .accounts
        .iter()
        .find(|a| a.account == utility)
        .unwrap();
    assert_eq!(u.total.0, n("110000"));
    assert_eq!(u.median.0, n("8000"));
    assert_eq!(stats.months.len(), 12);
}

#[test]
fn cost_management_moving_average_depreciation_and_variance() {
    let (_dir, mut store) = store();
    let mut book = new_book(&mut store, "sole_proprietor");
    let flour = apply(
        &mut store,
        &mut book,
        json!({"operation":"save_material","material":{"name":"小麦粉","unit":"kg","standard_cost":"200"}}),
    )
    .unwrap();
    let butter = apply(
        &mut store,
        &mut book,
        json!({"operation":"save_material","material":{"name":"バター","unit":"kg","standard_cost":"1500"}}),
    )
    .unwrap();
    for (m, date, q, c) in [
        (flour, "2026-01-05", "10", "180"),
        (flour, "2026-01-20", "10", "220"),
        (butter, "2026-01-05", "2", "1500"),
    ] {
        apply(
            &mut store,
            &mut book,
            json!({"operation":"save_material_move","material_move":{"material":m,"date":date,"kind":"purchase","quantity":q,"unit_cost":c}}),
        );
    }
    let oven = apply(
        &mut store,
        &mut book,
        json!({"operation":"save_equipment","equipment":{"name":"オーブン","acquired":"2025-07-15","cost":"600000","salvage":"0","life_years":5,"annual_hours":"1000","running_cost_per_hour":"30"}}),
    )
    .unwrap();
    let bread = apply(
        &mut store,
        &mut book,
        json!({"operation":"save_product","product":{"name":"食パン","unit":"斤","labor_hours":"0.1","labor_rate":"1200","overhead_percent":"10",
            "materials":[{"material":flour,"quantity":"0.25","loss_percent":"20"},{"material":butter,"quantity":"0.02"}],
            "equipment":[{"equipment":oven,"hours":"0.05"}]}}),
    )
    .unwrap();
    apply(
        &mut store,
        &mut book,
        json!({"operation":"save_production","production":{"product":bread,"date":"2026-01-25","quantity":"40","labor_hours":"5",
            "materials":[{"material":flour,"quantity":"12"},{"material":butter,"quantity":"0.8"}]}}),
    );
    let r = cost::report(&book, d("2026-01-31")).unwrap();
    let flour_stock = r.stock.iter().find(|s| s.material == flour).unwrap();
    assert_eq!(flour_stock.quantity.0, n("8"));
    assert_eq!(flour_stock.average_cost.0, n("200"));
    assert_eq!(flour_stock.value.0, n("1600"));
    let eq = &r.equipment[0];
    assert_eq!(eq.hourly_rate.0, n("150")); // 600000/5/1000 + 30
    assert_eq!(eq.schedule[0].fiscal_year, 2025);
    assert_eq!(eq.schedule[0].amount.0, n("60000")); // Jul-Dec: 6 of 60 months
    let total: Decimal = eq.schedule.iter().map(|r| r.amount.0).sum();
    assert_eq!(total, n("600000"));
    let st = &r.products[0].standard;
    // flour 0.25/(1-0.2)=0.3125kg*200=62.5, butter 0.02*1500=30, oven 0.05h*150=7.5, labor 120, overhead 22
    assert_eq!(st.materials.0, n("92.5"));
    assert_eq!(st.equipment.0, n("7.5"));
    assert_eq!(st.labor.0, n("120"));
    assert_eq!(st.overhead.0, n("22"));
    assert_eq!(st.total.0, n("242"));
    let p = &r.productions[0];
    // flour 12*200=2400, butter 0.8*1500=1200, oven 40*0.05h*150=300, labor 5*1200=6000, overhead 990
    assert_eq!(p.actual.total.0, n("10890"));
    assert_eq!(p.standard_total.0, n("9680"));
    assert_eq!(p.variance.0, n("1210"));
    assert_eq!(p.actual_unit_cost.0, n("272.25"));
    apply(
        &mut store,
        &mut book,
        json!({"operation":"post_depreciation","equipment":oven,"fiscal_year":2025,"date":"2025-12-31"}),
    );
    let r = cost::report(&book, d("2026-01-31")).unwrap();
    assert!(r.equipment[0].schedule[0].posted);
    assert!(
        store
            .apply(
                book.id,
                book.revision,
                &change(json!({"operation":"post_depreciation","equipment":oven,"fiscal_year":2025,"date":"2025-12-31"})),
                "t"
            )
            .is_err()
    );
}

#[test]
fn account_guide_finds_accounts_and_distinctions() {
    let (_dir, mut store) = store();
    let book = new_book(&mut store, "sole_proprietor");
    let g = chart::guide(&book, "ガソリン", None);
    assert_eq!(g.suggestions[0].name, "車両費");
    assert!(g.distinctions.iter().any(|d| d.title.contains("車両費")));
    let g = chart::guide(&book, "住民税", None);
    assert_eq!(g.suggestions[0].name, "事業主貸");
    let all = chart::guide(&book, "", None);
    assert!(all.distinctions.len() >= 15);
    let household = new_book(&mut store, "household");
    let g = chart::guide(&household, "スーパー", None);
    assert_eq!(g.suggestions[0].name, "食費");
}

#[test]
fn invalid_files_and_read_only_are_refused() {
    let dir = tempfile::tempdir().unwrap();
    let other = dir.path().join("other.sqlite3");
    rusqlite::Connection::open(&other)
        .unwrap()
        .execute_batch("CREATE TABLE x(a)")
        .unwrap();
    assert!(Store::open(&other, false).is_err());
    let path = dir.path().join("ok.sqlite3");
    Store::open(&path, false).unwrap();
    let mut ro = Store::open(&path, true).unwrap();
    assert!(
        ro.create(&serde_json::from_value(json!({"name":"x"})).unwrap(), "t")
            .is_err()
    );
}

#[test]
fn tax_returns_and_life_plans_round_trip_and_old_files_migrate() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("v1.sqlite3");
    {
        // A version 1 file: same tables minus the new ones.
        let mut store = Store::open(&path, false).unwrap();
        new_book(&mut store, "household");
    }
    {
        let c = rusqlite::Connection::open(&path).unwrap();
        c.execute_batch(
            "DROP TABLE tax_dependents;DROP TABLE tax_returns;DROP TABLE life_plan_members;DROP TABLE life_plan_flows;DROP TABLE life_plan_events;DROP TABLE life_plans;PRAGMA user_version=1;",
        )
        .unwrap();
    }
    // Read-only keeps the file as it is and shows no tax returns.
    let mut ro = Store::open(&path, true).unwrap();
    assert!(ro.get(1).unwrap().tax_returns.is_empty());
    drop(ro);
    let mut store = Store::open(&path, false).unwrap();
    let mut book = store.get(1).unwrap();
    let id = apply(
        &mut store,
        &mut book,
        json!({"operation":"save_tax_return","tax_return":{"year":2026,"salary":"6000000","social_insurance":"870000",
            "has_spouse":true,"spouse_income":"260000","spouse_birth_year":1988,
            "dependents":[{"name":"長男","birth_year":2010,"cohabiting":true},
                          {"name":"父","relation":"parent","birth_year":1954,"income":"400000","cohabiting":true}],
            "donation":"30000","housing_balance":"25000000","housing_limit":"40000000","housing_rate":"0.7",
            "housing_resident_cap":"97500","per_capita":"5000"}}),
    )
    .unwrap();
    let plan = apply(
        &mut store,
        &mut book,
        json!({"operation":"save_life_plan","life_plan":{"name":"現状","start_year":2027,"years":20,"savings":"3000000","return_percent":"1",
            "members":[{"name":"本人","relation":"本人","birth_year":1985}],
            "flows":[{"name":"手取り","kind":"income","amount":"5400000","start_year":2027,"end_year":2046},
                     {"name":"生活費","kind":"expense","amount":"4200000","start_year":2027,"end_year":2046,"growth_percent":"1.5"}],
            "events":[{"year":2030,"name":"車","kind":"expense","amount":"3000000"}]}}),
    )
    .unwrap();
    drop(store);
    let mut store = Store::open(&path, false).unwrap();
    let book = store.get(1).unwrap();
    let t = book.tax_returns.iter().find(|t| t.id == id).unwrap();
    assert_eq!(t.name, "2026年分");
    assert_eq!(t.dependents.len(), 2);
    assert_eq!(t.housing_rate, n("0.7"));
    let r = tax::calculate(&book, t).unwrap();
    assert_eq!(r.total_income.0, n("4360000"));
    // 2026年分の基礎控除は104万円、住民税は43万円
    let basic = r.lines.iter().find(|l| l.label == "基礎控除").unwrap();
    assert_eq!(
        (basic.income_tax.unwrap().0, basic.resident_tax.unwrap().0),
        (n("1040000"), n("430000"))
    );
    let p = book.life_plans.iter().find(|p| p.id == plan).unwrap();
    let cf = lifeplan::cash_flow(&book, p).unwrap();
    assert_eq!(cf.rows.len(), 20);
    assert_eq!(cf.rows[3].events[0].1.0, n("-3000000"));
    // An unsupported year and a non-JPY book are refused.
    let mut wrong = t.clone();
    wrong.id = 0;
    wrong.year = 2019;
    assert!(
        store
            .apply(
                1,
                book.revision,
                &Change::SaveTaxReturn {
                    tax_return: Box::new(wrong),
                },
                "test"
            )
            .is_err()
    );
}
