//! Deterministic demo data: a household book and a sole proprietor (bakery) book with
//! about eighteen months of history ending at `today`, plus plans, claims, budgets and costs.
use chrono::{Datelike, Months, NaiveDate};
use serde_json::{Value, json};
use std::path::Path;
use yy_accountant::{Book, Change, NewBook, Store, lifeplan, num, tax};
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

struct Demo<'a> {
    store: &'a mut Store,
    book: Book,
    seed: u64,
}
impl Demo<'_> {
    fn apply(&mut self, change: Value) -> Result<i64> {
        let parsed: Change = serde_json::from_value(change.clone())?;
        let applied = self
            .store
            .apply(self.book.id, self.book.revision, &parsed, "demo")
            .map_err(|e| format!("{e}: {change}"))?;
        self.book = applied.book;
        Ok(applied.id.unwrap_or(0))
    }
    fn code(&self, code: &str) -> i64 {
        self.book
            .accounts
            .iter()
            .find(|a| a.code == code)
            .map(|a| a.id)
            .unwrap_or_else(|| panic!("demo account {code}"))
    }
    /// Deterministic integer noise in 0..range.
    fn noise(&mut self, range: u64) -> i64 {
        self.seed = self
            .seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((self.seed >> 33) % range.max(1)) as i64
    }
    fn quick(
        &mut self,
        kind: &str,
        date: NaiveDate,
        amount: i64,
        account: &str,
        via: &str,
        text: &str,
    ) -> Result<i64> {
        let (a, v) = (self.code(account), self.code(via));
        self.apply(json!({"operation":"quick_entry","quick":{"kind":kind,"date":date,"amount":amount.to_string(),"account":a,"via":v,"description":text}}))
    }
}
fn day(month: NaiveDate, d: u32) -> NaiveDate {
    month.with_day(d.min(28)).unwrap_or(month)
}
fn months_back(today: NaiveDate, n: u32) -> NaiveDate {
    today
        .with_day(1)
        .unwrap()
        .checked_sub_months(Months::new(n))
        .unwrap()
}

pub fn create(path: &Path, today: NaiveDate) -> Result<()> {
    if path.exists() {
        return Err("Demo database must be a new file".into());
    }
    let mut store = Store::open(path, false)?;
    household(&mut store, today)?;
    bakery(&mut store, today)?;
    Ok(())
}

