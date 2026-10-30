//! Single-file SQLite storage.
//!
//! Every collection of a book is a relational table keyed by `(book_id, id)`; nested lists
//! (journal lines, budget lines, bills of materials) are child tables keyed by
//! `(book_id, parent_id, seq)`. Decimals are stored as canonical TEXT so they never pass
//! through binary floating point. Writes are serialized with `BEGIN IMMEDIATE`, checked
//! against the book revision and recorded in `audit`.
use crate::change::Change;
use crate::chart;
use crate::model::{Book, BookKind};
use crate::num::Rounding;
use crate::{Error, Result, invalid};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params, types::Value as Sql};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

const APPLICATION_ID: i64 = 0x5959_4143; // "YYAC"
const VERSION: i64 = 3;
/// Tables added in version 2; a version 1 file gets them when opened for writing.
const SINCE_V2: &[&str] = &["tax_returns", "life_plans"];
/// Version 3: answers to the study problems (not part of any book).
const STUDY: &str = "CREATE TABLE IF NOT EXISTS study_attempts(id INTEGER PRIMARY KEY,problem TEXT NOT NULL,actor TEXT NOT NULL,at TEXT NOT NULL,correct INTEGER NOT NULL,answer TEXT NOT NULL CHECK(json_valid(answer)));\
     CREATE INDEX IF NOT EXISTS study_attempts_actor ON study_attempts(actor,problem,id);";

struct Child {
    field: &'static str,
    table: &'static str,
    columns: &'static [&'static str],
}
struct Spec {
    field: &'static str,
    table: &'static str,
    columns: &'static [&'static str],
    children: &'static [Child],
}
/// Column suffix: `:i` integer, `:b` boolean, `:j` JSON text, none = text (decimals, dates, enums).
const SPECS: &[Spec] = &[
    Spec {
        field: "accounts",
        table: "accounts",
        columns: &[
            "code",
            "name",
            "category",
            "role",
            "parent:i",
            "payment:b",
            "guide",
            "keywords:j",
            "archived:b",
        ],
        children: &[],
    },
    Spec {
        field: "partners",
        table: "partners",
        columns: &["name", "note", "archived:b"],
        children: &[],
    },
    Spec {
        field: "claims",
        table: "claims",
        columns: &[
            "side",
            "title",
            "partner:i",
            "account:i",
            "principal",
            "issued",
            "due",
            "annual_rate",
            "repayment",
            "installments:i",
            "closed:b",
            "memo",
        ],
        children: &[],
    },
    Spec {
        field: "entries",
        table: "entries",
        columns: &["date", "description", "partner:i", "source", "void:b"],
        children: &[Child {
            field: "lines",
            table: "entry_lines",
            columns: &["account:i", "debit", "credit", "memo", "claim:i"],
        }],
    },
    Spec {
        field: "budgets",
        table: "budgets",
        columns: &["name", "fiscal_year:i", "memo"],
        children: &[Child {
            field: "lines",
            table: "budget_lines",
            columns: &["account:i", "amounts:j"],
        }],
    },
    Spec {
        field: "plans",
        table: "plans",
        columns: &[
            "name",
            "debit:i",
            "credit:i",
            "amount",
            "start",
            "end",
            "frequency",
            "interval:i",
            "partner:i",
            "active:b",
            "memo",
        ],
        children: &[],
    },
    Spec {
        field: "materials",
        table: "materials",
        columns: &["name", "unit", "standard_cost", "archived:b", "memo"],
        children: &[],
    },
    Spec {
        field: "material_moves",
        table: "material_moves",
        columns: &[
            "material:i",
            "date",
            "kind",
            "quantity",
            "unit_cost",
            "memo",
        ],
        children: &[],
    },
    Spec {
        field: "equipment",
        table: "equipment",
        columns: &[
            "name",
            "acquired",
            "cost",
            "salvage",
            "life_years:i",
            "annual_hours",
            "running_cost_per_hour",
            "archived:b",
            "memo",
        ],
        children: &[],
    },
    Spec {
        field: "products",
        table: "products",
        columns: &[
            "name",
            "unit",
            "labor_hours",
            "labor_rate",
            "overhead_percent",
            "archived:b",
            "memo",
        ],
        children: &[
            Child {
                field: "materials",
                table: "product_materials",
                columns: &["material:i", "quantity", "loss_percent"],
            },
            Child {
                field: "equipment",
                table: "product_equipment",
                columns: &["equipment:i", "hours"],
            },
        ],
    },
    Spec {
        field: "productions",
        table: "productions",
        columns: &["product:i", "date", "quantity", "labor_hours", "memo"],
        children: &[
            Child {
                field: "materials",
                table: "production_materials",
                columns: &["material:i", "quantity"],
            },
            Child {
                field: "equipment",
                table: "production_equipment",
                columns: &["equipment:i", "hours"],
            },
        ],
    },
    Spec {
        field: "tax_returns",
        table: "tax_returns",
        columns: &[
            "year:i",
            "name",
            "memo",
            "salary",
            "salary_withheld",
            "social_insurance",
            "side_kind",
            "side_revenue",
            "side_expenses",
            "side_withheld",
            "blue_deduction",
            "self_disability",
            "has_spouse:b",
            "spouse_income",
            "spouse_birth_year:i",
            "spouse_disability",
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
        ],
        children: &[Child {
            field: "dependents",
            table: "tax_dependents",
            columns: &[
                "name",
                "relation",
                "birth_year:i",
                "income",
                "cohabiting:b",
                "disability",
            ],
        }],
    },
    Spec {
        field: "life_plans",
        table: "life_plans",
        columns: &[
            "name",
            "memo",
            "start_year:i",
            "years:i",
            "savings",
            "return_percent",
        ],
        children: &[
            Child {
                field: "members",
                table: "life_plan_members",
                columns: &["name", "relation", "birth_year:i"],
            },
            Child {
                field: "flows",
                table: "life_plan_flows",
                columns: &[
                    "name",
                    "kind",
                    "amount",
                    "start_year:i",
                    "end_year:i",
                    "growth_percent",
                ],
            },
            Child {
                field: "events",
                table: "life_plan_events",
                columns: &["year:i", "name", "kind", "amount", "growth_percent"],
            },
        ],
    },
];
const BOOK_COLUMNS: &[&str] = &[
    "name",
    "kind",
    "currency",
    "minor_digits:i",
    "rounding",
    "fiscal_start_month:i",
    "revision:i",
    "next_ids:j",
];

