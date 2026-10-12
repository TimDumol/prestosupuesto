//! Database-backed read model assembled from canonical finance tables.

// The returned fields are consumed by the DB API during Phase D; Phase C uses
// the complete shape only for sanitized shadow parity.
#![allow(dead_code)]

use anyhow::bail;
use sqlx::{Row, SqlitePool};

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AccountSnapshot {
    pub id: String,
    pub name: String,
    pub account_type: Option<String>,
    pub currency: Option<String>,
    pub balance_native: Option<String>,
    pub actual_balance_native: Option<String>,
    pub reconciliation_native: Option<String>,
    pub balance_eur: Option<String>,
    pub actual_balance_eur: Option<String>,
    pub ledger_revision: i64,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BudgetCategorySnapshot {
    pub category_id: String,
    pub name: String,
    pub monthly_eur: Option<String>,
    pub expense_eur: Option<String>,
    pub remaining_eur: Option<String>,
    pub reallocation_impact_eur: Option<String>,
    #[serde(default)]
    pub carryover_eur: Option<String>,
    #[serde(default)]
    pub adjustment_eur: Option<String>,
    #[serde(default)]
    pub available_eur: Option<String>,
    #[serde(default)]
    pub net_spending_eur: Option<String>,
    #[serde(default)]
    pub correction_eur: Option<String>,
}

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BudgetMonthSnapshot {
    pub month: String,
    pub categories: Vec<BudgetCategorySnapshot>,
    pub status: String,
    pub carryover_provisional: bool,
}

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RecentTransaction {
    pub created_by: Option<String>,
    pub created_at: Option<String>,
    pub id: String,
    pub kind: String,
    pub effective_date: String,
    pub occurred_at: Option<String>,
    pub description: String,
    pub amount: String,
    pub input_currency: Option<String>,
    pub to_amount: Option<String>,
    pub to_currency: Option<String>,
    pub from_category: Option<String>,
    pub to_category: Option<String>,
    pub from_category_id: Option<String>,
    pub to_category_id: Option<String>,
    pub from_account: Option<String>,
    pub to_account: Option<String>,
    pub from_account_id: Option<String>,
    pub to_account_id: Option<String>,
    pub from_amount_native: Option<String>,
    pub from_amount_eur: Option<String>,
    pub to_amount_native: Option<String>,
    pub to_amount_eur: Option<String>,
    pub corrects_transaction_id: Option<String>,
    pub income_treatment: Option<String>,
    pub expected_income_month: Option<String>,
    pub has_category_correction: bool,
    pub revision: i64,
    pub can_edit: bool,
}

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct IncomeExpectation {
    pub amount: String,
    pub currency: String,
    pub amount_eur: String,
    pub recorded_amount: String,
    pub outstanding_eur: String,
    pub arrival_day: i64,
}

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AccountValuation {
    pub as_of: String,
    pub total_eur: String,
    pub rates: Vec<ConversionRate>,
}

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AccountHistoryBalance {
    pub account_id: String,
    pub name: String,
    pub currency: String,
    pub balance_native: String,
    pub balance_eur: Option<String>,
}

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AccountHistoryDate {
    pub date: String,
    pub total_eur: Option<String>,
    pub accounts: Vec<AccountHistoryBalance>,
}

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ConversionRate {
    pub currency: String,
    pub date: Option<String>,
    pub units_per_eur: String,
}

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ReconciliationAccountSnapshot {
    pub id: String,
    pub name: String,
    pub account_type: Option<String>,
    pub currency: Option<String>,
    pub observed_balance_native: Option<String>,
    pub closing_balance_native: Option<String>,
    pub variance_native: Option<String>,
    pub post_month_activity_native: Option<String>,
}

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ReconciliationBatchSnapshot {
    pub effective_date: String,
    pub submitted_at: String,
    pub observed_at: Option<String>,
    pub observed_date: Option<String>,
    pub is_complete: bool,
    pub is_current: bool,
    pub reconciled_account_count: usize,
    pub total_account_count: usize,
    pub accounts: Vec<ReconciliationAccountSnapshot>,
}

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ReconciliationStatusSnapshot {
    pub closing_date: String,
    pub batch: Option<ReconciliationBatchSnapshot>,
}

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ScheduledAllocation {
    pub effective_start_month: String,
    pub name: String,
    pub amount_eur: Option<String>,
}

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FinanceSnapshot {
    pub as_of: String,
    pub budget_month: String,
    pub can_reallocate: bool,
    pub scheduled_allocations: Vec<ScheduledAllocation>,
    pub transaction_count: usize,
    pub recent_transactions: Vec<RecentTransaction>,
    pub budgets: Vec<BudgetCategorySnapshot>,
    pub budget_history: Vec<BudgetMonthSnapshot>,
    pub accounts: Vec<AccountSnapshot>,
    pub account_history: Vec<AccountHistoryDate>,
    pub recorded_income_eur: String,
    pub income_expectation: IncomeExpectation,
    pub budget_coverage: Option<crate::budget_ledger::Coverage>,
    pub account_valuation: AccountValuation,
    pub reconciliation_status: ReconciliationStatusSnapshot,
}

/// Load the best saved reconciliation for a closing date. A completed batch
/// is the durable confirmation that a month was reconciled; its adjustment
/// amount can remain nonzero.
pub(crate) async fn load_reconciliation_batch(
    pool: &SqlitePool,
    closing_date: &str,
) -> anyhow::Result<Option<ReconciliationBatchSnapshot>> {
    let mut connection = pool.acquire().await?;
    load_reconciliation_batch_connection(&mut connection, closing_date).await
}
async fn load_reconciliation_batch_connection(
    connection: &mut sqlx::SqliteConnection, closing_date: &str,
) -> anyhow::Result<Option<ReconciliationBatchSnapshot>> {
    let row = sqlx::query(
        "SELECT rb.id, rb.effective_date, rb.submitted_at, rb.observed_at, rb.observed_date, (SELECT COUNT(DISTINCT ri.account_id) FROM reconciliation_items ri JOIN finance_accounts a ON a.id = ri.account_id WHERE ri.batch_id = rb.id AND a.is_real = 1 AND a.is_active = 1) AS reconciled_account_count, (SELECT COUNT(*) FROM finance_accounts WHERE is_real = 1 AND is_active = 1) AS total_account_count FROM reconciliation_batches rb JOIN finance_operations operation ON operation.operation_id=rb.operation_id WHERE rb.effective_date = ? ORDER BY CASE WHEN (SELECT COUNT(*) FROM finance_accounts WHERE is_real = 1 AND is_active = 1) > 0 AND (SELECT COUNT(DISTINCT ri.account_id) FROM reconciliation_items ri JOIN finance_accounts a ON a.id = ri.account_id WHERE ri.batch_id = rb.id AND a.is_real = 1 AND a.is_active = 1) = (SELECT COUNT(*) FROM finance_accounts WHERE is_real = 1 AND is_active = 1) THEN 0 ELSE 1 END, operation.rowid DESC LIMIT 1",
    )
    .bind(closing_date)
    .fetch_optional(&mut *connection)
    .await?;
    let Some(row) = row else {
        return Ok(None);
    };

    let id: String = row.try_get("id")?;
    let reconciled_account_count = row.try_get::<i64, _>("reconciled_account_count")? as usize;
    let total_account_count = row.try_get::<i64, _>("total_account_count")? as usize;
    let account_rows = sqlx::query(
        "SELECT a.id, a.source_name, a.account_type, a.currency, ri.observed_native_balance, ri.actual_native_balance, ri.variance_native, ri.post_month_activity_native FROM reconciliation_items ri JOIN finance_accounts a ON a.id = ri.account_id WHERE ri.batch_id = ? ORDER BY a.source_row",
    )
    .bind(&id)
    .fetch_all(&mut *connection)
    .await?;
    let accounts = account_rows
        .into_iter()
        .map(|account| {
            Ok(ReconciliationAccountSnapshot {
                id: account.try_get("id")?,
                name: account.try_get("source_name")?,
                account_type: account.try_get("account_type")?,
                currency: account.try_get("currency")?,
                observed_balance_native: account.try_get("observed_native_balance")?,
                closing_balance_native: account.try_get("actual_native_balance")?,
                variance_native: account.try_get("variance_native")?,
                post_month_activity_native: account.try_get("post_month_activity_native")?,
            })
        })
        .collect::<Result<Vec<_>, sqlx::Error>>()?;

    let is_current = crate::budget::reconciliation_current(connection, closing_date, &id).await?;
    Ok(Some(ReconciliationBatchSnapshot {
        is_current,
        effective_date: row.try_get("effective_date")?,
        submitted_at: row.try_get("submitted_at")?,
        observed_at: row.try_get("observed_at")?,
        observed_date: row.try_get("observed_date")?,
        is_complete: total_account_count > 0 && reconciled_account_count == total_account_count,
        reconciled_account_count,
        total_account_count,
        accounts,
    }))
}

pub(crate) fn can_reallocate_budget_month(
    selected_month: &str,
    _latest_month: Option<&str>,
    today: &str,
    is_closed: bool,
) -> bool {
    !is_closed
        && today
            .get(..7)
            .is_some_and(|current| selected_month <= current)
}

fn previous_month(month: &str) -> Option<String> {
    if month.len() != 7 || month.get(4..5) != Some("-") {
        return None;
    }
    let year = month.get(..4)?.parse::<u32>().ok()?;
    let month_number = month.get(5..)?.parse::<u32>().ok()?;
    if year == 0 || !(1..=12).contains(&month_number) {
        return None;
    }
    let (year, month_number) = if month_number == 1 {
        (year - 1, 12)
    } else {
        (year, month_number - 1)
    };
    Some(format!("{year:04}-{month_number:02}"))
}

const TRANSACTION_SELECT: &str = r#"SELECT json_extract(t.source_row_json,'$.correctsTransactionId') AS corrects_transaction_id, (SELECT treatment FROM budget_income_receipts WHERE transaction_id=t.id) AS income_treatment, (SELECT expected_month FROM budget_income_receipts WHERE transaction_id=t.id) AS expected_income_month, COALESCE((SELECT json_extract(snapshot_json,'$.recordedByName') FROM finance_transaction_revisions WHERE transaction_id=t.id AND revision=1), (SELECT username FROM pocket_sessions WHERE subject=t.created_by ORDER BY created_at DESC LIMIT 1),t.created_by) AS created_by, t.created_at, t.id, t.kind, t.effective_date, t.occurred_at, t.description, t.amount_decimal, t.input_currency, t.to_amount_decimal, t.to_currency, fc.name AS from_category, tc.name AS to_category, t.from_category_id, t.to_category_id, fa.source_name AS from_account, ta.source_name AS to_account, t.from_account_id, t.to_account_id, vf.amount_native AS from_amount_native, vf.amount_eur AS from_amount_eur, vt.amount_native AS to_amount_native, vt.amount_eur AS to_amount_eur, EXISTS (SELECT 1 FROM finance_category_corrections cc WHERE cc.original_transaction_id = t.id) AS has_category_correction, t.revision, (t.kind IN ('expense', 'income', 'transfer', 'reallocate') AND t.source_workbook_id = 'canonical-live' AND substr(t.effective_date, 1, 7) >= (SELECT MIN(effective_before_month) FROM budget_opening_state) AND substr(t.effective_date, 1, 7) <= ? AND NOT EXISTS (SELECT 1 FROM budget_month_closures bc WHERE bc.month = substr(t.effective_date, 1, 7)) AND NOT EXISTS (SELECT 1 FROM finance_accounts a WHERE a.id IN (t.from_account_id, t.to_account_id) AND a.is_real = 1 AND a.is_active = 0) ) AS can_edit FROM finance_transactions t LEFT JOIN finance_categories fc ON fc.id = t.from_category_id LEFT JOIN finance_categories tc ON tc.id = t.to_category_id LEFT JOIN finance_accounts fa ON fa.id = t.from_account_id LEFT JOIN finance_accounts ta ON ta.id = t.to_account_id LEFT JOIN transaction_valuations vf ON vf.transaction_id = t.id AND vf.side = 'from' AND vf.revision = (SELECT MAX(v2.revision) FROM transaction_valuations v2 WHERE v2.transaction_id = t.id AND v2.side = 'from') LEFT JOIN transaction_valuations vt ON vt.transaction_id = t.id AND vt.side = 'to' AND vt.revision = (SELECT MAX(v2.revision) FROM transaction_valuations v2 WHERE v2.transaction_id = t.id AND v2.side = 'to')"#;

fn map_transaction(row: sqlx::sqlite::SqliteRow) -> Result<RecentTransaction, sqlx::Error> {
    Ok(RecentTransaction {
        created_by: row.try_get("created_by")?,
        created_at: row.try_get("created_at")?,
        id: row.try_get("id")?,
        kind: row.try_get("kind")?,
        effective_date: row.try_get("effective_date")?,
        occurred_at: row.try_get("occurred_at")?,
        description: row.try_get("description")?,
        amount: row.try_get("amount_decimal")?,
        input_currency: row.try_get("input_currency")?,
        to_amount: row.try_get("to_amount_decimal")?,
        to_currency: row.try_get("to_currency")?,
        from_category: row.try_get("from_category")?,
        to_category: row.try_get("to_category")?,
        from_category_id: row.try_get("from_category_id")?,
        to_category_id: row.try_get("to_category_id")?,
        from_account: row.try_get("from_account")?,
        to_account: row.try_get("to_account")?,
        from_account_id: row.try_get("from_account_id")?,
        to_account_id: row.try_get("to_account_id")?,
        from_amount_native: row.try_get("from_amount_native")?,
        from_amount_eur: row.try_get("from_amount_eur")?,
        to_amount_native: row.try_get("to_amount_native")?,
        to_amount_eur: row.try_get("to_amount_eur")?,
        corrects_transaction_id: row.try_get("corrects_transaction_id")?,
        income_treatment: row.try_get("income_treatment")?,
        expected_income_month: row.try_get("expected_income_month")?,
        has_category_correction: row.try_get("has_category_correction")?,
        revision: row.try_get("revision")?,
        can_edit: row.try_get("can_edit")?,
    })
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TransactionPage {
    pub transactions: Vec<RecentTransaction>,
    pub total: i64,
    pub page: u32,
    pub page_size: u32,
}

/// Filter before paginating, so historical months never depend on recent rows.
pub(crate) async fn transaction(pool: &SqlitePool, id: &str, month: &str) -> anyhow::Result<Option<RecentTransaction>> {
    let query = format!("{TRANSACTION_SELECT} WHERE t.id = ?");
    let row = sqlx::query(sqlx::AssertSqlSafe(query.as_str())).bind(month).bind(id).fetch_optional(pool).await?;
    Ok(row.map(map_transaction).transpose()?)
}

pub(crate) async fn transactions(
    pool: &SqlitePool,
    month: Option<&str>,
    search: &str,
    kind: Option<&str>,
    category: Option<&str>,
    account: Option<&str>,
    page: u32,
    page_size: u32,
) -> anyhow::Result<TransactionPage> {
    transactions_sorted(
        pool, month, search, kind, category, account, page, page_size, "date", "desc",
    )
    .await
}

pub(crate) async fn transactions_sorted(
    pool: &SqlitePool,
    month: Option<&str>,
    search: &str,
    kind: Option<&str>,
    category: Option<&str>,
    account: Option<&str>,
    page: u32,
    page_size: u32,
    sort: &str,
    direction: &str,
) -> anyhow::Result<TransactionPage> {
    if page == 0 || !(1..=100).contains(&page_size) {
        bail!("Invalid transaction pagination.");
    }
    // SQL is composed exclusively of fixed internal fragments; all user values are bound.
    let filter = " WHERE (? IS NULL OR substr(t.effective_date,1,7)=?) AND (?='' OR instr(lower(t.description || ' ' || coalesce(fc.name,'') || ' ' || coalesce(tc.name,'') || ' ' || coalesce(fa.source_name,'') || ' ' || coalesce(ta.source_name,'')),lower(?))>0) AND (? IS NULL OR t.kind=?) AND (? IS NULL OR fc.name=? OR tc.name=?) AND (? IS NULL OR fa.source_name=? OR ta.source_name=?)";
    let mut snapshot = pool.begin().await?;
    let count_sql = format!(
        "SELECT COUNT(*) FROM finance_transactions t LEFT JOIN finance_categories fc ON fc.id=t.from_category_id LEFT JOIN finance_categories tc ON tc.id=t.to_category_id LEFT JOIN finance_accounts fa ON fa.id=t.from_account_id LEFT JOIN finance_accounts ta ON ta.id=t.to_account_id{filter}"
    );
    let total = sqlx::query_scalar::<_, i64>(sqlx::AssertSqlSafe(count_sql.as_str()))
        .bind(month)
        .bind(month)
        .bind(search)
        .bind(search)
        .bind(kind)
        .bind(kind)
        .bind(category)
        .bind(category)
        .bind(category)
        .bind(account)
        .bind(account)
        .bind(account)
        .fetch_one(&mut *snapshot)
        .await?;
    let order = match sort {
        "date" => "coalesce(t.occurred_at,t.effective_date)",
        "description" => "lower(t.description)",
        "type" => "t.kind",
        "amount" => "CASE WHEN t.kind='expense' THEN -abs(CAST(t.amount_decimal AS REAL)) WHEN t.kind='reconciliation' THEN -CAST(t.amount_decimal AS REAL) ELSE abs(CAST(t.amount_decimal AS REAL)) END",
        "id" => "t.id",
        _ => bail!("Invalid transaction sort."),
    };
    let direction = match direction {
        "asc" => "ASC",
        "desc" => "DESC",
        _ => bail!("Invalid sort direction."),
    };
    // Order expressions and direction also come exclusively from the whitelist above.
    let page_sql = format!(
        "{TRANSACTION_SELECT}{filter} ORDER BY {order} {direction}, t.effective_date DESC, t.source_row DESC, t.id DESC LIMIT ? OFFSET ?"
    );
    let transactions = sqlx::query(sqlx::AssertSqlSafe(page_sql.as_str()))
        .bind(&crate::budget::today()[..7])
        .bind(month)
        .bind(month)
        .bind(search)
        .bind(search)
        .bind(kind)
        .bind(kind)
        .bind(category)
        .bind(category)
        .bind(category)
        .bind(account)
        .bind(account)
        .bind(account)
        .bind(i64::from(page_size))
        .bind(i64::from(page - 1) * i64::from(page_size))
        .fetch_all(&mut *snapshot)
        .await?
        .into_iter()
        .map(map_transaction)
        .collect::<Result<Vec<_>, _>>()?;
    snapshot.commit().await?;
    Ok(TransactionPage {
        transactions,
        total,
        page,
        page_size,
    })
}

pub(crate) async fn load(
    pool: &SqlitePool,
    as_of: &str,
    recent_count: u32,
) -> anyhow::Result<FinanceSnapshot> {
    if !as_of.is_ascii()
        || as_of.len() != 10
        || as_of.get(4..5) != Some("-")
        || as_of.get(7..8) != Some("-")
        || recent_count > 200
        || !as_of.bytes().enumerate().all(|(index, byte)| {
            matches!(index, 4 | 7) == (byte == b'-')
                || (!matches!(index, 4 | 7) && byte.is_ascii_digit())
        })
    {
        bail!("Invalid finance snapshot request.");
    }
    let mut snapshot = pool.begin().await?;
    let budget_month = as_of.get(..7).unwrap_or_default().to_string();
    let transaction_count =
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM finance_transactions")
            .fetch_one(&mut *snapshot)
            .await? as usize;
    // Both fragments are fixed SQL, with the limit supplied as a bound parameter.
    let recent_sql = format!("{TRANSACTION_SELECT} ORDER BY t.source_row DESC LIMIT ?");
    let recent_transactions = sqlx::query(sqlx::AssertSqlSafe(recent_sql.as_str()))
        .bind(&crate::budget::today()[..7]).bind(i64::from(recent_count))
        .fetch_all(&mut *snapshot).await?.into_iter().map(map_transaction)
        .collect::<Result<Vec<_>,_>>()?;

    let accounts = sqlx::query(
        "SELECT id, source_name, account_type, currency, current_native_balance, actual_native_balance, reconciliation_native, current_eur_balance, actual_eur_balance, ledger_revision FROM finance_accounts WHERE is_real = 1 AND is_active = 1 ORDER BY source_row",
    )
    .fetch_all(&mut *snapshot)
    .await?
    .into_iter()
    .map(|row| {
        Ok(AccountSnapshot {
            id: row.try_get("id")?,
            name: row.try_get("source_name")?,
            account_type: row.try_get("account_type")?,
            currency: row.try_get("currency")?,
            balance_native: row.try_get("current_native_balance")?,
            actual_balance_native: row.try_get("actual_native_balance")?,
            reconciliation_native: row.try_get("reconciliation_native")?,
            balance_eur: row.try_get("current_eur_balance")?,
            actual_balance_eur: row.try_get("actual_eur_balance")?,
            ledger_revision: row.try_get("ledger_revision")?,
        })
    })
    .collect::<Result<Vec<_>, sqlx::Error>>()?;

    let budget_history = crate::budget::history(&mut snapshot, &budget_month).await?;
    let budgets = budget_history
        .iter()
        .find(|month| month.month == budget_month)
        .map(|month| month.categories.clone())
        .ok_or_else(|| anyhow::anyhow!("The requested budget month is not imported."))?;

    let today = crate::budget::today();
    let month_status = budget_history
        .iter()
        .find(|month| month.month == budget_month)
        .ok_or_else(|| anyhow::anyhow!("Budget month unavailable."))?;
    let can_reallocate = month_status.status == "open" && budget_month.as_str() <= &today[..7];

    let recorded_income_eur = load_monthly_income(&mut *snapshot, &budget_month).await?;
    let valuation_date = if budget_month == today[..7] {
        today.clone()
    } else {
        sqlx::query_scalar::<_, String>("SELECT date(? || '-01', '+1 month', '-1 day')")
            .bind(&budget_month)
            .fetch_one(&mut *snapshot)
            .await?
    };
    let observations = load_rates(&mut *snapshot).await?;
    let account_valuation = value_accounts(&accounts, &valuation_date, &observations)?;
    let account_history =
        load_account_history(&mut *snapshot, &accounts, &valuation_date, &observations).await?;
    let expectation_row = sqlx::query(
        "SELECT amount, currency, arrival_day FROM budget_income_expectations WHERE effective_start_month <= ? ORDER BY effective_start_month DESC LIMIT 1",
    ).bind(&budget_month).fetch_optional(&mut *snapshot).await?;
    let (amount, currency, arrival_day) = match expectation_row {
        Some(row) => (
            row.try_get::<String, _>("amount")?,
            row.try_get::<String, _>("currency")?,
            row.try_get::<i64, _>("arrival_day")?,
        ),
        None => ("10000".to_string(), "USD".to_string(), 7),
    };
    let recorded_amount = load_income_in_currency(
        &mut *snapshot,
        &budget_month,
        &currency,
        &valuation_date,
        &observations,
    )
    .await?;
    let outstanding = crate::finance_model::subtract_decimal_text(&amount, &recorded_amount)
        .map_err(anyhow::Error::msg)?;
    let outstanding = if outstanding.starts_with('-') {
        "0"
    } else {
        &outstanding
    };
    let mut income_expectation = IncomeExpectation {
        amount_eur: crate::finance_model::account_balance_eur(
            &amount,
            &currency,
            &valuation_date,
            &observations,
        )
        .map_err(anyhow::Error::msg)?,
        outstanding_eur: crate::finance_model::account_balance_eur(
            outstanding,
            &currency,
            &valuation_date,
            &observations,
        )
        .map_err(anyhow::Error::msg)?,
        amount,
        currency,
        recorded_amount,
        arrival_day,
    };
    let closing_date = if budget_month.as_str() < &today[..7] {
        let first = format!("{budget_month}-01");
        let date = chrono::NaiveDate::parse_from_str(&first, "%Y-%m-%d")?
            .checked_add_months(chrono::Months::new(1))
            .ok_or_else(|| anyhow::anyhow!("Date overflow"))?
            .pred_opt()
            .ok_or_else(|| anyhow::anyhow!("Date overflow"))?;
        date.to_string()
    } else {
        crate::budget::previous_month_end(&today)?
    };
    let batch = load_reconciliation_batch_connection(&mut snapshot, &closing_date).await?;
    let reconciliation_status = ReconciliationStatusSnapshot {
        closing_date,
        batch,
    };

    let scheduled_allocations = sqlx::query("SELECT p.effective_start_month,c.name,p.base_monthly_eur FROM category_budget_periods p JOIN finance_categories c ON c.id=p.category_id WHERE p.effective_start_month>? AND c.is_real=1 ORDER BY p.effective_start_month,c.source_row")
        .bind(&today[..7]).fetch_all(&mut *snapshot).await?.into_iter().map(|row| Ok(ScheduledAllocation {
            effective_start_month: row.try_get("effective_start_month")?,name: row.try_get("name")?,amount_eur:row.try_get("base_monthly_eur")?,
        })).collect::<Result<Vec<_>,sqlx::Error>>()?;
    let budget_coverage =
        crate::budget_ledger::coverage(&mut snapshot, &budget_month, &budgets).await?;
    if let Some(coverage) = &budget_coverage {
        if let Some(expected) = coverage
            .expectations
            .iter()
            .find(|e| e.month == budget_month)
        {
            income_expectation.recorded_amount = expected.received_amount.clone();
            income_expectation.amount = expected.amount.clone();
            income_expectation.currency = expected.currency.clone();
            income_expectation.amount_eur = expected.amount_eur.clone();
            income_expectation.outstanding_eur = expected.outstanding_eur.clone();
            income_expectation.arrival_day = expected.arrival_day;
        }
    }
    snapshot.commit().await?;
    Ok(FinanceSnapshot {
        as_of: as_of.to_string(),
        budget_month,
        can_reallocate,
        scheduled_allocations,
        transaction_count,
        recent_transactions,
        budgets,
        budget_history,
        accounts,
        account_history,
        recorded_income_eur,
        income_expectation,
        account_valuation,
        budget_coverage,
        reconciliation_status,
    })
}

async fn load_account_history<'e>(
    pool: &'e mut sqlx::SqliteConnection,
    accounts: &[AccountSnapshot],
    as_of: &str,
    rates: &[crate::finance_model::RateObservation],
) -> anyhow::Result<Vec<AccountHistoryDate>> {
    let rows = sqlx::query(
        "SELECT c.checkpoint_date, c.account_id, a.source_name, a.currency, c.balance_decimal FROM finance_account_balance_checkpoints c JOIN finance_accounts a ON a.id = c.account_id WHERE a.is_real = 1 AND a.is_active = 1 AND c.checkpoint_date IS NOT NULL AND c.checkpoint_date <= ? AND c.balance_decimal IS NOT NULL AND c.source_state IN ('formula', 'literal') ORDER BY c.checkpoint_date, a.source_row, length(c.source_column), c.source_column",
    )
    .bind(as_of)
    .fetch_all(&mut *pool)
    .await?;

    // Imported sheet quotes are EUR per native unit, the inverse of ECB quotes.
    // Use these only for archived checkpoints without a prior ECB observation.
    let imported_rates = sqlx::query("SELECT base_currency, rate_date, rate_decimal FROM exchange_rates WHERE (provider IS NULL OR provider != 'ECB') AND quote_currency = 'EUR' AND base_currency IN ('PHP', 'USD') AND rate_date IS NOT NULL AND rate_decimal IS NOT NULL AND CAST(rate_decimal AS REAL) > 0 ORDER BY rate_date, id")
        .fetch_all(&mut *pool).await?;

    let mut history = std::collections::BTreeMap::<String, Vec<AccountHistoryBalance>>::new();
    for row in rows {
        let date: String = row.try_get("checkpoint_date")?;
        let account_id: String = row.try_get("account_id")?;
        let name: String = row.try_get("source_name")?;
        let currency: Option<String> = row.try_get("currency")?;
        let currency = currency.unwrap_or_else(|| "EUR".to_string());
        let balance_native: String = row.try_get("balance_decimal")?;
        let has_prior_rate = currency == "EUR"
            || rates
                .iter()
                .any(|rate| rate.currency == currency && rate.date.as_str() <= date.as_str());
        let balance_eur = if has_prior_rate {
            Some(crate::finance_model::account_balance_eur(&balance_native, &currency, &date, rates)
                .map_err(anyhow::Error::msg)?)
        } else {
            let prior = imported_rates.iter().rev().find(|rate| {
                rate.get::<String, _>("base_currency") == currency
                    && rate.get::<String, _>("rate_date") <= date
            });
            prior.map(|rate| crate::finance_model::multiply_decimal_text(
                &balance_native, &rate.get::<String, _>("rate_decimal"),
            ).map_err(anyhow::Error::msg)).transpose()?
        };
        let balances = history.entry(date).or_default();
        // A workbook can contain repeated date headings. Keep the rightmost
        // archived observation for an account if that occurs.
        if let Some(existing) = balances
            .iter_mut()
            .find(|item| item.account_id == account_id)
        {
            *existing = AccountHistoryBalance {
                account_id,
                name,
                currency,
                balance_native,
                balance_eur,
            };
        } else {
            balances.push(AccountHistoryBalance {
                account_id,
                name,
                currency,
                balance_native,
                balance_eur,
            });
        }
    }

    let active_account_count = accounts.len();
    let mut result = Vec::with_capacity(history.len());
    for (date, balances) in history {
        let converted = balances
            .iter()
            .filter_map(|balance| balance.balance_eur.as_deref())
            .collect::<Vec<_>>();
        let total_eur =
            if balances.len() == active_account_count && converted.len() == active_account_count {
                let mut total = "0".to_string();
                for amount in converted {
                    total = crate::finance_model::add_decimal_text(&total, amount)
                        .map_err(anyhow::Error::msg)?;
                }
                Some(total)
            } else {
                None
            };
        result.push(AccountHistoryDate {
            date,
            total_eur,
            accounts: balances,
        });
    }
    Ok(result)
}

pub(crate) async fn load_rates<'e>(
    pool: impl sqlx::Executor<'e, Database = sqlx::Sqlite>,
) -> anyhow::Result<Vec<crate::finance_model::RateObservation>> {
    sqlx::query("SELECT quote_currency, rate_date, rate_decimal FROM exchange_rates WHERE provider = 'ECB' AND base_currency = 'EUR' AND quote_currency IN ('PHP', 'USD') AND rate_date IS NOT NULL AND rate_decimal IS NOT NULL ORDER BY rate_date, id")
        .fetch_all(pool).await?.into_iter().map(|row| {
            Ok(crate::finance_model::RateObservation {
                currency: row.try_get("quote_currency")?,
                date: row.try_get("rate_date")?,
                currency_units_per_eur: row.try_get("rate_decimal")?,
            })
        }).collect::<Result<Vec<_>, sqlx::Error>>().map_err(Into::into)
}

