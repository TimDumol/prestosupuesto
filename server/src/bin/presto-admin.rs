#[path = "../budget.rs"]
mod budget;
#[path = "../budget_ledger.rs"]
mod budget_ledger;
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    env, fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, bail};
use num_bigint::BigInt;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use sqlx::{
    AssertSqlSafe, Column, Row, SqlitePool, TypeInfo, ValueRef, sqlite::SqliteConnectOptions,
};
use uuid::Uuid;

#[path = "../finance_model.rs"]
mod finance_model;
#[path = "../finance_read_model.rs"]
mod finance_read_model;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Manifest {
    schema_version: u32,
    source_spreadsheet_id: String,
    export_file: String,
    export_sha256: String,
    exported_at: String,
    canonical_time_zone: String,
    current_canonical_month: String,
    test_workbook: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FinanceExport {
    schema_version: u32,
    source: ExportSource,
    exported_at: String,
    canonical_time_zone: String,
    workbook: Value,
    sheets: HashMap<String, SheetArchive>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExportSource {
    spreadsheet_id: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct SheetArchive {
    title: String,
    sheet_id: i64,
    range: String,
    grid_row_count: usize,
    grid_column_count: usize,
    column_count: usize,
    row_count: usize,
    rows: Vec<Vec<Option<ArchiveCell>>>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct ArchiveCell {
    entered: Option<Value>,
    effective: Option<Value>,
    formatted: Option<String>,
    data_source_formula: Option<Value>,
}

#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct IdentityMap {
    schema_version: u32,
    source_sha256: String,
    import_run_id: String,
    transaction_ids: BTreeMap<String, String>,
    account_ids: BTreeMap<String, String>,
    category_ids: BTreeMap<String, String>,
}

struct ArchiveBundle {
    directory: PathBuf,
    manifest: Manifest,
    export: FinanceExport,
    owner_review: OwnerReviewDecisions,
}

#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct OwnerReviewDecisions {
    schema_version: u32,
    source_sha256: String,
    #[serde(default)]
    transaction_rows: BTreeMap<String, String>,
    #[serde(default)]
    invalid_accounts: Vec<String>,
}

#[derive(Serialize)]
struct TransactionImport {
    id: String,
    kind: String,
    effective_date: String,
    description: String,
    amount_decimal: String,
    input_currency: Option<String>,
    to_amount_decimal: Option<String>,
    to_currency: Option<String>,
    from_category_id: Option<String>,
    to_category_id: Option<String>,
    from_account_id: Option<String>,
    to_account_id: Option<String>,
    currency_is_override: bool,
    to_amount_is_override: bool,
    created_at: Option<String>,
    created_by: Option<String>,
    source_row: usize,
    original_row_fingerprint: String,
    source_row_json: String,
    row: Vec<Option<ArchiveCell>>,
}

struct QuarantineImport {
    source_row: usize,
    reason: &'static str,
    fingerprint: String,
    row_json: String,
}

struct AccountImport {
    id: String,
    name: String,
    account_type: Option<String>,
    currency: Option<String>,
    is_real: bool,
    is_active: bool,
    current_native_balance: Option<String>,
    actual_native_balance: Option<String>,
    reconciliation_native: Option<String>,
    current_eur_balance: Option<String>,
    actual_eur_balance: Option<String>,
    source_row: usize,
    source_row_json: String,
}

struct AccountCheckpointImport {
    account_id: String,
    checkpoint_date: Option<String>,
    balance_decimal: Option<String>,
    source_state: &'static str,
    source_header_json: String,
    source_cell_json: String,
    source_row: usize,
    source_column: String,
}

struct AccountOpeningBalanceImport {
    account_id: String,
    effective_date: String,
    balance_decimal: String,
    source_row: usize,
    source_row_fingerprint: String,
    source_row_json: String,
}

struct CategoryImport {
    id: String,
    name: String,
    is_real: bool,
    base_monthly_eur: Option<String>,
    source_row: usize,
    source_row_json: String,
    base_cell_json: String,
}

struct BudgetMonthImport {
    category_id: String,
    month: String,
    expense_eur: Option<String>,
    remaining_eur: Option<String>,
    reallocation_impact_eur: String,
    expense_state: &'static str,
    remaining_state: &'static str,
    source_row: usize,
    source_cells_json: String,
}

struct RateImport {
    source_row: usize,
    source_column: String,
    rate_date: String,
    base_currency: String,
    quote_currency: String,
    source_pair: String,
    rate_decimal: String,
    provider: Option<String>,
    source_formula: Option<String>,
    source_value_json: String,
    row_fingerprint: String,
}

struct ImportPlan {
    accounts: Vec<AccountImport>,
    account_checkpoints: Vec<AccountCheckpointImport>,
    account_checkpoints_present: bool,
    account_opening_balances: Vec<AccountOpeningBalanceImport>,
    categories: Vec<CategoryImport>,
    transactions: Vec<TransactionImport>,
    quarantined: Vec<QuarantineImport>,
    budgets: Vec<BudgetMonthImport>,
    rate_rows: Vec<(usize, String, String)>,
    normalized_rates: Vec<RateImport>,
    validation_issues: Vec<ImportIssue>,
    review_warnings: Vec<ImportIssue>,
    source_transaction_input_rows: usize,
    counts: Value,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ImportIssue {
    source_sheet: &'static str,
    source_row: usize,
    reason_code: &'static str,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut args = env::args().skip(1);
    let domain = args.next().unwrap_or_default();
    let action = args.next().unwrap_or_default();
    if domain != "finance" {
        print_usage();
        bail!("Expected the finance admin command.");
    }
    let options = parse_options(args.collect())?;
    match action.as_str() {
        "import" => import_command(options).await,
        "verify" => verify_command(options).await,
        "export-sqlite" => export_sqlite_command(options).await,
        "restore-sqlite" => restore_sqlite_command(options).await,
        "shadow-simulate" => shadow_simulate_command(options).await,
        "budget-report" => budget_report_command(options).await,
        "budget-ledger-activate" => budget_ledger_activate_command(options).await,
        "compare-read-model" => compare_read_model_command(options).await,
        _ => {
            print_usage();
            bail!("Unknown finance admin action.")
        }
    }
}

fn print_usage() {
    eprintln!(
        "Usage:\n  presto-admin finance budget-ledger-activate --database FILE --commit\n  presto-admin finance budget-report --database FILE --month YYYY-MM [--calendar-month YYYY-MM] [--category NAME]\n  presto-admin finance import --input DIR [--database FILE] [--commit] [--allow-test-workbook]\n  presto-admin finance verify --input DIR [--database FILE] [--report FILE]\n  presto-admin finance export-sqlite --database FILE --output FILE\n  presto-admin finance restore-sqlite --input FILE --sha256 HEX --database NEW_FILE\n  presto-admin finance shadow-simulate --input DIR --database FILE --as-of YYYY-MM-DD\n  presto-admin finance compare-read-model --input DIR --database FILE --as-of YYYY-MM-DD"
    );
}

/// Explicit maintenance command; server startup uses the same atomic activation.
async fn budget_ledger_activate_command(options: HashMap<String, String>) -> anyhow::Result<()> {
    if !options.contains_key("commit") {
        bail!("--commit is required to activate the budget ledger.");
    }
    let pool = open_pool(&database_path(&options), false).await?;
    sqlx::migrate!().run(&pool).await?;
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
    budget::initialize(&mut tx).await?;
    budget_ledger::activate(&mut tx).await?;
    budget_ledger::sync(&mut tx, None).await?;
    let month = budget::today()[..7].to_owned();
    let categories = budget::categories(&mut tx, &month).await?;
    let coverage = budget_ledger::coverage(&mut tx, &month, &categories).await?;
    tx.commit().await?;
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({"activated":true,"month":month,"coverage":coverage}))?
    );
    Ok(())
}

/// Read-only reporting can simulate a later calendar month without changing
/// the server clock, creating snapshots, or modifying the selected database.
async fn budget_report_command(options: HashMap<String, String>) -> anyhow::Result<()> {
    let month =
        option(&options, "month").ok_or_else(|| anyhow::anyhow!("--month YYYY-MM is required"))?;
    let calendar =
        option(&options, "calendar-month").unwrap_or_else(|| budget::today()[..7].to_owned());
    let pool = open_pool(&database_path(&options), true).await?;
    let mut read = pool.begin().await?;
    let mut report = budget::history_at(&mut read, &month, &calendar)
        .await?
        .into_iter()
        .find(|entry| entry.month == month)
        .ok_or_else(|| anyhow::anyhow!("Budget month unavailable"))?;
    if let Some(category) = option(&options, "category") {
        report.categories.retain(|entry| entry.name == category);
    }
    println!("{}", serde_json::to_string_pretty(&report)?);
    read.commit().await?;
    Ok(())
}

async fn compare_read_model_command(options: HashMap<String, String>) -> anyhow::Result<()> {
    let input = option(&options, "input")
        .map(PathBuf::from)
        .ok_or_else(|| anyhow::anyhow!("--input DIR is required."))?;
    let bundle = read_bundle(&input)?;
    let identities = read_identity_map(&bundle)?;
    let plan = build_plan(&bundle, &identities)?;
    if !plan.validation_issues.is_empty() {
        bail!("The source archive has unresolved validation issues.");
    }
    let as_of = option(&options, "as-of")
        .ok_or_else(|| anyhow::anyhow!("--as-of YYYY-MM-DD is required."))?;
    if !valid_date(&as_of)
        || as_of.get(..7) != Some(bundle.manifest.current_canonical_month.as_str())
    {
        bail!("--as-of must fall within the source archive's current canonical month.");
    }
    let database = database_path(&options);
    let pool = open_pool(&database, true).await?;
    let import_count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM finance_import_runs WHERE source_digest = ?",
    )
    .bind(&bundle.manifest.export_sha256)
    .fetch_one(&pool)
    .await?;
    let total_imports = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM finance_import_runs")
        .fetch_one(&pool)
        .await?;
    if import_count != 1 || total_imports != 1 {
        bail!("Read-model comparison requires one import matching the source archive.");
    }
    let read_model = finance_read_model::load(&pool, &as_of, 200).await?;
    let comparison =
        compare_read_model_snapshot(&read_model, &plan, &bundle.manifest.current_canonical_month)?;
    println!(
        "Source archive read-model comparison completed: {}",
        json!({
            "sourceDigestPrefix": bundle.manifest.export_sha256.chars().take(8).collect::<String>(),
            "asOf": as_of,
            "comparison": comparison,
            "databaseRowsChanged": false,
            "workbookWrites": 0,
        })
    );
    pool.close().await;
    Ok(())
}