fn split(column: &str) -> (&str, char) {
    match column.split_once(':') {
        Some((name, t)) => (name, t.chars().next().unwrap_or('t')),
        None => (column, 't'),
    }
}
fn sql_type(t: char) -> &'static str {
    match t {
        'i' | 'b' => "INTEGER",
        _ => "TEXT",
    }
}
fn schema() -> String {
    let mut sql = String::from(
        "CREATE TABLE books(id INTEGER PRIMARY KEY,name TEXT NOT NULL,kind TEXT NOT NULL,currency TEXT NOT NULL,minor_digits INTEGER NOT NULL,rounding TEXT NOT NULL,fiscal_start_month INTEGER NOT NULL,revision INTEGER NOT NULL,next_ids TEXT NOT NULL CHECK(json_valid(next_ids)),created_at TEXT NOT NULL);\
         CREATE TABLE audit(id INTEGER PRIMARY KEY,book_id INTEGER NOT NULL REFERENCES books(id),at TEXT NOT NULL,actor TEXT NOT NULL,revision INTEGER NOT NULL,change TEXT NOT NULL CHECK(json_valid(change)));\
         CREATE INDEX audit_book ON audit(book_id,id);",
    );
    sql += &tables(|_| true);
    sql += "CREATE INDEX entries_date ON entries(book_id,date);CREATE INDEX entry_lines_account ON entry_lines(book_id,account);";
    sql += STUDY;
    sql
}
fn tables(include: impl Fn(&Spec) -> bool) -> String {
    let mut sql = String::new();
    for s in SPECS.iter().filter(|s| include(s)) {
        let cols: Vec<String> = s
            .columns
            .iter()
            .map(|c| {
                let (n, t) = split(c);
                format!("\"{n}\" {}", sql_type(t))
            })
            .collect();
        sql += &format!(
            "CREATE TABLE {}(book_id INTEGER NOT NULL REFERENCES books(id),id INTEGER NOT NULL,{},PRIMARY KEY(book_id,id));",
            s.table,
            cols.join(",")
        );
        for c in s.children {
            let cols: Vec<String> = c
                .columns
                .iter()
                .map(|c| {
                    let (n, t) = split(c);
                    format!("\"{n}\" {}", sql_type(t))
                })
                .collect();
            sql += &format!(
                "CREATE TABLE {}(book_id INTEGER NOT NULL,parent_id INTEGER NOT NULL,seq INTEGER NOT NULL,{},PRIMARY KEY(book_id,parent_id,seq),FOREIGN KEY(book_id,parent_id) REFERENCES {}(book_id,id));",
                c.table,
                cols.join(","),
                s.table
            );
        }
    }
    sql
}