fn household(store: &mut Store, today: NaiveDate) -> Result<()> {
    let book = store.create(
        &serde_json::from_value::<NewBook>(json!({"name":"わが家の家計","kind":"household"}))?,
        "demo",
    )?;
    let mut d = Demo {
        store,
        book,
        seed: 7,
    };
    let start = months_back(today, 18);
    let (bank, cash, open) = (d.code("111"), d.code("101"), d.code("301"));
    d.apply(json!({"operation":"save_entry","entry":{"date":start,"description":"記帳開始時の残高","lines":[
        {"account":bank,"debit":"1450000"},{"account":cash,"debit":"32000"},{"account":open,"credit":"1482000"}]}}))?;
    let super_ =
        d.apply(json!({"operation":"save_partner","partner":{"name":"まるやすスーパー"}}))?;
    let landlord =
        d.apply(json!({"operation":"save_partner","partner":{"name":"みどり不動産"}}))?;
    let rent = d.apply(json!({"operation":"save_plan","plan":{"name":"家賃","debit":d.code("521"),"credit":bank,"amount":"85000","start":day(start,27),"frequency":"monthly","partner":landlord}}))?;
    let nisa = d.apply(json!({"operation":"save_plan","plan":{"name":"つみたてNISA","debit":d.code("131"),"credit":bank,"amount":"30000","start":day(start,12),"frequency":"monthly"}}))?;
    let insurance_start = months_back(today, 14);
    let insurance = d.apply(json!({"operation":"save_plan","plan":{"name":"医療保険（年払い）","debit":d.code("571"),"credit":bank,"amount":"48000","start":day(insurance_start,5),"frequency":"yearly"}}))?;
    let fridge = today
        .with_day(1)
        .unwrap()
        .checked_add_months(Months::new(2))
        .unwrap();
    d.apply(json!({"operation":"save_plan","plan":{"name":"冷蔵庫の買い替え","debit":d.code("641"),"credit":d.code("201"),"amount":"168000","start":day(fridge,15),"frequency":"once","memo":"省エネ型を予定"}}))?;
    let loan_start = start;
    let car_loan = d.apply(json!({"operation":"save_claim","opening":d.code("641"),"claim":{"side":"payable","title":"自動車ローン","account":d.code("211"),"principal":"1200000","issued":loan_start,"due":day(loan_start.checked_add_months(Months::new(1)).unwrap(),27),"annual_rate":"2.4","repayment":"equal_payment","installments":36}}))?;
    let friend = d.apply(json!({"operation":"save_partner","partner":{"name":"佐藤さん"}}))?;
    let lend_day = months_back(today, 5);
    d.apply(json!({"operation":"save_claim","opening":bank,"claim":{"side":"receivable","title":"引越し費用の貸付","partner":friend,"account":d.code("141"),"principal":"50000","issued":day(lend_day,3),"due":day(lend_day.checked_add_months(Months::new(2)).unwrap(),28)}}))?;

    let mut card_balance = 0i64;
    for m in 0..=18u32 {
        let month = start.checked_add_months(Months::new(m)).unwrap();
        let last = m == 18;
        let limit = |dd: u32| !last || dd <= today.day();
        if limit(25) {
            let bonus = if [6, 12].contains(&month.month()) {
                450000
            } else {
                0
            };
            d.quick(
                "income",
                day(month, 25),
                318000 + bonus,
                "401",
                "111",
                "給与",
            )?;
        }
        if month.month() == 2 && limit(15) {
            d.quick("income", day(month, 15), 52000, "441", "111", "児童手当")?;
        }
        if card_balance > 0 && limit(10) {
            d.quick(
                "transfer",
                day(month, 10),
                card_balance,
                "201",
                "111",
                "カード引落し",
            )?;
            card_balance = 0;
        }
        for (week, dd) in [3u32, 10, 17, 24].iter().enumerate() {
            if !limit(*dd) {
                continue;
            }
            let amount = 7200
                + d.noise(2600)
                + if month.month() == 12 && week == 3 {
                    6000
                } else {
                    0
                };
            d.quick("expense", day(month, *dd), amount, "501", "201", "食料品")?;
            card_balance += amount;
        }
        if limit(18) {
            let eat = 6500 + d.noise(5000);
            d.quick("expense", day(month, 18), eat, "502", "201", "外食")?;
            card_balance += eat;
            let daily = 3800 + d.noise(2400);
            d.quick(
                "expense",
                day(month, 19),
                daily,
                "511",
                "101",
                "ドラッグストア",
            )?;
        }
        if limit(22) {
            let winter = matches!(month.month(), 12 | 1 | 2);
            let summer = matches!(month.month(), 7 | 8);
            let electric =
                7800 + if winter {
                    6200
                } else if summer {
                    4100
                } else {
                    0
                } + d.noise(900);
            d.quick("expense", day(month, 22), electric, "531", "111", "電気代")?;
            let gas = 4300 + if winter { 3500 } else { 0 } + d.noise(500);
            d.quick("expense", day(month, 22), gas, "531", "111", "ガス代")?;
            d.quick(
                "expense",
                day(month, 23),
                8800,
                "541",
                "111",
                "スマホ・光回線",
            )?;
        }
        if limit(14) {
            let fuel = 5200 + d.noise(1800);
            d.quick("expense", day(month, 14), fuel, "552", "201", "ガソリン")?;
            card_balance += fuel;
            let fun = 5000 + d.noise(9000) + if month.month() == 8 { 38000 } else { 0 };
            d.quick(
                "expense",
                day(month, 14),
                fun,
                "591",
                "201",
                if month.month() == 8 {
                    "夏の旅行"
                } else {
                    "趣味・レジャー"
                },
            )?;
            card_balance += fun;
        }
        if month.month() == 3 && limit(20) {
            d.quick(
                "expense",
                day(month, 20),
                21000,
                "611",
                "101",
                "友人の結婚祝い",
            )?;
            d.quick("transfer", day(month, 19), 30000, "101", "111", "ATM引出し")?;
        }
        if limit(5) {
            d.quick("transfer", day(month, 5), 20000, "101", "111", "ATM引出し")?;
        }
        for (plan, dd) in [(rent, 27u32), (nisa, 12)] {
            if limit(dd) {
                d.apply(json!({"operation":"post_plan","plan":plan,"occurrence":day(month,dd)}))?;
            }
        }
        if month.month() == insurance_start.month() && limit(5) && month >= insurance_start {
            d.apply(json!({"operation":"post_plan","plan":insurance,"occurrence":day(month,5)}))?;
        }
        if m >= 1 && limit(27) {
            let claim = d
                .book
                .claims
                .iter()
                .find(|c| c.id == car_loan)
                .unwrap()
                .clone();
            let schedule = yy_accountant::claims::schedule(&d.book, &claim)?;
            if let Some(i) = schedule.get(m as usize - 1) {
                d.apply(json!({"operation":"settle_claim","settlement":{"claim":car_loan,"date":i.date,"principal":i.principal,"interest":i.interest,"via":bank}}))?;
            }
        }
    }
    // Attach the supermarket to recent grocery entries through regular, audited changes.
    let recent: Vec<_> = d
        .book
        .entries
        .iter()
        .filter(|e| e.description == "食料品" && e.date >= months_back(today, 1))
        .cloned()
        .collect();
    for mut e in recent {
        e.partner = Some(super_);
        d.apply(json!({"operation":"save_entry","entry":e}))?;
    }
    let fy = today.year();
    let mut lines = vec![];
    for (code, monthly) in [
        ("501", 32000),
        ("502", 9000),
        ("511", 5000),
        ("521", 85000),
        ("531", 14000),
        ("541", 8800),
        ("552", 6000),
        ("591", 10000),
        ("611", 5000),
    ] {
        let mut amounts = vec![monthly.to_string(); 12];
        if code == "591" {
            amounts[7] = "45000".into();
        }
        if code == "531" {
            for i in [0usize, 1, 11] {
                amounts[i] = "20000".into();
            }
        }
        lines.push(json!({"account":d.code(code),"amounts":amounts}));
    }
    lines.push(json!({"account":d.code("401"),"amounts":(1..=12).map(|m| if m == 6 || m == 12 {"768000"} else {"318000"}).collect::<Vec<_>>()}));
    d.apply(json!({"operation":"save_budget","budget":{"name":format!("{fy}年 家計予算"),"fiscal_year":fy,"lines":lines,"memo":"食費は月3.2万円が目標"}}))?;
    planning(&mut d, today)
}