fn value_accounts(
    accounts: &[AccountSnapshot],
    as_of: &str,
    rates: &[crate::finance_model::RateObservation],
) -> anyhow::Result<AccountValuation> {
    let mut total = "0".to_string();
    let mut conversions = Vec::<ConversionRate>::new();
    for account in accounts {
        let currency = account.currency.as_deref().unwrap_or("EUR");
        let native = account
            .balance_native
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("Account balance is missing."))?;
        let eur = crate::finance_model::account_balance_eur(native, currency, as_of, rates)
            .map_err(anyhow::Error::msg)?;
        total = crate::finance_model::add_decimal_text(&total, &eur).map_err(anyhow::Error::msg)?;
        if !conversions.iter().any(|rate| rate.currency == currency) {
            let selected = crate::finance_model::select_prior_rate(currency, as_of, rates)
                .map_err(anyhow::Error::msg)?;
            conversions.push(ConversionRate {
                currency: currency.to_string(),
                date: selected.date,
                units_per_eur: selected.quote,
            });
        }
    }
    // Include the income currency even if there are no accounts in it.
    if !conversions.iter().any(|rate| rate.currency == "USD") {
        let selected = crate::finance_model::select_prior_rate("USD", as_of, rates)
            .map_err(anyhow::Error::msg)?;
        conversions.push(ConversionRate {
            currency: "USD".into(),
            date: selected.date,
            units_per_eur: selected.quote,
        });
    }
    Ok(AccountValuation {
        as_of: as_of.to_string(),
        total_eur: total,
        rates: conversions,
    })
}