fn to_sql(v: &Value, t: char) -> Result<Sql> {
    Ok(match (t, v) {
        (_, Value::Null) => Sql::Null,
        ('i', Value::Number(n)) => {
            Sql::Integer(n.as_i64().ok_or_else(|| invalid("整数が範囲外です"))?)
        }
        ('b', Value::Bool(b)) => Sql::Integer(i64::from(*b)),
        ('j', v) => Sql::Text(v.to_string()),
        ('t', Value::String(s)) => Sql::Text(s.clone()),
        _ => return Err(invalid(format!("内部の保存形式が一致しません: {v}"))),
    })
}
fn from_sql(v: Sql, t: char) -> Result<Value> {
    Ok(match (t, v) {
        (_, Sql::Null) => Value::Null,
        ('i', Sql::Integer(n)) => json!(n),
        ('b', Sql::Integer(n)) => json!(n != 0),
        ('j', Sql::Text(s)) => serde_json::from_str(&s)?,
        ('t', Sql::Text(s)) => json!(s),
        _ => return Err(invalid("データベースの値の型が不正です")),
    })
}
fn object_keys(o: &Map<String, Value>, expected: impl Iterator<Item = String>) -> Result<()> {
    let mut a: Vec<&String> = o.keys().collect();
    let mut b: Vec<String> = expected.collect();
    a.sort();
    b.sort();
    if a.iter()
        .map(|s| s.as_str())
        .ne(b.iter().map(String::as_str))
    {
        return Err(invalid(format!(
            "内部の保存形式が一致しません: {a:?} / {b:?}"
        )));
    }
    Ok(())
}

fn read_rows(
    c: &Connection,
    table: &str,
    key: &str,
    columns: &[&str],
    book: i64,
) -> Result<Vec<(i64, Map<String, Value>)>> {
    let names: Vec<String> = columns
        .iter()
        .map(|c| format!("\"{}\"", split(c).0))
        .collect();
    let order = if key == "id" { "id" } else { "parent_id,seq" };
    let mut q = c.prepare_cached(&format!(
        "SELECT {key},{} FROM {table} WHERE book_id=? ORDER BY {order}",
        names.join(",")
    ))?;
    let mut rows = q.query([book])?;
    let mut out = vec![];
    while let Some(r) = rows.next()? {
        let mut m = Map::new();
        for (i, col) in columns.iter().enumerate() {
            let (n, t) = split(col);
            m.insert(n.to_string(), from_sql(r.get::<_, Sql>(i + 1)?, t)?);
        }
        out.push((r.get(0)?, m));
    }
    Ok(out)
}