async fn shadow_simulate_command(options: HashMap<String, String>) -> anyhow::Result<()> {
    use finance_model::{
        POSTING_ROUNDING_MODE, RateObservation, RoundingMode, TransactionEffect,
        calculate_valuation, project_open_budget, project_transaction, select_prior_rate,
    };

    let input = option(&options, "input")
        .map(PathBuf::from)
        .ok_or_else(|| anyhow::anyhow!("--input DIR is required."))?;
    let bundle = read_bundle(&input)?;
    if !bundle.manifest.test_workbook
        || bundle.manifest.source_spreadsheet_id != "SANITIZED-PHASE-B-FIXTURE"
    {
        bail!("Shadow simulation only accepts the checked-in sanitized test fixture.");
    }
    let identities = read_identity_map(&bundle)?;
    let plan = build_plan(&bundle, &identities)?;
    if !plan.validation_issues.is_empty() {
        bail!("The sanitized fixture has unresolved source validation issues.");
    }
    let database = database_path(&options);
    let as_of = option(&options, "as-of")
        .ok_or_else(|| anyhow::anyhow!("--as-of YYYY-MM-DD is required."))?;
    if as_of != "2026-09-08" {
        bail!("The sanitized fixture simulation uses the fixed date 2026-09-08.");
    }
    let pool = open_pool(&database, true).await?;
    let fixture_imports = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM finance_import_runs WHERE json_extract(manifest_json, '$.sourceSpreadsheetId') = 'SANITIZED-PHASE-B-FIXTURE'",
    )
    .fetch_one(&pool)
    .await?;
    let total_imports = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM finance_import_runs")
        .fetch_one(&pool)
        .await?;
    if fixture_imports != 1 || total_imports != 1 {
        bail!("Shadow simulation only runs against one imported sanitized fixture.");
    }
    let read_model = finance_read_model::load(&pool, &as_of, 50).await?;
    finance_read_model::assert_fixture_snapshot(&read_model)?;
    let snapshot_comparison =
        compare_read_model_snapshot(&read_model, &plan, &bundle.manifest.current_canonical_month)?;

    let accounts = sqlx::query("SELECT id, source_name, currency, is_real FROM finance_accounts")
        .fetch_all(&pool)
        .await?;
    let mut account_ids = HashMap::new();
    let mut account_currencies = HashMap::new();
    let mut real_account_ids = HashSet::new();
    for row in accounts {
        let id = row.try_get::<String, _>("id")?;
        let name = row.try_get::<String, _>("source_name")?;
        let currency = row.try_get::<Option<String>, _>("currency")?;
        if row.try_get::<bool, _>("is_real")? {
            real_account_ids.insert(id.clone());
        }
        account_ids.insert(name, id.clone());
        account_currencies.insert(id, currency.unwrap_or_else(|| "EUR".into()));
    }
    let categories = sqlx::query("SELECT id, name, is_real FROM finance_categories")
        .fetch_all(&pool)
        .await?;
    let mut category_ids = HashMap::new();
    let mut real_category_ids = HashSet::new();
    for row in categories {
        let id = row.try_get::<String, _>("id")?;
        let name = row.try_get::<String, _>("name")?;
        if row.try_get::<bool, _>("is_real")? {
            real_category_ids.insert(id.clone());
        }
        category_ids.insert(name, id);
    }
    let account = |name: &str| -> anyhow::Result<String> {
        account_ids
            .get(name)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("Sanitized fixture is missing a required account."))
    };
    let category = |name: &str| -> anyhow::Result<String> {
        category_ids
            .get(name)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("Sanitized fixture is missing a required category."))
    };
    let account_currency = |id: &str| -> anyhow::Result<String> {
        account_currencies
            .get(id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("Sanitized fixture account currency is missing."))
    };

    // Synthetic daily ECB quotes exercise prior-date lookup, cross-currency
    // transfers, a foreign-currency amount override, and the stale-rate flag.
    let observations = vec![
        RateObservation {
            currency: "PHP".into(),
            date: "2026-08-31".into(),
            currency_units_per_eur: "56.10".into(),
        },
        RateObservation {
            currency: "USD".into(),
            date: "2026-08-31".into(),
            currency_units_per_eur: "1.09".into(),
        },
        RateObservation {
            currency: "PHP".into(),
            date: "2026-09-07".into(),
            currency_units_per_eur: "56.30".into(),
        },
        RateObservation {
            currency: "USD".into(),
            date: "2026-09-07".into(),
            currency_units_per_eur: "1.10".into(),
        },
    ];
    let eur_account = account("Fixture EUR account")?;
    let usd_account = account("Fixture USD account")?;
    let groceries = category("Fixture Groceries")?;
    let savings = category("Fixture Savings")?;
    let expense_account = account("Expense")?;
    let income_account = account("Income")?;
    let reallocation_account = account("Reallocation")?;
    let reconciliation_account = account("Reconciliation")?;
    let income_category = category("Income")?;
    let expense_category = category("Expense")?;
    let transfer_category = category("Fixture Transfers")?;
    let reconciliation_category = category("Reconciliation")?;

    struct Scenario {
        kind: &'static str,
        amount: &'static str,
        input_currency: &'static str,
        to_amount: &'static str,
        to_account_currency: &'static str,
        from_account: Option<String>,
        to_account: Option<String>,
        from_category: Option<String>,
        to_category: Option<String>,
    }
    let scenarios = vec![
        Scenario {
            kind: "expense",
            amount: "3.00",
            input_currency: "EUR",
            to_amount: "3.00",
            to_account_currency: "EUR",
            from_account: Some(eur_account.clone()),
            to_account: Some(expense_account.clone()),
            from_category: Some(groceries.clone()),
            to_category: Some(expense_category),
        },
        Scenario {
            kind: "income",
            amount: "5.00",
            input_currency: "EUR",
            to_amount: "5.00",
            to_account_currency: "EUR",
            from_account: Some(income_account),
            to_account: Some(eur_account.clone()),
            from_category: Some(income_category),
            to_category: Some(groceries.clone()),
        },
        Scenario {
            kind: "transfer",
            amount: "10.00",
            input_currency: "EUR",
            to_amount: "10.75",
            to_account_currency: "USD",
            from_account: Some(eur_account.clone()),
            to_account: Some(usd_account.clone()),
            from_category: Some(transfer_category.clone()),
            to_category: Some(transfer_category.clone()),
        },
        Scenario {
            kind: "reallocate",
            amount: "2.50",
            input_currency: "EUR",
            to_amount: "2.50",
            to_account_currency: "EUR",
            from_account: Some(reallocation_account.clone()),
            to_account: Some(reallocation_account),
            from_category: Some(groceries.clone()),
            to_category: Some(savings.clone()),
        },
        Scenario {
            kind: "expense",
            amount: "4.07",
            input_currency: "USD",
            to_amount: "4.07",
            to_account_currency: "EUR",
            from_account: Some(eur_account.clone()),
            to_account: Some(expense_account),
            from_category: Some(groceries.clone()),
            to_category: Some(category("Expense")?),
        },
        Scenario {
            kind: "expense",
            amount: "56.30",
            input_currency: "PHP",
            to_amount: "56.30",
            to_account_currency: "EUR",
            from_account: Some(eur_account.clone()),
            to_account: Some(account("Expense")?),
            from_category: Some(groceries.clone()),
            to_category: Some(category("Expense")?),
        },
        Scenario {
            kind: "reconciliation",
            amount: "-1.25",
            input_currency: "EUR",
            to_amount: "0",
            to_account_currency: "EUR",
            from_account: Some(eur_account.clone()),
            to_account: Some(reconciliation_account),
            from_category: Some(reconciliation_category.clone()),
            to_category: Some(reconciliation_category),
        },
        Scenario {
            kind: "reconciliation",
            amount: "0",
            input_currency: "EUR",
            to_amount: "0",
            to_account_currency: "EUR",
            from_account: Some(eur_account.clone()),
            to_account: Some(account("Reconciliation")?),
            from_category: Some(category("Reconciliation")?),
            to_category: Some(category("Reconciliation")?),
        },
    ];

    let mut account_deltas: HashMap<String, String> = HashMap::new();
    let mut category_deltas: HashMap<String, String> = HashMap::new();
    let mut reallocation_deltas: HashMap<String, String> = HashMap::new();
    let mut valuation_count = 0usize;
    let mut formula_parity_scenarios = 0usize;
    for scenario in scenarios {
        let from_currency = scenario
            .from_account
            .as_deref()
            .map(&account_currency)
            .transpose()?
            .unwrap_or_else(|| "EUR".into());
        let destination_currency = scenario
            .to_account
            .as_deref()
            .map(&account_currency)
            .transpose()?
            .unwrap_or_else(|| scenario.to_account_currency.to_string());
        let valuation = calculate_valuation(
            scenario.amount,
            scenario.input_currency,
            &from_currency,
            scenario.to_amount,
            &destination_currency,
            &as_of,
            &observations,
            POSTING_ROUNDING_MODE,
        )
        .map_err(|_| anyhow::anyhow!("A sanitized projection scenario could not be valued."))?;
        let formula_result = formula_reference_valuation(
            scenario.amount,
            scenario.input_currency,
            &from_currency,
            scenario.to_amount,
            &destination_currency,
            &as_of,
            &observations,
        )?;
        if valuation.from.amount_native != formula_result.from_amount_native
            || valuation.from.amount_eur != formula_result.from_amount_eur
            || valuation.to.amount_native != formula_result.to_amount_native
            || valuation.to.amount_eur != formula_result.to_amount_eur
        {
            bail!("The DB valuation differs from the independent sheet-formula calculation.");
        }
        formula_parity_scenarios += 1;
        if valuation.from.exchange_to_eur_numerator.is_empty()
            || valuation.from.exchange_to_native_numerator.is_empty()
            || valuation.from.exchange_to_eur_denominator == "0"
            || valuation.from.exchange_to_native_denominator == "0"
            || valuation.to.exchange_to_eur_numerator.is_empty()
            || valuation.to.exchange_to_eur_denominator == "0"
            || valuation.from.applied_rate_dates.is_empty()
            || valuation.to.applied_rate_dates.is_empty()
            || valuation.from.stale
            || valuation.to.stale
        {
            bail!("A fresh-rate rational valuation is missing its audit inputs.");
        }
        if scenario.kind == "transfer"
            && (valuation.to.amount_native != scenario.to_amount || valuation.to.amount_eur == "10")
        {
            bail!("The transfer projection did not preserve its actual received amount.");
        }
        if scenario.kind == "expense"
            && scenario.input_currency == "USD"
            && valuation.from.amount_native != "3.77"
        {
            bail!("The USD source-currency override projection failed.");
        }
        if scenario.kind == "expense"
            && scenario.input_currency == "PHP"
            && valuation.from.amount_native != "1.02"
        {
            bail!("The PHP source-currency override projection failed.");
        }
        valuation_count += 2;
        let projection = project_transaction(&TransactionEffect {
            from_account_id: scenario.from_account,
            to_account_id: scenario.to_account,
            from_category_id: scenario.from_category,
            to_category_id: scenario.to_category,
            kind: scenario.kind.into(),
            amount: scenario.amount.into(),
            valuation,
        })
        .map_err(|_| anyhow::anyhow!("A sanitized projection scenario could not be applied."))?;
        if scenario.kind == "reallocate" {
            for (id, delta) in &projection.account_deltas {
                if real_account_ids.contains(id) && !decimal_is_zero(delta)? {
                    bail!("Budget reallocation changed a real account.");
                }
            }
        }
        for (key, delta) in projection.account_deltas {
            let previous = account_deltas.get(&key).map(String::as_str).unwrap_or("0");
            account_deltas.insert(key, add_exact_decimals(previous, &delta)?);
        }
        for (key, delta) in projection.category_expense_deltas {
            let previous = category_deltas.get(&key).map(String::as_str).unwrap_or("0");
            category_deltas.insert(key, add_exact_decimals(previous, &delta)?);
        }
        for (key, delta) in projection.reallocation_impacts {
            let previous = reallocation_deltas
                .get(&key)
                .map(String::as_str)
                .unwrap_or("0");
            reallocation_deltas.insert(key, add_exact_decimals(previous, &delta)?);
        }
    }

    if real_category_ids.contains(&transfer_category) {
        bail!("Transfer projection changed a budget category.");
    }
    let allocation_total = reallocation_deltas
        .values()
        .try_fold("0".to_string(), |sum, value| {
            add_exact_decimals(&sum, value)
        })?;
    if !decimal_is_zero(&allocation_total)? {
        bail!("Reallocation projection did not conserve the total budget.");
    }
    if account_deltas.get(&eur_account).map(String::as_str) != Some("-11.54")
        || account_deltas.get(&usd_account).map(String::as_str) != Some("10.75")
        || category_deltas.get(&groceries).map(String::as_str) != Some("5.29")
        || category_deltas.get(&savings).map(String::as_str) != Some("-2.5")
        || reallocation_deltas.get(&groceries).map(String::as_str) != Some("2.5")
        || reallocation_deltas.get(&savings).map(String::as_str) != Some("-2.5")
    {
        bail!("A representative transaction projection differs from its expected result.");
    }

    let mut budgets_projected = 0usize;
    let mut frozen_budgets_checked = 0usize;
    for budget in &read_model.budgets {
        let Some(expense) = budget.expense_eur.as_deref() else {
            continue;
        };
        let Some(remaining) = budget.remaining_eur.as_deref() else {
            continue;
        };
        let delta = category_deltas
            .get(&budget.category_id)
            .map(String::as_str)
            .unwrap_or("0");
        let (open_expense, open_remaining) = project_open_budget(&expense, &remaining, delta, true)
            .map_err(|_| anyhow::anyhow!("Open-month budget projection failed."))?;
        let (closed_expense, closed_remaining) =
            project_open_budget(&expense, &remaining, delta, false)
                .map_err(|_| anyhow::anyhow!("Closed-month projection failed."))?;
        if expense != closed_expense || remaining != closed_remaining {
            bail!("Closed-month projection changed a frozen workbook value.");
        }
        let _ = (open_expense, open_remaining);
        budgets_projected += 1;
        frozen_budgets_checked += 1;
    }
    let prior = select_prior_rate("USD", "2026-09-08", &observations)
        .map_err(|_| anyhow::anyhow!("Prior-rate scenario failed."))?;
    let stale = select_prior_rate("USD", "2026-09-14", &observations)
        .map_err(|_| anyhow::anyhow!("Stale-rate scenario failed."))?;
    if prior.date.as_deref() != Some("2026-09-07") || stale.stale_business_days <= 3 {
        bail!("Prior or stale FX-rate scenario failed.");
    }
    let holiday_observation = RateObservation {
        currency: "USD".into(),
        date: "2026-04-02".into(),
        currency_units_per_eur: "1.10".into(),
    };
    let easter_holiday_rate = select_prior_rate("USD", "2026-04-07", &[holiday_observation])
        .map_err(|_| anyhow::anyhow!("TARGET holiday freshness scenario failed."))?;
    if easter_holiday_rate.stale_business_days != 1 {
        bail!("TARGET closing days were counted as ECB publication days.");
    }
    let half_away = calculate_valuation(
        "1.005",
        "EUR",
        "EUR",
        "0",
        "EUR",
        &as_of,
        &observations,
        RoundingMode::HalfAwayFromZero,
    )
    .map_err(|_| anyhow::anyhow!("Half-away rounding projection failed."))?;
    let half_even = calculate_valuation(
        "1.005",
        "EUR",
        "EUR",
        "0",
        "EUR",
        &as_of,
        &observations,
        RoundingMode::HalfEven,
    )
    .map_err(|_| anyhow::anyhow!("Half-even rounding projection failed."))?;
    let toward_zero = calculate_valuation(
        "1.005",
        "EUR",
        "EUR",
        "0",
        "EUR",
        &as_of,
        &observations,
        RoundingMode::TowardZero,
    )
    .map_err(|_| anyhow::anyhow!("Toward-zero rounding projection failed."))?;
    if half_away.from.amount_native != "1.01"
        || half_even.from.amount_native != "1"
        || toward_zero.from.amount_native != "1"
    {
        bail!("A rational rounding-mode scenario failed.");
    }
    let mut accounts_projected = 0usize;
    for (account_id, delta) in &account_deltas {
        let original = sqlx::query_scalar::<_, Option<String>>(
            "SELECT current_native_balance FROM finance_accounts WHERE id = ?",
        )
        .bind(account_id)
        .fetch_one(&pool)
        .await?;
        if let Some(original) = original {
            let _after = add_exact_decimals(&original, delta)?;
            accounts_projected += 1;
        }
    }

    println!(
        "Sanitized shadow projection completed: {}",
        json!({
            "scenarios": 8,
            "valuationSides": valuation_count,
            "formulaParityScenarios": formula_parity_scenarios,
            "readModelTransactions": read_model.recent_transactions.len(),
            "readModelRealAccounts": read_model.accounts.len(),
            "readModelRealBudgetCategories": read_model.budgets.len(),
            "readModelBudgetMonths": read_model.budget_history.len(),
            "snapshotComparison": snapshot_comparison,
            "accountsProjected": accounts_projected,
            "openBudgetsProjected": budgets_projected,
            "closedBudgetsKeptFixed": frozen_budgets_checked,
            "staleRateWarningChecked": true,
            "targetHolidayCalendarChecked": true,
            "rationalRateAuditInputsChecked": true,
            "roundingModesExercised": 3,
            "transferBudgetInvariant": true,
            "reallocationConservationInvariant": true,
            "databaseRowsChanged": false,
            "workbookWrites": 0,
        })
    );
    pool.close().await;
    Ok(())
}

struct FormulaReferenceValuation {
    from_amount_native: String,
    from_amount_eur: String,
    to_amount_native: String,
    to_amount_eur: String,
}

struct FormulaRatio {
    numerator: BigInt,
    denominator: BigInt,
}

impl FormulaRatio {
    fn new(numerator: BigInt, denominator: BigInt) -> anyhow::Result<Self> {
        if denominator == BigInt::from(0u8) {
            bail!("The sheet-formula oracle encountered a zero denominator.");
        }
        if denominator < BigInt::from(0u8) {
            Ok(Self {
                numerator: -numerator,
                denominator: -denominator,
            })
        } else {
            Ok(Self {
                numerator,
                denominator,
            })
        }
    }

    fn one() -> Self {
        Self {
            numerator: BigInt::from(1u8),
            denominator: BigInt::from(1u8),
        }
    }

    fn from_decimal(value: &str) -> anyhow::Result<Self> {
        let decimal = parse_exact_decimal(value)?;
        let mut numerator = decimal.digits.parse::<BigInt>()?;
        if decimal.negative {
            numerator = -numerator;
        }
        let scale =
            u32::try_from(decimal.scale).context("sheet-formula decimal scale is out of range")?;
        Self::new(numerator, BigInt::from(10u8).pow(scale))
    }

    fn multiply(&self, other: &Self) -> anyhow::Result<Self> {
        Self::new(
            &self.numerator * &other.numerator,
            &self.denominator * &other.denominator,
        )
    }

    fn divide(&self, other: &Self) -> anyhow::Result<Self> {
        Self::new(
            &self.numerator * &other.denominator,
            &self.denominator * &other.numerator,
        )
    }

    fn round_half_even_2(&self) -> anyhow::Result<String> {
        let scaled = &self.numerator * BigInt::from(100u8);
        let negative = scaled < BigInt::from(0u8);
        let magnitude = if negative { -scaled } else { scaled };
        let mut quotient = &magnitude / &self.denominator;
        let remainder = &magnitude % &self.denominator;
        let twice_remainder = &remainder * BigInt::from(2u8);
        let exact_half = twice_remainder == self.denominator;
        let round_up = twice_remainder > self.denominator
            || (exact_half && (&quotient % BigInt::from(2u8)) != BigInt::from(0u8));
        if round_up {
            quotient += BigInt::from(1u8);
        }
        let rounded = if negative { -quotient } else { quotient };
        let decimal_negative = rounded < BigInt::from(0u8);
        let digits = if decimal_negative {
            (-rounded).to_string()
        } else {
            rounded.to_string()
        };
        Ok(format_exact_decimal(ExactDecimal {
            negative: decimal_negative,
            digits,
            scale: 2,
        }))
    }
}

fn formula_reference_quote(
    currency: &str,
    as_of: &str,
    observations: &[finance_model::RateObservation],
) -> anyhow::Result<FormulaRatio> {
    if currency.eq_ignore_ascii_case("EUR") {
        return Ok(FormulaRatio::one());
    }
    let observation = observations
        .iter()
        .filter(|rate| rate.currency.eq_ignore_ascii_case(currency) && rate.date.as_str() <= as_of)
        .max_by(|left, right| left.date.cmp(&right.date))
        .ok_or_else(|| anyhow::anyhow!("The formula oracle has no prior FX observation."))?;
    let quote = FormulaRatio::from_decimal(&observation.currency_units_per_eur)?;
    if quote.numerator <= BigInt::from(0u8) {
        bail!("The formula oracle requires positive FX observations.");
    }
    Ok(quote)
}

/// Independent arithmetic for the sheet's K:N and P:Q valuation formulas.
/// The ECB observations are EUR-based, so cross-currency amounts derive their
/// multiplier from the two EUR quotes before the owner's half-even posting
/// rounding is applied.
fn formula_reference_valuation(
    amount: &str,
    input_currency: &str,
    from_account_currency: &str,
    to_amount: &str,
    to_account_currency: &str,
    as_of: &str,
    observations: &[finance_model::RateObservation],
) -> anyhow::Result<FormulaReferenceValuation> {
    let amount = FormulaRatio::from_decimal(amount)?;
    let received = FormulaRatio::from_decimal(to_amount)?;
    let input_currency = input_currency.to_ascii_uppercase();
    let from_account_currency = from_account_currency.to_ascii_uppercase();
    let to_account_currency = to_account_currency.to_ascii_uppercase();
    let input_quote = formula_reference_quote(&input_currency, as_of, observations)?;
    let from_quote = formula_reference_quote(&from_account_currency, as_of, observations)?;
    let to_quote = formula_reference_quote(&to_account_currency, as_of, observations)?;
    let source_adjustment = if input_currency == "EUR" {
        FormulaRatio::one()
    } else {
        FormulaRatio::from_decimal("1.02")?
    };

    let from_eur_rate = source_adjustment.divide(&input_quote)?;
    let from_native_rate = if input_currency == from_account_currency {
        FormulaRatio::one()
    } else {
        source_adjustment
            .multiply(&from_quote)?
            .divide(&input_quote)?
    };
    let to_eur_rate = FormulaRatio::one().divide(&to_quote)?;

    Ok(FormulaReferenceValuation {
        from_amount_native: amount.multiply(&from_native_rate)?.round_half_even_2()?,
        from_amount_eur: amount.multiply(&from_eur_rate)?.round_half_even_2()?,
        to_amount_native: received.round_half_even_2()?,
        to_amount_eur: received.multiply(&to_eur_rate)?.round_half_even_2()?,
    })
}

