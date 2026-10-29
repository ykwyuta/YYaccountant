//! Cost management: moving-average material inventory, equipment depreciation and hourly
//! rates, standard product costs (bill of materials) and actual production costs.
use crate::model::{Book, Equipment, EquipmentUse, Product, Production};
use crate::num::{self, D, Decimal};
use crate::{Result, invalid};
use chrono::{Datelike, Months, NaiveDate};
use serde::Serialize;
use std::collections::BTreeMap;

/// Unit prices and rates keep four decimal places; totals use the book's currency rounding.
fn unit(d: Decimal) -> Decimal {
    d.round_dp(4).normalize()
}
fn month_index(d: NaiveDate) -> i64 {
    i64::from(d.year()) * 12 + i64::from(d.month0())
}
pub fn fiscal_range(book: &Book, fiscal_year: i32) -> Result<(NaiveDate, NaiveDate)> {
    let start = NaiveDate::from_ymd_opt(fiscal_year, book.fiscal_start_month, 1)
        .ok_or_else(|| invalid("年度が不正です"))?;
    let end = start
        .checked_add_months(Months::new(12))
        .and_then(|d| d.pred_opt())
        .ok_or_else(|| invalid("年度が不正です"))?;
    Ok((start, end))
}
/// Fiscal year (by its starting calendar year) containing the date.
pub fn fiscal_year_of(book: &Book, date: NaiveDate) -> i32 {
    if date.month() >= book.fiscal_start_month {
        date.year()
    } else {
        date.year() - 1
    }
}

/// Straight-line depreciation with monthly proration (acquisition month counts in full).
pub fn accumulated(book: &Book, e: &Equipment, through: NaiveDate) -> Decimal {
    let total = i64::from(e.life_years) * 12;
    let used = (month_index(through) - month_index(e.acquired) + 1).clamp(0, total);
    let base = e.cost - e.salvage;
    if used == total {
        base
    } else {
        book.round(base * Decimal::from(used) / Decimal::from(total))
    }
}
pub fn depreciation(book: &Book, e: &Equipment, fiscal_year: i32) -> Result<Decimal> {
    let (start, end) = fiscal_range(book, fiscal_year)?;
    let before = start.pred_opt().ok_or_else(|| invalid("年度が不正です"))?;
    Ok(accumulated(book, e, end) - accumulated(book, e, before))
}
pub fn hourly_rate(e: &Equipment) -> Decimal {
    unit(
        (e.cost - e.salvage) / Decimal::from(e.life_years) / e.annual_hours
            + e.running_cost_per_hour,
    )
}

#[derive(Clone, Debug, Serialize)]
pub struct DepreciationRow {
    pub fiscal_year: i32,
    pub amount: D,
    pub accumulated: D,
    pub book_value: D,
    pub posted: bool,
}
pub fn depreciation_source(equipment: i64, fiscal_year: i32) -> String {
    format!("depreciation:{equipment}:{fiscal_year}")
}
pub fn depreciation_schedule(book: &Book, e: &Equipment) -> Result<Vec<DepreciationRow>> {
    let mut rows = vec![];
    let first = fiscal_year_of(book, e.acquired);
    for fy in first..first + e.life_years as i32 + 2 {
        let amount = depreciation(book, e, fy)?;
        if amount.is_zero() {
            if rows.is_empty() {
                continue;
            }
            break;
        }
        let (_, end) = fiscal_range(book, fy)?;
        let acc = accumulated(book, e, end);
        let key = depreciation_source(e.id, fy);
        rows.push(DepreciationRow {
            fiscal_year: fy,
            amount: D(amount),
            accumulated: D(acc),
            book_value: D(e.cost - acc),
            posted: book.entries.iter().any(|x| !x.void && x.source == key),
        });
    }
    Ok(rows)
}

