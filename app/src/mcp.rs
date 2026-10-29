//! stdio MCP server. The Web UI calls the same tools through [`call`].
use serde::Deserialize;
use serde_json::{Value, json};
use std::io::{self, BufRead, Read, Write};
use yy_accountant::{
    Book, BookKind, Category, Change, LifePlan, NewBook, Role, Store, TaxReturn, budget, chart,
    claims, cost, forecast, lifeplan, model, num, plans, reports, tax, tutorial,
};
const MAX_MESSAGE: usize = 2 * 1024 * 1024;
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn obj(properties: Value, required: &[&str]) -> Value {
    json!({"type":"object","properties":properties,"required":required,"additionalProperties":false})
}
fn op(name: &str, properties: Value, required: &[&str]) -> Value {
    let mut p = properties;
    p["operation"] = json!({"const": name});
    let mut r = vec!["operation"];
    r.extend_from_slice(required);
    obj(p, &r)
}
pub(crate) const MUTATING: &[&str] = &["book_create", "book_change"];

fn tools() -> Value {
    let id = json!({"type":"integer","minimum":1});
    let id0 = json!({"type":"integer","minimum":0,"description":"0 creates a new record"});
    let opt_id = json!({"type":["integer","null"],"minimum":1});
    let date = json!({"type":"string","format":"date"});
    let opt_date = json!({"type":["string","null"],"format":"date"});
    let dec = json!({"type":["string","integer"],"pattern":"^-?[0-9]+(\\.[0-9]+)?$","description":"Decimal as a string such as \"1234.5\"; JSON fractions are rejected"});
    let opt_dec = json!({"type":["string","integer","null"],"pattern":"^-?[0-9]+(\\.[0-9]+)?$"});
    let text = json!({"type":"string"});
    let flag = json!({"type":"boolean"});
    let enums = |v: Vec<Value>| json!({"enum": v});
    let category = enums(Category::ALL.iter().map(|c| json!(c)).collect());
    let role = json!({"enum": Role::ALL.iter().map(|r| json!(r)).chain([Value::Null]).collect::<Vec<_>>()});
    let line = obj(
        json!({"account":id,"debit":dec,"credit":dec,"memo":text,"claim":opt_id}),
        &["account"],
    );
    let entry = obj(
        json!({"id":id0,"date":date,"description":text,"partner":opt_id,"lines":{"type":"array","items":line,"minItems":2,"maxItems":200}}),
        &["date", "lines"],
    );
    let uses = |key: &str| json!({"type":"array","items":obj(json!({key:id,"quantity":dec,"loss_percent":dec,"hours":dec}),&[key])});
    let account = obj(
        json!({"id":id0,"code":text,"name":text,"category":category,"role":role,"parent":opt_id,"payment":flag,"guide":text,"keywords":{"type":"array","items":text},"archived":flag}),
        &["code", "name", "category"],
    );
    let tax_amounts = [
        "salary",
        "salary_withheld",
        "social_insurance",
        "side_revenue",
        "side_expenses",
        "side_withheld",
        "blue_deduction",
        "spouse_income",
        "mutual_aid",
        "life_general_new",
        "life_general_old",
        "life_care",
        "life_pension_new",
        "life_pension_old",
        "earthquake",
        "medical_paid",
        "medical_covered",
        "self_medication",
        "donation",
        "housing_balance",
        "housing_limit",
        "housing_rate",
        "housing_resident_cap",
        "per_capita",
    ];
    let disability = json!({"enum":["none","normal","special"]});
    let mut tax_props = json!({"id":id0,"year":{"type":"integer","enum":tax::YEARS},"name":text,"memo":text,
        "side_kind":{"enum":tax::SideKind::ALL.iter().map(|k| json!(k)).collect::<Vec<_>>()},
        "self_disability":disability,"has_spouse":flag,"spouse_birth_year":{"type":["integer","null"]},"spouse_disability":disability,
        "dependents":{"type":"array","maxItems":30,"items":obj(json!({"name":text,"relation":{"enum":["child","parent","other"]},"birth_year":{"type":"integer"},"income":dec,"cohabiting":flag,"disability":disability}),&["name","birth_year"])}});
    for k in tax_amounts {
        tax_props[k] = dec.clone();
    }
    let tax_return = obj(tax_props, &["year"]);
    let kind = json!({"enum":["income","expense"]});
    let year = json!({"type":"integer","minimum":1900,"maximum":2999});
    let life_plan = obj(
        json!({"id":id0,"name":text,"memo":text,"start_year":year,"years":{"type":"integer","minimum":1,"maximum":100},"savings":dec,"return_percent":dec,
            "members":{"type":"array","items":obj(json!({"name":text,"relation":text,"birth_year":year}),&["name","birth_year"])},
            "flows":{"type":"array","items":obj(json!({"name":text,"kind":kind,"amount":dec,"start_year":year,"end_year":year,"growth_percent":dec}),&["name","kind","amount","start_year","end_year"])},
            "events":{"type":"array","items":obj(json!({"year":year,"name":text,"kind":kind,"amount":dec,"growth_percent":dec}),&["year","name","kind","amount"])}}),
        &["name", "start_year", "years"],
    );
    let change = json!({"oneOf":[
        op("update_book", json!({"name":text,"currency":text,"minor_digits":{"type":"integer","minimum":0,"maximum":4},"rounding":{"enum":["half_up","down","up","half_even"]},"fiscal_start_month":{"type":"integer","minimum":1,"maximum":12}}), &["name","currency","minor_digits","rounding","fiscal_start_month"]),
        op("save_account", json!({"account":account}), &["account"]),
        op("save_partner", json!({"partner":obj(json!({"id":id0,"name":text,"note":text,"archived":flag}),&["name"])}), &["partner"]),
        op("save_entry", json!({"entry":entry}), &["entry"]),
        op("quick_entry", json!({"quick":obj(json!({"kind":{"enum":["expense","income","transfer"]},"date":date,"amount":dec,"account":id,"via":id,"description":text,"partner":opt_id,"business_percent":opt_dec,"memo":text}),&["kind","date","amount","account","via"])}), &["quick"]),
        op("void_entry", json!({"id":id,"void":flag}), &["id","void"]),
        op("save_claim", json!({"claim":obj(json!({"id":id0,"side":{"enum":["receivable","payable"]},"title":text,"partner":opt_id,"account":id,"principal":dec,"issued":date,"due":opt_date,"annual_rate":dec,"repayment":{"enum":["lump","equal_payment","equal_principal"]},"installments":{"type":"integer","minimum":0,"maximum":600},"closed":flag,"memo":text}),&["side","title","account","principal","issued"]),"opening":opt_id}), &["claim"]),
        op("settle_claim", json!({"settlement":obj(json!({"claim":id,"date":date,"principal":dec,"interest":dec,"via":id,"description":text}),&["claim","date","principal","via"])}), &["settlement"]),
        op("save_budget", json!({"budget":obj(json!({"id":id0,"name":text,"fiscal_year":{"type":"integer"},"memo":text,"lines":{"type":"array","items":obj(json!({"account":id,"amounts":{"type":"array","items":dec,"minItems":12,"maxItems":12}}),&["account","amounts"])}}),&["name","fiscal_year"])}), &["budget"]),
        op("save_plan", json!({"plan":obj(json!({"id":id0,"name":text,"debit":id,"credit":id,"amount":dec,"start":date,"end":opt_date,"frequency":{"enum":["once","weekly","monthly","yearly"]},"interval":{"type":"integer","minimum":1,"maximum":120},"partner":opt_id,"active":flag,"memo":text}),&["name","debit","credit","amount","start"])}), &["plan"]),
        op("post_plan", json!({"plan":id,"occurrence":date,"date":opt_date,"amount":opt_dec}), &["plan","occurrence"]),
        op("save_material", json!({"material":obj(json!({"id":id0,"name":text,"unit":text,"standard_cost":dec,"archived":flag,"memo":text}),&["name","unit"])}), &["material"]),
        op("save_material_move", json!({"material_move":obj(json!({"id":id0,"material":id,"date":date,"kind":{"enum":["purchase","adjust"]},"quantity":dec,"unit_cost":opt_dec,"memo":text}),&["material","date","quantity"])}), &["material_move"]),
        op("save_equipment", json!({"equipment":obj(json!({"id":id0,"name":text,"acquired":date,"cost":dec,"salvage":dec,"life_years":{"type":"integer","minimum":1,"maximum":100},"annual_hours":dec,"running_cost_per_hour":dec,"archived":flag,"memo":text}),&["name","acquired","cost","life_years","annual_hours"])}), &["equipment"]),
        op("post_depreciation", json!({"equipment":id,"fiscal_year":{"type":"integer"},"date":date}), &["equipment","fiscal_year","date"]),
        op("save_product", json!({"product":obj(json!({"id":id0,"name":text,"unit":text,"labor_hours":dec,"labor_rate":dec,"overhead_percent":dec,"archived":flag,"memo":text,"materials":uses("material"),"equipment":uses("equipment")}),&["name","unit"])}), &["product"]),
        op("save_production", json!({"production":obj(json!({"id":id0,"product":id,"date":date,"quantity":dec,"labor_hours":dec,"memo":text,"materials":uses("material"),"equipment":uses("equipment")}),&["product","date","quantity"])}), &["production"]),
        op("save_tax_return", json!({"tax_return":tax_return.clone()}), &["tax_return"]),
        op("save_life_plan", json!({"life_plan":life_plan.clone()}), &["life_plan"]),
        op("remove", json!({"target":{"enum":["account","partner","claim","budget","plan","material","material_move","equipment","product","production","tax_return","life_plan"]},"id":id}), &["target","id"]),
    ]});
    let book = json!({"book_id":id});
    let mut list = vec![];
    let mut add = |name: &str,
                   description: &str,
                   properties: Value,
                   required: &[&str],
                   read_only: bool| {
        let mut p = properties;
        if !["book_list", "book_create", "tutorial", "metadata"].contains(&name) {
            p["book_id"] = id.clone();
        }
        list.push(json!({"name":name,"description":description,"inputSchema":obj(p,required),"annotations":{"readOnlyHint":read_only,"destructiveHint":false,"idempotentHint":read_only,"openWorldHint":false}}));
    };
    add(
        "book_list",
        "List books (household or sole proprietor) and their revisions.",
        json!({}),
        &[],
        true,
    );
    add(
        "book_create",
        "Create a book with a Japanese chart of accounts that explains how to choose between similar accounts. kind: household or sole_proprietor.",
        json!({"name":text,"kind":enums(BookKind::ALL.iter().map(|k|json!(k)).collect()),"currency":text,"minor_digits":{"type":"integer","minimum":0,"maximum":4},"rounding":{"enum":["half_up","down","up","half_even"]},"fiscal_start_month":{"type":"integer","minimum":1,"maximum":12},"actor":text}),
        &["name", "actor"],
        false,
    );
    add(
        "book_get",
        "Read a book's settings and masters (accounts with usage guides, partners, claims, budgets, plans, materials, equipment, products) without journal entries. Use revision as expected_revision.",
        book.clone(),
        &["book_id"],
        true,
    );
    add(
        "book_change",
        "Apply one validated change atomically. Amounts are decimal strings. Every journal entry must balance (debit = credit). Prefer quick_entry for income/expense/transfer and account_guide to choose accounts. A stale expected_revision fails without overwriting.",
        json!({"expected_revision":id,"actor":text,"change":change}),
        &["book_id", "expected_revision", "actor", "change"],
        false,
    );
    add(
        "entry_list",
        "Page journal entries newest first with optional filters. limit 1..500 (default 100).",
        json!({"from":date,"to":date,"account":id,"partner":id,"query":text,"include_void":flag,"offset":{"type":"integer","minimum":0},"limit":{"type":"integer","minimum":1,"maximum":500}}),
        &["book_id"],
        true,
    );
    add(
        "account_guide",
        "Suggest accounts for a description (e.g. 'ガソリン', 'スーパーで食材') and explain how to choose between similar accounts. Without query, lists all distinctions.",
        json!({"query":text,"account":id}),
        &["book_id"],
        true,
    );
    add(
        "report",
        "Financial statements: trial_balance (from optional), income_statement (from required), balance_sheet, ledger (account required).",
        json!({"kind":{"enum":["trial_balance","income_statement","balance_sheet","ledger"]},"from":date,"to":date,"account":id}),
        &["book_id", "kind", "to"],
        true,
    );
    add(
        "statistics",
        "Monthly income/expense/net/savings rate and per-account total, mean, median, standard deviation, min, max, share and variability.",
        json!({"from":date,"to":date}),
        &["book_id", "from", "to"],
        true,
    );
    add(
        "forecast",
        "Forecast income and expenses per account for the months after as_of from history (moving average, linear trend or seasonal chosen by backtest) plus unposted plans and claim installments. Ranges are about 95%.",
        json!({"as_of":date,"months":{"type":"integer","minimum":1,"maximum":36},"history_months":{"type":"integer","minimum":3,"maximum":120},"method":{"enum":["auto","moving_average","linear","seasonal"]}}),
        &["book_id", "as_of"],
        true,
    );
    add(
        "budget_report",
        "Budget versus actual by fiscal month for one budget.",
        json!({"budget_id":id}),
        &["book_id", "budget_id"],
        true,
    );
    add(
        "claim_report",
        "Receivables/payables: outstanding, repayment schedules, arrears and aging buckets at as_of.",
        json!({"as_of":date,"include_closed":flag}),
        &["book_id", "as_of"],
        true,
    );
    add(
        "plan_calendar",
        "Occurrences of active plans in a period with posted state; post with book_change post_plan.",
        json!({"from":date,"to":date}),
        &["book_id", "from", "to"],
        true,
    );
    add(
        "cost_report",
        "Material stock (moving average), equipment hourly rates and depreciation schedules, standard product costs and actual production costs with variances.",
        json!({"as_of":date}),
        &["book_id", "as_of"],
        true,
    );
    add(
        "csv_export",
        "Journal lines as UTF-8 CSV text (BOM, CRLF). The client saves it.",
        json!({"from":date,"to":date}),
        &["book_id"],
        true,
    );
    add(
        "audit_list",
        "Up to 100 committed changes after an audit ID.",
        json!({"after":{"type":"integer","minimum":0}}),
        &["book_id"],
        true,
    );
    add(
        "tax_calculate",
        "Estimate Japanese income tax (with reconstruction tax) and resident tax for one year: salary and side income, spouse and dependents (children, parents living together), social insurance, iDeCo, life/earthquake insurance, medical expenses, furusato nozei (with the donation limit estimate) and the mortgage credit. Pass a saved tax_return_id or an unsaved tax_return. Supported years are in metadata tax_years; amounts follow that year's rules and should be checked against official guidance before filing.",
        json!({"tax_return_id":id,"tax_return":tax_return}),
        &["book_id"],
        true,
    );
    add(
        "life_plan_cash_flow",
        "Yearly cash-flow table of a life plan (ages, incomes, expenses, events, investment return, year-end savings, shortfall years). Pass a saved life_plan_id or an unsaved life_plan. With compare_id, also compares year-end savings with another saved plan (base = this plan, proposal = compare_id). With csv, adds the table as UTF-8 CSV text.",
        json!({"life_plan_id":id,"compare_id":id,"life_plan":life_plan,"csv":flag}),
        &["book_id"],
        true,
    );
    add(
        "life_plan_draft",
        "Current state from the book (last 12 months of income and expenses up to as_of, cash and net assets) and an unsaved draft life plan built from it. With tax_return_id, the draft uses that year's estimated take-home pay as income.",
        json!({"as_of":date,"tax_return_id":id}),
        &["book_id", "as_of"],
        true,
    );
    add(
        "fp_calculator",
        "Financial planning helpers. kind coefficients: the six coefficients for rate (annual %) and years. kind loan: monthly/annual payment of an equal-payment loan (principal, rate, years). kind education: education cost flows and entrance fee events for a child (child, birth_year, schools for kindergarten/elementary/junior/high/university each none, public or private, growth_percent).",
        json!({"kind":{"enum":["coefficients","loan","education"]},"rate":dec,"years":{"type":"integer","minimum":1,"maximum":100},"principal":dec,
            "child":text,"birth_year":year,"schools":{"type":"array","minItems":5,"maxItems":5,"items":{"enum":["none","public","private"]}},"growth_percent":dec}),
        &["book_id", "kind"],
        true,
    );
    add(
        "tutorial",
        "Beginner tutorial for bookkeeping, taxes and life plans, as chapters and steps.",
        json!({}),
        &[],
        true,
    );
    add(
        "metadata",
        "Labels for enumerations (categories, roles, kinds, rounding, frequencies, repayment methods).",
        json!({}),
        &[],
        true,
    );
    json!({"tools": list})
}