fn compare_read_model_snapshot(
    snapshot: &finance_read_model::FinanceSnapshot,
    plan: &ImportPlan,
    current_month: &str,
) -> anyhow::Result<Value> {
    let account_names = plan
        .accounts
        .iter()
        .map(|account| (account.id.clone(), account.name.clone()))
        .collect::<HashMap<_, _>>();
    let category_names = plan
        .categories
        .iter()
        .map(|category| (category.id.clone(), category.name.clone()))
        .collect::<HashMap<_, _>>();

    let expected_accounts = plan
        .accounts
        .iter()
        .filter(|account| account.is_real && account.is_active)
        .collect::<Vec<_>>();
    if snapshot.accounts.len() != expected_accounts.len() {
        bail!("DB account read-model count differs from the source archive.");
    }
    for expected in &expected_accounts {
        let actual = snapshot
            .accounts
            .iter()
            .find(|account| account.id == expected.id)
            .ok_or_else(|| anyhow::anyhow!("DB account read model is missing a source account."))?;
        if actual.name != expected.name
            || actual.account_type != expected.account_type
            || actual.currency != expected.currency
            || actual.balance_native != expected.current_native_balance
            || actual.actual_balance_native != expected.actual_native_balance
            || actual.reconciliation_native != expected.reconciliation_native
            || actual.balance_eur != expected.current_eur_balance
            || actual.actual_balance_eur != expected.actual_eur_balance
        {
            bail!("DB account read model differs from the source archive.");
        }
    }

    let real_categories = plan
        .categories
        .iter()
        .filter(|category| category.is_real)
        .collect::<Vec<_>>();
    let expected_months = plan
        .budgets
        .iter()
        .filter(|budget| budget.month <= snapshot.budget_month)
        .map(|budget| budget.month.clone())
        .collect::<std::collections::BTreeSet<_>>();
    if snapshot.budget_history.len() != expected_months.len() {
        bail!("DB budget-history month count differs from the source archive.");
    }
    let mut historical_budget_cells_compared = 0usize;
    for expected_month in &expected_months {
        let actual_month = snapshot
            .budget_history
            .iter()
            .find(|month| &month.month == expected_month)
            .ok_or_else(|| anyhow::anyhow!("DB budget history is missing a source month."))?;
        if actual_month.categories.len() != real_categories.len() {
            bail!("DB budget-history category count differs from the source archive.");
        }
        for category in &real_categories {
            let actual = actual_month
                .categories
                .iter()
                .find(|item| item.category_id == category.id)
                .ok_or_else(|| {
                    anyhow::anyhow!("DB budget history is missing a source category.")
                })?;
            let expected = plan.budgets.iter().find(|budget| {
                budget.category_id == category.id && budget.month == *expected_month
            });
            let expected_base = (expected_month.as_str() >= current_month)
                .then(|| category.base_monthly_eur.clone())
                .flatten();
            if actual.name != category.name
                || actual.monthly_eur != expected_base
                || actual.expense_eur != expected.and_then(|budget| budget.expense_eur.clone())
                || actual.remaining_eur != expected.and_then(|budget| budget.remaining_eur.clone())
                || actual.reallocation_impact_eur
                    != expected.map(|budget| budget.reallocation_impact_eur.clone())
            {
                bail!("DB budget-history values differ from the source archive.");
            }
            historical_budget_cells_compared += 1;
        }
    }

    if snapshot.transaction_count != plan.transactions.len() {
        bail!("DB transaction read-model count differs from the source archive.");
    }
    let mut expected_recent = plan.transactions.iter().collect::<Vec<_>>();
    expected_recent.sort_by(|left, right| right.source_row.cmp(&left.source_row));
    expected_recent.truncate(snapshot.recent_transactions.len());
    if expected_recent.len() != snapshot.recent_transactions.len() {
        bail!("DB recent-transaction count differs from the source archive.");
    }
    for (expected, actual) in expected_recent.iter().zip(&snapshot.recent_transactions) {
        let expected_from_account = expected
            .from_account_id
            .as_ref()
            .and_then(|id| account_names.get(id));
        let expected_to_account = expected
            .to_account_id
            .as_ref()
            .and_then(|id| account_names.get(id));
        let expected_from_category = expected
            .from_category_id
            .as_ref()
            .and_then(|id| category_names.get(id));
        let expected_to_category = expected
            .to_category_id
            .as_ref()
            .and_then(|id| category_names.get(id));
        if actual.id != expected.id
            || actual.kind != expected.kind
            || actual.effective_date != expected.effective_date
            || actual.description != expected.description
            || actual.amount != expected.amount_decimal
            || actual.input_currency != expected.input_currency
            || actual.to_amount != expected.to_amount_decimal
            || actual.to_currency != expected.to_currency
            || actual.from_account.as_ref() != expected_from_account
            || actual.to_account.as_ref() != expected_to_account
            || actual.from_category.as_ref() != expected_from_category
            || actual.to_category.as_ref() != expected_to_category
            || actual.from_amount_native != row_decimal(&expected.row, 13)
            || actual.from_amount_eur != row_decimal(&expected.row, 11)
            || actual.to_amount_native != row_decimal(&expected.row, 4)
            || actual.to_amount_eur != row_decimal(&expected.row, 16)
        {
            bail!("DB recent-transaction fields differ from the source archive.");
        }
    }

    Ok(json!({
        "accountRowsCompared": expected_accounts.len(),
        "recentTransactionRowsCompared": expected_recent.len(),
        "currentBudgetCategoriesCompared": snapshot.budgets.len(),
        "historicalBudgetCategoryCellsCompared": historical_budget_cells_compared,
        "matched": true,
    }))
}

fn parse_options(args: Vec<String>) -> anyhow::Result<HashMap<String, String>> {
    let mut options = HashMap::new();
    let mut index = 0;
    while index < args.len() {
        let arg = &args[index];
        if matches!(arg.as_str(), "--commit" | "--allow-test-workbook") {
            let key = if arg == "--commit" {
                "commit"
            } else {
                "allow-test-workbook"
            };
            options.insert(key.into(), "true".into());
            index += 1;
            continue;
        }
        let Some(name) = arg.strip_prefix("--") else {
            bail!("Unexpected positional argument.");
        };
        if name.is_empty() || index + 1 >= args.len() || args[index + 1].starts_with("--") {
            bail!("An admin option is missing its value.");
        }
        options.insert(name.to_string(), args[index + 1].clone());
        index += 2;
    }
    Ok(options)
}

fn option(options: &HashMap<String, String>, key: &str) -> Option<String> {
    options.get(key).cloned()
}

fn database_path(options: &HashMap<String, String>) -> PathBuf {
    option(options, "database")
        .map(PathBuf::from)
        .or_else(|| env::var_os("DATABASE_PATH").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../data/presto.sqlite"))
}

fn connect_options(path: &Path, read_only: bool) -> SqliteConnectOptions {
    SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(!read_only)
        .read_only(read_only)
        .foreign_keys(true)
}

async fn open_pool(path: &Path, read_only: bool) -> anyhow::Result<SqlitePool> {
    if read_only && !path.exists() {
        bail!("The selected SQLite database does not exist.");
    }
    sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(connect_options(path, read_only))
        .await
        .context("could not open the selected SQLite database")
}

fn read_bundle(input: &Path) -> anyhow::Result<ArchiveBundle> {
    let manifest_path = input.join("manifest.json");
    let manifest: Manifest = serde_json::from_slice(
        &fs::read(&manifest_path).context("could not read private export manifest")?,
    )
    .context("export manifest schema is invalid")?;
    if manifest.schema_version != 1
        || manifest.canonical_time_zone != "Europe/Madrid"
        || !valid_month(&manifest.current_canonical_month)
    {
        bail!("Export manifest version or canonical timezone is unsupported.");
    }
    if manifest.export_file != "finance-export.json" {
        bail!("Export manifest filename is unsupported.");
    }
    let export_path = input.join(&manifest.export_file);
    let bytes = fs::read(&export_path).context("could not read private finance archive")?;
    if sha256(&bytes) != manifest.export_sha256 {
        bail!("Finance archive digest does not match its manifest.");
    }
    let export: FinanceExport =
        serde_json::from_slice(&bytes).context("finance archive schema is invalid")?;
    if export.schema_version != 1
        || export.source.spreadsheet_id != manifest.source_spreadsheet_id
        || export.canonical_time_zone != manifest.canonical_time_zone
        || export.exported_at != manifest.exported_at
    {
        bail!("Finance archive identity does not match its manifest.");
    }
    for required in ["transactions", "accounts", "budgets", "exchangeRates"] {
        if !export.sheets.contains_key(required) {
            bail!("Finance archive is missing a required sheet section.");
        }
    }
    let review_path = input.join("owner-review-decisions.json");
    let owner_review = if review_path.exists() {
        let review: OwnerReviewDecisions = serde_json::from_slice(
            &fs::read(&review_path).context("could not read private owner-review decisions")?,
        )
        .context("owner-review decisions schema is invalid")?;
        if review.schema_version != 1 || review.source_sha256 != manifest.export_sha256 {
            bail!("Owner-review decisions belong to a different finance archive.");
        }
        for action in review.transaction_rows.values() {
            if !matches!(
                action.as_str(),
                "ignore"
                    | "ignore_destination_amount"
                    | "income"
                    | "from_category_groceries"
                    | "opening_balance"
            ) {
                bail!("Owner-review decisions contain an unsupported transaction action.");
            }
        }
        let mut invalid_account_names = HashSet::new();
        for name in &review.invalid_accounts {
            if name.trim().is_empty() || !invalid_account_names.insert(normalize_name(name)) {
                bail!("Owner-review decisions contain an invalid or duplicate account name.");
            }
        }
        review
    } else {
        OwnerReviewDecisions::default()
    };
    Ok(ArchiveBundle {
        directory: input.to_path_buf(),
        manifest,
        export,
        owner_review,
    })
}

fn identity_map(bundle: &ArchiveBundle) -> anyhow::Result<IdentityMap> {
    let path = bundle.directory.join("identity-map.json");
    let mut identities = if path.exists() {
        let saved: IdentityMap =
            serde_json::from_slice(&fs::read(&path).context("could not read import identity map")?)
                .context("import identity map schema is invalid")?;
        if saved.schema_version != 1 || saved.source_sha256 != bundle.manifest.export_sha256 {
            bail!("Import identity map belongs to a different finance archive.");
        }
        saved
    } else {
        IdentityMap {
            schema_version: 1,
            source_sha256: bundle.manifest.export_sha256.clone(),
            import_run_id: Uuid::now_v7().to_string(),
            ..IdentityMap::default()
        }
    };

    let accounts = bundle
        .export
        .sheets
        .get("accounts")
        .expect("validated archive");
    let account_headers = header_map(accounts);
    let account_name_column = required_column(&account_headers, "Name", "Accounts")?;
    for row in accounts.rows.iter().skip(1) {
        if let Some(name) = row_text(row, account_name_column).filter(|name| !name.is_empty()) {
            identities.account_ids.entry(name).or_insert_with(new_id);
        }
    }

    let budgets = bundle
        .export
        .sheets
        .get("budgets")
        .expect("validated archive");
    let budget_headers = header_map(budgets);
    let category_name_column = required_column(&budget_headers, "Category", "Expenses v3")
        .or_else(|_| Ok::<usize, anyhow::Error>(0))?;
    for row in budgets.rows.iter().skip(2) {
        if let Some(name) = row_text(row, category_name_column).filter(|name| !name.is_empty()) {
            identities.category_ids.entry(name).or_insert_with(new_id);
        }
    }

    let transactions = bundle
        .export
        .sheets
        .get("transactions")
        .expect("validated archive");
    for (index, row) in transactions.rows.iter().enumerate().skip(1) {
        if !transaction_inputs_populated(row) {
            continue;
        }
        let source_row = index + 1;
        if bundle
            .owner_review
            .transaction_rows
            .get(&source_row.to_string())
            .is_some_and(|action| action == "ignore")
        {
            continue;
        }
        let existing_id = row_text(row, 17).filter(|value| !value.is_empty());
        if existing_id.is_none() {
            identities
                .transaction_ids
                .entry(source_row.to_string())
                .or_insert_with(new_id);
        }
    }
    save_identity_map(&path, &identities)?;
    Ok(identities)
}

fn read_identity_map(bundle: &ArchiveBundle) -> anyhow::Result<IdentityMap> {
    let path = bundle.directory.join("identity-map.json");
    let saved: IdentityMap =
        serde_json::from_slice(&fs::read(&path).context("could not read import identity map")?)
            .context("import identity map schema is invalid")?;
    if saved.schema_version != 1 || saved.source_sha256 != bundle.manifest.export_sha256 {
        bail!("Import identity map belongs to a different finance archive.");
    }
    Ok(saved)
}

fn save_identity_map(path: &Path, identities: &IdentityMap) -> anyhow::Result<()> {
    let temporary = path.with_extension("json.tmp");
    let contents = serde_json::to_vec_pretty(identities)?;
    fs::write(&temporary, contents).context("could not persist import identity map")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&temporary, fs::Permissions::from_mode(0o600))?;
    }
    fs::rename(&temporary, path).context("could not atomically save import identity map")?;
    Ok(())
}

fn new_id() -> String {
    Uuid::now_v7().to_string()
}