async fn load_income_in_currency<'e>(
    pool: impl sqlx::Executor<'e, Database = sqlx::Sqlite>,
    month: &str,
    currency: &str,
    as_of: &str,
    rates: &[crate::finance_model::RateObservation],
) -> anyhow::Result<String> {
    let rows = sqlx::query("SELECT t.amount_decimal, t.input_currency, v.account_currency, v.amount_native, v.amount_eur FROM finance_transactions t LEFT JOIN transaction_valuations v ON v.transaction_id = t.id AND v.side = 'to' AND v.revision = (SELECT MAX(v2.revision) FROM transaction_valuations v2 WHERE v2.transaction_id = t.id AND v2.side = 'to') WHERE t.kind = 'income' AND substr(t.effective_date, 1, 7) = ?")
        .bind(month).fetch_all(pool).await?;
    let quote = crate::finance_model::select_prior_rate(currency, as_of, rates)
        .map_err(anyhow::Error::msg)?
        .quote;
    let mut total = "0".to_string();
    for row in rows {
        let input_currency: Option<String> = row.try_get("input_currency")?;
        let account_currency: Option<String> = row.try_get("account_currency")?;
        let amount = if input_currency.as_deref() == Some(currency) {
            row.try_get::<String, _>("amount_decimal")?
        } else if account_currency.as_deref() == Some(currency) {
            row.try_get::<Option<String>, _>("amount_native")?
                .ok_or_else(|| anyhow::anyhow!("Income native valuation is missing."))?
        } else {
            let eur = row
                .try_get::<Option<String>, _>("amount_eur")?
                .ok_or_else(|| anyhow::anyhow!("Income EUR valuation is missing."))?;
            crate::finance_model::multiply_decimal_text(&eur, &quote).map_err(anyhow::Error::msg)?
        };
        total =
            crate::finance_model::add_decimal_text(&total, &amount).map_err(anyhow::Error::msg)?;
    }
    Ok(total)
}