/// `v1` skips the tables a read-only version 1 file does not have yet.
fn load(c: &Connection, id: i64, v1: bool) -> Result<Book> {
    let cols: Vec<String> = BOOK_COLUMNS
        .iter()
        .map(|c| split(c).0.to_string())
        .collect();
    let row: Option<Vec<Sql>> = c
        .query_row(
            &format!("SELECT {} FROM books WHERE id=?", cols.join(",")),
            [id],
            |r| (0..cols.len()).map(|i| r.get::<_, Sql>(i)).collect(),
        )
        .optional()?;
    let row = row.ok_or_else(|| invalid("帳簿がありません"))?;
    let mut book = Map::new();
    book.insert("id".into(), json!(id));
    for (col, v) in BOOK_COLUMNS.iter().zip(row) {
        let (n, t) = split(col);
        book.insert(n.into(), from_sql(v, t)?);
    }
    for s in SPECS {
        if v1 && SINCE_V2.contains(&s.table) {
            book.insert(s.field.into(), Value::Array(vec![]));
            continue;
        }
        let mut items: Vec<(i64, Map<String, Value>)> = read_rows(c, s.table, "id", s.columns, id)?;
        for ch in s.children {
            let mut grouped: BTreeMap<i64, Vec<Value>> = BTreeMap::new();
            for (parent, m) in read_rows(c, ch.table, "parent_id", ch.columns, id)? {
                grouped.entry(parent).or_default().push(Value::Object(m));
            }
            for (item_id, m) in &mut items {
                m.insert(
                    ch.field.into(),
                    Value::Array(grouped.remove(item_id).unwrap_or_default()),
                );
            }
            if !grouped.is_empty() {
                return Err(invalid("明細の親レコードがありません"));
            }
        }
        let list: Vec<Value> = items
            .into_iter()
            .map(|(item_id, mut m)| {
                m.insert("id".into(), json!(item_id));
                Value::Object(m)
            })
            .collect();
        book.insert(s.field.into(), Value::Array(list));
    }
    let book: Book = serde_json::from_value(Value::Object(book))?;
    book.validate()?;
    Ok(book)
}