fn build_plan(bundle: &ArchiveBundle, identities: &IdentityMap) -> anyhow::Result<ImportPlan> {
    let accounts_sheet = bundle
        .export
        .sheets
        .get("accounts")
        .expect("validated archive");
    let account_headers = header_map(accounts_sheet);
    let account_columns = AccountColumns {
        name: required_column(&account_headers, "Name", "Accounts")?,
        account_type: required_column(&account_headers, "Type", "Accounts")?,
        is_real: required_column(&account_headers, "IsReal", "Accounts")?,
        currency: required_column(&account_headers, "Currency", "Accounts")?,
        is_active: account_headers
            .get("IsActive")
            .or_else(|| account_headers.get("IsLive"))
            .copied()
            .ok_or_else(|| anyhow::anyhow!("Accounts is missing IsActive/IsLive."))?,
        native: required_column(&account_headers, "Balance", "Accounts")?,
        actual_native: required_column(&account_headers, "Actual Balance", "Accounts")?,
        reconciliation: required_column(&account_headers, "Reconciliation Amount", "Accounts")?,
        eur: required_column(&account_headers, "Balance (EUR)", "Accounts")?,
        actual_eur: required_column(&account_headers, "Actual Balance (EUR)", "Accounts")?,
    };
    let mut account_imports = Vec::new();
    let mut account_checkpoint_imports = Vec::new();
    let mut account_checkpoint_issues = Vec::new();
    let mut account_names = HashMap::new();
    let mut account_lookup = HashMap::new();
    let mut account_real_by_id = HashMap::new();
    let mut account_type_by_id = HashMap::new();
    let mut invalid_account_ids = HashSet::new();
    let mut matched_invalid_account_names = HashSet::new();
    for (index, row) in accounts_sheet.rows.iter().enumerate().skip(1) {
        let Some(name) = row_text(row, account_columns.name).filter(|value| !value.is_empty())
        else {
            continue;
        };
        let Some(id) = identities.account_ids.get(&name) else {
            bail!("Accounts identity map is missing a source account.");
        };
        let currency = row_text(row, account_columns.currency)
            .filter(|value| !value.is_empty())
            .map(|value| value.to_ascii_uppercase());
        if currency
            .as_deref()
            .is_some_and(|value| !supported_currency(value))
        {
            bail!("Accounts row {} has an unsupported currency.", index + 1);
        }
        if account_names.insert(name.clone(), id.clone()).is_some() {
            bail!(
                "Accounts contains duplicate source names at or before row {}.",
                index + 1
            );
        }
        if account_lookup
            .insert(normalize_name(&name), id.clone())
            .is_some()
        {
            bail!("Accounts contains names that differ only by case.");
        }
        let is_real = row_flag(row, account_columns.is_real);
        let account_is_reviewed_invalid = bundle
            .owner_review
            .invalid_accounts
            .iter()
            .any(|review_name| review_name.eq_ignore_ascii_case(&name));
        if account_is_reviewed_invalid {
            invalid_account_ids.insert(id.clone());
            matched_invalid_account_names.insert(normalize_name(&name));
        }
        account_real_by_id.insert(id.clone(), is_real);
        account_type_by_id.insert(
            id.clone(),
            row_text(row, account_columns.account_type).unwrap_or_default(),
        );
        account_imports.push(AccountImport {
            id: id.clone(),
            name,
            account_type: row_text(row, account_columns.account_type),
            currency,
            is_real,
            is_active: row_flag(row, account_columns.is_active) && !account_is_reviewed_invalid,
            current_native_balance: row_decimal(row, account_columns.native),
            actual_native_balance: row_decimal(row, account_columns.actual_native),
            reconciliation_native: row_decimal(row, account_columns.reconciliation),
            current_eur_balance: row_decimal(row, account_columns.eur),
            actual_eur_balance: row_decimal(row, account_columns.actual_eur),
            source_row: index + 1,
            source_row_json: encode(row)?,
        });
    }
    if bundle
        .owner_review
        .invalid_accounts
        .iter()
        .any(|name| !matched_invalid_account_names.contains(&normalize_name(name)))
    {
        bail!("Owner-review decision names an account absent from this archive.");
    }

    let account_checkpoints_present = bundle.export.sheets.contains_key("accountCheckpoints");
    if let Some(checkpoints_sheet) = bundle.export.sheets.get("accountCheckpoints") {
        if checkpoints_sheet.column_count != 35 {
            bail!("Accounts historical checkpoint range must contain exactly W:BE.");
        }
        let checkpoint_headers = checkpoints_sheet.rows.first().cloned().unwrap_or_default();
        let blank_checkpoint_row = vec![None; 35];
        for (index, account_row) in accounts_sheet.rows.iter().enumerate().skip(1) {
            let row = checkpoints_sheet
                .rows
                .get(index)
                .unwrap_or(&blank_checkpoint_row);
            let has_source_data = row.iter().any(|cell| cell_state(cell.as_ref()) != "blank");
            let account_name =
                row_text(account_row, account_columns.name).filter(|name| !name.is_empty());
            if account_name.is_none() {
                if has_source_data {
                    account_checkpoint_issues.push(ImportIssue {
                        source_sheet: "Accounts",
                        source_row: index + 1,
                        reason_code: "historical_checkpoint_without_account",
                    });
                }
                continue;
            }
            let name = account_name.expect("checked account name");
            let Some(account_id) = account_names.get(&name) else {
                bail!("Accounts historical checkpoint row has no imported account.");
            };
            for column in 0..35 {
                let header_cell = checkpoint_headers.get(column).and_then(Option::as_ref);
                let source_cell = row.get(column).and_then(Option::as_ref);
                let checkpoint_date = header_cell.and_then(sheet_date);
                if checkpoint_date.is_none() && cell_state(source_cell) != "blank" {
                    account_checkpoint_issues.push(ImportIssue {
                        source_sheet: "Accounts",
                        source_row: index + 1,
                        reason_code: "historical_checkpoint_without_valid_date",
                    });
                }
                account_checkpoint_imports.push(AccountCheckpointImport {
                    account_id: account_id.clone(),
                    checkpoint_date,
                    balance_decimal: row_decimal(row, column),
                    source_state: cell_state(source_cell),
                    source_header_json: encode(&header_cell)?,
                    source_cell_json: encode(&source_cell)?,
                    source_row: index + 1,
                    source_column: column_name(23 + column),
                });
            }
        }
        for (index, row) in checkpoints_sheet
            .rows
            .iter()
            .enumerate()
            .skip(accounts_sheet.rows.len())
        {
            if row.iter().any(|cell| cell_state(cell.as_ref()) != "blank") {
                account_checkpoint_issues.push(ImportIssue {
                    source_sheet: "Accounts",
                    source_row: index + 1,
                    reason_code: "historical_checkpoint_without_account",
                });
            }
        }
    }

    let budgets_sheet = bundle
        .export
        .sheets
        .get("budgets")
        .expect("validated archive");
    let budget_headers = header_map(budgets_sheet);
    let category_name_column = budget_headers.get("Category").copied().unwrap_or(0);
    let is_real_column = required_column(&budget_headers, "IsReal", "Expenses v3")?;
    let mut categories = Vec::new();
    let mut category_ids = HashMap::new();
    let mut category_lookup = HashMap::new();
    for (index, row) in budgets_sheet.rows.iter().enumerate().skip(2) {
        let Some(name) = row_text(row, category_name_column).filter(|value| !value.is_empty())
        else {
            continue;
        };
        let Some(id) = identities.category_ids.get(&name) else {
            bail!("Expenses v3 identity map is missing a category.");
        };
        if category_ids.insert(name.clone(), id.clone()).is_some() {
            bail!(
                "Expenses v3 contains duplicate category names at or before row {}.",
                index + 1
            );
        }
        if category_lookup
            .insert(normalize_name(&name), id.clone())
            .is_some()
        {
            bail!("Expenses v3 contains category names that differ only by case.");
        }
        let base_cell = row.get(1).cloned().flatten();
        categories.push(CategoryImport {
            id: id.clone(),
            name,
            is_real: row_flag(row, is_real_column),
            base_monthly_eur: row_decimal(row, 1),
            source_row: index + 1,
            source_row_json: encode(row)?,
            base_cell_json: encode(&base_cell)?,
        });
    }

    let transactions_sheet = bundle
        .export
        .sheets
        .get("transactions")
        .expect("validated archive");
    let mut transactions = Vec::new();
    let mut quarantined = Vec::new();
    let mut account_opening_balances = Vec::new();
    let mut opening_account_ids = HashSet::new();
    let mut seen_ids = HashSet::new();
    let mut errors: BTreeMap<&'static str, Vec<usize>> = BTreeMap::new();
    let mut opening_review_warnings = Vec::new();
    for row_key in bundle.owner_review.transaction_rows.keys() {
        let row_number = row_key
            .parse::<usize>()
            .ok()
            .filter(|row_number| *row_number > 1)
            .ok_or_else(|| {
                anyhow::anyhow!("Owner-review decisions contain an invalid source row.")
            })?;
        let Some(row) = transactions_sheet.rows.get(row_number - 1) else {
            bail!("Owner-review decision does not point to a populated transaction row.");
        };
        if !transaction_inputs_populated(row) {
            bail!("Owner-review decision does not point to a populated transaction row.");
        }
        if bundle
            .owner_review
            .transaction_rows
            .get(row_key)
            .map(String::as_str)
            == Some("opening_balance")
            && (row
                .first()
                .and_then(Option::as_ref)
                .and_then(sheet_date)
                .as_deref()
                != Some("2025-05-01")
                || !row_text(row, 1)
                    .is_some_and(|description| description.eq_ignore_ascii_case("Opening Balance")))
        {
            bail!("Owner-reviewed opening balance does not match a May 1 opening row.");
        }
    }
    for (index, row) in transactions_sheet.rows.iter().enumerate().skip(1) {
        if !transaction_inputs_populated(row) {
            continue;
        }
        let source_row = index + 1;
        let row_json = encode(row)?;
        let row_fingerprint = sha256(row_json.as_bytes());
        let owner_action = bundle
            .owner_review
            .transaction_rows
            .get(&source_row.to_string())
            .map(String::as_str);
        if owner_action == Some("ignore") {
            quarantined.push(QuarantineImport {
                source_row,
                reason: "owner_reviewed_ignore",
                fingerprint: row_fingerprint,
                row_json,
            });
            continue;
        }
        let source_id = row_text(row, 17).filter(|value| !value.is_empty());
        let id = if let Some(source_id) = source_id {
            if Uuid::parse_str(&source_id).is_err() {
                record_error(&mut errors, "invalid_transaction_id", source_row);
                continue;
            }
            source_id
        } else {
            let Some(id) = identities.transaction_ids.get(&source_row.to_string()) else {
                bail!("Transaction identity map is missing a historical ID.");
            };
            id.clone()
        };
        if !seen_ids.insert(id.clone()) {
            record_error(&mut errors, "duplicate_transaction_id", source_row);
            continue;
        }
        let latest_date = format!("{}-31", bundle.manifest.current_canonical_month);
        let Some(date) = row
            .get(0)
            .and_then(Option::as_ref)
            .and_then(sheet_date)
            .filter(|date| date.as_str() >= "1900-01-01" && date.as_str() <= latest_date.as_str())
        else {
            quarantined.push(QuarantineImport {
                source_row,
                reason: "missing_or_invalid_effective_date",
                fingerprint: row_fingerprint,
                row_json,
            });
            continue;
        };
        let description = row_text(row, 1).unwrap_or_default();
        let mut from_category = row_text(row, 5).filter(|value| !value.is_empty());
        if owner_action == Some("from_category_groceries") {
            from_category = Some("Groceries".into());
        }
        let to_category = row_text(row, 6).filter(|value| !value.is_empty());
        let from_name = row_text(row, 7).filter(|value| !value.is_empty());
        let to_name = row_text(row, 8).filter(|value| !value.is_empty());
        let classified_kind = classify(
            &from_category,
            &to_category,
            &from_name,
            &to_name,
            &description,
        );
        let kind = if owner_action == Some("income")
            && from_name
                .as_deref()
                .is_some_and(|name| name.eq_ignore_ascii_case("Income"))
            && to_name
                .as_deref()
                .is_some_and(|name| !is_system_account(name))
        {
            Some("income")
        } else {
            classified_kind
        };
        if date == "2025-05-01" && description.eq_ignore_ascii_case("Opening Balance") {
            let amount = row_decimal(row, 2);
            let real_account_ids = [&from_name, &to_name]
                .into_iter()
                .flatten()
                .filter_map(|name| account_lookup.get(&normalize_name(name)))
                .filter(|account_id| account_real_by_id.get(*account_id) == Some(&true))
                .cloned()
                .collect::<HashSet<_>>();
            if real_account_ids.len() == 1 {
                let account_id = real_account_ids.into_iter().next().expect("one account");
                if invalid_account_ids.contains(&account_id) {
                    quarantined.push(QuarantineImport {
                        source_row,
                        reason: "owner_reviewed_invalid_account_opening_seed",
                        fingerprint: row_fingerprint,
                        row_json,
                    });
                    continue;
                }
                let Some(balance_decimal) = amount.clone() else {
                    record_error(&mut errors, "invalid_account_opening_balance", source_row);
                    continue;
                };
                if !opening_account_ids.insert(account_id.clone()) {
                    record_error(&mut errors, "duplicate_account_opening_balance", source_row);
                    continue;
                }
                let account_type = account_type_by_id
                    .get(&account_id)
                    .map(String::as_str)
                    .unwrap_or_default();
                if !balance_decimal.starts_with('-')
                    && !decimal_is_zero(&balance_decimal)?
                    && account_type.eq_ignore_ascii_case("credit card")
                {
                    opening_review_warnings.push(ImportIssue {
                        source_sheet: "Transactions",
                        source_row,
                        reason_code: "positive_credit_card_opening_balance_preserved",
                    });
                }
                if balance_decimal.starts_with('-')
                    && !account_type.eq_ignore_ascii_case("credit card")
                    && owner_action != Some("opening_balance")
                {
                    opening_review_warnings.push(ImportIssue {
                        source_sheet: "Transactions",
                        source_row,
                        reason_code: "negative_opening_balance_account_type_review",
                    });
                }
                account_opening_balances.push(AccountOpeningBalanceImport {
                    account_id,
                    effective_date: date,
                    balance_decimal,
                    source_row,
                    source_row_fingerprint: row_fingerprint,
                    source_row_json: row_json,
                });
                continue;
            }
            let zero_amount = amount
                .as_deref()
                .map(decimal_is_zero)
                .transpose()?
                .unwrap_or(false);
            if real_account_ids.is_empty() && zero_amount {
                quarantined.push(QuarantineImport {
                    source_row,
                    reason: "owner_ignored_virtual_opening_helper",
                    fingerprint: row_fingerprint,
                    row_json,
                });
                continue;
            }
            record_error(
                &mut errors,
                "unresolved_account_opening_balance",
                source_row,
            );
            continue;
        }
        if date.starts_with(&bundle.manifest.current_canonical_month)
            && (9..=16).any(|column| {
                let cell = row.get(column).and_then(Option::as_ref);
                cell_state(cell) == "blank" || cell_is_error(cell) || !has_effective_value(cell)
            })
        {
            record_error(
                &mut errors,
                "current_period_valuation_gap_or_error",
                source_row,
            );
            continue;
        }
        let Some(amount) = row_decimal(row, 2) else {
            if kind == Some("reconciliation") {
                quarantined.push(QuarantineImport {
                    source_row,
                    reason: "owner_ignored_blank_reconciliation_sink",
                    fingerprint: row_fingerprint,
                    row_json,
                });
            } else {
                record_error(&mut errors, "invalid_amount", source_row);
            }
            continue;
        };
        let input_currency = cell_is_error(row.get(3).and_then(Option::as_ref))
            .then_some(None)
            .unwrap_or_else(|| row_text(row, 3).filter(|value| !value.is_empty()))
            .map(|value| value.to_ascii_uppercase());
        if input_currency
            .as_deref()
            .is_some_and(|value| !supported_currency(value))
        {
            record_error(&mut errors, "unsupported_input_currency", source_row);
            continue;
        }
        let Some(kind) = kind else {
            record_error(&mut errors, "unrecognized_operation_shape", source_row);
            continue;
        };
        if kind != "reconciliation" && !positive_decimal(&amount) {
            record_error(&mut errors, "nonpositive_normal_amount", source_row);
            continue;
        }
        let to_amount = if kind == "transfer" {
            row_decimal(row, 4)
        } else {
            None
        };
        let to_currency = if kind == "transfer" {
            row_text(row, 14)
                .filter(|value| !value.is_empty())
                .map(|value| value.to_ascii_uppercase())
        } else {
            None
        };
        if to_currency
            .as_deref()
            .is_some_and(|value| !supported_currency(value))
            || (kind == "transfer"
                && to_amount
                    .as_deref()
                    .is_some_and(|value| !positive_decimal(value)))
        {
            record_error(&mut errors, "invalid_transfer_destination", source_row);
            continue;
        }
        for category in [&from_category, &to_category].into_iter().flatten() {
            if !category_lookup.contains_key(&normalize_name(category)) {
                record_error(&mut errors, "unresolved_category_reference", source_row);
            }
        }
        for account in [&from_name, &to_name].into_iter().flatten() {
            if !is_system_account(account) && !account_lookup.contains_key(&normalize_name(account))
            {
                record_error(&mut errors, "unresolved_account_reference", source_row);
            }
        }
        transactions.push(TransactionImport {
            id,
            kind: kind.into(),
            effective_date: date,
            description,
            amount_decimal: amount,
            input_currency,
            to_amount_decimal: to_amount,
            to_currency,
            from_category_id: from_category
                .and_then(|value| category_lookup.get(&normalize_name(&value)).cloned()),
            to_category_id: to_category
                .and_then(|value| category_lookup.get(&normalize_name(&value)).cloned()),
            from_account_id: from_name
                .and_then(|value| account_lookup.get(&normalize_name(&value)).cloned()),
            to_account_id: to_name
                .and_then(|value| account_lookup.get(&normalize_name(&value)).cloned()),
            currency_is_override: cell_state(row.get(3).and_then(Option::as_ref)) == "literal",
            to_amount_is_override: cell_state(row.get(4).and_then(Option::as_ref)) == "literal"
                && owner_action != Some("ignore_destination_amount"),
            created_at: row_text(row, 18).filter(|value| !value.is_empty()),
            created_by: row_text(row, 19).filter(|value| !value.is_empty()),
            source_row,
            original_row_fingerprint: row_fingerprint,
            source_row_json: row_json,
            row: row.clone(),
        });
    }
    let mut validation_issues = errors
        .iter()
        .flat_map(|(reason, rows)| {
            rows.iter().map(|row| ImportIssue {
                source_sheet: "Transactions",
                source_row: *row,
                reason_code: reason,
            })
        })
        .collect::<Vec<_>>();
    validation_issues.extend(account_checkpoint_issues);
    let source_transaction_input_rows = transactions_sheet
        .rows
        .iter()
        .skip(1)
        .filter(|row| transaction_inputs_populated(row))
        .count();
    let classified_source_rows =
        transactions.len() + quarantined.len() + account_opening_balances.len();
    if classified_source_rows != source_transaction_input_rows {
        let classified_rows = transactions
            .iter()
            .map(|row| row.source_row)
            .chain(quarantined.iter().map(|row| row.source_row))
            .chain(account_opening_balances.iter().map(|row| row.source_row))
            .collect::<HashSet<_>>();
        for (index, row) in transactions_sheet.rows.iter().enumerate().skip(1) {
            let source_row = index + 1;
            if transaction_inputs_populated(row) && !classified_rows.contains(&source_row) {
                validation_issues.push(ImportIssue {
                    source_sheet: "Transactions",
                    source_row,
                    reason_code: "unclassified_transaction_source_row",
                });
            }
        }
    }

    let mut reallocation_impacts: HashMap<(String, String), String> = HashMap::new();
    for transaction in &transactions {
        if transaction.kind != "reallocate" {
            continue;
        }
        let month = transaction.effective_date[..7].to_string();
        let amount = decimal_absolute(&transaction.amount_decimal)?;
        if let Some(category_id) = &transaction.from_category_id {
            let key = (category_id.clone(), month.clone());
            add_decimal_delta(&mut reallocation_impacts, key, &amount)?;
        }
        if let Some(category_id) = &transaction.to_category_id {
            let key = (category_id.clone(), month);
            add_decimal_delta(&mut reallocation_impacts, key, &negate_decimal(&amount)?)?;
        }
    }

    let mut budgets = Vec::new();
    let mut budget_errors = Vec::new();
    let mut months: BTreeMap<usize, String> = BTreeMap::new();
    let header_row = budgets_sheet.rows.first().cloned().unwrap_or_default();
    let date_row = budgets_sheet.rows.get(1).cloned().unwrap_or_default();
    for column in 0..budgets_sheet.column_count {
        if row_text(&header_row, column).as_deref() != Some("Expense") {
            continue;
        }
        let Some(month) = date_row
            .get(column)
            .and_then(Option::as_ref)
            .and_then(sheet_date)
            .map(|date| date[..7].to_string())
        else {
            continue;
        };
        // Future template columns are source provenance, not imported budget history.
        if month <= bundle.manifest.current_canonical_month
            && row_text(&header_row, column + 1).as_deref() == Some("Remaining")
        {
            months.insert(column, month);
        }
    }
    for (index, row) in budgets_sheet.rows.iter().enumerate().skip(2) {
        let Some(category_name) =
            row_text(row, category_name_column).filter(|value| !value.is_empty())
        else {
            continue;
        };
        let Some(category_id) = category_ids.get(&category_name) else {
            continue;
        };
        for (expense_column, month) in &months {
            let expense_cell = row.get(*expense_column).and_then(Option::as_ref);
            let remaining_cell = row.get(*expense_column + 1).and_then(Option::as_ref);
            if month == &bundle.manifest.current_canonical_month
                && row_flag(row, is_real_column)
                && (!has_effective_value(expense_cell)
                    || !has_effective_value(remaining_cell)
                    || cell_is_error(expense_cell)
                    || cell_is_error(remaining_cell))
            {
                budget_errors.push(index + 1);
            }
            if expense_cell.is_none() && remaining_cell.is_none() {
                continue;
            }
            budgets.push(BudgetMonthImport {
                category_id: category_id.clone(),
                month: month.clone(),
                expense_eur: row_decimal(row, *expense_column),
                remaining_eur: row_decimal(row, *expense_column + 1),
                reallocation_impact_eur: reallocation_impacts
                    .get(&(category_id.clone(), month.clone()))
                    .cloned()
                    .unwrap_or_else(|| "0".into()),
                expense_state: cell_state(expense_cell),
                remaining_state: cell_state(remaining_cell),
                source_row: index + 1,
                source_cells_json: encode(&json!({
                    "expense": expense_cell,
                    "remaining": remaining_cell,
                }))?,
            });
        }
    }
    validation_issues.extend(budget_errors.into_iter().map(|row| ImportIssue {
        source_sheet: "Expenses v3",
        source_row: row,
        reason_code: "current_period_budget_gap_or_error",
    }));

    let rates = bundle
        .export
        .sheets
        .get("exchangeRates")
        .expect("validated archive");
    let mut rate_rows = Vec::new();
    let mut normalized_rates = Vec::new();
    let header_row = rates.rows.first().cloned().unwrap_or_default();
    let subheader_row = rates.rows.get(1).cloned().unwrap_or_default();
    for column in 0..rates.column_count.saturating_sub(1) {
        let Some(pair) = row_text(&header_row, column).filter(|value| !value.is_empty()) else {
            continue;
        };
        if row_text(&subheader_row, column).as_deref() != Some("Date")
            || row_text(&subheader_row, column + 1).as_deref() != Some("Close")
        {
            continue;
        }
        let Some((base_currency, quote_currency)) = rate_pair(&pair) else {
            continue;
        };
        for (index, row) in rates.rows.iter().enumerate().skip(2) {
            let date_cell = row.get(column).and_then(Option::as_ref);
            let rate_cell = row.get(column + 1).and_then(Option::as_ref);
            let Some(rate_date) = date_cell.and_then(sheet_date) else {
                continue;
            };
            let Some(rate_decimal) = row_decimal(row, column + 1) else {
                continue;
            };
            let date_formula = date_cell
                .and_then(|cell| cell.entered.as_ref())
                .and_then(|value| value.get("formulaValue"))
                .and_then(Value::as_str)
                .map(str::to_string)
                .or_else(|| {
                    date_cell
                        .and_then(|cell| cell.data_source_formula.as_ref())
                        .map(Value::to_string)
                });
            let source_formula = rate_cell
                .and_then(|cell| cell.entered.as_ref())
                .and_then(|value| value.get("formulaValue"))
                .and_then(Value::as_str)
                .map(str::to_string)
                .or_else(|| {
                    rate_cell
                        .and_then(|cell| cell.data_source_formula.as_ref())
                        .map(Value::to_string)
                })
                .or(date_formula);
            let provider = source_formula
                .as_deref()
                .filter(|formula| formula.to_ascii_uppercase().contains("GOOGLEFINANCE"))
                .map(|_| "GOOGLEFINANCE".to_string());
            let row_json = encode(row)?;
            normalized_rates.push(RateImport {
                source_row: index + 1,
                source_column: column_name(column + 1),
                rate_date,
                base_currency: base_currency.clone(),
                quote_currency: quote_currency.clone(),
                source_pair: pair.clone(),
                rate_decimal,
                provider,
                source_formula,
                source_value_json: encode(&json!({
                    "dateCell": date_cell,
                    "rateCell": rate_cell,
                }))?,
                row_fingerprint: sha256(row_json.as_bytes()),
            });
        }
    }
    for (index, row) in rates.rows.iter().enumerate().skip(1) {
        if !row_populated(row, 0, rates.column_count) {
            continue;
        }
        let json = encode(row)?;
        rate_rows.push((index + 1, sha256(json.as_bytes()), json));
    }
    let counts = json!({
        "accounts": account_imports.len(),
        "accountCheckpointCells": account_checkpoint_imports.len(),
        "accountCheckpointsArchived": account_checkpoints_present,
        "accountOpeningBalances": account_opening_balances.len(),
        "categories": categories.len(),
        "datedTransactions": transactions.len(),
        "quarantinedTransactions": quarantined.len(),
        "sourceTransactionInputRows": source_transaction_input_rows,
        "classifiedTransactionSourceRows": classified_source_rows,
        "budgetSnapshots": budgets.len(),
        "exchangeRateSourceRows": rate_rows.len(),
        "normalizedExchangeRates": normalized_rates.len(),
        "validationIssues": validation_issues.len(),
        "reviewWarnings": opening_review_warnings.len(),
    });
    Ok(ImportPlan {
        accounts: account_imports,
        account_checkpoints: account_checkpoint_imports,
        account_checkpoints_present,
        account_opening_balances,
        categories,
        transactions,
        quarantined,
        budgets,
        rate_rows,
        normalized_rates,
        validation_issues,
        review_warnings: opening_review_warnings,
        source_transaction_input_rows,
        counts,
    })
}