/// Gross material requirement per unit including loss: quantity / (1 - loss%).
pub fn gross(quantity: Decimal, loss_percent: Decimal) -> Decimal {
    quantity / (Decimal::ONE - loss_percent / num::hundred())
}
/// Actual or (when not entered) standard consumption of a production run.
pub fn consumption(book: &Book, p: &Production) -> Vec<(i64, Decimal)> {
    if !p.materials.is_empty() {
        return p
            .materials
            .iter()
            .map(|m| (m.material, m.quantity))
            .collect();
    }
    book.products
        .iter()
        .find(|x| x.id == p.product)
        .map(|prod| {
            prod.materials
                .iter()
                .map(|m| {
                    (
                        m.material,
                        unit(gross(m.quantity, m.loss_percent) * p.quantity),
                    )
                })
                .collect()
        })
        .unwrap_or_default()
}
fn machine_hours(book: &Book, p: &Production) -> Vec<EquipmentUse> {
    if !p.equipment.is_empty() {
        return p.equipment.clone();
    }
    book.products
        .iter()
        .find(|x| x.id == p.product)
        .map(|prod| {
            prod.equipment
                .iter()
                .map(|u| EquipmentUse {
                    equipment: u.equipment,
                    hours: u.hours * p.quantity,
                })
                .collect()
        })
        .unwrap_or_default()
}
fn labor_hours(book: &Book, p: &Production) -> Decimal {
    if !p.labor_hours.is_zero() {
        return p.labor_hours;
    }
    book.products
        .iter()
        .find(|x| x.id == p.product)
        .map(|x| x.labor_hours * p.quantity)
        .unwrap_or_default()
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Stock {
    pub material: i64,
    pub name: String,
    pub unit: String,
    pub quantity: D,
    pub average_cost: D,
    pub value: D,
    pub standard_cost: D,
    pub warnings: Vec<String>,
}
struct Valuation {
    stock: BTreeMap<i64, Stock>,
    /// (production, material) -> unit cost at issue
    issues: BTreeMap<(i64, i64), Decimal>,
}
/// Moving-average valuation through `as_of`. Same-day receipts precede issues.
fn valuation(book: &Book, as_of: Option<NaiveDate>) -> Valuation {
    enum Ev {
        In(Decimal, Option<Decimal>),
        Out(Decimal, Option<i64>),
    }
    let mut events: Vec<(NaiveDate, u8, i64, i64, Ev)> = vec![];
    for m in &book.material_moves {
        let ev = if m.quantity.is_sign_negative() {
            Ev::Out(-m.quantity, None)
        } else {
            Ev::In(m.quantity, m.unit_cost)
        };
        events.push((
            m.date,
            if matches!(ev, Ev::In(..)) { 0 } else { 2 },
            m.id,
            m.material,
            ev,
        ));
    }
    for p in &book.productions {
        for (material, q) in consumption(book, p) {
            events.push((p.date, 1, p.id, material, Ev::Out(q, Some(p.id))));
        }
    }
    events.sort_by_key(|e| (e.0, e.1, e.2, e.3));
    let mut state: BTreeMap<i64, (Decimal, Decimal, Decimal)> = BTreeMap::new(); // qty, value, avg
    let mut warnings: BTreeMap<i64, Vec<String>> = BTreeMap::new();
    let mut issues = BTreeMap::new();
    for (date, _, _, material, ev) in events {
        if as_of.is_some_and(|d| date > d) {
            continue;
        }
        let standard = book
            .materials
            .iter()
            .find(|m| m.id == material)
            .map(|m| m.standard_cost)
            .unwrap_or_default();
        let s = state
            .entry(material)
            .or_insert((Decimal::ZERO, Decimal::ZERO, standard));
        match ev {
            Ev::In(q, cost) => {
                let c = cost.unwrap_or(s.2);
                if s.0 <= Decimal::ZERO {
                    // A shortage was issued at the last known cost; revalue what remains at this receipt.
                    s.0 += q;
                    s.1 = s.0 * c;
                    s.2 = c;
                } else {
                    s.0 += q;
                    s.1 += q * c;
                    s.2 = s.1 / s.0;
                }
            }
            Ev::Out(q, production) => {
                if q > s.0 {
                    warnings
                        .entry(material)
                        .or_default()
                        .push(format!("{date} に在庫を超えて払い出しています"));
                }
                if let Some(p) = production {
                    issues.insert((p, material), s.2);
                }
                s.0 -= q;
                s.1 -= q * s.2;
                if s.0.is_zero() {
                    s.1 = Decimal::ZERO;
                }
            }
        }
    }
    let mut stock = BTreeMap::new();
    for m in &book.materials {
        let (q, v, a) =
            state
                .get(&m.id)
                .copied()
                .unwrap_or((Decimal::ZERO, Decimal::ZERO, m.standard_cost));
        stock.insert(
            m.id,
            Stock {
                material: m.id,
                name: m.name.clone(),
                unit: m.unit.clone(),
                quantity: D(q.normalize()),
                average_cost: D(unit(a)),
                value: D(book.round(v)),
                standard_cost: D(m.standard_cost),
                warnings: warnings.remove(&m.id).unwrap_or_default(),
            },
        );
    }
    Valuation { stock, issues }
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct CostLine {
    pub kind: &'static str,
    pub id: i64,
    pub name: String,
    pub quantity: D,
    pub unit_cost: D,
    pub amount: D,
}
#[derive(Clone, Debug, Default, Serialize)]
pub struct CostBreakdown {
    pub lines: Vec<CostLine>,
    pub materials: D,
    pub equipment: D,
    pub labor: D,
    pub overhead: D,
    pub total: D,
}
fn finish(
    book: &Book,
    mut b: CostBreakdown,
    labor: Decimal,
    overhead_percent: Decimal,
) -> CostBreakdown {
    let sum = |kind| -> Decimal {
        b.lines
            .iter()
            .filter(|l| l.kind == kind)
            .map(|l| l.amount.0)
            .sum()
    };
    let materials = sum("material");
    let equipment = sum("equipment");
    let overhead = (materials + equipment + labor) * overhead_percent / num::hundred();
    b.materials = D(book.round(materials));
    b.equipment = D(book.round(equipment));
    b.labor = D(book.round(labor));
    b.overhead = D(book.round(overhead));
    b.total = D(b.materials.0 + b.equipment.0 + b.labor.0 + b.overhead.0);
    b
}
/// Standard cost of one unit. Lines keep 4 decimals so per-unit costs below one currency unit stay visible.
pub fn standard(book: &Book, p: &Product, stock: &BTreeMap<i64, Stock>) -> CostBreakdown {
    let mut b = CostBreakdown::default();
    for m in &p.materials {
        let mat = book.materials.iter().find(|x| x.id == m.material);
        let price = mat
            .map(|x| x.standard_cost)
            .filter(|c| !c.is_zero())
            .unwrap_or_else(|| {
                stock
                    .get(&m.material)
                    .map(|s| s.average_cost.0)
                    .unwrap_or_default()
            });
        let q = unit(gross(m.quantity, m.loss_percent));
        b.lines.push(CostLine {
            kind: "material",
            id: m.material,
            name: mat.map(|x| x.name.clone()).unwrap_or_default(),
            quantity: D(q),
            unit_cost: D(price),
            amount: D(unit(q * price)),
        });
    }
    for u in &p.equipment {
        if let Some(e) = book.equipment.iter().find(|x| x.id == u.equipment) {
            let rate = hourly_rate(e);
            b.lines.push(CostLine {
                kind: "equipment",
                id: e.id,
                name: e.name.clone(),
                quantity: D(u.hours),
                unit_cost: D(rate),
                amount: D(unit(u.hours * rate)),
            });
        }
    }
    let sum = |kind| -> Decimal {
        b.lines
            .iter()
            .filter(|l| l.kind == kind)
            .map(|l| l.amount.0)
            .sum()
    };
    let (materials, equipment) = (sum("material"), sum("equipment"));
    let labor = unit(p.labor_hours * p.labor_rate);
    let overhead = unit((materials + equipment + labor) * p.overhead_percent / num::hundred());
    b.materials = D(materials);
    b.equipment = D(equipment);
    b.labor = D(labor);
    b.overhead = D(overhead);
    b.total = D(materials + equipment + labor + overhead);
    b
}

#[derive(Clone, Debug, Serialize)]
pub struct ProductCost {
    pub product: i64,
    pub name: String,
    pub unit: String,
    pub standard: CostBreakdown,
}
#[derive(Clone, Debug, Serialize)]
pub struct ProductionCost {
    pub production: i64,
    pub product: i64,
    pub name: String,
    pub date: NaiveDate,
    pub quantity: D,
    pub actual: CostBreakdown,
    pub actual_unit_cost: D,
    pub standard_total: D,
    pub variance: D,
    pub variance_materials: D,
    pub variance_equipment: D,
    pub variance_labor: D,
    pub variance_overhead: D,
}
#[derive(Clone, Debug, Serialize)]
pub struct EquipmentCost {
    pub equipment: i64,
    pub name: String,
    pub hourly_rate: D,
    pub annual_depreciation: D,
    pub book_value: D,
    pub schedule: Vec<DepreciationRow>,
}
#[derive(Clone, Debug, Serialize)]
pub struct CostReport {
    pub as_of: NaiveDate,
    pub stock: Vec<Stock>,
    pub inventory_value: D,
    pub equipment: Vec<EquipmentCost>,
    pub products: Vec<ProductCost>,
    pub productions: Vec<ProductionCost>,
}

pub fn report(book: &Book, as_of: NaiveDate) -> Result<CostReport> {
    let current = valuation(book, Some(as_of));
    let all = valuation(book, None);
    let mut equipment = vec![];
    for e in book.equipment.iter().filter(|e| !e.archived) {
        let annual = (e.cost - e.salvage) / Decimal::from(e.life_years);
        equipment.push(EquipmentCost {
            equipment: e.id,
            name: e.name.clone(),
            hourly_rate: D(hourly_rate(e)),
            annual_depreciation: D(book.round(annual)),
            book_value: D(e.cost - accumulated(book, e, as_of)),
            schedule: depreciation_schedule(book, e)?,
        });
    }
    let mut products = vec![];
    for p in book.products.iter().filter(|p| !p.archived) {
        products.push(ProductCost {
            product: p.id,
            name: p.name.clone(),
            unit: p.unit.clone(),
            standard: standard(book, p, &current.stock),
        });
    }
    let mut productions = vec![];
    for run in book.productions.iter().filter(|r| r.date <= as_of) {
        let Some(product) = book.products.iter().find(|p| p.id == run.product) else {
            continue;
        };
        let mut b = CostBreakdown::default();
        for (material, q) in consumption(book, run) {
            let price = all
                .issues
                .get(&(run.id, material))
                .copied()
                .unwrap_or_default();
            b.lines.push(CostLine {
                kind: "material",
                id: material,
                name: book
                    .materials
                    .iter()
                    .find(|m| m.id == material)
                    .map(|m| m.name.clone())
                    .unwrap_or_default(),
                quantity: D(q),
                unit_cost: D(unit(price)),
                amount: D(q * price),
            });
        }
        for u in machine_hours(book, run) {
            if let Some(e) = book.equipment.iter().find(|x| x.id == u.equipment) {
                let rate = hourly_rate(e);
                b.lines.push(CostLine {
                    kind: "equipment",
                    id: e.id,
                    name: e.name.clone(),
                    quantity: D(u.hours),
                    unit_cost: D(rate),
                    amount: D(u.hours * rate),
                });
            }
        }
        let labor = labor_hours(book, run) * product.labor_rate;
        let mut actual = finish(book, b, labor, product.overhead_percent);
        for l in &mut actual.lines {
            l.amount = D(book.round(l.amount.0));
        }
        let st = standard(book, product, &current.stock);
        let q = run.quantity;
        let std_m = book.round(st.materials.0 * q);
        let std_e = book.round(st.equipment.0 * q);
        let std_l = book.round(st.labor.0 * q);
        let std_o = book.round(st.overhead.0 * q);
        let std_total = std_m + std_e + std_l + std_o;
        productions.push(ProductionCost {
            production: run.id,
            product: product.id,
            name: product.name.clone(),
            date: run.date,
            quantity: D(q),
            actual_unit_cost: D(unit(actual.total.0 / q)),
            standard_total: D(std_total),
            variance: D(actual.total.0 - std_total),
            variance_materials: D(actual.materials.0 - std_m),
            variance_equipment: D(actual.equipment.0 - std_e),
            variance_labor: D(actual.labor.0 - std_l),
            variance_overhead: D(actual.overhead.0 - std_o),
            actual,
        });
    }
    let stock: Vec<Stock> = current.stock.into_values().collect();
    let inventory_value = D(stock.iter().map(|s| s.value.0).sum());
    Ok(CostReport {
        as_of,
        stock,
        inventory_value,
        equipment,
        products,
        productions,
    })
}