// Sum every recorded income row using its latest destination EUR valuation.
// This deliberately does not depend on the recent-transactions display limit.
async fn load_monthly_income<'e>(pool: impl sqlx::Executor<'e, Database = sqlx::Sqlite>, month: &str) -> anyhow::Result<String> {
    let amounts = sqlx::query_scalar::<_, Option<String>>(
        "SELECT v.amount_eur FROM finance_transactions t LEFT JOIN transaction_valuations v ON v.transaction_id = t.id AND v.side = 'to' AND v.revision = (SELECT MAX(v2.revision) FROM transaction_valuations v2 WHERE v2.transaction_id = t.id AND v2.side = 'to') WHERE t.kind = 'income' AND substr(t.effective_date, 1, 7) = ?",
    ).bind(month).fetch_all(pool).await?;
    let mut total = "0".to_string();
    for amount in amounts {
        let amount = amount
            .ok_or_else(|| anyhow::anyhow!("Recorded income is missing its EUR valuation."))?;
        total =
            crate::finance_model::add_decimal_text(&total, &amount).map_err(anyhow::Error::msg)?;
    }
    Ok(total)
}

pub(crate) fn assert_fixture_snapshot(snapshot: &FinanceSnapshot) -> anyhow::Result<()> {
    if snapshot.as_of != "2026-09-08"
        || snapshot.budget_month != "2026-09"
        || snapshot.transaction_count == 0
        || snapshot.recent_transactions.is_empty()
        || snapshot.budgets.is_empty()
        || snapshot.budget_history.is_empty()
        || snapshot.accounts.is_empty()
    {
        bail!("The sanitized finance read model is incomplete.");
    }
    Ok(())
}