pub(crate) fn metadata() -> Value {
    let pairs = |v: Vec<(Value, &str)>| {
        v.into_iter()
            .map(|(k, l)| json!([k, l]))
            .collect::<Vec<_>>()
    };
    json!({
        "kinds": pairs(BookKind::ALL.iter().map(|k| (json!(k), k.label())).collect()),
        "categories": pairs(Category::ALL.iter().map(|k| (json!(k), k.label())).collect()),
        "roles": pairs(Role::ALL.iter().map(|k| (json!(k), k.label())).collect()),
        "rounding": pairs(num::Rounding::ALL.iter().map(|k| (json!(k), k.label())).collect()),
        "frequencies": pairs(model::Frequency::ALL.iter().map(|k| (json!(k), k.label())).collect()),
        "repayments": pairs(model::Repayment::ALL.iter().map(|k| (json!(k), k.label())).collect()),
        "side_kinds": pairs(tax::SideKind::ALL.iter().map(|k| (json!(k), k.label())).collect()),
        "disabilities": pairs(tax::Disability::ALL.iter().map(|k| (json!(k), k.label())).collect()),
        "relations": pairs(tax::Relation::ALL.iter().map(|k| (json!(k), k.label())).collect()),
        "flow_kinds": pairs(lifeplan::FlowKind::ALL.iter().map(|k| (json!(k), k.label())).collect()),
        "tax_years": tax::YEARS,
        "education": lifeplan::EDUCATION,
        "methods": pairs([forecast::Method::Auto, forecast::Method::MovingAverage, forecast::Method::Linear, forecast::Method::Seasonal].iter().map(|k| (json!(k), k.label())).collect()),
    })
}