struct AccountColumns {
    name: usize,
    account_type: usize,
    is_real: usize,
    currency: usize,
    is_active: usize,
    native: usize,
    actual_native: usize,
    reconciliation: usize,
    eur: usize,
    actual_eur: usize,
}

async fn import_command(options: HashMap<String, String>) -> anyhow::Result<()> {
    let input = option(&options, "input")
        .map(PathBuf::from)
        .ok_or_else(|| anyhow::anyhow!("--input is required."))?;
    let bundle = read_bundle(&input)?;
    let identities = identity_map(&bundle)?;
    let plan = build_plan(&bundle, &identities)?;
    let path = database_path(&options);
    let commit = option(&options, "commit").as_deref() == Some("true");
    if commit
        && bundle.manifest.test_workbook
        && (option(&options, "allow-test-workbook").as_deref() != Some("true")
            || option(&options, "database").is_none())
    {
        bail!(
            "Test-workbook imports require an explicit --database FILE and --allow-test-workbook."
        );
    }
    if commit && !plan.account_checkpoints_present {
        bail!("Finance commits require an Accounts W:BE historical checkpoint archive.");
    }
    if let Some(report_path) = option(&options, "report") {
        write_private_json(
            &PathBuf::from(report_path),
            &json!({
                "sourceDigest": bundle.manifest.export_sha256,
                "counts": plan.counts,
                "reviewWarnings": plan.review_warnings,
                "issues": plan.validation_issues,
            }),
        )?;
    }
    if commit {
        if !plan.validation_issues.is_empty() {
            bail!(
                "Import validation found {} issue(s); see the private --report file or rerun without --commit.",
                plan.validation_issues.len()
            );
        }
        commit_import(&bundle, &identities, &plan, &path).await?;
        println!("Finance import committed: {}", plan.counts);
    } else {
        if path.exists() {
            inspect_existing_import(&path, &bundle).await?;
        }
        if plan.validation_issues.is_empty() {
            println!("Dry run valid: {}", plan.counts);
        } else {
            println!("Dry run rejected: {}", plan.counts);
            println!("No SQLite rows were changed. Use --report PATH for row-number diagnostics.");
        }
        println!(
            "No SQLite database rows were changed. Add --commit to import into the existing database."
        );
    }
    Ok(())
}

fn write_private_json(path: &Path, value: &Value) -> anyhow::Result<()> {
    if path.exists() {
        bail!("Refusing to overwrite the requested private report.");
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, serde_json::to_vec_pretty(value)?)
        .context("could not write private admin report")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

async fn inspect_existing_import(path: &Path, bundle: &ArchiveBundle) -> anyhow::Result<()> {
    let pool = open_pool(path, true).await?;
    if table_exists(&pool, "finance_import_runs").await? {
        let existing = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM finance_import_runs WHERE source_digest = ?",
        )
        .bind(&bundle.manifest.export_sha256)
        .fetch_one(&pool)
        .await?;
        if existing > 0 {
            println!("This exact archive has already been imported.");
        }
    }
    pool.close().await;
    Ok(())
}