/// Writes the differences between two versions of a book.
fn write(c: &Connection, old: Option<&Book>, new: &Book) -> Result<()> {
    let newv = serde_json::to_value(new)?;
    let oldv = old.map(serde_json::to_value).transpose()?;
    let mut values = vec![Sql::Integer(new.id)];
    for col in BOOK_COLUMNS {
        let (n, t) = split(col);
        values.push(to_sql(&newv[n], t)?);
    }
    if old.is_none() {
        values.push(Sql::Text(chrono::Utc::now().to_rfc3339()));
        c.execute(
            "INSERT INTO books(id,name,kind,currency,minor_digits,rounding,fiscal_start_month,revision,next_ids,created_at) VALUES(?,?,?,?,?,?,?,?,?,?)",
            rusqlite::params_from_iter(values),
        )?;
    } else {
        values.rotate_left(1);
        c.execute(
            "UPDATE books SET name=?,kind=?,currency=?,minor_digits=?,rounding=?,fiscal_start_month=?,revision=?,next_ids=? WHERE id=?",
            rusqlite::params_from_iter(values),
        )?;
    }
    for s in SPECS {
        let index = |v: Option<&Value>| -> BTreeMap<i64, Value> {
            v.and_then(|v| v[s.field].as_array())
                .map(|a| {
                    a.iter()
                        .map(|x| (x["id"].as_i64().unwrap_or(0), x.clone()))
                        .collect()
                })
                .unwrap_or_default()
        };
        let before = index(oldv.as_ref());
        let after = index(Some(&newv));
        let delete = |id: i64| -> Result<()> {
            for ch in s.children {
                c.execute(
                    &format!("DELETE FROM {} WHERE book_id=? AND parent_id=?", ch.table),
                    params![new.id, id],
                )?;
            }
            c.execute(
                &format!("DELETE FROM {} WHERE book_id=? AND id=?", s.table),
                params![new.id, id],
            )?;
            Ok(())
        };
        for id in before.keys().filter(|id| !after.contains_key(id)) {
            delete(*id)?;
        }
        for (id, item) in &after {
            if before.get(id) == Some(item) {
                continue;
            }
            if before.contains_key(id) {
                delete(*id)?;
            }
            let o = item
                .as_object()
                .ok_or_else(|| invalid("内部形式が不正です"))?;
            object_keys(
                o,
                s.columns
                    .iter()
                    .map(|c| split(c).0.to_string())
                    .chain(s.children.iter().map(|c| c.field.to_string()))
                    .chain(["id".to_string()]),
            )?;
            let names: Vec<String> = s
                .columns
                .iter()
                .map(|c| format!("\"{}\"", split(c).0))
                .collect();
            let mut values = vec![Sql::Integer(new.id), Sql::Integer(*id)];
            for col in s.columns {
                let (n, t) = split(col);
                values.push(to_sql(&o[n], t)?);
            }
            c.prepare_cached(&format!(
                "INSERT INTO {}(book_id,id,{}) VALUES(?,?{})",
                s.table,
                names.join(","),
                ",?".repeat(names.len())
            ))?
            .execute(rusqlite::params_from_iter(values))?;
            for ch in s.children {
                let names: Vec<String> = ch
                    .columns
                    .iter()
                    .map(|c| format!("\"{}\"", split(c).0))
                    .collect();
                let mut stmt = c.prepare_cached(&format!(
                    "INSERT INTO {}(book_id,parent_id,seq,{}) VALUES(?,?,?{})",
                    ch.table,
                    names.join(","),
                    ",?".repeat(names.len())
                ))?;
                for (seq, row) in o[ch.field].as_array().into_iter().flatten().enumerate() {
                    let r = row
                        .as_object()
                        .ok_or_else(|| invalid("内部形式が不正です"))?;
                    object_keys(r, ch.columns.iter().map(|c| split(c).0.to_string()))?;
                    let mut values = vec![
                        Sql::Integer(new.id),
                        Sql::Integer(*id),
                        Sql::Integer(seq as i64),
                    ];
                    for col in ch.columns {
                        let (n, t) = split(col);
                        values.push(to_sql(&r[n], t)?);
                    }
                    stmt.execute(rusqlite::params_from_iter(values))?;
                }
            }
        }
    }
    Ok(())
}

