//! Append-only plan movements. Cash remains exclusively in the account ledger.
#![allow(dead_code)]
use crate::finance_model::{add_decimal_text, subtract_decimal_text};
use crate::finance_read_model::{BudgetCategorySnapshot, BudgetMonthSnapshot};
use anyhow::{Context, bail};
use chrono::{Months, NaiveDate};
use serde::{Deserialize, Serialize};
use sqlx::{Row, SqliteConnection};
use std::collections::BTreeMap;

pub(crate) const UNALLOCATED: &str = "budget-unallocated";
fn add(a: &str, b: &str) -> anyhow::Result<String> {
    add_decimal_text(a, b).map_err(anyhow::Error::msg)
}
fn sub(a: &str, b: &str) -> anyhow::Result<String> {
    subtract_decimal_text(a, b).map_err(anyhow::Error::msg)
}
fn positive(a: &str) -> String {
    if a.starts_with('-') {
        "0".into()
    } else {
        a.into()
    }
}
fn next(month: &str) -> anyhow::Result<String> {
    Ok(
        NaiveDate::parse_from_str(&format!("{month}-01"), "%Y-%m-%d")?
            .checked_add_months(Months::new(1))
            .context("Month overflow")?
            .format("%Y-%m")
            .to_string(),
    )
}
async fn rates(
    c: &mut SqliteConnection,
) -> anyhow::Result<Vec<crate::finance_model::RateObservation>> {
    sqlx::query("SELECT quote_currency,rate_date,rate_decimal FROM exchange_rates WHERE provider='ECB' AND base_currency='EUR' AND rate_date IS NOT NULL AND rate_decimal IS NOT NULL ORDER BY rate_date,id")
        .fetch_all(&mut *c).await?.into_iter().map(|r| Ok(crate::finance_model::RateObservation {
            currency:r.try_get("quote_currency")?, date:r.try_get("rate_date")?, currency_units_per_eur:r.try_get("rate_decimal")?
        })).collect()
}
pub(crate) async fn start(c: &mut SqliteConnection) -> anyhow::Result<Option<String>> {
    let exists:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='budget_ledger_state')").fetch_one(&mut *c).await?;
    if !exists {
        return Ok(None);
    }
    Ok(
        sqlx::query_scalar("SELECT start_month FROM budget_ledger_state WHERE singleton=1")
            .fetch_optional(&mut *c)
            .await?,
    )
}
/// Activate once in the current month, preserving existing budget results.
pub(crate) async fn activate(c: &mut SqliteConnection) -> anyhow::Result<()> {
    if start(c).await?.is_some() || crate::budget::first_month(c).await?.is_none() {
        return Ok(());
    }
    let month = crate::budget::today()[..7].to_owned();
    let categories = crate::budget::categories(c, &month).await?;
    let mut remaining = "0".to_owned();
    let mut allocations = "0".to_owned();
    for category in &categories {
        remaining = add(
            &remaining,
            category
                .remaining_eur
                .as_deref()
                .context("Missing category opening")?,
        )?;
        allocations = add(
            &allocations,
            category
                .monthly_eur
                .as_deref()
                .context("Missing allocation")?,
        )?;
    }
    let cash = cash(c, &crate::budget::today()).await?;
    sqlx::query("INSERT INTO finance_categories(id,name,is_real,source_row,source_row_json) VALUES (?,'Unallocated',1,1000000,'[]')")
        .bind(UNALLOCATED).execute(&mut *c).await?;
    // Mark pre-activation receipts explicitly so their existing category effect
    // survives migration. Do not fabricate historical expectations.
    sqlx::query("INSERT INTO budget_income_receipts(transaction_id,treatment) SELECT id,'legacy' FROM finance_transactions WHERE kind='income'")
        .execute(&mut *c).await?;
    sqlx::query("INSERT INTO budget_ledger_state VALUES(1,?,'0',?,strftime('%Y-%m-%dT%H:%M:%fZ','now'))")
        .bind(&month).bind(serde_json::json!({"categories":categories,"cashEur":cash,"policy":"preserve-existing-category-results"}).to_string()).execute(&mut *c).await?;
    ensure_income(c, &month).await?;
    sqlx::query("UPDATE budget_income_receipts SET expected_month=? WHERE treatment='legacy' AND transaction_id IN(SELECT id FROM finance_transactions WHERE substr(effective_date,1,7)=?)")
        .bind(&month).bind(&month).execute(&mut *c).await?;
    let income = income(c, &month, &month)
        .await?
        .context("Missing current expectation")?;
    // Initial Unallocated is the current coverage surplus plus outstanding
    // forecast funding. Offset recurring funding/allocation movements once.
    let opening = sub(
        &add(&sub(&cash, &remaining)?, &income.outstanding_eur)?,
        &sub(&income.amount_eur, &allocations)?,
    )?;
    sqlx::query("UPDATE budget_ledger_state SET opening_unallocated_eur=? WHERE singleton=1")
        .bind(&opening)
        .execute(&mut *c)
        .await?;
    sync(c, None).await?;
    Ok(())
}
async fn ensure_income(c: &mut SqliteConnection, month: &str) -> anyhow::Result<()> {
    let exists: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM budget_plan_income WHERE month=?)")
            .bind(month)
            .fetch_one(&mut *c)
            .await?;
    if exists {
        return Ok(());
    }
    let row=sqlx::query("SELECT amount,currency,arrival_day FROM budget_income_expectations WHERE effective_start_month<=? ORDER BY effective_start_month DESC LIMIT 1").bind(month).fetch_optional(&mut *c).await?;
    let (amount, currency, day) = match row {
        Some(r) => (
            r.try_get::<String, _>("amount")?,
            r.try_get::<String, _>("currency")?,
            r.try_get::<i64, _>("arrival_day")?,
        ),
        None => ("10000".into(), "USD".into(), 7),
    };
    let date = format!("{month}-01");
    let eur =
        crate::finance_model::account_balance_eur(&amount, &currency, &date, &rates(c).await?)
            .map_err(anyhow::Error::msg)?;
    sqlx::query("INSERT INTO budget_plan_income(month,amount,currency,amount_eur,arrival_day) VALUES(?,?,?,?,?)").bind(month).bind(amount).bind(currency).bind(eur).bind(day).execute(&mut *c).await?;
    Ok(())
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ExpectedIncome {
    pub month: String,
    pub amount: String,
    pub currency: String,
    pub amount_eur: String,
    pub received_eur: String,
    pub received_amount: String,
    pub outstanding_eur: String,
    pub arrival_day: i64,
    pub completed_month: Option<String>,
}
struct MatchedReceipt {
    date: String,
    treatment: String,
    actual_eur: String,
    native: String,
}
async fn matched_receipts(
    c: &mut SqliteConnection,
    month: &str,
    through: &str,
    amount: &str,
    currency: &str,
    planned_eur: &str,
) -> anyhow::Result<Vec<MatchedReceipt>> {
    let rows=sqlx::query("SELECT t.effective_date,t.amount_decimal,t.input_currency,a.currency AS account_currency,v.amount_native,v.amount_eur,r.treatment FROM budget_income_receipts r JOIN finance_transactions t ON t.id=r.transaction_id JOIN finance_accounts a ON a.id=t.to_account_id JOIN transaction_valuations v ON v.transaction_id=t.id AND v.side='to' AND v.revision=(SELECT MAX(revision) FROM transaction_valuations WHERE transaction_id=t.id AND side='to') WHERE r.expected_month=? AND t.kind='income' AND substr(t.effective_date,1,7)<=? AND r.treatment IN ('salary','legacy') ORDER BY t.effective_date,t.id")
        .bind(month).bind(through).fetch_all(&mut *c).await?;
    rows.into_iter()
        .map(|r| {
            let actual_eur: String = r.try_get("amount_eur")?;
            let input: Option<String> = r.try_get("input_currency")?;
            let account: Option<String> = r.try_get("account_currency")?;
            let native = if input.as_deref() == Some(currency) {
                r.try_get("amount_decimal")?
            } else if account.as_deref() == Some(currency) {
                r.try_get("amount_native")?
            } else if planned_eur == "0" {
                "0".into()
            } else {
                crate::finance_model::proportional_decimal_text(&actual_eur, amount, planned_eur)
                    .map_err(anyhow::Error::msg)?
            };
            Ok(MatchedReceipt {
                date: r.try_get("effective_date")?,
                treatment: r.try_get("treatment")?,
                actual_eur,
                native,
            })
        })
        .collect()
}
fn funding_portion(native: &str, amount: &str, eur: &str) -> anyhow::Result<String> {
    if amount == "0" {
        return Ok("0".into());
    }
    crate::finance_model::proportional_decimal_text(native, eur, amount).map_err(anyhow::Error::msg)
}
pub(crate) async fn income(
    c: &mut SqliteConnection,
    month: &str,
    through: &str,
) -> anyhow::Result<Option<ExpectedIncome>> {
    let Some(row) = sqlx::query("SELECT * FROM budget_plan_income WHERE month=?")
        .bind(month)
        .fetch_optional(&mut *c)
        .await?
    else {
        return Ok(None);
    };
    let amount: String = row.try_get("amount")?;
    let currency: String = row.try_get("currency")?;
    let amount_eur: String = row.try_get("amount_eur")?;
    let mut received = "0".to_owned();
    let mut received_amount = "0".to_owned();
    for receipt in matched_receipts(c, month, through, &amount, &currency, &amount_eur).await? {
        received = add(&received, &receipt.actual_eur)?;
        received_amount = add(&received_amount, &receipt.native)?;
    }
    let completed_month: Option<String> = row.try_get("completed_month")?;
    let outstanding = if completed_month.as_deref().is_some_and(|m| m <= through) {
        "0".into()
    } else {
        funding_portion(
            &positive(&sub(&amount, &received_amount)?),
            &amount,
            &amount_eur,
        )?
    };
    Ok(Some(ExpectedIncome {
        month: month.into(),
        amount,
        currency,
        amount_eur,
        received_eur: received,
        received_amount,
        outstanding_eur: outstanding,
        arrival_day: row.try_get("arrival_day")?,
        completed_month,
    }))
}
/// Only current-month salary is matched automatically. Older outstanding
/// expectations are selected explicitly by the user.
pub(crate) async fn receipt(
    c: &mut SqliteConnection,
    id: &str,
    treatment: Option<&str>,
    expected: Option<&str>,
) -> anyhow::Result<()> {
    if start(c).await?.is_none() {
        return Ok(());
    }
    let row = sqlx::query("SELECT kind,effective_date FROM finance_transactions WHERE id=?")
        .bind(id)
        .fetch_one(&mut *c)
        .await?;
    if row.try_get::<String, _>("kind")? != "income" {
        if treatment.is_some() || expected.is_some() {
            bail!("Income options require an income transaction.");
        }
        sqlx::query(
            "DELETE FROM budget_income_receipts WHERE transaction_id=? AND treatment<>'legacy'",
        )
        .bind(id)
        .execute(&mut *c)
        .await?;
        return Ok(());
    }
    let date: String = row.try_get("effective_date")?;
    let month = &date[..7];
    let old = sqlx::query(
        "SELECT treatment,expected_month FROM budget_income_receipts WHERE transaction_id=?",
    )
    .bind(id)
    .fetch_optional(&mut *c)
    .await?;
    if treatment.is_none() && expected.is_none() && old.is_some() {
        return Ok(());
    }
    let treatment = treatment.unwrap_or("salary");
    if !matches!(treatment, "salary" | "unexpected" | "refund") {
        bail!("Choose salary, unexpected income, or refund.");
    }
    let matching = if treatment == "salary" {
        let target = expected.unwrap_or(month);
        if target > month || start(c).await?.as_deref().is_none_or(|s| target < s) {
            bail!("Choose an existing expected-income month on or before the receipt month.");
        }
        ensure_income(c, target).await?;
        let expectation = income(c, target, month)
            .await?
            .context("Expected income unavailable")?;
        let same_match = old.as_ref().is_some_and(|row| {
            row.try_get::<String, _>("treatment").ok().as_deref() == Some("salary")
                && row
                    .try_get::<Option<String>, _>("expected_month")
                    .ok()
                    .flatten()
                    .as_deref()
                    == Some(target)
        });
        if expectation.completed_month.is_some() && !same_match {
            bail!(
                "This expectation is already complete; record extra income as unexpected income."
            );
        }
        Some(target)
    } else {
        if expected.is_some() {
            bail!("Only salary can settle expected income.");
        }
        None
    };
    sqlx::query("INSERT INTO budget_income_receipts VALUES(?,?,?) ON CONFLICT(transaction_id) DO UPDATE SET treatment=excluded.treatment,expected_month=excluded.expected_month")
        .bind(id).bind(treatment).bind(matching).execute(&mut *c).await?;
    Ok(())
}
// A source's desired effect is reconciled with prior movements using appended
// differences. Edits and deletions never erase the original plan movements.
async fn movement(
    c: &mut SqliteConnection,
    month: &str,
    category: &str,
    kind: &str,
    key: &str,
    wanted: &str,
    operation: Option<&str>,
) -> anyhow::Result<()> {
    let amounts=sqlx::query_scalar::<_,String>("SELECT amount_eur FROM budget_ledger_movements WHERE month=? AND category_id=? AND kind=? AND source_key=?").bind(month).bind(category).bind(kind).bind(key).fetch_all(&mut *c).await?;
    let mut prior = "0".to_owned();
    for amount in amounts {
        prior = add(&prior, &amount)?;
    }
    let delta = sub(wanted, &prior)?;
    if delta != "0" {
        sqlx::query("INSERT INTO budget_ledger_movements(month,category_id,kind,source_key,amount_eur,operation_id) VALUES(?,?,?,?,?,?)").bind(month).bind(category).bind(kind).bind(key).bind(delta).bind(operation).execute(&mut *c).await?;
    }
    Ok(())
}
pub(crate) async fn sync(c: &mut SqliteConnection, operation: Option<&str>) -> anyhow::Result<()> {
    let Some(first) = start(c).await? else {
        return Ok(());
    };
    let today = crate::budget::today();
    let current = &today[..7];
    let mut cursor = first;
    while cursor.as_str() <= current {
        let closed: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM budget_month_closures WHERE month=?)")
                .bind(&cursor)
                .fetch_one(&mut *c)
                .await?;
        if !closed {
            ensure_income(c, &cursor).await?;
            let planned: String =
                sqlx::query_scalar("SELECT amount_eur FROM budget_plan_income WHERE month=?")
                    .bind(&cursor)
                    .fetch_one(&mut *c)
                    .await?;
            movement(
                c,
                &cursor,
                UNALLOCATED,
                "funding",
                &format!("expected:{cursor}"),
                &planned,
                operation,
            )
            .await?;
            let allocations=sqlx::query("SELECT c.id,(SELECT p.base_monthly_eur FROM category_budget_periods p WHERE p.category_id=c.id AND p.effective_start_month<=? ORDER BY p.effective_start_month DESC LIMIT 1) AS amount FROM finance_categories c WHERE c.is_real=1 AND c.id<>?").bind(&cursor).bind(UNALLOCATED).fetch_all(&mut *c).await?;
            let mut total = "0".to_owned();
            for r in allocations {
                let id: String = r.try_get("id")?;
                let amount: Option<String> = r.try_get("amount")?;
                let amount = amount.context("Missing allocation")?;
                total = add(&total, &amount)?;
                movement(
                    c,
                    &cursor,
                    &id,
                    "allocation",
                    &format!("allocation:{cursor}:{id}"),
                    &amount,
                    operation,
                )
                .await?;
            }
            movement(
                c,
                &cursor,
                UNALLOCATED,
                "allocation",
                &format!("allocations:{cursor}"),
                &sub("0", &total)?,
                operation,
            )
            .await?;
            sync_transactions(c, &cursor, operation).await?;
            let corrections=sqlx::query("SELECT i.category_id,i.expense_delta_eur,correction.operation_id FROM finance_category_correction_items i JOIN finance_category_corrections correction ON correction.operation_id=i.operation_id WHERE correction.applied_month=?").bind(&cursor).fetch_all(&mut *c).await?;
            for r in corrections {
                let id: String = r.try_get("category_id")?;
                let key: String = r.try_get("operation_id")?;
                let amount: String = r.try_get("expense_delta_eur")?;
                movement(
                    c,
                    &cursor,
                    &id,
                    "correction",
                    &format!("correction:{key}:{id}"),
                    &sub("0", &amount)?,
                    operation,
                )
                .await?;
            }
        }
        cursor = next(&cursor)?;
    }
    // Excess follows the month where it is received. Completing a short
    // expectation records its funding reduction in the completion month.
    let months =
        sqlx::query_scalar::<_, String>("SELECT month FROM budget_plan_income ORDER BY month")
            .fetch_all(&mut *c)
            .await?;
    for month in months {
        let mut received = "0".to_owned();
        let mut wanted = BTreeMap::<String, String>::new();
        let expectation =
            sqlx::query("SELECT amount,currency,amount_eur FROM budget_plan_income WHERE month=?")
                .bind(&month)
                .fetch_one(&mut *c)
                .await?;
        let planned: String = expectation.try_get("amount_eur")?;
        let native_plan: String = expectation.try_get("amount")?;
        let currency: String = expectation.try_get("currency")?;
        let rows = matched_receipts(c, &month, current, &native_plan, &currency, &planned).await?;
        for receipt in rows {
            let before = sub(&native_plan, &positive(&sub(&native_plan, &received)?))?;
            received = add(&received, &receipt.native)?;
            let after = sub(&native_plan, &positive(&sub(&native_plan, &received)?))?;
            let covered = sub(
                &funding_portion(&after, &native_plan, &planned)?,
                &funding_portion(&before, &native_plan, &planned)?,
            )?;
            let difference = sub(&receipt.actual_eur, &covered)?;
            if receipt.treatment != "legacy" {
                let m = receipt.date[..7].to_owned();
                let previous = wanted.get(&m).cloned().unwrap_or("0".into());
                wanted.insert(m, add(&previous, &difference)?);
            }
        }
        let existing = sqlx::query_scalar::<_, String>(
            "SELECT DISTINCT month FROM budget_ledger_movements WHERE source_key=?",
        )
        .bind(format!("excess:{month}"))
        .fetch_all(&mut *c)
        .await?;
        for m in existing {
            wanted.entry(m).or_insert("0".into());
        }
        for (m, value) in wanted {
            let closed: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM budget_month_closures WHERE month=?)",
            )
            .bind(&m)
            .fetch_one(&mut *c)
            .await?;
            if !closed {
                movement(
                    c,
                    &m,
                    UNALLOCATED,
                    "income",
                    &format!("excess:{month}"),
                    &value,
                    operation,
                )
                .await?;
            }
        }
        let completion = sqlx::query(
            "SELECT completed_month,completion_shortfall_eur FROM budget_plan_income WHERE month=?",
        )
        .bind(&month)
        .fetch_one(&mut *c)
        .await?;
        if let Some(m) = completion.try_get::<Option<String>, _>("completed_month")? {
            let shortfall: String = completion
                .try_get::<Option<String>, _>("completion_shortfall_eur")?
                .context("Missing completion amount")?;
            let closed: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM budget_month_closures WHERE month=?)",
            )
            .bind(&m)
            .fetch_one(&mut *c)
            .await?;
            if !closed {
                movement(
                    c,
                    &m,
                    UNALLOCATED,
                    "settlement",
                    &format!("complete:{month}"),
                    &sub("0", &shortfall)?,
                    operation,
                )
                .await?;
            }
        }
    }
    Ok(())
}
async fn sync_transactions(
    c: &mut SqliteConnection,
    month: &str,
    operation: Option<&str>,
) -> anyhow::Result<()> {
    let mut wanted = BTreeMap::<(String, String, String), String>::new();
    let rows=sqlx::query("SELECT t.id,t.kind,t.amount_decimal,t.from_category_id,t.to_category_id,r.treatment,vf.amount_eur AS from_eur,vt.amount_eur AS to_eur FROM finance_transactions t LEFT JOIN budget_income_receipts r ON r.transaction_id=t.id LEFT JOIN transaction_valuations vf ON vf.transaction_id=t.id AND vf.side='from' AND vf.revision=(SELECT MAX(revision) FROM transaction_valuations WHERE transaction_id=t.id AND side='from') LEFT JOIN transaction_valuations vt ON vt.transaction_id=t.id AND vt.side='to' AND vt.revision=(SELECT MAX(revision) FROM transaction_valuations WHERE transaction_id=t.id AND side='to') WHERE substr(t.effective_date,1,7)=?").bind(month).fetch_all(&mut *c).await?;
    let real = sqlx::query_scalar::<_, String>("SELECT id FROM finance_categories WHERE is_real=1")
        .fetch_all(&mut *c)
        .await?;
    for r in rows {
        let id: String = r.try_get("id")?;
        let kind: String = r.try_get("kind")?;
        let treatment: Option<String> = r.try_get("treatment")?;
        if kind == "transfer" || kind == "reconciliation" {
            continue;
        }
        if kind == "income" && !matches!(treatment.as_deref(), Some("refund" | "legacy")) {
            if treatment.as_deref() == Some("unexpected") || treatment.is_none() {
                let amount: Option<String> = r.try_get("to_eur")?;
                wanted.insert(
                    (
                        UNALLOCATED.into(),
                        "income".into(),
                        format!("transaction:{id}"),
                    ),
                    amount.context("Missing income valuation")?,
                );
            }
            continue;
        }
        for (column, value, incoming) in [
            ("from_category_id", "from_eur", false),
            ("to_category_id", "to_eur", true),
        ] {
            let category: Option<String> = r.try_get(column)?;
            let Some(category) = category.filter(|c| real.contains(c)) else {
                continue;
            };
            let amount = if kind == "reallocate" {
                r.try_get::<String, _>("amount_decimal")?
                    .trim_start_matches('-')
                    .to_owned()
            } else {
                r.try_get::<Option<String>, _>(value)?
                    .context("Missing category valuation")?
            };
            let effect = if incoming { amount } else { sub("0", &amount)? };
            let movement_kind = if kind == "reallocate" {
                "reallocation"
            } else {
                "spending"
            };
            let key = (category, movement_kind.into(), format!("transaction:{id}"));
            let previous = wanted.get(&key).cloned().unwrap_or("0".into());
            wanted.insert(key, add(&previous, &effect)?);
        }
    }
    let rows=sqlx::query("SELECT DISTINCT category_id,kind,source_key FROM budget_ledger_movements WHERE month=? AND source_key LIKE 'transaction:%'").bind(month).fetch_all(&mut *c).await?;
    for r in rows {
        wanted
            .entry((
                r.try_get("category_id")?,
                r.try_get("kind")?,
                r.try_get("source_key")?,
            ))
            .or_insert("0".into());
    }
    for ((category, kind, key), amount) in wanted {
        movement(c, month, &category, &kind, &key, &amount, operation).await?;
    }
    Ok(())
}
/// Reconstruct category components from movements, preserving earlier history.
pub(crate) async fn apply(
    c: &mut SqliteConnection,
    history: &mut [BudgetMonthSnapshot],
) -> anyhow::Result<()> {
    let Some(first) = start(c).await? else {
        return Ok(());
    };
    let mut unallocated: String = sqlx::query_scalar(
        "SELECT opening_unallocated_eur FROM budget_ledger_state WHERE singleton=1",
    )
    .fetch_one(&mut *c)
    .await?;
    for month in history {
        if month.month < first {
            continue;
        }
        if month.status == "closed" {
            if let Some(u) = month
                .categories
                .iter()
                .find(|c| c.category_id == UNALLOCATED)
            {
                unallocated = u
                    .remaining_eur
                    .clone()
                    .context("Missing frozen Unallocated")?;
            }
            continue;
        }
        let carry = unallocated.clone();
        let rows=sqlx::query("SELECT category_id,kind,amount_eur FROM budget_ledger_movements WHERE month=? ORDER BY id").bind(&month.month).fetch_all(&mut *c).await?;
        let mut components = BTreeMap::<(String, String), String>::new();
        for r in rows {
            let key = (
                r.try_get::<String, _>("category_id")?,
                r.try_get::<String, _>("kind")?,
            );
            let value: String = r.try_get("amount_eur")?;
            let prior = components.get(&key).cloned().unwrap_or("0".into());
            components.insert(key, add(&prior, &value)?);
        }
        if !components.contains_key(&(UNALLOCATED.into(), "funding".into())) {
            let row=sqlx::query("SELECT amount,currency FROM budget_income_expectations WHERE effective_start_month<=? ORDER BY effective_start_month DESC LIMIT 1").bind(&month.month).fetch_optional(&mut *c).await?;
            let (amount, currency) = match row {
                Some(r) => (
                    r.try_get::<String, _>("amount")?,
                    r.try_get::<String, _>("currency")?,
                ),
                None => ("10000".into(), "USD".into()),
            };
            let eur = crate::finance_model::account_balance_eur(
                &amount,
                &currency,
                &format!("{}-01", month.month),
                &rates(c).await?,
            )
            .map_err(anyhow::Error::msg)?;
            let mut allocated = "0".to_owned();
            for category in &month.categories {
                allocated = add(
                    &allocated,
                    category
                        .monthly_eur
                        .as_deref()
                        .context("Missing allocation")?,
                )?;
            }
            components.insert((UNALLOCATED.into(), "funding".into()), eur);
            components.insert(
                (UNALLOCATED.into(), "allocation".into()),
                sub("0", &allocated)?,
            );
        }
        let get = |kind: &str| {
            components
                .get(&(UNALLOCATED.into(), kind.into()))
                .cloned()
                .unwrap_or("0".into())
        };
        let mut delta = "0".to_owned();
        for ((id, _), value) in &components {
            if id == UNALLOCATED {
                delta = add(&delta, value)?;
            }
        }
        unallocated = add(&unallocated, &delta)?;
        month.categories.push(BudgetCategorySnapshot {
            category_id: UNALLOCATED.into(),
            name: "Unallocated".into(),
            monthly_eur: Some("0".into()),
            expense_eur: Some("0".into()),
            remaining_eur: Some(unallocated.clone()),
            reallocation_impact_eur: Some(sub("0", &get("reallocation"))?),
            carryover_eur: Some(carry),
            adjustment_eur: Some(delta),
            available_eur: Some(unallocated.clone()),
            net_spending_eur: Some("0".into()),
            correction_eur: Some("0".into()),
        });
    }
    Ok(())
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Coverage {
    pub actual_cash_eur: String,
    pub assigned_remaining_eur: String,
    pub unallocated_eur: String,
    pub outstanding_expected_eur: String,
    pub current_gap_eur: String,
    pub projected_gap_eur: String,
    pub expectations: Vec<ExpectedIncome>,
    pub frozen: bool,
}
pub(crate) async fn cash(c: &mut SqliteConnection, date: &str) -> anyhow::Result<String> {
    let rates = rates(c).await?;
    let accounts=sqlx::query("SELECT id,currency,current_native_balance FROM finance_accounts WHERE is_real=1 AND is_active=1").fetch_all(&mut *c).await?;
    let mut total = "0".to_owned();
    for a in accounts {
        let id: String = a.try_get("id")?;
        let currency: Option<String> = a.try_get("currency")?;
        let balance: Option<String> = a.try_get("current_native_balance")?;
        let mut balance = balance.context("Missing cash account balance")?;
        let activity=sqlx::query("SELECT t.from_account_id,t.to_account_id,vf.amount_native AS from_native,vt.amount_native AS to_native FROM finance_transactions t LEFT JOIN transaction_valuations vf ON vf.transaction_id=t.id AND vf.side='from' AND vf.revision=(SELECT MAX(revision) FROM transaction_valuations WHERE transaction_id=t.id AND side='from') LEFT JOIN transaction_valuations vt ON vt.transaction_id=t.id AND vt.side='to' AND vt.revision=(SELECT MAX(revision) FROM transaction_valuations WHERE transaction_id=t.id AND side='to') WHERE t.effective_date>? AND (t.from_account_id=? OR t.to_account_id=?)").bind(date).bind(&id).bind(&id).fetch_all(&mut *c).await?;
        for r in activity {
            if r.try_get::<Option<String>, _>("from_account_id")?
                .as_deref()
                == Some(&id)
            {
                balance = add(
                    &balance,
                    &r.try_get::<Option<String>, _>("from_native")?
                        .context("Missing account valuation")?,
                )?;
            }
            if r.try_get::<Option<String>, _>("to_account_id")?.as_deref() == Some(&id) {
                balance = sub(
                    &balance,
                    &r.try_get::<Option<String>, _>("to_native")?
                        .context("Missing account valuation")?,
                )?;
            }
        }
        let value = crate::finance_model::account_balance_eur(
            &balance,
            currency.as_deref().unwrap_or("EUR"),
            date,
            &rates,
        )
        .map_err(anyhow::Error::msg)?;
        total = add(&total, &value)?;
    }
    Ok(total)
}
pub(crate) async fn coverage(
    c: &mut SqliteConnection,
    month: &str,
    categories: &[BudgetCategorySnapshot],
) -> anyhow::Result<Option<Coverage>> {
    if start(c).await?.as_deref().is_none_or(|s| month < s) {
        return Ok(None);
    }
    let frozen: Option<String> =
        sqlx::query_scalar("SELECT snapshot_json FROM budget_coverage_closures WHERE month=?")
            .bind(month)
            .fetch_optional(&mut *c)
            .await?;
    if let Some(frozen) = frozen {
        let mut result: Coverage = serde_json::from_str(&frozen)?;
        result.frozen = true;
        return Ok(Some(result));
    }
    let today = crate::budget::today();
    let date = if month == &today[..7] {
        today
    } else {
        NaiveDate::parse_from_str(&format!("{}-01", next(month)?), "%Y-%m-%d")?
            .pred_opt()
            .context("Date overflow")?
            .to_string()
    };
    let actual = cash(c, &date).await?;
    let mut assigned = "0".to_owned();
    let mut unallocated = "0".to_owned();
    for category in categories {
        let balance = category
            .remaining_eur
            .as_deref()
            .context("Missing remaining category budget")?;
        if category.category_id == UNALLOCATED {
            unallocated = balance.into();
        } else {
            assigned = add(&assigned, balance)?;
        }
    }
    let months = sqlx::query_scalar::<_, String>(
        "SELECT month FROM budget_plan_income WHERE month<=? ORDER BY month",
    )
    .bind(month)
    .fetch_all(&mut *c)
    .await?;
    let mut outstanding = "0".to_owned();
    let mut expectations = Vec::new();
    for m in months {
        let i = income(c, &m, month).await?.context("Missing expectation")?;
        outstanding = add(&outstanding, &i.outstanding_eur)?;
        expectations.push(i);
    }
    // Coverage asks whether assigned category budgets are backed by cash.
    // Unallocated is shown separately, so a negative bucket cannot hide a gap.
    let gap = sub(&actual, &assigned)?;
    Ok(Some(Coverage {
        actual_cash_eur: actual,
        assigned_remaining_eur: assigned,
        unallocated_eur: unallocated,
        outstanding_expected_eur: outstanding.clone(),
        projected_gap_eur: add(&gap, &outstanding)?,
        current_gap_eur: gap,
        expectations,
        frozen: false,
    }))
}