#[cfg(test)]
mod income_tests {
    use super::*;

    #[tokio::test]
    async fn archived_balances_use_imported_rates_only_before_ecb_coverage() -> anyhow::Result<()> {
        use sqlx::Connection;
        let mut connection = sqlx::SqliteConnection::connect(":memory:").await?;
        sqlx::raw_sql("CREATE TABLE finance_accounts (id TEXT, source_name TEXT, currency TEXT, is_real INTEGER, is_active INTEGER, source_row INTEGER);
            CREATE TABLE finance_account_balance_checkpoints (checkpoint_date TEXT, account_id TEXT, balance_decimal TEXT, source_state TEXT, source_column TEXT);
            CREATE TABLE exchange_rates (id TEXT, provider TEXT, base_currency TEXT, quote_currency TEXT, rate_date TEXT, rate_decimal TEXT);
            INSERT INTO finance_accounts VALUES ('usd', 'USD account', 'USD', 1, 1, 1);
            INSERT INTO finance_account_balance_checkpoints VALUES ('2025-02-01', 'usd', '100', 'literal', 'A'), ('2025-08-01', 'usd', '100', 'literal', 'B'), ('2026-09-30', 'usd', '100', 'literal', 'C');
            INSERT INTO exchange_rates VALUES ('old', NULL, 'USD', 'EUR', '2025-03-01', '0.9'), ('future', NULL, 'USD', 'EUR', '2025-08-02', '0.8'), ('recent', NULL, 'USD', 'EUR', '2026-09-30', '0.7'), ('zero', NULL, 'USD', 'EUR', '2025-07-31', '0');")
            .execute(&mut connection).await?;
        let history = load_account_history(&mut connection, &[], "2026-09-30", &observations()).await?;
        assert_eq!(history[0].accounts[0].balance_eur, None);
        assert_eq!(history[1].accounts[0].balance_eur.as_deref(), Some("90"));
        // ECB wins even when an imported rate has the same checkpoint date.
        assert_eq!(history[2].accounts[0].balance_eur.as_deref(), Some("80"));
        Ok(())
    }

    fn observations() -> Vec<crate::finance_model::RateObservation> {
        [
            ("2026-09-28", "1.10"),
            ("2026-09-30", "1.25"),
            ("2026-10-01", "1.50"),
        ]
        .into_iter()
        .map(|(date, quote)| crate::finance_model::RateObservation {
            currency: "USD".into(),
            date: date.into(),
            currency_units_per_eur: quote.into(),
        })
        .collect()
    }

    #[test]
    fn account_totals_use_latest_prior_rate_without_future_rates() -> anyhow::Result<()> {
        let account = AccountSnapshot {
            id: "usd".into(),
            name: "USD account".into(),
            account_type: None,
            currency: Some("USD".into()),
            balance_native: Some("1500".into()),
            actual_balance_native: None,
            reconciliation_native: None,
            balance_eur: Some("999".into()),
            actual_balance_eur: None,
            ledger_revision: 0,
        };
        let past = value_accounts(&[account.clone()], "2026-09-30", &observations())?;
        assert_eq!(past.total_eur, "1200");
        assert_eq!(past.rates[0].date.as_deref(), Some("2026-09-30"));
        let current = value_accounts(&[account], "2026-10-01", &observations())?;
        assert_eq!(current.total_eur, "1000");
        assert_eq!(current.rates[0].date.as_deref(), Some("2026-10-01"));
        Ok(())
    }

    #[tokio::test]
    async fn dollar_income_is_fully_received_despite_posting_fx_changes() -> anyhow::Result<()> {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await?;
        sqlx::query("CREATE TABLE finance_transactions (id TEXT, kind TEXT, effective_date TEXT, amount_decimal TEXT, input_currency TEXT)").execute(&pool).await?;
        sqlx::query("CREATE TABLE transaction_valuations (transaction_id TEXT, side TEXT, revision INTEGER, account_currency TEXT, amount_native TEXT, amount_eur TEXT)").execute(&pool).await?;
        sqlx::query("INSERT INTO finance_transactions VALUES ('first', 'income', '2026-10-07', '4000', 'USD'), ('second', 'income', '2026-10-08', '6000', 'USD')").execute(&pool).await?;
        sqlx::query("INSERT INTO transaction_valuations VALUES ('first', 'to', 1, 'USD', '4000', '3636.36'), ('second', 'to', 1, 'USD', '6000', '4800')").execute(&pool).await?;
        assert_eq!(load_monthly_income(&pool, "2026-10").await?, "8436.36");
        assert_eq!(
            load_income_in_currency(&pool, "2026-10", "USD", "2026-10-08", &observations()).await?,
            "10000"
        );
        Ok(())
    }

    #[tokio::test]
    async fn monthly_income_uses_all_rows_latest_eur_values_and_selected_month()
    -> anyhow::Result<()> {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await?;
        sqlx::query("CREATE TABLE finance_transactions (id TEXT, kind TEXT, effective_date TEXT)")
            .execute(&pool)
            .await?;
        sqlx::query("CREATE TABLE transaction_valuations (transaction_id TEXT, side TEXT, revision INTEGER, amount_eur TEXT)").execute(&pool).await?;
        // More than the UI's 200-row limit; no SQL floating-point summation.
        for id in 0..205 {
            sqlx::query("INSERT INTO finance_transactions VALUES (?, 'income', '2026-10-07')")
                .bind(id.to_string())
                .execute(&pool)
                .await?;
            sqlx::query("INSERT INTO transaction_valuations VALUES (?, 'to', 1, '0.10')")
                .bind(id.to_string())
                .execute(&pool)
                .await?;
        }
        sqlx::query("INSERT INTO transaction_valuations VALUES ('0', 'to', 2, '1.20'), ('0', 'from', 1, '999'), ('prior', 'to', 1, '500'), ('expense', 'to', 1, '300')").execute(&pool).await?;
        sqlx::query("INSERT INTO finance_transactions VALUES ('prior', 'income', '2026-09-07'), ('expense', 'expense', '2026-10-08')").execute(&pool).await?;
        assert_eq!(load_monthly_income(&pool, "2026-10").await?, "21.6");
        assert_eq!(load_monthly_income(&pool, "2026-09").await?, "500");
        assert_eq!(load_monthly_income(&pool, "2026-11").await?, "0");
        sqlx::query("INSERT INTO finance_transactions VALUES ('missing', 'income', '2026-10-08')")
            .execute(&pool)
            .await?;
        assert!(load_monthly_income(&pool, "2026-10").await.is_err());
        Ok(())
    }
}


#[cfg(test)]
mod transaction_pagination_tests {
    use super::*;