/// 税額計算とライフプランの例：副業のある会社員、妻、高校生の子、同居の父母、住宅ローン、
/// 保険とふるさと納税。ライフプランは現状と提案（保険の見直しと繰上返済）。
fn planning(d: &mut Demo, today: NaiveDate) -> Result<()> {
    let y = today.year();
    let year = *tax::YEARS
        .iter()
        .rev()
        .find(|t| **t <= y)
        .unwrap_or(&tax::YEARS[0]);
    d.apply(json!({"operation":"save_tax_return","tax_return":{
        "year":year,"memo":"例：会社員（副業あり）・妻・子・同居の父母",
        "salary":"6000000","salary_withheld":"80000","social_insurance":"870000","mutual_aid":"144000",
        "side_kind":"miscellaneous","side_revenue":"600000","side_expenses":"200000",
        "has_spouse":true,"spouse_income":"260000","spouse_birth_year":year - 38,
        "dependents":[
            {"name":"長男","relation":"child","birth_year":year - 16,"cohabiting":true},
            {"name":"父","relation":"parent","birth_year":year - 72,"income":"400000","cohabiting":true},
            {"name":"母","relation":"parent","birth_year":year - 70,"cohabiting":true}],
        "life_general_new":"100000","life_care":"40000","earthquake":"30000",
        "medical_paid":"250000","donation":"30000",
        "housing_balance":"25000000","housing_limit":"40000000","housing_rate":"0.7",
        "housing_resident_cap":"97500","per_capita":"5000"}}))?;
    let start = y + 1;
    let me = start - 43;
    let child = start - 17;
    let (education, entry) = lifeplan::education(&lifeplan::EducationRequest {
        child: "長男".into(),
        birth_year: child,
        schools: [
            lifeplan::School::None,
            lifeplan::School::None,
            lifeplan::School::None,
            lifeplan::School::Public,
            lifeplan::School::Private,
        ],
        growth_percent: num::Decimal::ONE,
    })?;
    let flow = |name: &str, kind: &str, amount: i64, from: i32, to: i32, growth: &str| json!({"name":name,"kind":kind,"amount":amount.to_string(),"start_year":from,"end_year":to,"growth_percent":growth});
    let mut flows = vec![
        flow("給与の手取り", "income", 4_900_000, start, me + 64, "1"),
        flow(
            "副業の収入（経費控除後）",
            "income",
            400_000,
            start,
            start + 9,
            "0",
        ),
        flow(
            "妻のパート収入",
            "income",
            1_000_000,
            start,
            start + 19,
            "0",
        ),
        flow(
            "公的年金（夫婦）",
            "income",
            2_600_000,
            me + 65,
            start + 39,
            "0",
        ),
        flow("基本生活費", "expense", 3_600_000, start, start + 39, "1"),
        flow(
            "住宅ローン返済",
            "expense",
            1_016_000,
            start,
            start + 24,
            "0",
        ),
        flow(
            "生命保険・医療保険",
            "expense",
            240_000,
            start,
            start + 19,
            "0",
        ),
        flow(
            "ふるさと納税（自己負担）",
            "expense",
            2_000,
            start,
            start + 19,
            "0",
        ),
        flow(
            "父母の医療・介護費",
            "expense",
            400_000,
            start + 3,
            start + 14,
            "2",
        ),
    ];
    flows.extend(education.iter().map(|f| serde_json::to_value(f).unwrap()));
    let mut events = vec![
        json!({"year":start + 3,"name":"車の買い替え","kind":"expense","amount":"3000000"}),
        json!({"year":me + 65,"name":"退職金","kind":"income","amount":"15000000"}),
    ];
    events.extend(entry.iter().map(|e| serde_json::to_value(e).unwrap()));
    let members = json!([
        {"name":"本人","relation":"本人","birth_year":me},
        {"name":"妻","relation":"配偶者","birth_year":start - 41},
        {"name":"長男","relation":"子","birth_year":child},
        {"name":"父","relation":"父","birth_year":start - 73},
        {"name":"母","relation":"母","birth_year":start - 71}]);
    let base = json!({"name":"現状","start_year":start,"years":40,"savings":"4000000","return_percent":"0.5",
        "memo":"例：手取り・生活費は今の価格。生活費は毎年1%上昇",
        "members":members,"flows":flows,"events":events});
    d.apply(json!({"operation":"save_life_plan","life_plan":base}))?;
    // 提案：保険を月1万円に見直し、浮いた分と預金で2年目に300万円繰上返済して返済期間を4年短縮
    let mut proposal = base.clone();
    proposal["name"] = json!("提案（保険見直し・繰上返済）");
    proposal["memo"] =
        json!("保険料を年12万円に見直し、2年目に300万円を繰上返済（期間短縮型で4年短縮）");
    for f in proposal["flows"].as_array_mut().unwrap() {
        match f["name"].as_str() {
            Some("生命保険・医療保険") => f["amount"] = json!("120000"),
            Some("住宅ローン返済") => f["end_year"] = json!(start + 20),
            _ => {}
        }
    }
    proposal["events"].as_array_mut().unwrap().push(
        json!({"year":start + 1,"name":"住宅ローン繰上返済","kind":"expense","amount":"3000000"}),
    );
    d.apply(json!({"operation":"save_life_plan","life_plan":proposal}))?;
    Ok(())
}