async fn commit_import(
    bundle: &ArchiveBundle,
    identities: &IdentityMap,
    plan: &ImportPlan,
    database: &Path,
) -> anyhow::Result<()> {
    if let Some(parent) = database
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
    {
        fs::create_dir_all(parent).context("could not create database directory")?;
    }
    let pool = open_pool(database, false).await?;
    sqlx::migrate!()
        .run(&pool)
        .await
        .context("could not apply database migrations")?;
    let existing = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM finance_import_runs WHERE source_digest = ?",
    )
    .bind(&bundle.manifest.export_sha256)
    .fetch_one(&pool)
    .await?;
    if existing > 0 {
        pool.close().await;
        println!("This exact archive has already been imported; no rows changed.");
        return Ok(());
    }
    for table in [
        "finance_accounts",
        "finance_account_opening_balances",
        "finance_categories",
        "category_budget_periods",
        "finance_transactions",
        "finance_transaction_revisions",
        "transaction_valuations",
        "transaction_valuation_source_cells",
        "budget_month_snapshots",
        "finance_exchange_rate_source_rows",
        "exchange_rates",
        "finance_account_actuals",
        "finance_account_balance_checkpoints",
        "finance_operations",
        "finance_quarantined_rows",
        "finance_source_artifacts",
    ] {
        let count = table_row_count(&pool, table).await?;
        if count != 0 {
            pool.close().await;
            bail!(
                "Finance tables contain data from another source. Use a fresh auth database or an explicit reviewed reset."
            );
        }
    }

    let mut tx = pool
        .begin()
        .await
        .context("could not begin finance import transaction")?;
    for account in &plan.accounts {
        sqlx::query(
            "INSERT INTO finance_accounts (id, source_name, account_type, currency, is_real, is_active, current_native_balance, actual_native_balance, reconciliation_native, current_eur_balance, actual_eur_balance, source_row, source_row_json) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&account.id).bind(&account.name).bind(&account.account_type).bind(&account.currency)
        .bind(account.is_real).bind(account.is_active).bind(&account.current_native_balance)
        .bind(&account.actual_native_balance).bind(&account.reconciliation_native)
        .bind(&account.current_eur_balance).bind(&account.actual_eur_balance)
        .bind(account.source_row as i64).bind(&account.source_row_json)
        .execute(&mut *tx).await?;
        sqlx::query(
            "INSERT INTO finance_account_actuals (account_id, actual_native_balance, source_reconciliation_native, source_row, source_row_json) VALUES (?, ?, ?, ?, ?)",
        )
        .bind(&account.id).bind(&account.actual_native_balance).bind(&account.reconciliation_native)
        .bind(account.source_row as i64).bind(&account.source_row_json)
        .execute(&mut *tx).await?;
    }
    for checkpoint in &plan.account_checkpoints {
        sqlx::query(
            "INSERT INTO finance_account_balance_checkpoints (source_workbook_id, source_sheet, source_row, source_column, account_id, checkpoint_date, balance_decimal, source_state, source_header_json, source_cell_json) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&bundle.manifest.source_spreadsheet_id)
        .bind(plan_sheet_title(&bundle.export, "accounts"))
        .bind(checkpoint.source_row as i64)
        .bind(&checkpoint.source_column)
        .bind(&checkpoint.account_id)
        .bind(&checkpoint.checkpoint_date)
        .bind(&checkpoint.balance_decimal)
        .bind(checkpoint.source_state)
        .bind(&checkpoint.source_header_json)
        .bind(&checkpoint.source_cell_json)
        .execute(&mut *tx)
        .await?;
    }
    for opening in &plan.account_opening_balances {
        sqlx::query(
            "INSERT INTO finance_account_opening_balances (source_workbook_id, source_sheet, source_row, account_id, effective_date, balance_decimal, original_row_fingerprint, source_row_json) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&bundle.manifest.source_spreadsheet_id)
        .bind(plan_sheet_title(&bundle.export, "transactions"))
        .bind(opening.source_row as i64)
        .bind(&opening.account_id)
        .bind(&opening.effective_date)
        .bind(&opening.balance_decimal)
        .bind(&opening.source_row_fingerprint)
        .bind(&opening.source_row_json)
        .execute(&mut *tx)
        .await?;
    }
    for category in &plan.categories {
        sqlx::query(
            "INSERT INTO finance_categories (id, name, is_real, source_row, source_row_json) VALUES (?, ?, ?, ?, ?)",
        )
        .bind(&category.id).bind(&category.name).bind(category.is_real)
        .bind(category.source_row as i64).bind(&category.source_row_json)
        .execute(&mut *tx).await?;
        sqlx::query(
            "INSERT INTO category_budget_periods (category_id, effective_start_month, base_monthly_eur, source_row, source_cell_json) VALUES (?, ?, ?, ?, ?)",
        )
        .bind(&category.id).bind(&bundle.manifest.current_canonical_month)
        .bind(&category.base_monthly_eur).bind(category.source_row as i64)
        .bind(&category.base_cell_json).execute(&mut *tx).await?;
    }

    for transaction in &plan.transactions {
        sqlx::query(
            "INSERT INTO finance_transactions (id, kind, effective_date, description, amount_decimal, input_currency, to_amount_decimal, to_currency, from_category_id, to_category_id, from_account_id, to_account_id, currency_is_override, to_amount_is_override, created_at, created_by, source_workbook_id, source_sheet, source_row, original_row_fingerprint, source_row_json) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&transaction.id).bind(&transaction.kind).bind(&transaction.effective_date)
        .bind(&transaction.description).bind(&transaction.amount_decimal).bind(&transaction.input_currency)
        .bind(&transaction.to_amount_decimal).bind(&transaction.to_currency)
        .bind(&transaction.from_category_id).bind(&transaction.to_category_id)
        .bind(&transaction.from_account_id).bind(&transaction.to_account_id)
        .bind(transaction.currency_is_override).bind(transaction.to_amount_is_override)
        .bind(&transaction.created_at).bind(&transaction.created_by)
        .bind(&bundle.manifest.source_spreadsheet_id).bind(&plan_sheet_title(&bundle.export, "transactions"))
        .bind(transaction.source_row as i64).bind(&transaction.original_row_fingerprint)
        .bind(&transaction.source_row_json).execute(&mut *tx).await?;
        sqlx::query(
            "INSERT INTO finance_transaction_revisions (transaction_id, revision, operation_id, snapshot_json, recorded_at) VALUES (?, 1, NULL, ?, ?)",
        )
        .bind(&transaction.id)
        .bind(encode(transaction)?)
        .bind(now_utc())
        .execute(&mut *tx)
        .await?;
        for (
            side,
            currency_column,
            eur_rate_column,
            native_rate_column,
            native_amount_column,
            eur_amount_column,
            source_columns,
        ) in [
            (
                "from",
                9usize,
                10usize,
                Some(12usize),
                13usize,
                11usize,
                vec![9usize, 10, 11, 12, 13],
            ),
            (
                "to",
                14usize,
                15usize,
                None,
                4usize,
                16usize,
                vec![4usize, 14, 15, 16],
            ),
        ] {
            let source_cells = source_columns
                .into_iter()
                .map(|index| {
                    json!({
                        "column": column_name(index + 1),
                        "cell": transaction.row.get(index).and_then(Option::as_ref),
                    })
                })
                .collect::<Vec<_>>();
            sqlx::query(
                "INSERT INTO transaction_valuations (transaction_id, side, account_currency, exchange_to_eur, exchange_to_native, amount_native, amount_eur, effective_date, valuation_source, rate_set_version, revision, source_cells_json) VALUES (?, ?, ?, ?, ?, ?, ?, ?, 'sheet-evaluated-baseline', NULL, 1, ?)",
            )
            .bind(&transaction.id)
            .bind(side)
            .bind(row_text(&transaction.row, currency_column))
            .bind(row_decimal(&transaction.row, eur_rate_column))
            .bind(native_rate_column.and_then(|column| row_decimal(&transaction.row, column)))
            .bind(row_decimal(&transaction.row, native_amount_column))
            .bind(row_decimal(&transaction.row, eur_amount_column))
            .bind(&transaction.effective_date)
            .bind(encode(&source_cells)?)
            .execute(&mut *tx)
            .await?;
        }
        for (column, index) in [
            ('J', 9),
            ('K', 10),
            ('L', 11),
            ('M', 12),
            ('N', 13),
            ('O', 14),
            ('P', 15),
            ('Q', 16),
        ] {
            let cell = transaction.row.get(index).and_then(Option::as_ref);
            let state = cell_state(cell);
            let formula = cell
                .and_then(|value| value.entered.as_ref())
                .and_then(|value| value.get("formulaValue"))
                .and_then(Value::as_str)
                .map(str::to_string)
                .or_else(|| {
                    cell.and_then(|value| value.data_source_formula.as_ref())
                        .map(Value::to_string)
                });
            let evaluated = cell
                .and_then(|value| value.effective.as_ref())
                .filter(|value| !value.is_null())
                .map(encode)
                .transpose()?;
            let formatted = cell.and_then(|value| value.formatted.as_deref());
            sqlx::query(
                "INSERT INTO transaction_valuation_source_cells (transaction_id, source_column, source_state, source_formula, evaluated_value_json, formatted_value) VALUES (?, ?, ?, ?, ?, ?)",
            )
            .bind(&transaction.id).bind(column.to_string()).bind(state).bind(formula)
            .bind(evaluated).bind(formatted).execute(&mut *tx).await?;
        }
    }
    for item in &plan.quarantined {
        sqlx::query(
            "INSERT INTO finance_quarantined_rows (source_workbook_id, source_sheet, source_row, reason_code, original_row_fingerprint, source_row_json) VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(&bundle.manifest.source_spreadsheet_id)
        .bind(plan_sheet_title(&bundle.export, "transactions"))
        .bind(item.source_row as i64).bind(item.reason).bind(&item.fingerprint).bind(&item.row_json)
        .execute(&mut *tx).await?;
    }
    for budget in &plan.budgets {
        sqlx::query(
            "INSERT INTO budget_month_snapshots (category_id, month, expense_eur, remaining_eur, reallocation_impact_eur, source_expense_state, source_remaining_state, source_row, source_cells_json) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&budget.category_id).bind(&budget.month).bind(&budget.expense_eur)
        .bind(&budget.remaining_eur).bind(&budget.reallocation_impact_eur)
        .bind(budget.expense_state).bind(budget.remaining_state)
        .bind(budget.source_row as i64).bind(&budget.source_cells_json)
        .execute(&mut *tx).await?;
    }
    for (source_row, row_fingerprint, row_json) in &plan.rate_rows {
        sqlx::query(
            "INSERT INTO finance_exchange_rate_source_rows (source_workbook_id, source_sheet, source_row, original_row_fingerprint, source_row_json) VALUES (?, ?, ?, ?, ?)",
        )
        .bind(&bundle.manifest.source_spreadsheet_id)
        .bind(plan_sheet_title(&bundle.export, "exchangeRates"))
        .bind(*source_row as i64).bind(row_fingerprint).bind(row_json)
        .execute(&mut *tx).await?;
    }
    for rate in &plan.normalized_rates {
        sqlx::query(
            "INSERT INTO exchange_rates (id, rate_date, base_currency, quote_currency, rate_decimal, provider, source_pair, source_column, source_formula, source_value_json, source_workbook_id, source_sheet, source_row, original_row_fingerprint) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(new_id())
        .bind(&rate.rate_date)
        .bind(&rate.base_currency)
        .bind(&rate.quote_currency)
        .bind(&rate.rate_decimal)
        .bind(&rate.provider)
        .bind(&rate.source_pair)
        .bind(&rate.source_column)
        .bind(&rate.source_formula)
        .bind(&rate.source_value_json)
        .bind(&bundle.manifest.source_spreadsheet_id)
        .bind(plan_sheet_title(&bundle.export, "exchangeRates"))
        .bind(rate.source_row as i64)
        .bind(&rate.row_fingerprint)
        .execute(&mut *tx)
        .await?;
    }
    let artifacts = json!({
        "workbook": bundle.export.workbook,
        "sourceTimeZone": bundle.export.workbook.get("timeZone"),
        "canonicalTimeZone": bundle.export.canonical_time_zone,
        "exportedAt": bundle.export.exported_at,
        "ownerReviewDecisions": bundle.owner_review,
        "exchangeRatesSheet": bundle.export.sheets.get("exchangeRates"),
        "transactionSheetMetadata": sheet_metadata(&bundle.export, "transactions"),
        "budgetSheetMetadata": sheet_metadata(&bundle.export, "budgets"),
    });
    sqlx::query(
        "INSERT INTO finance_source_artifacts (source_workbook_id, artifact_key, source_json) VALUES (?, 'phase-b-workbook-and-rate-provenance', ?)",
    )
    .bind(&bundle.manifest.source_spreadsheet_id).bind(encode(&artifacts)?)
    .execute(&mut *tx).await?;
    sqlx::query(
        "INSERT INTO finance_import_runs (id, source_workbook_id, source_digest, manifest_json, counts_json, committed_at) VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(&identities.import_run_id).bind(&bundle.manifest.source_spreadsheet_id)
    .bind(&bundle.manifest.export_sha256).bind(encode(&json!({
        "schemaVersion": bundle.manifest.schema_version,
        "sourceSpreadsheetId": bundle.manifest.source_spreadsheet_id,
        "exportSha256": bundle.manifest.export_sha256,
        "exportedAt": bundle.manifest.exported_at,
        "canonicalTimeZone": bundle.manifest.canonical_time_zone,
        "currentCanonicalMonth": bundle.manifest.current_canonical_month,
        "ownerReviewDecisions": bundle.owner_review,
    }))?)
    .bind(encode(&plan.counts)?).bind(now_utc())
    .execute(&mut *tx).await?;
    tx.commit()
        .await
        .context("could not commit finance import transaction")?;
    pool.close().await;
    Ok(())
}

fn plan_sheet_title<'a>(export: &'a FinanceExport, key: &str) -> &'a str {
    export
        .sheets
        .get(key)
        .map(|sheet| sheet.title.as_str())
        .unwrap_or("")
}

fn sheet_metadata(export: &FinanceExport, key: &str) -> Value {
    export
        .sheets
        .get(key)
        .map(|sheet| {
            json!({
                "title": sheet.title,
                "sheetId": sheet.sheet_id,
                "range": sheet.range,
                "gridRowCount": sheet.grid_row_count,
                "gridColumnCount": sheet.grid_column_count,
                "rowCount": sheet.row_count,
                "columnCount": sheet.column_count,
            })
        })
        .unwrap_or(Value::Null)
}

async fn verify_command(options: HashMap<String, String>) -> anyhow::Result<()> {
    let input = option(&options, "input")
        .map(PathBuf::from)
        .ok_or_else(|| anyhow::anyhow!("--input is required."))?;
    let bundle = read_bundle(&input)?;
    let identities = identity_map(&bundle)?;
    let plan = build_plan(&bundle, &identities)?;
    let database = database_path(&options);
    let pool = open_pool(&database, true).await?;
    if !table_exists(&pool, "finance_import_runs").await? {
        bail!("Selected database does not contain the Phase B finance schema.");
    }
    let import_count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM finance_import_runs WHERE source_digest = ?",
    )
    .bind(&bundle.manifest.export_sha256)
    .fetch_one(&pool)
    .await?;
    if import_count != 1 {
        bail!("Selected database does not contain exactly one import of this archive.");
    }
    let mut matched_transactions = 0usize;
    let mut missing_transactions = Vec::new();
    let mut changed_transactions = Vec::new();
    for transaction in &plan.transactions {
        let actual = sqlx::query_scalar::<_, String>(
            "SELECT original_row_fingerprint FROM finance_transactions WHERE source_workbook_id = ? AND source_sheet = ? AND source_row = ?",
        )
        .bind(&bundle.manifest.source_spreadsheet_id)
        .bind(plan_sheet_title(&bundle.export, "transactions"))
        .bind(transaction.source_row as i64)
        .fetch_optional(&pool).await?;
        match actual {
            None => missing_transactions.push(transaction.source_row),
            Some(actual) if actual != transaction.original_row_fingerprint => {
                changed_transactions.push(transaction.source_row)
            }
            Some(_) => matched_transactions += 1,
        }
    }
    let source_transaction_count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM finance_transactions WHERE source_workbook_id = ? AND source_sheet = ?",
    )
    .bind(&bundle.manifest.source_spreadsheet_id)
    .bind(plan_sheet_title(&bundle.export, "transactions"))
    .fetch_one(&pool)
    .await? as usize;
    let mut reference_row_mismatches = Vec::new();
    for account in &plan.accounts {
        let actual = sqlx::query_scalar::<_, String>(
            "SELECT source_row_json FROM finance_accounts WHERE source_name = ?",
        )
        .bind(&account.name)
        .fetch_optional(&pool)
        .await?;
        if actual.as_deref() != Some(account.source_row_json.as_str()) {
            reference_row_mismatches.push(json!({
                "sheet": "Accounts",
                "row": account.source_row,
            }));
        }
    }
    for category in &plan.categories {
        let actual = sqlx::query_scalar::<_, String>(
            "SELECT source_row_json FROM finance_categories WHERE name = ?",
        )
        .bind(&category.name)
        .fetch_optional(&pool)
        .await?;
        if actual.as_deref() != Some(category.source_row_json.as_str()) {
            reference_row_mismatches.push(json!({
                "sheet": "Expenses v3",
                "row": category.source_row,
            }));
        }
    }
    let mut budget_row_mismatches = Vec::new();
    for budget in &plan.budgets {
        let actual = sqlx::query(
            "SELECT source_cells_json, reallocation_impact_eur FROM budget_month_snapshots WHERE category_id = ? AND month = ?",
        )
        .bind(&budget.category_id)
        .bind(&budget.month)
        .fetch_optional(&pool)
        .await?;
        let matches = if let Some(row) = actual {
            row.try_get::<String, _>(0)? == budget.source_cells_json
                && row.try_get::<String, _>(1)? == budget.reallocation_impact_eur
        } else {
            false
        };
        if !matches {
            budget_row_mismatches.push(json!({
                "sheet": "Expenses v3",
                "row": budget.source_row,
                "month": budget.month,
            }));
        }
    }
    let quarantined_count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM finance_quarantined_rows WHERE source_workbook_id = ? AND source_sheet = ?",
    ).bind(&bundle.manifest.source_spreadsheet_id).bind(plan_sheet_title(&bundle.export, "transactions"))
        .fetch_one(&pool).await? as usize;
    let rate_count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM finance_exchange_rate_source_rows WHERE source_workbook_id = ? AND source_sheet = ?",
    ).bind(&bundle.manifest.source_spreadsheet_id).bind(plan_sheet_title(&bundle.export, "exchangeRates"))
        .fetch_one(&pool).await? as usize;
    let mut missing_rate_rows = Vec::new();
    for (row, fingerprint, _) in &plan.rate_rows {
        let actual = sqlx::query_scalar::<_, String>(
            "SELECT original_row_fingerprint FROM finance_exchange_rate_source_rows WHERE source_workbook_id = ? AND source_sheet = ? AND source_row = ?",
        ).bind(&bundle.manifest.source_spreadsheet_id)
            .bind(plan_sheet_title(&bundle.export, "exchangeRates"))
            .bind(*row as i64).fetch_optional(&pool).await?;
        if actual.as_deref() != Some(fingerprint) {
            missing_rate_rows.push(*row);
        }
    }
    let normalized_rate_count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM exchange_rates WHERE source_workbook_id = ? AND source_sheet = ?",
    )
    .bind(&bundle.manifest.source_spreadsheet_id)
    .bind(plan_sheet_title(&bundle.export, "exchangeRates"))
    .fetch_one(&pool)
    .await? as usize;
    let mut quarantine_row_mismatches = Vec::new();
    for item in &plan.quarantined {
        let actual = sqlx::query_scalar::<_, String>(
            "SELECT original_row_fingerprint FROM finance_quarantined_rows WHERE source_workbook_id = ? AND source_sheet = ? AND source_row = ?",
        )
        .bind(&bundle.manifest.source_spreadsheet_id)
        .bind(plan_sheet_title(&bundle.export, "transactions"))
        .bind(item.source_row as i64)
        .fetch_optional(&pool)
        .await?;
        if actual.as_deref() != Some(item.fingerprint.as_str()) {
            quarantine_row_mismatches.push(item.source_row);
        }
    }
    let valuation_count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM transaction_valuations WHERE transaction_id IN (SELECT id FROM finance_transactions WHERE source_workbook_id = ? AND source_sheet = ?)",
    )
    .bind(&bundle.manifest.source_spreadsheet_id)
    .bind(plan_sheet_title(&bundle.export, "transactions"))
    .fetch_one(&pool)
    .await? as usize;
    let transaction_revision_count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM finance_transaction_revisions WHERE transaction_id IN (SELECT id FROM finance_transactions WHERE source_workbook_id = ? AND source_sheet = ?)",
    )
    .bind(&bundle.manifest.source_spreadsheet_id)
    .bind(plan_sheet_title(&bundle.export, "transactions"))
    .fetch_one(&pool)
    .await? as usize;
    let source_cell_count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM transaction_valuation_source_cells WHERE transaction_id IN (SELECT id FROM finance_transactions WHERE source_workbook_id = ? AND source_sheet = ?)",
    )
    .bind(&bundle.manifest.source_spreadsheet_id)
    .bind(plan_sheet_title(&bundle.export, "transactions"))
    .fetch_one(&pool)
    .await? as usize;
    let expected_valuation_count = plan.transactions.len() * 2;
    let expected_source_cell_count = plan.transactions.len() * 8;
    let classified_source_rows =
        plan.transactions.len() + plan.quarantined.len() + plan.account_opening_balances.len();
    let source_coverage_matches = classified_source_rows == plan.source_transaction_input_rows;
    let budget_period_count =
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM category_budget_periods")
            .fetch_one(&pool)
            .await? as usize;
    let account_actual_count =
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM finance_account_actuals")
            .fetch_one(&pool)
            .await? as usize;
    let account_checkpoint_count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM finance_account_balance_checkpoints WHERE source_workbook_id = ? AND source_sheet = ?",
    )
    .bind(&bundle.manifest.source_spreadsheet_id)
    .bind(plan_sheet_title(&bundle.export, "accounts"))
    .fetch_one(&pool)
    .await? as usize;
    let account_opening_balance_count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM finance_account_opening_balances WHERE source_workbook_id = ? AND source_sheet = ?",
    )
    .bind(&bundle.manifest.source_spreadsheet_id)
    .bind(plan_sheet_title(&bundle.export, "transactions"))
    .fetch_one(&pool)
    .await? as usize;
    let mut account_opening_balance_mismatches = Vec::new();
    for opening in &plan.account_opening_balances {
        let actual = sqlx::query(
            "SELECT account_id, effective_date, balance_decimal, original_row_fingerprint, source_row_json FROM finance_account_opening_balances WHERE source_workbook_id = ? AND source_sheet = ? AND source_row = ?",
        )
        .bind(&bundle.manifest.source_spreadsheet_id)
        .bind(plan_sheet_title(&bundle.export, "transactions"))
        .bind(opening.source_row as i64)
        .fetch_optional(&pool)
        .await?;
        let matches = if let Some(row) = actual {
            row.try_get::<String, _>(0)? == opening.account_id
                && row.try_get::<String, _>(1)? == opening.effective_date
                && row.try_get::<String, _>(2)? == opening.balance_decimal
                && row.try_get::<String, _>(3)? == opening.source_row_fingerprint
                && row.try_get::<String, _>(4)? == opening.source_row_json
        } else {
            false
        };
        if !matches {
            account_opening_balance_mismatches.push(opening.source_row);
        }
    }
    let mut account_checkpoint_mismatches = Vec::new();
    for checkpoint in &plan.account_checkpoints {
        let actual = sqlx::query(
            "SELECT account_id, checkpoint_date, balance_decimal, source_state, source_header_json, source_cell_json FROM finance_account_balance_checkpoints WHERE source_workbook_id = ? AND source_sheet = ? AND source_row = ? AND source_column = ?",
        )
        .bind(&bundle.manifest.source_spreadsheet_id)
        .bind(plan_sheet_title(&bundle.export, "accounts"))
        .bind(checkpoint.source_row as i64)
        .bind(&checkpoint.source_column)
        .fetch_optional(&pool)
        .await?;
        let matches = if let Some(row) = actual {
            row.try_get::<String, _>(0)? == checkpoint.account_id
                && row.try_get::<Option<String>, _>(1)? == checkpoint.checkpoint_date
                && row.try_get::<Option<String>, _>(2)? == checkpoint.balance_decimal
                && row.try_get::<String, _>(3)? == checkpoint.source_state
                && row.try_get::<String, _>(4)? == checkpoint.source_header_json
                && row.try_get::<String, _>(5)? == checkpoint.source_cell_json
        } else {
            false
        };
        if !matches {
            account_checkpoint_mismatches.push(json!({
                "row": checkpoint.source_row,
                "column": checkpoint.source_column,
            }));
        }
    }
    let report = json!({
        "sourceDigest": bundle.manifest.export_sha256,
        "transactions": {
            "expected": plan.transactions.len(),
            "matched": matched_transactions,
            "databaseRows": source_transaction_count,
            "missingRows": missing_transactions,
            "changedRows": changed_transactions,
            "quarantinedExpected": plan.quarantined.len(),
            "quarantinedDatabase": quarantined_count,
            "quarantinedChangedRows": quarantine_row_mismatches,
        },
        "transactionSourceCoverage": {
            "inputRows": plan.source_transaction_input_rows,
            "classifiedRows": classified_source_rows,
            "matches": source_coverage_matches,
        },
        "exchangeRateSourceRows": {
            "expected": plan.rate_rows.len(),
            "database": rate_count,
            "missingOrChangedRows": missing_rate_rows,
        },
        "normalizedExchangeRates": {
            "expected": plan.normalized_rates.len(),
            "database": normalized_rate_count,
        },
        "valuationCellCount": {
            "expected": expected_source_cell_count,
            "database": source_cell_count,
        },
        "normalizedValuationCount": {
            "expected": expected_valuation_count,
            "database": valuation_count,
        },
        "transactionRevisionCount": {
            "expected": plan.transactions.len(),
            "database": transaction_revision_count,
        },
        "budgetPeriodCount": {
            "expected": plan.categories.len(),
            "database": budget_period_count,
        },
        "accountActualCount": {
            "expected": plan.accounts.len(),
            "database": account_actual_count,
        },
        "accountBalanceCheckpoints": {
            "archived": plan.account_checkpoints_present,
            "expectedCells": plan.account_checkpoints.len(),
            "databaseCells": account_checkpoint_count,
            "missingOrChangedCells": account_checkpoint_mismatches,
        },
        "accountOpeningBalances": {
            "expected": plan.account_opening_balances.len(),
            "database": account_opening_balance_count,
            "missingOrChangedRows": account_opening_balance_mismatches,
        },
        "referenceRowMismatches": reference_row_mismatches,
        "budgetCellMismatches": budget_row_mismatches,
        "referenceAndBudgetCounts": plan.counts,
    });
    let perfect = missing_transactions.is_empty()
        && changed_transactions.is_empty()
        && source_transaction_count == plan.transactions.len()
        && missing_rate_rows.is_empty()
        && rate_count == plan.rate_rows.len()
        && normalized_rate_count == plan.normalized_rates.len()
        && quarantined_count == plan.quarantined.len()
        && quarantine_row_mismatches.is_empty()
        && valuation_count == expected_valuation_count
        && transaction_revision_count == plan.transactions.len()
        && source_cell_count == expected_source_cell_count
        && source_coverage_matches
        && budget_period_count == plan.categories.len()
        && account_actual_count == plan.accounts.len()
        && plan.account_checkpoints_present
        && account_checkpoint_count == plan.account_checkpoints.len()
        && account_checkpoint_mismatches.is_empty()
        && account_opening_balance_count == plan.account_opening_balances.len()
        && account_opening_balance_mismatches.is_empty()
        && reference_row_mismatches.is_empty()
        && budget_row_mismatches.is_empty();
    let summary = json!({
        "sourceDigest": bundle.manifest.export_sha256,
        "transactionRowsMatched": matched_transactions,
        "transactionRowsExpected": plan.transactions.len(),
        "transactionRowsMissingOrChanged": missing_transactions.len() + changed_transactions.len(),
        "transactionSourceRowsClassified": classified_source_rows,
        "transactionSourceRowsExpected": plan.source_transaction_input_rows,
        "rateRowsMatched": plan.rate_rows.len().saturating_sub(missing_rate_rows.len()),
        "rateRowsExpected": plan.rate_rows.len(),
        "normalizedRatesDatabase": normalized_rate_count,
        "normalizedRatesExpected": plan.normalized_rates.len(),
        "normalizedValuationsDatabase": valuation_count,
        "normalizedValuationsExpected": expected_valuation_count,
        "transactionRevisionsDatabase": transaction_revision_count,
        "transactionRevisionsExpected": plan.transactions.len(),
        "valuationSourceCellsDatabase": source_cell_count,
        "valuationSourceCellsExpected": expected_source_cell_count,
        "accountBalanceCheckpointCellsDatabase": account_checkpoint_count,
        "accountBalanceCheckpointCellsExpected": plan.account_checkpoints.len(),
        "accountBalanceCheckpointsArchived": plan.account_checkpoints_present,
        "accountBalanceCheckpointCellMismatches": account_checkpoint_mismatches.len(),
        "accountOpeningBalancesDatabase": account_opening_balance_count,
        "accountOpeningBalancesExpected": plan.account_opening_balances.len(),
        "accountOpeningBalanceMismatches": account_opening_balance_mismatches.len(),
        "referenceRowMismatches": reference_row_mismatches.len(),
        "budgetCellMismatches": budget_row_mismatches.len(),
        "parity": if perfect { "source-row-exact" } else { "differences-found" },
    });
    println!("Redacted finance parity: {summary}");
    if let Some(report_path) = option(&options, "report") {
        let path = PathBuf::from(report_path);
        if path.exists() {
            bail!("Refusing to overwrite the requested private report.");
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&path, serde_json::to_vec_pretty(&report)?)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
        }
        println!(
            "Private row-level parity report saved to {}.",
            path.display()
        );
    }
    pool.close().await;
    if !perfect {
        bail!("Finance source-row verification found differences.");
    }
    Ok(())
}

