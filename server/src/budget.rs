//! Open budgets are calculated from facts. Only historical and closed results
//! are read from snapshots; imported open snapshots are provenance, not balances.
#![allow(dead_code)]
use crate::finance_model::{add_decimal_text as add, subtract_decimal_text as sub};
use crate::finance_read_model::{BudgetCategorySnapshot, BudgetMonthSnapshot};
use anyhow::{Context, bail};
use chrono::{Datelike, Months, NaiveDate, Utc};
use sqlx::{Row, SqliteConnection};
use std::collections::BTreeMap;

pub(crate) fn today() -> String {
    household_date(Utc::now())
}
fn household_date(instant: chrono::DateTime<Utc>) -> String {
    instant
        .with_timezone(&chrono_tz::Europe::Madrid)
        .format("%Y-%m-%d")
        .to_string()
}
fn month_date(month: &str) -> anyhow::Result<NaiveDate> {
    Ok(NaiveDate::parse_from_str(
        &format!("{month}-01"),
        "%Y-%m-%d",
    )?)
}
fn next_month(month: &str) -> anyhow::Result<String> {
    Ok(month_date(month)?
        .checked_add_months(Months::new(1))
        .context("Month overflow")?
        .format("%Y-%m")
        .to_string())
}
pub(crate) fn previous_month_end(date: &str) -> anyhow::Result<String> {
    let date = NaiveDate::parse_from_str(date, "%Y-%m-%d")?;
    Ok(date
        .with_day(1)
        .context("Invalid month")?
        .pred_opt()
        .context("Date overflow")?
        .to_string())
}
pub(crate) async fn first_month(c: &mut SqliteConnection) -> anyhow::Result<Option<String>> {
    let manifests = sqlx::query_scalar::<_, String>(
        "SELECT manifest_json FROM finance_import_runs ORDER BY committed_at",
    )
    .fetch_all(&mut *c)
    .await?;
    let mut first = None;
    for manifest in manifests {
        let value: serde_json::Value = serde_json::from_str(&manifest)?;
        let month = value["currentCanonicalMonth"]
            .as_str()
            .context("Import canonical month missing")?;
        month_date(month)?;
        if first.as_deref().is_none_or(|previous| month < previous) {
            first = Some(month.to_owned());
        }
    }
    Ok(first)
}
/// Seed verified carryover once. No monthly balance or funding is created here.
pub(crate) async fn initialize(c: &mut SqliteConnection) -> anyhow::Result<()> {
    let Some(first) = first_month(c).await? else {
        return Ok(());
    };
    let missing: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM finance_categories category WHERE category.is_real=1 AND category.id<>'budget-unallocated' AND NOT EXISTS(SELECT 1 FROM budget_opening_state opening WHERE opening.category_id=category.id AND opening.effective_before_month=?))")
        .bind(&first).fetch_one(&mut *c).await?;
    if !missing {
        return Ok(());
    }
    let prior = month_date(&first)?
        .checked_sub_months(Months::new(1))
        .context("Month overflow")?
        .format("%Y-%m")
        .to_string();
    let rows = sqlx::query("SELECT s.category_id, s.remaining_eur, s.source_cells_json FROM budget_month_snapshots s JOIN finance_categories category ON category.id=s.category_id WHERE s.month=? AND category.is_real=1")
        .bind(&prior).fetch_all(&mut *c).await?;
    for row in rows {
        let balance: Option<String> = row.try_get("remaining_eur")?;
        let balance = balance.context("Imported opening balance unavailable")?;
        sqlx::query("INSERT INTO budget_opening_state (category_id,effective_before_month,remaining_eur,verified_by,verified_at,source_json) VALUES (?,?,?,'verified-canonical-import',strftime('%Y-%m-%dT%H:%M:%fZ','now'),?) ON CONFLICT DO NOTHING")
            .bind(row.try_get::<String,_>("category_id")?).bind(&first).bind(balance)
            .bind(row.try_get::<String,_>("source_cells_json")?).execute(&mut *c).await?;
    }
    Ok(())
}
pub(crate) async fn require_open(c: &mut SqliteConnection, month: &str) -> anyhow::Result<()> {
    let first = first_month(c)
        .await?
        .context("No canonical budget opening")?;
    month_date(month)?;
    if month < first.as_str() || month > &today()[..7] {
        bail!("Transactions require an open month from {first} through the current Madrid month.");
    }
    let closed: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM budget_month_closures WHERE month=?)")
            .bind(month)
            .fetch_one(&mut *c)
            .await?;
    if closed {
        bail!("This budget month is closed. Record a correction in an open month.");
    }
    Ok(())
}
fn arithmetic(result: Result<String, String>) -> anyhow::Result<String> {
    result.map_err(anyhow::Error::msg)
}