fn summary(book: &Book) -> Result<Value> {
    let mut v = serde_json::to_value(book)?;
    let o = v.as_object_mut().ok_or("Invalid book")?;
    o.remove("entries");
    o.remove("next_ids");
    o.insert("entry_count".into(), json!(book.entries.len()));
    Ok(v)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BookOnly {
    book_id: i64,
}
/// Unknown fields are rejected by the flattened inner type.
#[derive(Deserialize)]
struct Create {
    actor: String,
    #[serde(flatten)]
    book: NewBook,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Apply {
    book_id: i64,
    expected_revision: i64,
    actor: String,
    change: Change,
}
/// Unknown fields are rejected by the flattened inner type.
#[derive(Deserialize)]
struct Entries {
    book_id: i64,
    #[serde(flatten)]
    filter: reports::EntryFilter,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Guide {
    book_id: i64,
    #[serde(default)]
    query: String,
    #[serde(default)]
    account: Option<i64>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
enum ReportKind {
    TrialBalance,
    IncomeStatement,
    BalanceSheet,
    Ledger,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Report {
    book_id: i64,
    kind: ReportKind,
    #[serde(default)]
    from: Option<chrono::NaiveDate>,
    to: chrono::NaiveDate,
    #[serde(default)]
    account: Option<i64>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Period {
    book_id: i64,
    from: chrono::NaiveDate,
    to: chrono::NaiveDate,
}
/// Unknown fields are rejected by the flattened inner type.
#[derive(Deserialize)]
struct Forecast {
    book_id: i64,
    #[serde(flatten)]
    request: forecast::ForecastRequest,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BudgetArgs {
    book_id: i64,
    budget_id: i64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AsOf {
    book_id: i64,
    as_of: chrono::NaiveDate,
    #[serde(default)]
    include_closed: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Csv {
    book_id: i64,
    #[serde(default)]
    from: Option<chrono::NaiveDate>,
    #[serde(default)]
    to: Option<chrono::NaiveDate>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TaxArgs {
    book_id: i64,
    #[serde(default)]
    tax_return_id: Option<i64>,
    #[serde(default)]
    tax_return: Option<TaxReturn>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PlanArgs {
    book_id: i64,
    #[serde(default)]
    life_plan_id: Option<i64>,
    #[serde(default)]
    compare_id: Option<i64>,
    #[serde(default)]
    life_plan: Option<LifePlan>,
    #[serde(default)]
    csv: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DraftArgs {
    book_id: i64,
    as_of: chrono::NaiveDate,
    #[serde(default)]
    tax_return_id: Option<i64>,
}
#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum FpKind {
    Coefficients,
    Loan,
    Education,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FpArgs {
    book_id: i64,
    kind: FpKind,
    #[serde(default)]
    rate: Option<num::D>,
    #[serde(default)]
    years: Option<u32>,
    #[serde(default)]
    principal: Option<num::D>,
    #[serde(default)]
    child: Option<String>,
    #[serde(default)]
    birth_year: Option<i32>,
    #[serde(default)]
    schools: Option<[lifeplan::School; 5]>,
    #[serde(default)]
    growth_percent: Option<num::D>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Audit {
    book_id: i64,
    #[serde(default)]
    after: i64,
}
fn args<T: serde::de::DeserializeOwned>(v: Value) -> Result<T> {
    Ok(serde_json::from_value(v)?)
}

pub(crate) fn call(store: &mut Store, name: &str, a: Value) -> Result<Value> {
    if !a.is_object() {
        return Err("Tool arguments must be an object".into());
    }
    Ok(match name {
        "book_list" => {
            if !a.as_object().is_some_and(|o| o.is_empty()) {
                return Err("Unknown tool argument".into());
            }
            json!({"books": store.list()?})
        }
        "metadata" => metadata(),
        "book_create" => {
            let c: Create = args(a)?;
            summary(&store.create(&c.book, &c.actor)?)?
        }
        "book_get" => summary(&store.get(args::<BookOnly>(a)?.book_id)?)?,
        "book_change" => {
            let c: Apply = args(a)?;
            let applied = store.apply(c.book_id, c.expected_revision, &c.change, &c.actor)?;
            json!({"revision": applied.book.revision, "id": applied.id})
        }
        "entry_list" => {
            let e: Entries = args(a)?;
            serde_json::to_value(reports::entries(&store.get(e.book_id)?, &e.filter))?
        }
        "account_guide" => {
            let g: Guide = args(a)?;
            serde_json::to_value(chart::guide(&store.get(g.book_id)?, &g.query, g.account))?
        }
        "report" => {
            let r: Report = args(a)?;
            let book = store.get(r.book_id)?;
            match r.kind {
                ReportKind::TrialBalance => {
                    serde_json::to_value(reports::trial_balance(&book, r.from, r.to))?
                }
                ReportKind::IncomeStatement => serde_json::to_value(reports::income_statement(
                    &book,
                    r.from.ok_or("from is required")?,
                    r.to,
                ))?,
                ReportKind::BalanceSheet => {
                    serde_json::to_value(reports::balance_sheet(&book, r.to))?
                }
                ReportKind::Ledger => serde_json::to_value(reports::ledger(
                    &book,
                    r.account.ok_or("account is required")?,
                    r.from.ok_or("from is required")?,
                    r.to,
                )?)?,
            }
        }
        "statistics" => {
            let p: Period = args(a)?;
            serde_json::to_value(reports::statistics(&store.get(p.book_id)?, p.from, p.to)?)?
        }
        "forecast" => {
            let f: Forecast = args(a)?;
            serde_json::to_value(forecast::forecast(&store.get(f.book_id)?, &f.request)?)?
        }
        "budget_report" => {
            let b: BudgetArgs = args(a)?;
            serde_json::to_value(budget::report(&store.get(b.book_id)?, b.budget_id)?)?
        }
        "claim_report" => {
            let c: AsOf = args(a)?;
            serde_json::to_value(claims::report(
                &store.get(c.book_id)?,
                c.as_of,
                c.include_closed,
            )?)?
        }
        "plan_calendar" => {
            let p: Period = args(a)?;
            if (p.to - p.from).num_days() > 3660 || p.to < p.from {
                return Err("Period must be 0..3660 days".into());
            }
            json!({"occurrences": plans::calendar(&store.get(p.book_id)?, p.from, p.to)})
        }
        "cost_report" => {
            let c: AsOf = args(a)?;
            serde_json::to_value(cost::report(&store.get(c.book_id)?, c.as_of)?)?
        }
        "csv_export" => {
            let c: Csv = args(a)?;
            json!({"mime_type":"text/csv; charset=utf-8","content":reports::journal_csv(&store.get(c.book_id)?, c.from, c.to)})
        }
        "tax_calculate" => {
            let t: TaxArgs = args(a)?;
            let book = store.get(t.book_id)?;
            let input = match (t.tax_return_id, t.tax_return) {
                (Some(id), None) => book
                    .tax_returns
                    .iter()
                    .find(|x| x.id == id)
                    .cloned()
                    .ok_or("税額計算がありません")?,
                (None, Some(input)) => input,
                _ => return Err("Pass either tax_return_id or tax_return".into()),
            };
            serde_json::to_value(tax::calculate(&book, &input)?)?
        }
        "life_plan_cash_flow" => {
            let p: PlanArgs = args(a)?;
            let book = store.get(p.book_id)?;
            let find = |id: i64| -> Result<LifePlan> {
                Ok(book
                    .life_plans
                    .iter()
                    .find(|x| x.id == id)
                    .cloned()
                    .ok_or("ライフプランがありません")?)
            };
            let plan = match (p.life_plan_id, p.life_plan) {
                (Some(id), None) => find(id)?,
                (None, Some(plan)) => plan,
                _ => return Err("Pass either life_plan_id or life_plan".into()),
            };
            let flow = lifeplan::cash_flow(&book, &plan)?;
            let mut v = serde_json::to_value(&flow)?;
            if p.csv {
                v["csv"] = json!(lifeplan::csv(&book, &flow));
            }
            if let Some(other) = p.compare_id {
                v["comparison"] =
                    serde_json::to_value(lifeplan::compare(&book, &plan, &find(other)?)?)?;
            }
            v
        }
        "life_plan_draft" => {
            let d: DraftArgs = args(a)?;
            let book = store.get(d.book_id)?;
            let take_home = match d.tax_return_id {
                Some(id) => {
                    let t = book
                        .tax_returns
                        .iter()
                        .find(|x| x.id == id)
                        .ok_or("税額計算がありません")?;
                    Some(tax::calculate(&book, t)?.take_home.0)
                }
                None => None,
            };
            json!({"baseline": lifeplan::baseline(&book, d.as_of)?, "draft": lifeplan::draft(&book, d.as_of, take_home)?})
        }
        "fp_calculator" => {
            let f: FpArgs = args(a)?;
            let book = store.get(f.book_id)?;
            let need = |v: Option<num::D>, what: &str| -> Result<num::Decimal> {
                Ok(v.ok_or(format!("{what} is required"))?.0)
            };
            let years = || f.years.ok_or("years is required");
            match f.kind {
                FpKind::Coefficients => {
                    serde_json::to_value(lifeplan::coefficients(need(f.rate, "rate")?, years()?)?)?
                }
                FpKind::Loan => serde_json::to_value(lifeplan::loan_payment(
                    &book,
                    need(f.principal, "principal")?,
                    need(f.rate, "rate")?,
                    years()?,
                )?)?,
                FpKind::Education => {
                    let (flows, events) = lifeplan::education(&lifeplan::EducationRequest {
                        child: f.child.clone().ok_or("child is required")?,
                        birth_year: f.birth_year.ok_or("birth_year is required")?,
                        schools: f.schools.ok_or("schools is required")?,
                        growth_percent: f.growth_percent.map(|d| d.0).unwrap_or_default(),
                    })?;
                    json!({"flows": flows, "events": events})
                }
            }
        }
        "tutorial" => {
            if !a.as_object().is_some_and(|o| o.is_empty()) {
                return Err("Unknown tool argument".into());
            }
            json!({"chapters": tutorial::CHAPTERS})
        }
        "audit_list" => {
            let x: Audit = args(a)?;
            json!({"changes": store.audit(x.book_id, x.after)?})
        }
        _ => return Err("Unknown tool".into()),
    })
}

#[derive(Default)]
pub(crate) struct Session {
    initialized: bool,
    ready: bool,
}
impl Session {
    pub(crate) fn handle(&mut self, store: &mut Store, request: Value) -> Option<Value> {
        if !request.is_object()
            || request["jsonrpc"] != "2.0"
            || !request["method"].is_string()
            || request
                .get("id")
                .is_some_and(|id| !(id.is_null() || id.is_string() || id.is_i64() || id.is_u64()))
        {
            return Some(
                json!({"jsonrpc":"2.0","id":null,"error":{"code":-32600,"message":"Invalid JSON-RPC request"}}),
            );
        }
        let id = request.get("id").cloned();
        let method = request["method"].as_str().unwrap_or("");
        let Some(id) = id else {
            if method == "notifications/initialized" && self.initialized {
                self.ready = true;
            }
            return None;
        };
        let result = match method {
            "initialize"
                if !request["params"]["protocolVersion"].is_string()
                    || !request["params"]["capabilities"].is_object()
                    || !request["params"]["clientInfo"]["name"].is_string() =>
            {
                Err((-32602, "Invalid initialization parameters"))
            }
            "initialize" if !self.initialized => {
                self.initialized = true;
                let requested = request["params"]["protocolVersion"].as_str().unwrap_or("");
                let version = if ["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"]
                    .contains(&requested)
                {
                    requested
                } else {
                    "2025-11-25"
                };
                Ok(
                    json!({"protocolVersion":version,"capabilities":{"tools":{"listChanged":false}},"serverInfo":{"name":"yyaccountant","version":env!("CARGO_PKG_VERSION")},
                    "instructions":"Read book_list/book_get first. Changes need expected_revision and are rejected on conflict. Use decimal strings for amounts. Use account_guide when unsure which account to use. The database path and read-only mode are fixed by the person launching this server."}),
                )
            }
            "ping" => Ok(json!({})),
            _ if !self.ready => Err((
                -32000,
                "Complete initialize and notifications/initialized first",
            )),
            "tools/list" => Ok(tools()),
            "tools/call" => {
                let params = &request["params"];
                Ok(
                    match call(
                        store,
                        params["name"].as_str().unwrap_or(""),
                        params.get("arguments").cloned().unwrap_or(json!({})),
                    ) {
                        Ok(data) => {
                            json!({"content":[{"type":"text","text":data.to_string()}],"structuredContent":data,"isError":false})
                        }
                        Err(error) => {
                            json!({"content":[{"type":"text","text":error.to_string()}],"isError":true})
                        }
                    },
                )
            }
            _ => Err((-32601, "Method not found")),
        };
        Some(match result {
            Ok(result) => json!({"jsonrpc":"2.0","id":id,"result":result}),
            Err((code, message)) => {
                json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message}})
            }
        })
    }
}

pub fn serve(store: &mut Store, mut input: impl BufRead, mut output: impl Write) -> io::Result<()> {
    let mut session = Session::default();
    loop {
        let mut line = vec![];
        let count = input
            .by_ref()
            .take((MAX_MESSAGE + 1) as u64)
            .read_until(b'\n', &mut line)?;
        if count == 0 {
            break;
        }
        let response = if count > MAX_MESSAGE {
            while !line.ends_with(b"\n") {
                line.clear();
                if input
                    .by_ref()
                    .take(MAX_MESSAGE as u64)
                    .read_until(b'\n', &mut line)?
                    == 0
                {
                    break;
                }
            }
            Some(
                json!({"jsonrpc":"2.0","id":null,"error":{"code":-32600,"message":"Message exceeds 2 MiB"}}),
            )
        } else if line.iter().all(u8::is_ascii_whitespace) {
            None
        } else {
            match serde_json::from_slice::<Value>(&line) {
                Ok(request) => session.handle(store, request),
                Err(_) => Some(
                    json!({"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":"Invalid JSON"}}),
                ),
            }
        };
        if let Some(response) = response {
            serde_json::to_writer(&mut output, &response)?;
            output.write_all(b"\n")?;
            output.flush()?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn session(store: &mut Store, requests: Vec<Value>) -> Vec<Value> {
        let bytes: String = requests.into_iter().map(|v| v.to_string() + "\n").collect();
        let mut out = vec![];
        serve(store, std::io::Cursor::new(bytes), &mut out).unwrap();
        String::from_utf8(out)
            .unwrap()
            .lines()
            .map(|s| serde_json::from_str(s).unwrap())
            .collect()
    }
    fn tool(id: i64, name: &str, arguments: Value) -> Value {
        json!({"jsonrpc":"2.0","id":id,"method":"tools/call","params":{"name":name,"arguments":arguments}})
    }
    #[test]
    fn lifecycle_tools_and_decimal_changes() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(&dir.path().join("t.sqlite3"), false).unwrap();
        let out = session(
            &mut store,
            vec![
                json!({"jsonrpc":"2.0","id":0,"method":"tools/list"}),
                json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"t","version":"1"}}}),
                json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
                json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}),
                tool(3, "book_create", json!({"name":"家計","actor":"AI"})),
                tool(
                    4,
                    "book_change",
                    json!({"book_id":1,"expected_revision":1,"actor":"AI","change":{"operation":"quick_entry","quick":{"kind":"expense","date":"2026-10-01","amount":"1234.5","account":18,"via":1}}}),
                ),
                tool(
                    5,
                    "book_change",
                    json!({"book_id":1,"expected_revision":1,"actor":"AI","change":{"operation":"quick_entry","quick":{"kind":"expense","date":"2026-10-01","amount":"1234","account":18,"via":1}}}),
                ),
                tool(
                    6,
                    "book_change",
                    json!({"book_id":1,"expected_revision":1,"actor":"AI","change":{"operation":"quick_entry","quick":{"kind":"expense","date":"2026-10-01","amount":"1234","account":18,"via":1}}}),
                ),
                tool(7, "entry_list", json!({"book_id":1,"limit":1})),
                tool(8, "account_guide", json!({"book_id":1,"query":"スーパー"})),
                tool(9, "book_list", json!({"x":1})),
            ],
        );
        assert_eq!(out[0]["error"]["code"], -32000);
        let tools = out[2]["result"]["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 21);
        for t in tools {
            assert_eq!(t["inputSchema"]["additionalProperties"], false);
        }
        assert_eq!(out[3]["result"]["structuredContent"]["revision"], 1);
        assert_eq!(
            out[4]["result"]["isError"], true,
            "JPY has no fraction digits"
        );
        assert_eq!(out[5]["result"]["structuredContent"]["revision"], 2);
        assert_eq!(out[6]["result"]["isError"], true, "stale revision");
        assert_eq!(
            out[7]["result"]["structuredContent"]["entries"][0]["lines"][0]["debit"],
            "1234"
        );
        assert_eq!(
            out[8]["result"]["structuredContent"]["suggestions"][0]["name"],
            "食費"
        );
        assert_eq!(out[9]["result"]["isError"], true);
    }
}