async fn export_sqlite_command(options: HashMap<String, String>) -> anyhow::Result<()> {
    let database = database_path(&options);
    let output = option(&options, "output")
        .map(PathBuf::from)
        .ok_or_else(|| anyhow::anyhow!("--output is required."))?;
    if output.exists() {
        bail!("Refusing to overwrite an existing SQLite export.");
    }
    let pool = open_pool(&database, true).await?;
    if !table_exists(&pool, "finance_import_runs").await? {
        bail!("Selected database does not contain the Phase B finance schema.");
    }
    let tables = [
        "finance_accounts",
        "finance_account_opening_balances",
        "finance_categories",
        "category_budget_periods",
        "finance_transactions",
        "finance_transaction_revisions",
        "transaction_valuations",
        "transaction_valuation_source_cells",
        "budget_month_snapshots",
        "budget_opening_state",
        "finance_exchange_rate_source_rows",
        "exchange_rates",
        "finance_account_actuals",
        "finance_account_balance_checkpoints",
        "finance_operations",
        "finance_category_corrections",
        "finance_category_correction_items",
        "reconciliation_batches",
        "reconciliation_items",
        "finance_import_runs",
        "finance_quarantined_rows",
        "finance_source_artifacts",
        "budget_income_expectations",
        "budget_ledger_state",
        "budget_plan_income",
        "budget_income_receipts",
        "budget_ledger_movements",
        "budget_month_closures",
        "budget_coverage_closures",
    ];
    let mut snapshot = pool
        .begin()
        .await
        .context("could not begin SQLite snapshot")?;
    let mut content = Map::new();
    let mut counts = Map::new();
    for table in tables {
        let query = match table {
            "finance_accounts" => sqlx::query("SELECT * FROM finance_accounts"),
            "finance_account_opening_balances" => {
                sqlx::query("SELECT * FROM finance_account_opening_balances")
            }
            "finance_categories" => sqlx::query("SELECT * FROM finance_categories"),
            "category_budget_periods" => sqlx::query("SELECT * FROM category_budget_periods"),
            "finance_transactions" => sqlx::query("SELECT * FROM finance_transactions"),
            "finance_transaction_revisions" => {
                sqlx::query("SELECT * FROM finance_transaction_revisions")
            }
            "transaction_valuations" => sqlx::query("SELECT * FROM transaction_valuations"),
            "transaction_valuation_source_cells" => {
                sqlx::query("SELECT * FROM transaction_valuation_source_cells")
            }
            "budget_month_snapshots" => sqlx::query("SELECT * FROM budget_month_snapshots"),
            "budget_month_closures" => sqlx::query("SELECT * FROM budget_month_closures"),
            "budget_opening_state" => sqlx::query("SELECT * FROM budget_opening_state"),
            "finance_exchange_rate_source_rows" => {
                sqlx::query("SELECT * FROM finance_exchange_rate_source_rows")
            }
            "exchange_rates" => sqlx::query("SELECT * FROM exchange_rates"),
            "finance_account_actuals" => sqlx::query("SELECT * FROM finance_account_actuals"),
            "finance_account_balance_checkpoints" => {
                sqlx::query("SELECT * FROM finance_account_balance_checkpoints")
            }
            "finance_operations" => sqlx::query("SELECT * FROM finance_operations"),
            "finance_category_corrections" => {
                sqlx::query("SELECT * FROM finance_category_corrections")
            }
            "finance_category_correction_items" => {
                sqlx::query("SELECT * FROM finance_category_correction_items")
            }
            "reconciliation_batches" => sqlx::query("SELECT * FROM reconciliation_batches"),
            "reconciliation_items" => sqlx::query("SELECT * FROM reconciliation_items"),
            "finance_import_runs" => sqlx::query("SELECT * FROM finance_import_runs"),
            "finance_quarantined_rows" => sqlx::query("SELECT * FROM finance_quarantined_rows"),
            "finance_source_artifacts" => sqlx::query("SELECT * FROM finance_source_artifacts"),
            "budget_income_expectations" => sqlx::query("SELECT * FROM budget_income_expectations"),
            "budget_ledger_state" => sqlx::query("SELECT * FROM budget_ledger_state"),
            "budget_plan_income" => sqlx::query("SELECT * FROM budget_plan_income"),
            "budget_income_receipts" => sqlx::query("SELECT * FROM budget_income_receipts"),
            "budget_ledger_movements" => sqlx::query("SELECT * FROM budget_ledger_movements"),
            "budget_coverage_closures" => sqlx::query("SELECT * FROM budget_coverage_closures"),
            _ => bail!("Unknown finance export table."),
        };
        let rows = query
            .fetch_all(&mut *snapshot)
            .await
            .context("could not read finance database table")?;
        let encoded = rows
            .iter()
            .map(sqlite_row_json)
            .collect::<anyhow::Result<Vec<_>>>()?;
        counts.insert(table.into(), json!(encoded.len()));
        content.insert(table.into(), Value::Array(encoded));
    }
    let database_schema_version: i64 = sqlx::query_scalar(
        "SELECT COALESCE(MAX(version), 0) FROM _sqlx_migrations WHERE success = 1",
    )
    .fetch_one(&mut *snapshot)
    .await
    .context("could not read the finance schema version")?;
    let operation_watermark: Option<String> =
        sqlx::query_scalar("SELECT MAX(created_at) FROM finance_operations")
            .fetch_one(&mut *snapshot)
            .await
            .context("could not read the finance operation watermark")?;
    snapshot
        .commit()
        .await
        .context("could not finish SQLite snapshot")?;
    let data = json!({
        "interchangeFormatVersion": 1,
        "databaseSchemaVersion": database_schema_version,
        "exportedAt": now_utc(),
        "operationWatermark": operation_watermark,
        "tableCounts": counts.clone(),
        "tables": content,
    });
    let bytes = serde_json::to_vec(&data)?;
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)?;
    }
    write_private_new_file(&output, &bytes)
        .context("could not write private SQLite finance export")?;
    println!("Private SQLite export saved: {}", output.display());
    println!("Finance row counts: {}", Value::Object(counts));
    println!("Export SHA-256: {}", sha256(&bytes));
    pool.close().await;
    Ok(())
}

async fn restore_sqlite_command(options: HashMap<String, String>) -> anyhow::Result<()> {
    let input = option(&options, "input")
        .map(PathBuf::from)
        .ok_or_else(|| anyhow::anyhow!("--input FILE is required."))?;
    let expected_digest =
        option(&options, "sha256").ok_or_else(|| anyhow::anyhow!("--sha256 HEX is required."))?;
    if expected_digest.len() != 64 || !expected_digest.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        bail!("--sha256 must be a 64-character hexadecimal digest.");
    }
    let database = option(&options, "database")
        .map(PathBuf::from)
        .ok_or_else(|| anyhow::anyhow!("--database NEW_FILE is required."))?;
    if database.exists() {
        bail!("Refusing to restore over an existing SQLite database.");
    }
    let bytes = fs::read(&input).context("could not read private SQLite finance export")?;
    let actual_digest = sha256(&bytes);
    if !actual_digest.eq_ignore_ascii_case(&expected_digest) {
        bail!("SQLite finance export digest does not match --sha256.");
    }
    let export: Value =
        serde_json::from_slice(&bytes).context("SQLite finance export JSON is invalid")?;
    if export
        .get("interchangeFormatVersion")
        .and_then(Value::as_u64)
        != Some(1)
    {
        bail!("SQLite finance export format is unsupported.");
    }
    let snapshot_tables = export
        .get("tables")
        .and_then(Value::as_object)
        .ok_or_else(|| anyhow::anyhow!("SQLite finance export has no table data."))?;
    let table_names = [
        "finance_accounts",
        "finance_account_opening_balances",
        "finance_categories",
        "category_budget_periods",
        "finance_transactions",
        "finance_transaction_revisions",
        "transaction_valuations",
        "transaction_valuation_source_cells",
        "budget_month_snapshots",
        "budget_opening_state",
        "finance_exchange_rate_source_rows",
        "exchange_rates",
        "finance_account_actuals",
        "finance_account_balance_checkpoints",
        "finance_operations",
        "finance_category_corrections",
        "finance_category_correction_items",
        "reconciliation_batches",
        "reconciliation_items",
        "finance_import_runs",
        "finance_quarantined_rows",
        "finance_source_artifacts",
        "budget_income_expectations",
        "budget_ledger_state",
        "budget_plan_income",
        "budget_income_receipts",
        "budget_ledger_movements",
        "budget_month_closures",
        "budget_coverage_closures",
    ];
    if snapshot_tables.len() != table_names.len()
        || table_names
            .iter()
            .any(|table| !snapshot_tables.contains_key(*table))
    {
        bail!("SQLite finance export does not contain the complete finance table set.");
    }
    let declared_counts = export
        .get("tableCounts")
        .and_then(Value::as_object)
        .ok_or_else(|| anyhow::anyhow!("SQLite finance export has no table counts."))?;
    if declared_counts.len() != table_names.len() {
        bail!("SQLite finance export table counts are incomplete.");
    }
    let mut expected_counts = BTreeMap::new();
    for table in table_names {
        let rows = snapshot_tables
            .get(table)
            .and_then(Value::as_array)
            .ok_or_else(|| anyhow::anyhow!("SQLite finance export table {table} is invalid."))?;
        if declared_counts.get(table).and_then(Value::as_u64) != Some(rows.len() as u64) {
            bail!("SQLite finance export table counts do not match its rows.");
        }
        expected_counts.insert(table.to_string(), rows.len());
    }

    if let Some(parent) = database
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
    {
        fs::create_dir_all(parent).context("could not create restore directory")?;
    }
    create_private_empty_file(&database)
        .context("could not create a private new restore target")?;
    let pool = open_pool(&database, false).await?;
    sqlx::migrate!()
        .run(&pool)
        .await
        .context("could not apply database migrations to the restore target")?;
    let mut transaction = pool.begin().await.context("could not begin restore")?;
    sqlx::query("PRAGMA defer_foreign_keys = ON")
        .execute(&mut *transaction)
        .await
        .context("could not defer restore foreign keys")?;
    for table in table_names {
        let rows = snapshot_tables
            .get(table)
            .and_then(Value::as_array)
            .ok_or_else(|| anyhow::anyhow!("SQLite finance export table {table} is invalid."))?;
        // `table` comes from the fixed allowlist above; identifiers below come
        // from the local migration schema and are quoted before interpolation.
        let column_rows = sqlx::query(AssertSqlSafe(format!(
            "SELECT name FROM pragma_table_info('{}') ORDER BY cid",
            table
        )))
        .fetch_all(&mut *transaction)
        .await
        .context("could not inspect restore table")?;
        let columns = column_rows
            .iter()
            .map(|row| row.try_get::<String, _>("name"))
            .collect::<Result<Vec<_>, _>>()?;
        let insert_columns = columns
            .iter()
            .map(|column| format!("\"{column}\""))
            .collect::<Vec<_>>()
            .join(", ");
        let select_values = columns
            .iter()
            .map(|column| format!("json_extract(?1, '$.\"{column}\"')"))
            .collect::<Vec<_>>()
            .join(", ");
        let insert = format!("INSERT INTO \"{table}\" ({insert_columns}) SELECT {select_values}");
        for row in rows {
            let object = row.as_object().ok_or_else(|| {
                anyhow::anyhow!("SQLite finance export contains an invalid {table} row.")
            })?;
            if object.len() != columns.len()
                || columns.iter().any(|column| !object.contains_key(column))
            {
                bail!("SQLite finance export columns do not match the {table} schema.");
            }
            sqlx::query(AssertSqlSafe(insert.clone()))
                .bind(serde_json::to_string(row)?)
                .execute(&mut *transaction)
                .await
                .with_context(|| format!("could not restore {table} row"))?;
        }
    }
    transaction
        .commit()
        .await
        .context("could not commit restored database")?;
    let integrity: String = sqlx::query_scalar("PRAGMA integrity_check")
        .fetch_one(&pool)
        .await
        .context("could not check restored database integrity")?;
    if integrity != "ok" {
        bail!("Restored database failed SQLite integrity check.");
    }
    let foreign_key_issues = sqlx::query("PRAGMA foreign_key_check")
        .fetch_all(&pool)
        .await
        .context("could not check restored database foreign keys")?;
    if !foreign_key_issues.is_empty() {
        bail!("Restored database contains broken finance references.");
    }
    for (table, expected) in &expected_counts {
        let actual = table_row_count(&pool, table).await? as usize;
        if actual != *expected {
            bail!("Restored database row counts differ from the snapshot.");
        }
    }
    pool.close().await;
    println!(
        "Private SQLite finance snapshot restored: {}",
        database.display()
    );
    println!("Finance row counts: {}", json!(expected_counts));
    println!("Verified export SHA-256: {actual_digest}");
    Ok(())
}

fn write_private_new_file(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    use std::io::Write;
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(path)
        .context("output file already exists or cannot be created")?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

fn create_private_empty_file(path: &Path) -> anyhow::Result<()> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    drop(
        options
            .open(path)
            .context("restore target already exists or cannot be created")?,
    );
    Ok(())
}

fn sqlite_row_json(row: &sqlx::sqlite::SqliteRow) -> anyhow::Result<Value> {
    let mut object = Map::new();
    for (index, column) in row.columns().iter().enumerate() {
        let raw = row.try_get_raw(index)?;
        let value = if raw.is_null() {
            Value::Null
        } else {
            match raw.type_info().name() {
                "INTEGER" => json!(row.try_get::<i64, _>(index)?),
                "REAL" => json!(row.try_get::<f64, _>(index)?),
                "BLOB" => {
                    let bytes: Vec<u8> = row.try_get(index)?;
                    json!(hex::encode(bytes))
                }
                _ => json!(row.try_get::<String, _>(index)?),
            }
        };
        object.insert(column.name().to_string(), value);
    }
    Ok(Value::Object(object))
}

async fn table_exists(pool: &SqlitePool, name: &str) -> anyhow::Result<bool> {
    let count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?",
    )
    .bind(name)
    .fetch_one(pool)
    .await?;
    Ok(count > 0)
}

async fn table_row_count(pool: &SqlitePool, name: &str) -> anyhow::Result<i64> {
    let count = match name {
        "finance_accounts" => {
            sqlx::query_scalar("SELECT COUNT(*) FROM finance_accounts")
                .fetch_one(pool)
                .await?
        }
        "finance_account_opening_balances" => {
            sqlx::query_scalar("SELECT COUNT(*) FROM finance_account_opening_balances")
                .fetch_one(pool)
                .await?
        }
        "finance_categories" => {
            sqlx::query_scalar("SELECT COUNT(*) FROM finance_categories")
                .fetch_one(pool)
                .await?
        }
        "category_budget_periods" => {
            sqlx::query_scalar("SELECT COUNT(*) FROM category_budget_periods")
                .fetch_one(pool)
                .await?
        }
        "finance_transactions" => {
            sqlx::query_scalar("SELECT COUNT(*) FROM finance_transactions")
                .fetch_one(pool)
                .await?
        }
        "finance_transaction_revisions" => {
            sqlx::query_scalar("SELECT COUNT(*) FROM finance_transaction_revisions")
                .fetch_one(pool)
                .await?
        }
        "transaction_valuations" => {
            sqlx::query_scalar("SELECT COUNT(*) FROM transaction_valuations")
                .fetch_one(pool)
                .await?
        }
        "transaction_valuation_source_cells" => {
            sqlx::query_scalar("SELECT COUNT(*) FROM transaction_valuation_source_cells")
                .fetch_one(pool)
                .await?
        }
        "budget_month_snapshots" => {
            sqlx::query_scalar("SELECT COUNT(*) FROM budget_month_snapshots")
                .fetch_one(pool)
                .await?
        }
        "budget_month_closures" => {
            sqlx::query_scalar("SELECT COUNT(*) FROM budget_month_closures")
                .fetch_one(pool)
                .await?
        }
        "budget_opening_state" => {
            sqlx::query_scalar("SELECT COUNT(*) FROM budget_opening_state")
                .fetch_one(pool)
                .await?
        }
        "finance_exchange_rate_source_rows" => {
            sqlx::query_scalar("SELECT COUNT(*) FROM finance_exchange_rate_source_rows")
                .fetch_one(pool)
                .await?
        }
        "exchange_rates" => {
            sqlx::query_scalar("SELECT COUNT(*) FROM exchange_rates")
                .fetch_one(pool)
                .await?
        }
        "finance_account_actuals" => {
            sqlx::query_scalar("SELECT COUNT(*) FROM finance_account_actuals")
                .fetch_one(pool)
                .await?
        }
        "finance_account_balance_checkpoints" => {
            sqlx::query_scalar("SELECT COUNT(*) FROM finance_account_balance_checkpoints")
                .fetch_one(pool)
                .await?
        }
        "finance_operations" => {
            sqlx::query_scalar("SELECT COUNT(*) FROM finance_operations")
                .fetch_one(pool)
                .await?
        }
        "finance_category_corrections" => {
            sqlx::query_scalar("SELECT COUNT(*) FROM finance_category_corrections")
                .fetch_one(pool)
                .await?
        }
        "finance_category_correction_items" => {
            sqlx::query_scalar("SELECT COUNT(*) FROM finance_category_correction_items")
                .fetch_one(pool)
                .await?
        }
        "reconciliation_batches" => {
            sqlx::query_scalar("SELECT COUNT(*) FROM reconciliation_batches")
                .fetch_one(pool)
                .await?
        }
        "reconciliation_items" => {
            sqlx::query_scalar("SELECT COUNT(*) FROM reconciliation_items")
                .fetch_one(pool)
                .await?
        }
        "finance_import_runs" => {
            sqlx::query_scalar("SELECT COUNT(*) FROM finance_import_runs")
                .fetch_one(pool)
                .await?
        }
        "finance_quarantined_rows" => {
            sqlx::query_scalar("SELECT COUNT(*) FROM finance_quarantined_rows")
                .fetch_one(pool)
                .await?
        }
        "finance_source_artifacts" => {
            sqlx::query_scalar("SELECT COUNT(*) FROM finance_source_artifacts")
                .fetch_one(pool)
                .await?
        }
        "budget_ledger_state"
        | "budget_plan_income"
        | "budget_income_receipts"
        | "budget_ledger_movements"
        | "budget_coverage_closures" => {
            sqlx::query_scalar(AssertSqlSafe(format!("SELECT COUNT(*) FROM {name}")))
                .fetch_one(pool)
                .await?
        }
        "budget_income_expectations" => {
            sqlx::query_scalar("SELECT COUNT(*) FROM budget_income_expectations")
                .fetch_one(pool)
                .await?
        }
        _ => bail!("Unknown finance table."),
    };
    Ok(count)
}

