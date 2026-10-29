//! Atomic changes. Each change is applied to a copy of the book, the whole book is
//! validated, and only then is it persisted with a new revision and an audit record.
use crate::claims;
use crate::cost;
use crate::model::*;
use crate::num::{self, Decimal, Rounding};
use crate::plans;
use crate::{Result, invalid};
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuickKind {
    /// 支出: 借方=費用（または資産）, 貸方=支払手段
    Expense,
    /// 収入: 借方=入金先, 貸方=収益
    Income,
    /// 振替: 借方=移動先, 貸方=移動元
    Transfer,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuickEntry {
    pub kind: QuickKind,
    pub date: NaiveDate,
    #[serde(with = "num::dec")]
    pub amount: Decimal,
    /// Expense/income account, or the destination of a transfer.
    pub account: i64,
    /// Payment means (cash, bank, card), or the source of a transfer.
    pub via: i64,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub partner: Option<i64>,
    /// 事業割合（%）: the rest of an expense goes to 事業主貸 (sole proprietors).
    #[serde(default, with = "num::opt")]
    pub business_percent: Option<Decimal>,
    #[serde(default)]
    pub memo: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settlement {
    pub claim: i64,
    pub date: NaiveDate,
    /// 元本の回収・返済額
    #[serde(with = "num::dec")]
    pub principal: Decimal,
    /// 利息
    #[serde(default, with = "num::dec")]
    pub interest: Decimal,
    /// 入出金口座
    pub via: i64,
    #[serde(default)]
    pub description: String,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Target {
    Account,
    Partner,
    Claim,
    Budget,
    Plan,
    Material,
    MaterialMove,
    Equipment,
    Product,
    Production,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum Change {
    UpdateBook {
        name: String,
        currency: String,
        minor_digits: u32,
        rounding: Rounding,
        fiscal_start_month: u32,
    },
    SaveAccount {
        account: Account,
    },
    SavePartner {
        partner: Partner,
    },
    /// New (id 0) or replaced journal entry.
    SaveEntry {
        entry: Entry,
    },
    QuickEntry {
        quick: QuickEntry,
    },
    /// 取消／取消の解除。仕訳は削除せず監査のため残す。
    VoidEntry {
        id: i64,
        void: bool,
    },
    /// `opening` posts the claim's origin against this counter account (new claims only).
    SaveClaim {
        claim: Claim,
        #[serde(default)]
        opening: Option<i64>,
    },
    SettleClaim {
        settlement: Settlement,
    },
    SaveBudget {
        budget: Budget,
    },
    SavePlan {
        plan: Plan,
    },
    /// Posts one occurrence of a plan; `date` and `amount` default to the plan's.
    PostPlan {
        plan: i64,
        occurrence: NaiveDate,
        #[serde(default)]
        date: Option<NaiveDate>,
        #[serde(default, with = "num::opt")]
        amount: Option<Decimal>,
    },
    SaveMaterial {
        material: Material,
    },
    SaveMaterialMove {
        material_move: MaterialMove,
    },
    SaveEquipment {
        equipment: Equipment,
    },
    PostDepreciation {
        equipment: i64,
        fiscal_year: i32,
        date: NaiveDate,
    },
    SaveProduct {
        product: Product,
    },
    SaveProduction {
        production: Production,
    },
    Remove {
        target: Target,
        id: i64,
    },
}

/// Inserts (id 0) or replaces by id.
fn upsert<T: Clone>(
    list: &mut Vec<T>,
    mut item: T,
    id: impl Fn(&T) -> i64,
    set: impl Fn(&mut T, i64),
    allocate: impl FnOnce(&[T]) -> i64,
    what: &str,
) -> Result<i64> {
    let current = id(&item);
    if current == 0 {
        let new = allocate(list);
        set(&mut item, new);
        list.push(item);
        Ok(new)
    } else {
        let slot = list
            .iter_mut()
            .find(|x| id(x) == current)
            .ok_or_else(|| invalid(format!("{what} {current} がありません")))?;
        *slot = item;
        Ok(current)
    }
}
macro_rules! save {
    ($book:ident, $field:ident, $item:expr, $kind:literal, $what:literal) => {{
        let mut next = $book.next_ids.clone();
        let id = upsert(
            &mut $book.$field,
            $item,
            |x| x.id,
            |x, id| x.id = id,
            |list| {
                let n = list
                    .iter()
                    .map(|x| x.id)
                    .max()
                    .unwrap_or(0)
                    .max(next.get($kind).copied().unwrap_or(1) - 1)
                    + 1;
                next.insert($kind.to_string(), n + 1);
                n
            },
            $what,
        )?;
        $book.next_ids = next;
        id
    }};
}

fn line(account: i64, debit: Decimal, credit: Decimal) -> Line {
    Line {
        account,
        debit,
        credit,
        memo: String::new(),
        claim: None,
    }
}

impl Book {
    fn active_account(&self, id: i64) -> Result<&Account> {
        let a = self.account(id)?;
        if a.archived {
            return Err(invalid(format!("勘定科目「{}」は使用停止中です", a.name)));
        }
        Ok(a)
    }
    fn push_entry(&mut self, mut entry: Entry) -> Result<i64> {
        for l in &entry.lines {
            self.active_account(l.account)?;
        }
        entry
            .lines
            .retain(|l| !(l.debit.is_zero() && l.credit.is_zero()));
        entry.id = 0;
        Ok(save!(self, entries, entry, "entry", "仕訳"))
    }
    fn quick(&mut self, q: &QuickEntry) -> Result<i64> {
        self.money(q.amount, "金額")?;
        if q.amount <= Decimal::ZERO {
            return Err(invalid("金額は正の値にしてください"));
        }
        let account = self.active_account(q.account)?.clone();
        let via = self.active_account(q.via)?.clone();
        if q.account == q.via {
            return Err(invalid("科目と入出金の科目は別にしてください"));
        }
        let money_side = |a: &Account| {
            matches!(
                a.category,
                Category::Asset | Category::Liability | Category::Equity
            )
        };
        if !money_side(&via) {
            return Err(invalid(
                "支払・入金の科目には資産・負債・純資産の科目を選んでください",
            ));
        }
        let mut lines = vec![];
        match q.kind {
            QuickKind::Expense => {
                if !matches!(account.category, Category::Expense | Category::Asset) {
                    return Err(invalid(
                        "支出の科目には費用または資産の科目を選んでください",
                    ));
                }
                let business = match q.business_percent {
                    Some(p) if p < num::hundred() => {
                        if p.is_sign_negative() || p.scale() > 4 {
                            return Err(invalid("事業割合は0〜100%にしてください"));
                        }
                        self.round(q.amount * p / num::hundred())
                    }
                    Some(p) if p > num::hundred() => {
                        return Err(invalid("事業割合は0〜100%にしてください"));
                    }
                    _ => q.amount,
                };
                if !business.is_zero() {
                    lines.push(line(account.id, business, Decimal::ZERO));
                }
                if business != q.amount {
                    let draw = self.role(Role::OwnerDraw)?.id;
                    let mut l = line(draw, q.amount - business, Decimal::ZERO);
                    l.memo = format!(
                        "家事按分（事業割合{}%）",
                        num::text(q.business_percent.unwrap_or_default())
                    );
                    lines.push(l);
                }
                lines.push(line(via.id, Decimal::ZERO, q.amount));
            }
            QuickKind::Income => {
                if !matches!(
                    account.category,
                    Category::Income | Category::Equity | Category::Liability
                ) {
                    return Err(invalid("収入の科目には収益の科目を選んでください"));
                }
                lines.push(line(via.id, q.amount, Decimal::ZERO));
                lines.push(line(account.id, Decimal::ZERO, q.amount));
            }
            QuickKind::Transfer => {
                if !money_side(&account) {
                    return Err(invalid(
                        "振替先には資産・負債・純資産の科目を選んでください",
                    ));
                }
                lines.push(line(account.id, q.amount, Decimal::ZERO));
                lines.push(line(via.id, Decimal::ZERO, q.amount));
            }
        }
        if let Some(first) = lines.first_mut()
            && first.memo.is_empty()
        {
            first.memo = q.memo.clone();
        }
        self.push_entry(Entry {
            id: 0,
            date: q.date,
            description: q.description.clone(),
            partner: q.partner,
            source: String::new(),
            void: false,
            lines,
        })
    }

    /// Applies a change in memory. Returns the ID of the created or changed record when there is one.
    pub fn apply(&mut self, change: &Change) -> Result<Option<i64>> {
        let id = match change {
            Change::UpdateBook {
                name,
                currency,
                minor_digits,
                rounding,
                fiscal_start_month,
            } => {
                self.name = name.trim().to_string();
                self.currency = currency.clone();
                self.minor_digits = *minor_digits;
                self.rounding = *rounding;
                self.fiscal_start_month = *fiscal_start_month;
                None
            }
            Change::SaveAccount { account } => {
                let mut a = account.clone();
                a.code = a.code.trim().into();
                a.name = a.name.trim().into();
                Some(save!(self, accounts, a, "account", "勘定科目"))
            }
            Change::SavePartner { partner } => {
                let mut p = partner.clone();
                p.name = p.name.trim().into();
                Some(save!(self, partners, p, "partner", "取引先"))
            }
            Change::SaveEntry { entry } => {
                let mut e = entry.clone();
                for l in &e.lines {
                    self.active_account(l.account)?;
                }
                if e.id == 0 {
                    if !e.source.is_empty() {
                        return Err(invalid("自動計上キーはサーバーが設定します"));
                    }
                } else {
                    let old = self
                        .entries
                        .iter()
                        .find(|x| x.id == e.id)
                        .ok_or_else(|| invalid("仕訳がありません"))?;
                    // The automatic posting key and void state are server-maintained.
                    e.source = old.source.clone();
                    e.void = old.void;
                }
                Some(save!(self, entries, e, "entry", "仕訳"))
            }
            Change::QuickEntry { quick } => Some(self.quick(quick)?),
            Change::VoidEntry { id, void } => {
                let e = self
                    .entries
                    .iter_mut()
                    .find(|e| e.id == *id)
                    .ok_or_else(|| invalid("仕訳がありません"))?;
                if e.void == *void {
                    return Err(invalid(if *void {
                        "すでに取消済みです"
                    } else {
                        "取消されていません"
                    }));
                }
                e.void = *void;
                Some(*id)
            }
            Change::SaveClaim { claim, opening } => {
                let new = claim.id == 0;
                if !new && opening.is_some() {
                    return Err(invalid("発生仕訳は新規登録時だけ作成できます"));
                }
                let id = save!(self, claims, claim.clone(), "claim", "債権債務");
                if let Some(counter) = opening {
                    self.active_account(*counter)?;
                    let c = self.claims.iter().find(|c| c.id == id).unwrap().clone();
                    let mut own = line(c.account, Decimal::ZERO, Decimal::ZERO);
                    own.claim = Some(id);
                    let mut other = line(*counter, Decimal::ZERO, Decimal::ZERO);
                    match c.side {
                        Side::Receivable => {
                            own.debit = c.principal;
                            other.credit = c.principal;
                        }
                        Side::Payable => {
                            other.debit = c.principal;
                            own.credit = c.principal;
                        }
                    }
                    let mut entry = Entry {
                        id: 0,
                        date: c.issued,
                        description: c.title.clone(),
                        partner: c.partner,
                        source: format!("claim:{id}:opening"),
                        void: false,
                        lines: vec![own, other],
                    };
                    entry.lines.sort_by_key(|l| l.debit.is_zero());
                    self.push_entry(entry)?;
                }
                Some(id)
            }
            Change::SettleClaim { settlement: s } => {
                let c = self
                    .claims
                    .iter()
                    .find(|c| c.id == s.claim)
                    .ok_or_else(|| invalid("債権債務がありません"))?
                    .clone();
                if c.closed {
                    return Err(invalid("完了済みの債権債務です"));
                }
                self.money(s.principal, "元本")?;
                self.money(s.interest, "利息")?;
                if s.principal.is_sign_negative()
                    || s.interest.is_sign_negative()
                    || (s.principal + s.interest).is_zero()
                {
                    return Err(invalid("元本・利息は0以上で、合計を正にしてください"));
                }
                let left = claims::outstanding(self, &c);
                if s.principal > left {
                    return Err(invalid(format!(
                        "元本の残高 {} を超えています",
                        num::text(left)
                    )));
                }
                self.active_account(s.via)?;
                let total = s.principal + s.interest;
                let mut lines = vec![];
                let mut own = line(c.account, Decimal::ZERO, Decimal::ZERO);
                own.claim = Some(c.id);
                match c.side {
                    Side::Receivable => {
                        lines.push(line(s.via, total, Decimal::ZERO));
                        own.credit = s.principal;
                        lines.push(own);
                        if !s.interest.is_zero() {
                            lines.push(line(
                                self.role(Role::InterestIncome)?.id,
                                Decimal::ZERO,
                                s.interest,
                            ));
                        }
                    }
                    Side::Payable => {
                        own.debit = s.principal;
                        lines.push(own);
                        if !s.interest.is_zero() {
                            lines.push(line(
                                self.role(Role::InterestExpense)?.id,
                                s.interest,
                                Decimal::ZERO,
                            ));
                        }
                        lines.push(line(s.via, Decimal::ZERO, total));
                    }
                }
                let seq = self
                    .entries
                    .iter()
                    .filter(|e| e.source.starts_with(&format!("settle:{}:", c.id)))
                    .count()
                    + 1;
                let id = self.push_entry(Entry {
                    id: 0,
                    date: s.date,
                    description: if s.description.trim().is_empty() {
                        format!(
                            "{}（{}）",
                            c.title,
                            if c.side == Side::Receivable {
                                "回収"
                            } else {
                                "返済"
                            }
                        )
                    } else {
                        s.description.clone()
                    },
                    partner: c.partner,
                    source: format!("settle:{}:{seq}", c.id),
                    void: false,
                    lines,
                })?;
                if s.principal == left {
                    self.claims
                        .iter_mut()
                        .find(|x| x.id == c.id)
                        .unwrap()
                        .closed = true;
                }
                Some(id)
            }
            Change::SaveBudget { budget } => {
                Some(save!(self, budgets, budget.clone(), "budget", "予算"))
            }
            Change::SavePlan { plan } => Some(save!(self, plans, plan.clone(), "plan", "計画")),
            Change::PostPlan {
                plan,
                occurrence,
                date,
                amount,
            } => {
                let p = self
                    .plans
                    .iter()
                    .find(|p| p.id == *plan)
                    .ok_or_else(|| invalid("計画がありません"))?
                    .clone();
                if !plans::occurrences(&p, *occurrence, *occurrence).contains(occurrence) {
                    return Err(invalid("指定日は計画の予定日ではありません"));
                }
                let amount = amount.unwrap_or(p.amount);
                self.money(amount, "金額")?;
                if amount <= Decimal::ZERO {
                    return Err(invalid("金額は正の値にしてください"));
                }
                Some(self.push_entry(Entry {
                    id: 0,
                    date: date.unwrap_or(*occurrence),
                    description: p.name.clone(),
                    partner: p.partner,
                    source: plans::source(p.id, *occurrence),
                    void: false,
                    lines: vec![
                        line(p.debit, amount, Decimal::ZERO),
                        line(p.credit, Decimal::ZERO, amount),
                    ],
                })?)
            }
            Change::SaveMaterial { material } => Some(save!(
                self,
                materials,
                material.clone(),
                "material",
                "原材料"
            )),
            Change::SaveMaterialMove { material_move } => Some(save!(
                self,
                material_moves,
                material_move.clone(),
                "material_move",
                "入出庫"
            )),
            Change::SaveEquipment { equipment } => Some(save!(
                self,
                equipment,
                equipment.clone(),
                "equipment",
                "設備"
            )),
            Change::PostDepreciation {
                equipment,
                fiscal_year,
                date,
            } => {
                let e = self
                    .equipment
                    .iter()
                    .find(|e| e.id == *equipment)
                    .ok_or_else(|| invalid("設備がありません"))?
                    .clone();
                let amount = cost::depreciation(self, &e, *fiscal_year)?;
                if amount.is_zero() {
                    return Err(invalid("この年度の償却額はありません"));
                }
                let (start, end) = cost::fiscal_range(self, *fiscal_year)?;
                if *date < start || *date > end {
                    return Err(invalid("計上日は対象年度内にしてください"));
                }
                let dep = self.role(Role::Depreciation)?.id;
                let acc = self.role(Role::AccumulatedDepreciation)?.id;
                Some(self.push_entry(Entry {
                    id: 0,
                    date: *date,
                    description: format!("{} 減価償却（{}年度）", e.name, fiscal_year),
                    partner: None,
                    source: cost::depreciation_source(e.id, *fiscal_year),
                    void: false,
                    lines: vec![
                        line(dep, amount, Decimal::ZERO),
                        line(acc, Decimal::ZERO, amount),
                    ],
                })?)
            }
            Change::SaveProduct { product } => {
                Some(save!(self, products, product.clone(), "product", "製品"))
            }
            Change::SaveProduction { production } => Some(save!(
                self,
                productions,
                production.clone(),
                "production",
                "製造実績"
            )),
            Change::Remove { target, id } => {
                self.remove(*target, *id)?;
                None
            }
        };
        Ok(id)
    }

    fn remove(&mut self, target: Target, id: i64) -> Result<()> {
        let used = |cond: bool, what: &str| -> Result<()> {
            if cond {
                Err(invalid(format!(
                    "{what}で使われているため削除できません。使用停止にしてください"
                )))
            } else {
                Ok(())
            }
        };
        fn take<T>(list: &mut Vec<T>, id: i64, get: impl Fn(&T) -> i64) -> Result<()> {
            let n = list.len();
            list.retain(|x| get(x) != id);
            if n == list.len() {
                return Err(invalid("対象がありません"));
            }
            Ok(())
        }
        match target {
            Target::Account => {
                used(
                    self.entries
                        .iter()
                        .any(|e| e.lines.iter().any(|l| l.account == id)),
                    "仕訳",
                )?;
                used(self.claims.iter().any(|c| c.account == id), "債権債務")?;
                used(
                    self.plans.iter().any(|p| p.debit == id || p.credit == id),
                    "計画",
                )?;
                used(
                    self.budgets
                        .iter()
                        .any(|b| b.lines.iter().any(|l| l.account == id)),
                    "予算",
                )?;
                used(self.accounts.iter().any(|a| a.parent == Some(id)), "子科目")?;
                take(&mut self.accounts, id, |x| x.id)
            }
            Target::Partner => {
                used(
                    self.entries.iter().any(|e| e.partner == Some(id))
                        || self.claims.iter().any(|c| c.partner == Some(id))
                        || self.plans.iter().any(|p| p.partner == Some(id)),
                    "仕訳・債権債務・計画",
                )?;
                take(&mut self.partners, id, |x| x.id)
            }
            Target::Claim => {
                used(
                    self.entries
                        .iter()
                        .any(|e| e.lines.iter().any(|l| l.claim == Some(id))),
                    "仕訳",
                )?;
                take(&mut self.claims, id, |x| x.id)
            }
            Target::Budget => take(&mut self.budgets, id, |x| x.id),
            Target::Plan => take(&mut self.plans, id, |x| x.id),
            Target::Material => {
                used(
                    self.material_moves.iter().any(|m| m.material == id)
                        || self
                            .products
                            .iter()
                            .any(|p| p.materials.iter().any(|m| m.material == id))
                        || self
                            .productions
                            .iter()
                            .any(|p| p.materials.iter().any(|m| m.material == id)),
                    "入出庫・製品・製造実績",
                )?;
                take(&mut self.materials, id, |x| x.id)
            }
            Target::MaterialMove => take(&mut self.material_moves, id, |x| x.id),
            Target::Equipment => {
                used(
                    self.products
                        .iter()
                        .any(|p| p.equipment.iter().any(|u| u.equipment == id))
                        || self
                            .productions
                            .iter()
                            .any(|p| p.equipment.iter().any(|u| u.equipment == id)),
                    "製品・製造実績",
                )?;
                take(&mut self.equipment, id, |x| x.id)
            }
            Target::Product => {
                used(self.productions.iter().any(|p| p.product == id), "製造実績")?;
                take(&mut self.products, id, |x| x.id)
            }
            Target::Production => take(&mut self.productions, id, |x| x.id),
        }
    }
}