#[derive(Clone, Debug, Serialize)]
pub struct BookInfo {
    pub id: i64,
    pub name: String,
    pub kind: BookKind,
    pub revision: i64,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NewBook {
    pub name: String,
    #[serde(default)]
    pub kind: BookKind,
    #[serde(default = "jpy")]
    pub currency: String,
    #[serde(default)]
    pub minor_digits: u32,
    #[serde(default)]
    pub rounding: Rounding,
    #[serde(default = "january")]
    pub fiscal_start_month: u32,
}
impl Book {
    /// A new book with the template chart, as [`Store::create`] stores it.
    pub fn blank(id: i64, request: &NewBook) -> Book {
        let accounts = chart::accounts(request.kind);
        Book {
            id,
            name: request.name.trim().to_string(),
            kind: request.kind,
            currency: request.currency.clone(),
            minor_digits: request.minor_digits,
            rounding: request.rounding,
            fiscal_start_month: request.fiscal_start_month,
            revision: 1,
            next_ids: BTreeMap::from([("account".into(), accounts.len() as i64 + 1)]),
            accounts,
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
}
fn jpy() -> String {
    "JPY".into()
}
fn january() -> u32 {
    1
}
#[derive(Clone, Debug, Serialize)]
pub struct Applied {
    pub book: Book,
    /// ID of the record the change created or updated, when there is one.
    pub id: Option<i64>,
}

pub struct Store {
    connection: Connection,
    read_only: bool,
    /// A version 1 file opened read-only (no tax or life plan tables).
    v1: bool,
    /// False for a read-only file older than version 3 (no study table yet).
    study: bool,
}
fn actor_ok(actor: &str) -> Result<()> {
    if actor.trim().is_empty() || actor.len() > 256 || actor.chars().any(char::is_control) {
        return Err(invalid("操作者名は1〜256バイトで指定してください"));
    }
    Ok(())
}
impl Store {
    pub fn open(path: &Path, read_only: bool) -> Result<Self> {
        let flags = if read_only {
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY
        } else {
            rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE | rusqlite::OpenFlags::SQLITE_OPEN_CREATE
        };
        let c = Connection::open_with_flags(path, flags)?;
        c.busy_timeout(Duration::from_secs(5))?;
        c.pragma_update(None, "foreign_keys", true)?;
        let application: i64 = c.pragma_query_value(None, "application_id", |r| r.get(0))?;
        let version: i64 = c.pragma_query_value(None, "user_version", |r| r.get(0))?;
        if application == 0 && version == 0 && !read_only {
            let tables: i64 = c.query_row(
                "SELECT count(*) FROM sqlite_master WHERE name NOT LIKE 'sqlite_%'",
                [],
                |r| r.get(0),
            )?;
            if tables != 0 {
                return Err(invalid(
                    "このSQLiteファイルはYYaccountantのデータベースではありません",
                ));
            }
            c.execute_batch(&format!(
                "BEGIN IMMEDIATE;{}PRAGMA application_id={APPLICATION_ID};PRAGMA user_version={VERSION};COMMIT;",
                schema()
            ))?;
        } else if application == APPLICATION_ID && (1..VERSION).contains(&version) && !read_only {
            let v2 = if version == 1 {
                tables(|s| SINCE_V2.contains(&s.table))
            } else {
                String::new()
            };
            c.execute_batch(&format!(
                "BEGIN IMMEDIATE;{v2}{STUDY}PRAGMA user_version={VERSION};COMMIT;"
            ))?;
        } else if application != APPLICATION_ID || !(1..=VERSION).contains(&version) {
            return Err(invalid("対応していないデータベース形式です"));
        }
        let v1 = version == 1 && read_only;
        let study = version >= 3 || !read_only;
        if !read_only {
            c.pragma_update(None, "journal_mode", "WAL")?;
        }
        Ok(Self {
            connection: c,
            read_only,
            v1,
            study,
        })
    }
    pub fn read_only(&self) -> bool {
        self.read_only
    }
    pub fn list(&self) -> Result<Vec<BookInfo>> {
        let mut q = self
            .connection
            .prepare("SELECT id,name,kind,revision FROM books ORDER BY id")?;
        let rows = q.query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, i64>(3)?,
            ))
        })?;
        let mut out = vec![];
        for row in rows {
            let (id, name, kind, revision) = row?;
            out.push(BookInfo {
                id,
                name,
                kind: serde_json::from_value(json!(kind))?,
                revision,
            });
        }
        Ok(out)
    }
    pub fn get(&mut self, id: i64) -> Result<Book> {
        let tx = self.connection.transaction()?;
        let book = load(&tx, id, self.v1)?;
        tx.commit()?;
        Ok(book)
    }
    pub fn revision(&self, id: i64) -> Result<i64> {
        self.connection
            .query_row("SELECT revision FROM books WHERE id=?", [id], |r| r.get(0))
            .optional()?
            .ok_or_else(|| invalid("帳簿がありません"))
    }
    pub fn create(&mut self, request: &NewBook, actor: &str) -> Result<Book> {
        if self.read_only {
            return Err(invalid("読み取り専用で開いています"));
        }
        actor_ok(actor)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let id: i64 = tx.query_row("SELECT coalesce(max(id),0)+1 FROM books", [], |r| r.get(0))?;
        let book = Book::blank(id, request);
        book.validate()?;
        write(&tx, None, &book)?;
        tx.execute(
            "INSERT INTO audit(book_id,at,actor,revision,change) VALUES(?,?,?,1,?)",
            params![
                id,
                chrono::Utc::now().to_rfc3339(),
                actor,
                json!({"operation":"create_book","name":book.name,"kind":book.kind}).to_string()
            ],
        )?;
        tx.commit()?;
        Ok(book)
    }
    /// Applies a change atomically. Fails with [`Error::Conflict`] when the revision moved.
    pub fn apply(
        &mut self,
        id: i64,
        expected_revision: i64,
        change: &Change,
        actor: &str,
    ) -> Result<Applied> {
        if self.read_only {
            return Err(invalid("読み取り専用で開いています"));
        }
        actor_ok(actor)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let old = load(&tx, id, false)?;
        if old.revision != expected_revision {
            return Err(Error::Conflict);
        }
        let mut book = old.clone();
        let changed = book.apply(change)?;
        book.revision += 1;
        book.validate()?;
        write(&tx, Some(&old), &book)?;
        let mut record = serde_json::to_value(change)?;
        if let Some(changed) = changed {
            record["result_id"] = json!(changed);
        }
        tx.execute(
            "INSERT INTO audit(book_id,at,actor,revision,change) VALUES(?,?,?,?,?)",
            params![
                id,
                chrono::Utc::now().to_rfc3339(),
                actor,
                book.revision,
                record.to_string()
            ],
        )?;
        tx.commit()?;
        Ok(Applied { book, id: changed })
    }
    pub fn audit(&self, id: i64, after: i64) -> Result<Vec<Value>> {
        let mut q = self
            .connection
            .prepare("SELECT id,at,actor,revision,change FROM audit WHERE book_id=? AND id>? ORDER BY id LIMIT 100")?;
        let rows = q.query_map(params![id, after], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, String>(4)?,
            ))
        })?;
        let mut out = vec![];
        for row in rows {
            let (aid, at, actor, revision, change) = row?;
            out.push(json!({"id":aid,"at":at,"actor":actor,"revision":revision,"change":serde_json::from_str::<Value>(&change)?}));
        }
        Ok(out)
    }
    /// Records an answer to a study problem. Returns the attempt ID.
    pub fn record_attempt(
        &mut self,
        actor: &str,
        problem: &str,
        answer: &Value,
        correct: bool,
    ) -> Result<i64> {
        if self.read_only {
            return Err(invalid("読み取り専用で開いています"));
        }
        actor_ok(actor)?;
        if problem.is_empty() || problem.len() > 64 {
            return Err(invalid("問題IDが不正です"));
        }
        let text = answer.to_string();
        if text.len() > 64 * 1024 {
            return Err(invalid("解答が大きすぎます"));
        }
        self.connection.execute(
            "INSERT INTO study_attempts(problem,actor,at,correct,answer) VALUES(?,?,?,?,?)",
            params![
                problem,
                actor,
                chrono::Utc::now().to_rfc3339(),
                correct,
                text
            ],
        )?;
        Ok(self.connection.last_insert_rowid())
    }
    /// Every attempt of one learner, oldest first.
    pub fn attempts(&self, actor: &str) -> Result<Vec<crate::study::Attempt>> {
        if !self.study {
            return Ok(vec![]);
        }
        let mut q = self.connection.prepare(
            "SELECT id,problem,at,correct,answer FROM study_attempts WHERE actor=? ORDER BY id",
        )?;
        let rows = q.query_map([actor], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, bool>(3)?,
                r.get::<_, String>(4)?,
            ))
        })?;
        let mut out = vec![];
        for row in rows {
            let (id, problem, at, correct, answer) = row?;
            out.push(crate::study::Attempt {
                id,
                problem,
                at,
                correct,
                answer: serde_json::from_str(&answer)?,
            });
        }
        Ok(out)
    }
    /// Changes whenever another connection commits; used to refresh UIs cheaply.
    pub fn data_version(&self) -> Result<i64> {
        Ok(self
            .connection
            .pragma_query_value(None, "data_version", |r| r.get(0))?)
    }
    /// Consistent copy into a new SQLite file.
    pub fn backup(&self, path: &Path) -> Result<()> {
        if path.exists() {
            return Err(invalid("バックアップ先は新しいファイルにしてください"));
        }
        let target = path.to_str().ok_or_else(|| invalid("パスが不正です"))?;
        self.connection.execute("VACUUM INTO ?", [target])?;
        Ok(())
    }
}