fn header_map(sheet: &SheetArchive) -> HashMap<String, usize> {
    let row = sheet.rows.first().cloned().unwrap_or_default();
    row.iter()
        .enumerate()
        .filter_map(|(index, cell)| {
            let name = cell_text(cell.as_ref())?.trim().to_string();
            if name.is_empty() {
                None
            } else {
                Some((name, index))
            }
        })
        .collect()
}

fn required_column(
    headers: &HashMap<String, usize>,
    name: &str,
    sheet: &str,
) -> anyhow::Result<usize> {
    headers
        .get(name)
        .copied()
        .ok_or_else(|| anyhow::anyhow!("{sheet} is missing required header {name}."))
}

fn record_error(errors: &mut BTreeMap<&'static str, Vec<usize>>, reason: &'static str, row: usize) {
    errors.entry(reason).or_default().push(row);
}

fn cell_text(cell: Option<&ArchiveCell>) -> Option<String> {
    if cell_is_error(cell) {
        return None;
    }
    let value = cell_value(cell, "effective").or_else(|| {
        let entered = cell_value(cell, "entered")?;
        if entered.get("formulaValue").is_some() {
            None
        } else {
            Some(entered)
        }
    })?;
    match value {
        Value::String(value) => Some(value),
        Value::Number(value) => Some(value.to_string()),
        Value::Bool(value) => Some(value.to_string()),
        Value::Object(value) if value.contains_key("type") => None,
        _ => None,
    }
}

fn row_text(row: &[Option<ArchiveCell>], column: usize) -> Option<String> {
    row.get(column)
        .and_then(Option::as_ref)
        .and_then(|cell| cell_text(Some(cell)))
}

fn cell_value(cell: Option<&ArchiveCell>, key: &str) -> Option<Value> {
    let container = match key {
        "entered" => cell?.entered.as_ref(),
        "effective" => cell?.effective.as_ref(),
        _ => None,
    }?;
    [
        "stringValue",
        "numberValue",
        "boolValue",
        "errorValue",
        "formulaValue",
    ]
    .iter()
    .find_map(|key| container.get(*key).cloned())
}

fn row_decimal(row: &[Option<ArchiveCell>], column: usize) -> Option<String> {
    let cell = row.get(column)?.as_ref()?;
    let value = cell_value(Some(cell), "effective")?;
    let text = match value {
        Value::Number(number) => number.to_string(),
        Value::String(value) => value,
        _ => return None,
    };
    if valid_decimal_text(&text) {
        parse_exact_decimal(&text).ok().map(format_exact_decimal)
    } else {
        None
    }
}

fn row_flag(row: &[Option<ArchiveCell>], column: usize) -> bool {
    let Some(value) = row
        .get(column)
        .and_then(Option::as_ref)
        .and_then(|cell| cell_value(Some(cell), "effective"))
    else {
        return false;
    };
    match value {
        Value::Bool(value) => value,
        Value::Number(value) => value.as_i64().unwrap_or_default() != 0,
        Value::String(value) => matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "true" | "1" | "yes"
        ),
        _ => false,
    }
}

fn row_populated(row: &[Option<ArchiveCell>], start: usize, end: usize) -> bool {
    row.iter().take(end).skip(start).any(|cell| {
        let Some(cell) = cell else {
            return false;
        };
        let entered = cell.entered.as_ref();
        let effective = cell.effective.as_ref();
        !is_empty_value(entered) || !is_empty_value(effective)
    })
}

fn transaction_inputs_populated(row: &[Option<ArchiveCell>]) -> bool {
    [0, 1, 2, 5, 6, 7, 8].iter().any(|column| {
        row.get(*column)
            .and_then(Option::as_ref)
            .is_some_and(|cell| {
                !is_empty_value(cell.entered.as_ref()) || !is_empty_value(cell.effective.as_ref())
            })
    })
}

fn is_empty_value(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => true,
        Some(Value::Object(value)) if value.is_empty() => true,
        Some(Value::Object(value))
            if value.len() == 1 && value.get("stringValue").and_then(Value::as_str) == Some("") =>
        {
            true
        }
        _ => false,
    }
}

fn cell_is_error(cell: Option<&ArchiveCell>) -> bool {
    cell.and_then(|cell| cell.effective.as_ref())
        .is_some_and(|value| value.get("errorValue").is_some())
}

fn cell_state(cell: Option<&ArchiveCell>) -> &'static str {
    if cell_is_error(cell) {
        return "error";
    }
    if cell.is_some_and(|cell| cell.data_source_formula.is_some()) {
        return "formula";
    }
    match cell.and_then(|cell| cell.entered.as_ref()) {
        Some(value) if value.get("formulaValue").is_some() => "formula",
        Some(value) if !is_empty_value(Some(value)) => "literal",
        _ => "blank",
    }
}

fn sheet_date(cell: &ArchiveCell) -> Option<String> {
    let value = cell_value(Some(cell), "effective")?;
    if let Value::String(value) = value {
        if valid_date(&value) {
            return Some(value);
        }
        return None;
    }
    let number = match value {
        Value::Number(value) => value.as_f64()?,
        _ => return None,
    };
    if !number.is_finite() || !(0.0..=3_000_000.0).contains(&number) {
        return None;
    }
    let days_from_unix = number.floor() as i64 - 25_569;
    Some(civil_date(days_from_unix))
}

fn civil_date(days_from_unix: i64) -> String {
    let z = days_from_unix + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    year += if month <= 2 { 1 } else { 0 };
    format!("{year:04}-{month:02}-{day:02}")
}

fn valid_date(date: &str) -> bool {
    let bytes = date.as_bytes();
    bytes.len() == 10
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes
            .iter()
            .enumerate()
            .all(|(index, byte)| matches!(index, 4 | 7) || byte.is_ascii_digit())
        && date[..4]
            .parse::<u32>()
            .is_ok_and(|year| (1900..=9999).contains(&year))
        && chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d").is_ok()
}

fn valid_month(month: &str) -> bool {
    let bytes = month.as_bytes();
    bytes.len() == 7
        && bytes[4] == b'-'
        && bytes
            .iter()
            .enumerate()
            .all(|(index, byte)| index == 4 || byte.is_ascii_digit())
        && month[..4].parse::<u32>().is_ok_and(|year| year >= 1900)
        && month[5..]
            .parse::<u32>()
            .is_ok_and(|value| (1..=12).contains(&value))
}

fn classify(
    from_category: &Option<String>,
    to_category: &Option<String>,
    from: &Option<String>,
    to: &Option<String>,
    description: &str,
) -> Option<&'static str> {
    let f = from.as_deref().unwrap_or("");
    let t = to.as_deref().unwrap_or("");
    let fc = from_category.as_deref().unwrap_or("");
    let tc = to_category.as_deref().unwrap_or("");
    if !f.is_empty()
        && t.eq_ignore_ascii_case("Reconciliation")
        && ((fc.eq_ignore_ascii_case("Reconciliation")
            && tc.eq_ignore_ascii_case("Reconciliation"))
            || description.trim().eq_ignore_ascii_case("reconciliation"))
    {
        Some("reconciliation")
    } else if f.eq_ignore_ascii_case("Reallocation") && t.eq_ignore_ascii_case("Reallocation") {
        Some("reallocate")
    } else if f.eq_ignore_ascii_case("Income")
        && fc.eq_ignore_ascii_case("Income")
        && !t.is_empty()
        && !is_system_account(t)
    {
        Some("income")
    } else if (t.eq_ignore_ascii_case("Expense") || tc.eq_ignore_ascii_case("Expense"))
        && !f.is_empty()
        && !is_system_account(f)
    {
        Some("expense")
    } else if !f.is_empty() && !t.is_empty() && !is_system_account(f) && !is_system_account(t) {
        Some("transfer")
    } else {
        None
    }
}

fn is_system_account(name: &str) -> bool {
    ["Expense", "Income", "Reallocation", "Reconciliation"]
        .iter()
        .any(|system| name.eq_ignore_ascii_case(system))
}

fn normalize_name(value: &str) -> String {
    value.to_lowercase()
}

fn rate_pair(value: &str) -> Option<(String, String)> {
    if value.len() != 6 || !value.is_ascii() {
        return None;
    }
    let base = &value[..3];
    let quote = &value[3..];
    if supported_currency(base) && supported_currency(quote) && !base.eq_ignore_ascii_case(quote) {
        Some((base.to_ascii_uppercase(), quote.to_ascii_uppercase()))
    } else {
        None
    }
}

fn column_name(mut column: usize) -> String {
    let mut result = String::new();
    while column > 0 {
        column -= 1;
        result.insert(0, char::from(b'A' + (column % 26) as u8));
        column /= 26;
    }
    result
}

fn supported_currency(value: &str) -> bool {
    ["EUR", "PHP", "USD"]
        .iter()
        .any(|currency| value.eq_ignore_ascii_case(currency))
}

fn has_effective_value(cell: Option<&ArchiveCell>) -> bool {
    cell.and_then(|cell| cell.effective.as_ref())
        .is_some_and(|value| {
            value.get("errorValue").is_none()
                && (value.get("numberValue").is_some()
                    || value.get("boolValue").is_some()
                    || value
                        .get("stringValue")
                        .and_then(Value::as_str)
                        .is_some_and(|text| !text.is_empty()))
        })
}

fn positive_decimal(value: &str) -> bool {
    !value.trim_start().starts_with('-')
        && valid_decimal_text(value)
        && value
            .chars()
            .take_while(|character| *character != 'e' && *character != 'E')
            .any(|character| character.is_ascii_digit() && character != '0')
}

fn decimal_is_zero(value: &str) -> anyhow::Result<bool> {
    Ok(!parse_exact_decimal(value)?
        .digits
        .bytes()
        .any(|digit| digit != b'0'))
}

fn valid_decimal_text(value: &str) -> bool {
    let value = value.trim();
    if value.is_empty() {
        return false;
    }
    let unsigned = value
        .strip_prefix('-')
        .or_else(|| value.strip_prefix('+'))
        .unwrap_or(value);
    let mut parts = unsigned.split(|character| character == 'e' || character == 'E');
    let mantissa = parts.next().unwrap_or_default();
    let exponent = parts.next();
    if parts.next().is_some() || exponent.is_some_and(|item| item.parse::<i32>().is_err()) {
        return false;
    }
    let mut dot_count = 0;
    let mut digit_count = 0;
    for character in mantissa.chars() {
        if character == '.' {
            dot_count += 1;
        } else if character.is_ascii_digit() {
            digit_count += 1;
        } else {
            return false;
        }
    }
    digit_count > 0 && dot_count <= 1
}

#[derive(Clone)]
struct ExactDecimal {
    negative: bool,
    digits: String,
    scale: usize,
}

fn parse_exact_decimal(value: &str) -> anyhow::Result<ExactDecimal> {
    if !valid_decimal_text(value) {
        bail!("Invalid decimal source value.");
    }
    let value = value.trim();
    let negative = value.starts_with('-');
    let unsigned = value
        .strip_prefix('-')
        .or_else(|| value.strip_prefix('+'))
        .unwrap_or(value);
    let mut parts = unsigned.split(|character| character == 'e' || character == 'E');
    let mantissa = parts.next().unwrap_or_default();
    let exponent = parts
        .next()
        .map(str::parse::<i32>)
        .transpose()
        .context("decimal exponent is out of range")?
        .unwrap_or(0);
    if exponent.unsigned_abs() > 10_000 {
        bail!("Decimal exponent exceeds the supported import limit.");
    }
    let fraction_digits = mantissa
        .find('.')
        .map(|position| mantissa.len() - position - 1)
        .unwrap_or(0);
    let mut digits = mantissa
        .chars()
        .filter(|character| *character != '.')
        .collect::<String>();
    let mut scale = fraction_digits as i64 - i64::from(exponent);
    if scale < 0 {
        digits.extend(std::iter::repeat_n('0', (-scale) as usize));
        scale = 0;
    }
    if scale > 10_000 {
        bail!("Decimal scale exceeds the supported import limit.");
    }
    Ok(ExactDecimal {
        negative,
        digits: trim_leading_zeros(&digits),
        scale: scale as usize,
    })
}

fn decimal_absolute(value: &str) -> anyhow::Result<String> {
    let mut value = parse_exact_decimal(value)?;
    value.negative = false;
    Ok(format_exact_decimal(value))
}

fn negate_decimal(value: &str) -> anyhow::Result<String> {
    let mut value = parse_exact_decimal(value)?;
    if value.digits.bytes().any(|digit| digit != b'0') {
        value.negative = !value.negative;
    }
    Ok(format_exact_decimal(value))
}

fn add_decimal_delta(
    totals: &mut HashMap<(String, String), String>,
    key: (String, String),
    delta: &str,
) -> anyhow::Result<()> {
    let old = totals.get(&key).map(String::as_str).unwrap_or("0");
    totals.insert(key, add_exact_decimals(old, delta)?);
    Ok(())
}

fn add_exact_decimals(left: &str, right: &str) -> anyhow::Result<String> {
    let mut left = parse_exact_decimal(left)?;
    let mut right = parse_exact_decimal(right)?;
    let scale = left.scale.max(right.scale);
    left.digits
        .extend(std::iter::repeat_n('0', scale - left.scale));
    right
        .digits
        .extend(std::iter::repeat_n('0', scale - right.scale));
    let (negative, digits) = if left.negative == right.negative {
        (left.negative, add_magnitudes(&left.digits, &right.digits))
    } else {
        match compare_magnitudes(&left.digits, &right.digits) {
            std::cmp::Ordering::Greater => (
                left.negative,
                subtract_magnitudes(&left.digits, &right.digits),
            ),
            std::cmp::Ordering::Less => (
                right.negative,
                subtract_magnitudes(&right.digits, &left.digits),
            ),
            std::cmp::Ordering::Equal => (false, "0".into()),
        }
    };
    Ok(format_exact_decimal(ExactDecimal {
        negative,
        digits: trim_leading_zeros(&digits),
        scale,
    }))
}

fn add_magnitudes(left: &str, right: &str) -> String {
    let mut left = left.bytes().rev();
    let mut right = right.bytes().rev();
    let mut carry = 0u8;
    let mut result = Vec::new();
    loop {
        let a = left.next().map(|digit| digit - b'0');
        let b = right.next().map(|digit| digit - b'0');
        if a.is_none() && b.is_none() && carry == 0 {
            break;
        }
        let sum = a.unwrap_or(0) + b.unwrap_or(0) + carry;
        result.push(b'0' + sum % 10);
        carry = sum / 10;
    }
    result.reverse();
    String::from_utf8(result).expect("decimal digits are valid UTF-8")
}

fn subtract_magnitudes(larger: &str, smaller: &str) -> String {
    let mut larger = larger.bytes().rev();
    let mut smaller = smaller.bytes().rev();
    let mut borrow = 0i16;
    let mut result = Vec::new();
    while let Some(digit) = larger.next() {
        let mut difference = i16::from(digit - b'0')
            - borrow
            - i16::from(smaller.next().map(|value| value - b'0').unwrap_or(0));
        if difference < 0 {
            difference += 10;
            borrow = 1;
        } else {
            borrow = 0;
        }
        result.push(b'0' + difference as u8);
    }
    while result.len() > 1 && result.last() == Some(&b'0') {
        result.pop();
    }
    result.reverse();
    String::from_utf8(result).expect("decimal digits are valid UTF-8")
}

fn compare_magnitudes(left: &str, right: &str) -> std::cmp::Ordering {
    let left = trim_leading_zeros(left);
    let right = trim_leading_zeros(right);
    left.len().cmp(&right.len()).then_with(|| left.cmp(&right))
}

fn trim_leading_zeros(value: &str) -> String {
    let trimmed = value.trim_start_matches('0');
    if trimmed.is_empty() {
        "0".into()
    } else {
        trimmed.into()
    }
}

fn format_exact_decimal(mut value: ExactDecimal) -> String {
    value.digits = trim_leading_zeros(&value.digits);
    if value.digits == "0" {
        return "0".into();
    }
    if value.scale > 0 {
        if value.digits.len() <= value.scale {
            value.digits = format!(
                "{}{}",
                "0".repeat(value.scale + 1 - value.digits.len()),
                value.digits
            );
        }
        let split_at = value.digits.len() - value.scale;
        value.digits.insert(split_at, '.');
        while value.digits.ends_with('0') {
            value.digits.pop();
        }
        if value.digits.ends_with('.') {
            value.digits.pop();
        }
    }
    if value.negative {
        format!("-{}", value.digits)
    } else {
        value.digits
    }
}

fn encode(value: &impl Serialize) -> anyhow::Result<String> {
    serde_json::to_string(value).context("could not encode private source provenance")
}

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn now_utc() -> String {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let days = (seconds / 86_400) as i64;
    let day_seconds = seconds % 86_400;
    format!(
        "{}T{:02}:{:02}:{:02}Z",
        civil_date(days),
        day_seconds / 3_600,
        day_seconds % 3_600 / 60,
        day_seconds % 60
    )
}

#[cfg(test)]
mod date_validation_tests {
    use super::*;

    #[test]
    fn imported_dates_must_exist_in_the_calendar() {
        for date in ["2024-02-29", "2026-09-30", "1900-01-01", "9999-12-31"] {
            assert!(valid_date(date), "{date}");
        }
        for date in [
            "2026-02-29", "2026-04-31", "2026-00-10", "2026-13-01",
            "2026-01-00", "2026-01-99", "2026-2-01", "1899-12-31",
        ] {
            assert!(!valid_date(date), "{date}");
            let cell = ArchiveCell {
                entered: None,
                effective: Some(json!({"stringValue": date})),
                formatted: None,
                data_source_formula: None,
            };
            assert_eq!(sheet_date(&cell), None, "{date}");
        }
    }
}