pub(crate) async fn history(
    c: &mut SqliteConnection,
    through: &str,
) -> anyhow::Result<Vec<BudgetMonthSnapshot>> {
    history_at(c, through, &today()[..7]).await
}

pub(crate) async fn history_at(
    c: &mut SqliteConnection,
    through: &str,
    calendar_month: &str,
) -> anyhow::Result<Vec<BudgetMonthSnapshot>> {
    month_date(through)?;
    month_date(calendar_month)?;
    if through > calendar_month {
        bail!("Future budget months are plans, not available balances.");
    }
    let first = first_month(c).await?;
    let categories = sqlx::query(
        "SELECT id,name FROM finance_categories WHERE is_real=1 AND id<>'budget-unallocated' ORDER BY source_row,id",
    )
    .fetch_all(&mut *c)
    .await?;
    let mut months = sqlx::query_scalar::<_, String>(
        "SELECT DISTINCT month FROM budget_month_snapshots WHERE month<=? ORDER BY month",
    )
    .bind(through)
    .fetch_all(&mut *c)
    .await?;
    if let Some(first) = &first {
        let mut cursor = first.clone();
        while cursor.as_str() <= through {
            if !months.contains(&cursor) {
                months.push(cursor.clone());
            }
            cursor = next_month(&cursor)?;
        }
    }
    months.sort();
    // Categories created after a frozen month have no entry in that snapshot.
    // Their verified opening remains the fallback until an open month computes it.
    let mut balances = BTreeMap::<String, String>::new();
    if let Some(first) = &first {
        balances = sqlx::query_as::<_, (String, String)>("SELECT category_id,remaining_eur FROM budget_opening_state WHERE effective_before_month=?")
            .bind(first).fetch_all(&mut *c).await?.into_iter().collect();
    }
    let mut history = Vec::new();
    let mut provisional = false;
    for month in months {
        let dynamic = first.as_ref().is_some_and(|first| &month >= first);
        let closed: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM budget_month_closures WHERE month=?)")
                .bind(&month)
                .fetch_one(&mut *c)
                .await?;
        if closed {
            let frozen: String =
                sqlx::query_scalar("SELECT snapshot_json FROM budget_month_closures WHERE month=?")
                    .bind(&month)
                    .fetch_one(&mut *c)
                    .await?;
            let frozen: Vec<BudgetCategorySnapshot> = serde_json::from_str(&frozen)?;
            for category in &frozen {
                if let Some(remaining) = &category.remaining_eur {
                    balances.insert(category.category_id.clone(), remaining.clone());
                }
            }
            history.push(BudgetMonthSnapshot {
                month,
                categories: frozen,
                status: "closed".to_owned(),
                carryover_provisional: false,
            });
            continue;
        }
        let status = if !dynamic {
            "historical"
        } else if closed {
            "closed"
        } else {
            "open"
        };
        let mut results = Vec::new();
        for category in &categories {
            let id: String = category.try_get("id")?;
            let name: String = category.try_get("name")?;
            let monthly=sqlx::query_scalar::<_,Option<String>>("SELECT base_monthly_eur FROM category_budget_periods WHERE category_id=? AND effective_start_month<=? ORDER BY effective_start_month DESC LIMIT 1")
                .bind(&id).bind(&month).fetch_optional(&mut *c).await?.flatten();
            if !dynamic || closed {
                let row=sqlx::query("SELECT expense_eur,remaining_eur,reallocation_impact_eur FROM budget_month_snapshots WHERE category_id=? AND month=?")
                    .bind(&id).bind(&month).fetch_optional(&mut *c).await?;
                let (expense, remaining, impact) = match row {
                    Some(row) => (
                        row.try_get("expense_eur")?,
                        row.try_get::<Option<String>, _>("remaining_eur")?,
                        row.try_get("reallocation_impact_eur")?,
                    ),
                    None => (None, None, None),
                };
                if let Some(remaining) = &remaining {
                    balances.insert(id.clone(), remaining.clone());
                }
                results.push(BudgetCategorySnapshot {
                    category_id: id,
                    name,
                    monthly_eur: monthly,
                    expense_eur: expense,
                    remaining_eur: remaining,
                    reallocation_impact_eur: impact,
                    carryover_eur: None,
                    adjustment_eur: None,
                    available_eur: None,
                    net_spending_eur: None,
                    correction_eur: None,
                });
                continue;
            }
            let seed=sqlx::query_scalar::<_,String>("SELECT remaining_eur FROM budget_opening_state WHERE category_id=? AND effective_before_month=?")
                .bind(&id).bind(first.as_deref().unwrap()).fetch_optional(&mut *c).await?;
            let carryover = if first.as_ref() == Some(&month) {
                match seed {
                    Some(seed) => Some(seed),
                    None => {
                        let previous = month_date(&month)?
                            .checked_sub_months(Months::new(1))
                            .context("Month overflow")?
                            .format("%Y-%m")
                            .to_string();
                        sqlx::query_scalar::<_,Option<String>>("SELECT remaining_eur FROM budget_month_snapshots WHERE category_id=? AND month=?")
                            .bind(&id).bind(previous).fetch_optional(&mut *c).await?.flatten()
                    }
                }
            } else {
                balances.get(&id).cloned()
            };
            let carryover = carryover
                .with_context(|| format!("Missing opening balance for {name} in {month}"))?;
            let mut monthly = monthly
                .with_context(|| format!("Missing monthly allocation for {name} in {month}"))?;
            let entries=sqlx::query("SELECT t.kind,t.amount_decimal,t.from_category_id,t.to_category_id,vf.amount_eur AS from_eur,vt.amount_eur AS to_eur FROM finance_transactions t LEFT JOIN transaction_valuations vf ON vf.transaction_id=t.id AND vf.side='from' AND vf.revision=(SELECT MAX(v.revision) FROM transaction_valuations v WHERE v.transaction_id=t.id AND v.side='from') LEFT JOIN transaction_valuations vt ON vt.transaction_id=t.id AND vt.side='to' AND vt.revision=(SELECT MAX(v.revision) FROM transaction_valuations v WHERE v.transaction_id=t.id AND v.side='to') WHERE substr(t.effective_date,1,7)=? AND (t.from_category_id=? OR t.to_category_id=?)")
                .bind(&month).bind(&id).bind(&id).fetch_all(&mut *c).await?;
            let mut expense = "0".to_owned();
            let mut impact = "0".to_owned();
            for entry in entries {
                let from: Option<String> = entry.try_get("from_category_id")?;
                let to: Option<String> = entry.try_get("to_category_id")?;
                if entry.try_get::<String, _>("kind")? == "reallocate" {
                    let amount: String = entry.try_get("amount_decimal")?;
                    let amount = amount.strip_prefix('-').unwrap_or(&amount);
                    if from.as_deref() == Some(&id) {
                        impact = arithmetic(add(&impact, amount))?;
                    }
                    if to.as_deref() == Some(&id) {
                        impact = arithmetic(sub(&impact, amount))?;
                    }
                } else {
                    for (matches, column, incoming) in [
                        (from.as_deref() == Some(&id), "from_eur", false),
                        (to.as_deref() == Some(&id), "to_eur", true),
                    ] {
                        if matches {
                            let amount: Option<String> = entry.try_get(column)?;
                            let amount = amount.context("Missing transaction EUR valuation")?;
                            expense = arithmetic(if incoming {
                                sub(&expense, &amount)
                            } else {
                                add(&expense, &amount)
                            })?;
                        }
                    }
                }
            }
            if crate::budget_ledger::start(c)
                .await?
                .as_deref()
                .is_some_and(|start| month.as_str() >= start)
            {
                let materialized:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM budget_plan_income WHERE month=?)").bind(&month).fetch_one(&mut *c).await?;
                if materialized {
                    monthly="0".into();
                    let allocations=sqlx::query_scalar::<_,String>("SELECT amount_eur FROM budget_ledger_movements WHERE month=? AND category_id=? AND kind='allocation'").bind(&month).bind(&id).fetch_all(&mut *c).await?;
                    for allocation in allocations { monthly=arithmetic(add(&monthly,&allocation))?; }
                }
                expense = "0".into();
                impact = "0".into();
                let movements = sqlx::query("SELECT kind,amount_eur FROM budget_ledger_movements WHERE month=? AND category_id=? AND kind IN ('spending','reallocation')")
                    .bind(&month).bind(&id).fetch_all(&mut *c).await?;
                for movement in movements {
                    let amount: String = movement.try_get("amount_eur")?;
                    if movement.try_get::<String, _>("kind")? == "spending" {
                        expense = arithmetic(sub(&expense, &amount))?;
                    } else {
                        impact = arithmetic(sub(&impact, &amount))?;
                    }
                }
            }
            let deltas=sqlx::query_scalar::<_,String>("SELECT i.expense_delta_eur FROM finance_category_correction_items i JOIN finance_category_corrections correction ON correction.operation_id=i.operation_id WHERE i.category_id=? AND correction.applied_month=?")
                .bind(&id).bind(&month).fetch_all(&mut *c).await?;
            let mut correction = "0".to_owned();
            for delta in deltas {
                correction = arithmetic(add(&correction, &delta))?;
            }
            // Keep legacy expense totals inclusive of corrections for existing clients.
            let net_spending = expense.clone();
            expense = arithmetic(add(&expense, &correction))?;
            let adjustment =
                arithmetic(sub("0", &impact).and_then(|amount| sub(&amount, &correction)))?;
            let available =
                arithmetic(add(&carryover, &monthly).and_then(|sum| add(&sum, &adjustment)))?;
            let remaining = arithmetic(sub(&available, &net_spending))?;
            balances.insert(id.clone(), remaining.clone());
            results.push(BudgetCategorySnapshot {
                category_id: id,
                name,
                monthly_eur: Some(monthly),
                expense_eur: Some(expense),
                remaining_eur: Some(remaining),
                reallocation_impact_eur: Some(impact),
                carryover_eur: Some(carryover),
                adjustment_eur: Some(adjustment),
                available_eur: Some(available),
                net_spending_eur: Some(net_spending),
                correction_eur: Some(correction),
            });
        }
        history.push(BudgetMonthSnapshot {
            month: month.clone(),
            categories: results,
            status: status.to_owned(),
            carryover_provisional: dynamic && provisional,
        });
        if dynamic && !closed && month.as_str() < calendar_month {
            provisional = true;
        }
    }
    crate::budget_ledger::apply(c, &mut history).await?;
    Ok(history)
}
/// Reconciliation stays valid until a later operation changes account activity
/// on or before its closing date. Receipt order avoids timestamp collisions.
pub(crate) async fn reconciliation_current(
    c: &mut SqliteConnection,
    date: &str,
    batch: &str,
) -> anyhow::Result<bool> {
    let watermark: i64 = sqlx::query_scalar("SELECT operation.rowid FROM reconciliation_batches batch JOIN finance_operations operation ON operation.operation_id=batch.operation_id WHERE batch.id=?")
        .bind(batch).fetch_one(&mut *c).await?;
    let stale: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM finance_transactions t JOIN finance_transaction_revisions r ON r.transaction_id=t.id JOIN finance_operations operation ON operation.operation_id=r.operation_id WHERE t.effective_date<=? AND t.kind<>'reallocate' AND operation.rowid>? AND EXISTS(SELECT 1 FROM finance_accounts a WHERE a.is_real=1 AND a.id IN(t.from_account_id,t.to_account_id))) OR EXISTS(SELECT 1 FROM finance_operations operation WHERE operation.rowid>? AND json_extract(operation.response_json,'$.deleted')=1 AND json_extract(operation.response_json,'$.deletedTransaction.effectiveDate')<=? AND json_extract(operation.response_json,'$.deletedTransaction.kind')<>'reallocate')")
        .bind(date).bind(watermark).bind(watermark).bind(date).fetch_one(&mut *c).await?;
    Ok(!stale)
}