    #[tokio::test]
    async fn historical_pages_and_all_history_search_are_complete_and_exact() -> anyhow::Result<()>
    {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await?;
        sqlx::migrate!().run(&pool).await?;
        sqlx::query("INSERT INTO finance_accounts (id,source_name,is_real,is_active,source_row,source_row_json) VALUES ('bank','Bank',1,1,1,'{}'),('wallet','Wallet',1,1,2,'{}')").execute(&pool).await?;
        sqlx::query("INSERT INTO finance_categories (id,name,is_real,source_row,source_row_json) VALUES ('food','Food',1,1,'{}'),('home','Home',1,2,'{}')").execute(&pool).await?;
        for index in 0..210 {
            let date = if index < 205 {
                "2000-01-15"
            } else {
                "2020-02-15"
            };
            let category = if index == 0 { "home" } else { "food" };
            let account = if index == 0 { "wallet" } else { "bank" };
            let description = if index == 0 {
                "Older special payment"
            } else {
                "Ordinary payment"
            };
            sqlx::query("INSERT INTO finance_transactions (id,kind,effective_date,description,amount_decimal,from_category_id,from_account_id,currency_is_override,to_amount_is_override,source_workbook_id,source_sheet,source_row,original_row_fingerprint,source_row_json) VALUES (?,'expense',?,?,'12.34',?,?,0,0,'archive','Transactions',?,'fingerprint','{}')")
                .bind(format!("entry-{index}")).bind(date).bind(description).bind(category).bind(account).bind(index)
                .execute(&pool).await?;
        }
        let first = transactions(&pool, Some("2000-01"), "", None, None, None, 1, 100).await?;
        let second = transactions(&pool, Some("2000-01"), "", None, None, None, 2, 100).await?;
        let third = transactions(&pool, Some("2000-01"), "", None, None, None, 3, 100).await?;
        assert_eq!(first.total, 205);
        assert_eq!(second.transactions.len(), 100);
        assert_eq!(third.transactions.len(), 5);
        let ids: std::collections::HashSet<_> = first
            .transactions
            .iter()
            .chain(&second.transactions)
            .chain(&third.transactions)
            .map(|entry| &entry.id)
            .collect();
        assert_eq!(ids.len(), 205);
        let searched = transactions(&pool, None, "OLDER SPECIAL", None, None, None, 1, 50).await?;
        assert_eq!(searched.total, 1);
        assert_eq!(searched.transactions[0].id, "entry-0");
        let exact = transactions(
            &pool,
            Some("2000-01"),
            "",
            Some("expense"),
            Some("Home"),
            Some("Wallet"),
            1,
            50,
        )
        .await?;
        assert_eq!(exact.total, 1);
        let alphabetical = transactions_sorted(
            &pool,
            Some("2000-01"),
            "",
            None,
            None,
            None,
            1,
            50,
            "description",
            "asc",
        )
        .await?;
        assert_eq!(alphabetical.transactions[0].id, "entry-0");
        assert!(
            transactions_sorted(
                &pool,
                None,
                "",
                None,
                None,
                None,
                1,
                50,
                "description; DROP TABLE finance_transactions",
                "asc"
            )
            .await
            .is_err()
        );
        assert_eq!(
            transactions(&pool, None, "Wallet", None, None, None, 1, 50)
                .await?
                .total,
            1
        );
        assert_eq!(
            transactions(&pool, None, "", Some("income"), None, None, 1, 50)
                .await?
                .total,
            0
        );
        assert_eq!(
            transactions(&pool, None, "' OR 1=1 --", None, None, None, 1, 50)
                .await?
                .total,
            0
        );
        assert_eq!(
            transactions(&pool, Some("1999-01"), "", None, None, None, 1, 50)
                .await?
                .total,
            0
        );
        assert!(
            transactions(&pool, None, "", None, None, None, 0, 50)
                .await
                .is_err()
        );
        assert!(
            transactions(&pool, None, "", None, None, None, 1, 101)
                .await
                .is_err()
        );
        Ok(())
    }
}