fn bakery(store: &mut Store, today: NaiveDate) -> Result<()> {
    let book = store.create(
        &serde_json::from_value::<NewBook>(
            json!({"name":"こむぎ工房（個人事業）","kind":"sole_proprietor"}),
        )?,
        "demo",
    )?;
    let mut d = Demo {
        store,
        book,
        seed: 11,
    };
    let start = months_back(today, 18);
    let (bank, cash, capital, recv) = (d.code("111"), d.code("101"), d.code("301"), d.code("121"));
    d.apply(json!({"operation":"save_entry","entry":{"date":start,"description":"開業時の元入金","lines":[
        {"account":bank,"debit":"1800000"},{"account":cash,"debit":"50000"},{"account":capital,"credit":"1850000"}]}}))?;
    let cafe = d.apply(json!({"operation":"save_partner","partner":{"name":"カフェ・ひだまり","note":"月末締め翌月末払い"}}))?;
    let mill = d.apply(json!({"operation":"save_partner","partner":{"name":"北の製粉"}}))?;
    let shinkin =
        d.apply(json!({"operation":"save_partner","partner":{"name":"みなと信用金庫"}}))?;
    let flour = d.apply(json!({"operation":"save_material","material":{"name":"強力粉","unit":"kg","standard_cost":"260"}}))?;
    let butter = d.apply(json!({"operation":"save_material","material":{"name":"発酵バター","unit":"kg","standard_cost":"1800"}}))?;
    let yeast = d.apply(json!({"operation":"save_material","material":{"name":"天然酵母","unit":"kg","standard_cost":"3200"}}))?;
    let oven_day = day(start, 10);
    let oven = d.apply(json!({"operation":"save_equipment","equipment":{"name":"デッキオーブン","acquired":oven_day,"cost":"1320000","salvage":"0","life_years":8,"annual_hours":"1600","running_cost_per_hour":"95","memo":"耐用年数は業務用厨房機器として8年で設定"}}))?;
    let mixer = d.apply(json!({"operation":"save_equipment","equipment":{"name":"ミキサー","acquired":oven_day,"cost":"360000","salvage":"0","life_years":8,"annual_hours":"800","running_cost_per_hour":"20"}}))?;
    let (machine, payable_loan) = (d.code("162"), d.code("221"));
    d.apply(json!({"operation":"save_entry","entry":{"date":oven_day,"description":"厨房機器の購入","lines":[
        {"account":machine,"debit":"1680000"},{"account":bank,"credit":"1680000"}]}}))?;
    let loan = d.apply(json!({"operation":"save_claim","opening":bank,"claim":{"side":"payable","title":"設備資金の借入","partner":shinkin,"account":payable_loan,"principal":"1500000","issued":day(start,8),"due":day(start.checked_add_months(Months::new(1)).unwrap(),25),"annual_rate":"1.8","repayment":"equal_principal","installments":60}}))?;
    let bread = d.apply(json!({"operation":"save_product","product":{"name":"天然酵母食パン","unit":"斤","labor_hours":"0.12","labor_rate":"1300","overhead_percent":"12",
        "materials":[{"material":flour,"quantity":"0.3","loss_percent":"4"},{"material":butter,"quantity":"0.015"},{"material":yeast,"quantity":"0.006"}],
        "equipment":[{"equipment":oven,"hours":"0.04"},{"equipment":mixer,"hours":"0.02"}]}}))?;
    let croissant = d.apply(json!({"operation":"save_product","product":{"name":"クロワッサン","unit":"個","labor_hours":"0.05","labor_rate":"1300","overhead_percent":"12",
        "materials":[{"material":flour,"quantity":"0.05","loss_percent":"6"},{"material":butter,"quantity":"0.03"},{"material":yeast,"quantity":"0.002"}],
        "equipment":[{"equipment":oven,"hours":"0.01"},{"equipment":mixer,"hours":"0.005"}]}}))?;
    let rent = d.apply(json!({"operation":"save_plan","plan":{"name":"工房の家賃","debit":d.code("526"),"credit":bank,"amount":"95000","start":day(start,25),"frequency":"monthly"}}))?;
    for m in 0..=18u32 {
        let month = start.checked_add_months(Months::new(m)).unwrap();
        let last = m == 18;
        let limit = |dd: u32| !last || dd <= today.day();
        let season = match month.month() {
            12 => 160,
            11 | 3 | 4 | 5 => 110,
            1 | 2 => 85,
            _ => 100,
        } as i64;
        let growth = 100 + m as i64 * 2;
        if limit(3) {
            for (material, base, price) in [(flour, 106, 255), (butter, 17, 1850), (yeast, 2, 3150)]
            {
                let kg = base * season * growth / 10000 + 1;
                let price = price + d.noise(30) - 15;
                d.apply(json!({"operation":"save_material_move","material_move":{"material":material,"date":day(month,3),"kind":"purchase","quantity":kg.to_string(),"unit_cost":price.to_string()}}))?;
                let amount = kg * price;
                let materials = d.code("132");
                d.apply(json!({"operation":"save_entry","entry":{"date":day(month,3),"description":"原材料の仕入","partner":mill,"lines":[{"account":materials,"debit":amount.to_string()},{"account":d.code("201"),"credit":amount.to_string()}]}}))?;
            }
        }
        if limit(20) {
            let loaves = 260 * season * growth / 10000 + d.noise(20);
            let pieces = 420 * season * growth / 10000 + d.noise(30);
            for (product, qty) in [(bread, loaves), (croissant, pieces)] {
                d.apply(json!({"operation":"save_production","production":{"product":product,"date":day(month,20),"quantity":qty.to_string()}}))?;
            }
            let report = yy_accountant::cost::report(&d.book, day(month, 20))?;
            let used: yy_accountant::num::Decimal = report
                .productions
                .iter()
                .filter(|p| p.date == day(month, 20))
                .map(|p| p.actual.materials.0)
                .sum();
            let (material_cost, materials) = (d.code("502"), d.code("132"));
            d.apply(json!({"operation":"save_entry","entry":{"date":day(month,20),"description":"製造で消費した原材料の振替","lines":[{"account":material_cost,"debit":used},{"account":materials,"credit":used}]}}))?;
            let shop = (loaves * 420 + pieces * 280) * 7 / 10;
            d.quick("income", day(month, 20), shop, "401", "101", "店頭販売")?;
            d.quick(
                "transfer",
                day(month, 21),
                shop - 20000,
                "111",
                "101",
                "売上金の預け入れ",
            )?;
            let wholesale = (loaves * 420 + pieces * 280) * 3 / 10;
            let sales = d.code("401");
            d.apply(json!({"operation":"save_entry","entry":{"date":day(month,28),"description":"カフェ卸（月末締め）","partner":cafe,"lines":[{"account":recv,"debit":wholesale.to_string()},{"account":sales,"credit":wholesale.to_string()}]}}))?;
            if m >= 1 {
                let prev = d
                    .book
                    .entries
                    .iter()
                    .rev()
                    .find(|e| e.description == "カフェ卸（月末締め）" && e.date < day(month, 1))
                    .map(|e| e.total());
                if let Some(amount) = prev {
                    d.apply(json!({"operation":"save_entry","entry":{"date":day(month,27),"description":"カフェ卸 入金","partner":cafe,"lines":[{"account":bank,"debit":amount},{"account":recv,"credit":amount}]}}))?;
                }
            }
        }
        if limit(10) && m >= 1 {
            let payable = d.code("201");
            let prev: yy_accountant::num::Decimal = d
                .book
                .entries
                .iter()
                .filter(|e| {
                    e.description == "原材料の仕入"
                        && e.date.month()
                            == month.checked_sub_months(Months::new(1)).unwrap().month()
                        && e.date < day(month, 1)
                })
                .map(|e| e.total())
                .sum();
            if !prev.is_zero() {
                d.apply(json!({"operation":"save_entry","entry":{"date":day(month,10),"description":"仕入代金の支払","partner":mill,"lines":[{"account":payable,"debit":prev},{"account":bank,"credit":prev}]}}))?;
            }
        }
        if limit(22) {
            let power = 18000
                + if matches!(month.month(), 12 | 1 | 2) {
                    6000
                } else {
                    0
                }
                + d.noise(2500);
            d.quick("expense", day(month, 22), power, "513", "111", "電気・ガス")?;
            let v = 3200 + d.noise(800);
            d.quick("expense", day(month, 22), v, "512", "101", "包材・配送")?;
            let phone = d.code("515");
            d.apply(json!({"operation":"quick_entry","quick":{"kind":"expense","date":day(month,23),"amount":"9800","account":phone,"via":bank,"business_percent":"50","description":"スマホ・回線（家事按分50%）"}}))?;
            let v = 4000 + d.noise(6000);
            d.quick("expense", day(month, 24), v, "520", "202", "消耗品")?;
            let v = 2000 + d.noise(4000);
            d.quick("expense", day(month, 26), v, "516", "202", "SNS広告")?;
        }
        if limit(25) {
            d.apply(json!({"operation":"post_plan","plan":rent,"occurrence":day(month,25)}))?;
            d.quick("transfer", day(month, 25), 200000, "191", "111", "生活費へ")?;
        }
        if m >= 1 && limit(25) {
            let claim = d.book.claims.iter().find(|c| c.id == loan).unwrap().clone();
            let schedule = yy_accountant::claims::schedule(&d.book, &claim)?;
            if let Some(i) = schedule.get(m as usize - 1) {
                d.apply(json!({"operation":"settle_claim","settlement":{"claim":loan,"date":i.date,"principal":i.principal,"interest":i.interest,"via":bank}}))?;
            }
        }
        if limit(28) && month.month().is_multiple_of(2) {
            let card = d.code("202");
            let owed: yy_accountant::num::Decimal =
                yy_accountant::reports::trial_balance(&d.book, None, day(month, 27))
                    .rows
                    .iter()
                    .find(|r| r.account == card)
                    .map(|r| r.closing.0)
                    .unwrap_or_default();
            if !owed.is_zero() {
                d.apply(json!({"operation":"save_entry","entry":{"date":day(month,28),"description":"事業用カード引落し","lines":[{"account":card,"debit":owed},{"account":bank,"credit":owed}]}}))?;
            }
        }
    }
    let first_fy = yy_accountant::cost::fiscal_year_of(&d.book, oven_day);
    for fy in first_fy..today.year() {
        for e in [oven, mixer] {
            d.apply(json!({"operation":"post_depreciation","equipment":e,"fiscal_year":fy,"date":format!("{fy}-12-31")}))?;
        }
    }
    let fy = today.year();
    let lines: Vec<Value> = [
        ("401", "520000"),
        ("513", "21000"),
        ("526", "95000"),
        ("520", "7000"),
        ("516", "4000"),
        ("512", "3500"),
    ]
    .iter()
    .map(|(code, v)| json!({"account":d.code(code),"amounts":vec![*v; 12]}))
    .collect();
    d.apply(json!({"operation":"save_budget","budget":{"name":format!("{fy}年 事業計画"),"fiscal_year":fy,"lines":lines}}))?;
    Ok(())
}