pub(crate) async fn categories(
    c: &mut SqliteConnection,
    month: &str,
) -> anyhow::Result<Vec<BudgetCategorySnapshot>> {
    history(c, month)
        .await?
        .into_iter()
        .find(|item| item.month == month)
        .map(|item| item.categories)
        .context("Budget month unavailable")
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::Connection;

    async fn fixture() -> anyhow::Result<SqliteConnection> {
        let mut c = SqliteConnection::connect("sqlite::memory:").await?;
        sqlx::migrate!().run(&mut c).await?;
        for (id, balance, monthly) in [
            ("hanna", "5250.300512973243", "410"),
            ("food", "100", "200"),
        ] {
            sqlx::query("INSERT INTO finance_categories VALUES (?,?,1,1,'[]')")
                .bind(id)
                .bind(id)
                .execute(&mut c)
                .await?;
            sqlx::query("INSERT INTO budget_month_snapshots VALUES (?,'2026-09','0',?,'0','literal','literal',1,'{}')").bind(id).bind(balance).execute(&mut c).await?;
            // An incorrect imported open balance must have no effect.
            sqlx::query("INSERT INTO budget_month_snapshots VALUES (?,'2026-10','0','999','0','literal','literal',1,'{}')").bind(id).execute(&mut c).await?;
            sqlx::query("INSERT INTO category_budget_periods VALUES (?,'2026-10',?,1,'{}')")
                .bind(id)
                .bind(monthly)
                .execute(&mut c)
                .await?;
        }
        sqlx::query("INSERT INTO finance_import_runs VALUES ('import','fixture','digest','{\"currentCanonicalMonth\":\"2026-10\"}','{}','2026-10-01')").execute(&mut c).await?;
        initialize(&mut c).await?;
        Ok(c)
    }
    fn balance(history: &[BudgetMonthSnapshot], month: &str, category: &str) -> String {
        history
            .iter()
            .find(|entry| entry.month == month)
            .unwrap()
            .categories
            .iter()
            .find(|entry| entry.category_id == category)
            .unwrap()
            .remaining_eur
            .clone()
            .unwrap()
    }
    async fn entry(
        c: &mut SqliteConnection,
        id: &str,
        kind: &str,
        date: &str,
        from: Option<&str>,
        to: Option<&str>,
        amount: &str,
    ) -> anyhow::Result<()> {
        sqlx::query("INSERT INTO finance_transactions (id,kind,effective_date,description,amount_decimal,input_currency,from_category_id,to_category_id,currency_is_override,to_amount_is_override,source_workbook_id,source_sheet,source_row,original_row_fingerprint,source_row_json) VALUES (?,?,?,'fixture',?,'EUR',?,?,0,0,'fixture',?,1,'fingerprint','{}')")
            .bind(id).bind(kind).bind(date).bind(amount).bind(from).bind(to).bind(id).execute(&mut *c).await?;
        for side in ["from", "to"] {
            sqlx::query("INSERT INTO transaction_valuations (transaction_id,side,amount_eur,effective_date,valuation_source,revision,source_cells_json) VALUES (?,?,?,?,'fixture',1,'{}')")
                .bind(id).bind(side).bind(amount).bind(date).execute(&mut *c).await?;
        }
        Ok(())
    }
    #[tokio::test]
    async fn new_category_after_closed_month_has_zero_carryover() -> anyhow::Result<()> {
        let mut c = fixture().await?;
        let frozen = history_at(&mut c, "2026-10", "2026-11").await?.pop().unwrap();
        sqlx::query("INSERT INTO finance_operations VALUES ('close','owner','digest','[]','{}','2026-11-01')").execute(&mut c).await?;
        sqlx::query("INSERT INTO budget_month_closures VALUES ('2026-10','close','2026-11-01',?,?)")
            .bind(serde_json::to_string(&frozen.categories)?).bind("0".repeat(64)).execute(&mut c).await?;
        sqlx::query("INSERT INTO finance_categories VALUES ('home','Home',1,2,'{}')").execute(&mut c).await?;
        sqlx::query("INSERT INTO budget_opening_state VALUES ('home','2026-10','0','owner','2026-11-01','{}')").execute(&mut c).await?;
        sqlx::query("INSERT INTO category_budget_periods VALUES ('home','2026-10','0',2,'null'),('home','2026-11','25',2,'null')").execute(&mut c).await?;
        let result = history_at(&mut c, "2026-12", "2026-12").await?;
        assert_eq!(balance(&result, "2026-11", "home"), "25");
        assert_eq!(balance(&result, "2026-12", "home"), "50");
        assert_eq!(result.iter().find(|m| m.month == "2026-10").unwrap().categories.len(), frozen.categories.len());
        Ok(())
    }

    #[tokio::test]
    async fn skipped_months_and_backdated_changes_are_calculated_without_snapshot_writes()
    -> anyhow::Result<()> {
        let mut c = fixture().await?;
        let october = history_at(&mut c, "2026-10", "2026-10").await?;
        assert_eq!(balance(&october, "2026-10", "hanna"), "5660.300512973243");
        assert!(!october.last().unwrap().carryover_provisional);
        entry(
            &mut c,
            "expense",
            "expense",
            "2026-10-01",
            Some("hanna"),
            None,
            "35.94",
        )
        .await?;
        let before = history_at(&mut c, "2026-12", "2026-12").await?;
        assert_eq!(balance(&before, "2026-11", "hanna"), "6034.360512973243");
        assert_eq!(balance(&before, "2026-12", "hanna"), "6444.360512973243");
        assert!(before.last().unwrap().carryover_provisional);
        entry(
            &mut c,
            "move",
            "reallocate",
            "2026-10-31",
            Some("hanna"),
            Some("food"),
            "100",
        )
        .await?;
        let after = history_at(&mut c, "2026-12", "2026-12").await?;
        assert_eq!(balance(&after, "2026-12", "hanna"), "6344.360512973243");
        assert_eq!(balance(&after, "2026-12", "food"), "800");
        sqlx::query("DELETE FROM finance_transactions WHERE id='expense'")
            .execute(&mut c)
            .await?;
        let deleted = history_at(&mut c, "2026-12", "2026-12").await?;
        assert_eq!(balance(&deleted, "2026-12", "hanna"), "6380.300512973243");
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM budget_month_snapshots WHERE month>'2026-10'"
            )
            .fetch_one(&mut c)
            .await?,
            0
        );
        assert_eq!(sqlx::query_scalar::<_,String>("SELECT remaining_eur FROM budget_month_snapshots WHERE category_id='hanna' AND month='2026-10'").fetch_one(&mut c).await?,"999");
        Ok(())
    }
    #[tokio::test]
    async fn scheduled_allocations_latest_valuations_and_frozen_closes() -> anyhow::Result<()> {
        let mut c = fixture().await?;
        sqlx::query("INSERT INTO category_budget_periods VALUES ('hanna','2026-11','500',1,'{}')")
            .execute(&mut c)
            .await?;
        entry(
            &mut c,
            "expense",
            "expense",
            "2026-10-01",
            Some("hanna"),
            None,
            "10",
        )
        .await?;
        sqlx::query("INSERT INTO transaction_valuations (transaction_id,side,amount_eur,effective_date,valuation_source,revision,source_cells_json) VALUES ('expense','from','20','2026-10-01','fixture',2,'{}')").execute(&mut c).await?;
        let october = history_at(&mut c, "2026-10", "2026-11")
            .await?
            .pop()
            .unwrap();
        assert_eq!(
            balance(&[october.clone()], "2026-10", "hanna"),
            "5640.300512973243"
        );
        sqlx::query("INSERT INTO finance_operations VALUES ('close','owner','digest','[]','{}','2026-11-01')").execute(&mut c).await?;
        sqlx::query(
            "INSERT INTO budget_month_closures VALUES ('2026-10','close','2026-11-01',?,?)",
        )
        .bind(serde_json::to_string(&october.categories)?)
        .bind("0".repeat(64))
        .execute(&mut c)
        .await?;
        let november = history_at(&mut c, "2026-11", "2026-11").await?;
        assert_eq!(balance(&november, "2026-11", "hanna"), "6140.300512973243");
        assert!(!november.last().unwrap().carryover_provisional);
        assert!(require_open(&mut c, "2026-10").await.is_err());
        assert!(
            sqlx::query(
                "UPDATE budget_month_snapshots SET remaining_eur='0' WHERE month='2026-10'"
            )
            .execute(&mut c)
            .await
            .is_err()
        );
        // Even later source changes cannot alter an immutable close.
        sqlx::query("UPDATE transaction_valuations SET amount_eur='40' WHERE revision=2")
            .execute(&mut c)
            .await?;
        assert_eq!(
            balance(
                &history_at(&mut c, "2026-11", "2026-11").await?,
                "2026-11",
                "hanna"
            ),
            "6140.300512973243"
        );
        Ok(())
    }
    #[tokio::test]
    async fn corrections_are_explicit_adjustments_and_reconciliation_uses_operation_order()
    -> anyhow::Result<()> {
        let mut c = fixture().await?;
        entry(
            &mut c,
            "original",
            "expense",
            "2026-10-01",
            Some("hanna"),
            None,
            "10",
        )
        .await?;
        sqlx::query("INSERT INTO finance_operations VALUES ('correction','owner','digest','[]','{}','2026-11-01')").execute(&mut c).await?;
        sqlx::query("INSERT INTO finance_category_corrections (operation_id,original_transaction_id,original_effective_date,applied_month,reason,category_deltas_json,recorded_at) VALUES ('correction','original','2026-10-01','2026-11','fixture','[]','2026-11-01')").execute(&mut c).await?;
        sqlx::query("INSERT INTO finance_category_correction_items VALUES ('correction','hanna','15'),('correction','food','-15')").execute(&mut c).await?;
        let history = history_at(&mut c, "2026-11", "2026-11").await?;
        let hanna = history
            .last()
            .unwrap()
            .categories
            .iter()
            .find(|item| item.category_id == "hanna")
            .unwrap();
        assert_eq!(hanna.net_spending_eur.as_deref(), Some("0"));
        assert_eq!(hanna.adjustment_eur.as_deref(), Some("-15"));
        assert_eq!(hanna.correction_eur.as_deref(), Some("15"));
        assert_eq!(hanna.remaining_eur.as_deref(), Some("6045.300512973243"));
        assert_eq!(balance(&history, "2026-10", "hanna"), "5650.300512973243");
        assert_eq!(balance(&history, "2026-11", "food"), "515");
        sqlx::query("INSERT INTO finance_accounts (id,source_name,is_real,is_active,source_row,source_row_json) VALUES ('account','Account',1,1,1,'{}')").execute(&mut c).await?;
        sqlx::query("INSERT INTO finance_operations VALUES ('recon','owner','digest','[]','{}','2026-11-01')").execute(&mut c).await?;
        sqlx::query("INSERT INTO reconciliation_batches (id,operation_id,effective_date,actor_subject,submitted_at) VALUES ('recon','recon','2026-10-31','owner','2026-11-01')").execute(&mut c).await?;
        entry(
            &mut c,
            "later",
            "expense",
            "2026-11-01",
            Some("hanna"),
            None,
            "10",
        )
        .await?;
        sqlx::query("UPDATE finance_transactions SET from_account_id='account' WHERE id='later'")
            .execute(&mut c)
            .await?;
        sqlx::query("INSERT INTO finance_operations VALUES ('later','owner','digest','[]','{}','2026-11-01')").execute(&mut c).await?;
        sqlx::query("INSERT INTO finance_transaction_revisions VALUES ('later',1,'later','{}','2026-11-01')").execute(&mut c).await?;
        assert!(reconciliation_current(&mut c, "2026-10-31", "recon").await?);
        // Identical timestamps still detect a later operation affecting October.
        sqlx::query("UPDATE finance_transactions SET effective_date='2026-10-30' WHERE id='later'")
            .execute(&mut c)
            .await?;
        assert!(!reconciliation_current(&mut c, "2026-10-31", "recon").await?);
        Ok(())
    }

    #[test]
    fn household_calendar_observes_madrid_midnight_and_dst() {
        let instant = |value: &str| {
            chrono::DateTime::parse_from_rfc3339(value)
                .unwrap()
                .with_timezone(&Utc)
        };
        assert_eq!(
            household_date(instant("2026-10-31T23:30:00Z")),
            "2026-11-01"
        );
        assert_eq!(
            household_date(instant("2026-09-30T22:30:00Z")),
            "2026-10-01"
        );
        assert_eq!(next_month("2026-12").unwrap(), "2027-01");
        assert_eq!(previous_month_end("2028-03-01").unwrap(), "2028-02-29");
    }
}
