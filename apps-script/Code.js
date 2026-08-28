/* global LockService, Session, Sheets, SpreadsheetApp */

var TRANSACTION_HEADERS_ = [
  'Date', 'Description', 'Amount', 'Currency', 'To Amount', 'From Category',
  'To Category', 'From', 'To', 'From Account Currency',
  'From Account Currency Exchange EUR', 'From Account Amount EUR',
  'From Account Currency Exchange Rate Native', 'From Account Amount Native',
  'To Account Currency', 'To Account Currency Exchange EUR', 'To Account Amount EUR',
  'Transaction ID', 'Created At', 'Created By',
];
var FORMULA_COLUMNS_ = [3, 4, 9, 10, 11, 12, 13, 14, 15, 16];
var INPUT_COLUMNS_ = [0, 1, 2, 5, 6, 7, 8];
var CURRENCIES_ = ['EUR', 'PHP', 'USD'];
var SYSTEM_ = {
  expenseCategory: 'Expense',
  incomeCategory: 'Income',
  transferCategory: 'Balance Transfer',
  expenseAccount: 'Expense',
  incomeAccount: 'Income',
  reallocationAccount: 'Reallocation',
};

function bootstrapConfiguration(configuration) {
  configuration = configuration || {};
  var properties = PropertiesService.getScriptProperties();
  if (properties.getProperty('SPREADSHEET_ID')) {
    throw new Error('Gateway configuration already exists.');
  }
  var email = activeEmail_();
  var spreadsheetId = requiredString_(configuration.spreadsheetId, 'spreadsheetId');
  var createdBy = requiredString_(configuration.createdBy, 'createdBy');
  SpreadsheetApp.openById(spreadsheetId).getName();
  properties.setProperties({
    SPREADSHEET_ID: spreadsheetId,
    OWNER_EMAIL: email,
    ALLOWED_USERS: JSON.stringify([email]),
    CREATED_BY_MAP: JSON.stringify((function () {
      var result = {};
      result[email] = createdBy;
      return result;
    }())),
  });
  return { configured: true, allowedUser: email, createdBy: createdBy };
}

function bootstrapTestConfiguration(configuration) {
  return bootstrapConfiguration(configuration);
}

function setAllowedUsers(users) {
  var caller = assertAllowedCaller_();
  var owner = PropertiesService.getScriptProperties().getProperty('OWNER_EMAIL');
  if (caller !== owner) throw new Error('Only the gateway owner can change the user allowlist.');
  if (!Array.isArray(users) || users.length < 1 || users.length > 5) {
    throw new Error('users must contain between one and five entries.');
  }
  var normalized = users.map(function (user) {
    if (!user || !String(user.email || '').includes('@')) throw new Error('Each user needs an email.');
    return {
      email: String(user.email).trim().toLowerCase(),
      createdBy: requiredString_(user.createdBy, 'createdBy'),
    };
  });
  var emails = normalized.map(function (user) { return user.email; });
  var labels = {};
  normalized.forEach(function (user) { labels[user.email] = user.createdBy; });
  PropertiesService.getScriptProperties().setProperties({
    ALLOWED_USERS: JSON.stringify(emails),
    CREATED_BY_MAP: JSON.stringify(labels),
  });
  return { allowedUsers: emails.length };
}

function healthCheck() {
  var caller = assertAllowedCaller_();
  var spreadsheet = configuredSpreadsheet_();
  assertTransactionHeaders_(spreadsheet.getSheetByName('Transactions'));
  return {
    ok: true,
    caller: caller,
    workbook: spreadsheet.getName(),
    timezone: spreadsheet.getSpreadsheetTimeZone(),
  };
}

function setConfiguredSpreadsheet(configuration) {
  configuration = configuration || {};
  var caller = assertAllowedCaller_();
  var properties = PropertiesService.getScriptProperties();
  if (caller !== properties.getProperty('OWNER_EMAIL')) {
    throw new Error('Only the gateway owner can change the configured workbook.');
  }
  var spreadsheetId = requiredString_(configuration.spreadsheetId, 'spreadsheetId');
  var spreadsheet = SpreadsheetApp.openById(spreadsheetId);
  assertTransactionHeaders_(spreadsheet.getSheetByName('Transactions'));
  var accountSheet = spreadsheet.getSheetByName('Accounts');
  var accountHeaders = accountSheet.getRange(1, 1, 1, accountSheet.getLastColumn()).getValues()[0].map(String);
  if (accountHeaders.indexOf('IsLive') === -1) {
    throw new Error('The target workbook must have an Accounts IsLive column.');
  }
  accountColumns_(accountHeaders);
  properties.setProperty('SPREADSHEET_ID', spreadsheetId);
  return {
    configured: true,
    workbook: spreadsheet.getName(),
    timezone: spreadsheet.getSpreadsheetTimeZone(),
  };
}

function getFinanceSnapshot(request) {
  assertAllowedCaller_();
  request = request || {};
  var asOf = String(request.asOf || Utilities.formatDate(new Date(), 'UTC', 'yyyy-MM-dd'));
  if (!/^\d{4}-\d{2}-\d{2}$/.test(asOf)) throw new Error('asOf must use YYYY-MM-DD.');
  var recentCount = request.recentCount === undefined ? 50 : Number(request.recentCount);
  if (!Number.isInteger(recentCount) || recentCount < 0 || recentCount > 200) {
    throw new Error('recentCount must be an integer between 0 and 200.');
  }
  var spreadsheet = configuredSpreadsheet_();
  var transactionSheet = spreadsheet.getSheetByName('Transactions');
  assertTransactionHeaders_(transactionSheet);
  var transactionRange = transactionSheet.getRange(1, 1, transactionSheet.getMaxRows(), 20);
  var transactionValues = transactionRange.getValues();
  var transactionFormulas = transactionRange.getFormulas();
  var records = [];
  for (var rowIndex = 1; rowIndex < transactionValues.length; rowIndex += 1) {
    var row = transactionValues[rowIndex];
    if (INPUT_COLUMNS_.some(function (column) { return populated_(row[column]); })) {
      records.push(transactionResult_(row, rowIndex + 1, spreadsheet.getSpreadsheetTimeZone(), transactionFormulas[rowIndex]));
    }
  }

  var budgetSheet = spreadsheet.getSheetByName('Expenses v3');
  var budgetValues = budgetSheet.getRange(1, 1, budgetSheet.getLastRow(), 42).getValues();
  var targetMonth = asOf.slice(0, 7);
  var expenseColumn = -1;
  for (var budgetColumn = 0; budgetColumn < budgetValues[0].length; budgetColumn += 1) {
    if (budgetValues[0][budgetColumn] === 'Expense'
        && monthKey_(budgetValues[1][budgetColumn], spreadsheet.getSpreadsheetTimeZone()) === targetMonth) {
      expenseColumn = budgetColumn;
      break;
    }
  }
  if (expenseColumn === -1) throw new Error('No budget columns found for ' + targetMonth + '.');
  var budgets = [];
  for (var budgetRow = 2; budgetRow < budgetValues.length; budgetRow += 1) {
    var budget = budgetValues[budgetRow];
    if (populated_(budget[0]) && Boolean(budget[41])) {
      budgets.push({
        name: String(budget[0]),
        monthlyEur: nullable_(budget[1]),
        expenseEur: nullable_(budget[expenseColumn]),
        remainingEur: nullable_(budget[expenseColumn + 1]),
      });
    }
  }

  var accountSheet = spreadsheet.getSheetByName('Accounts');
  var accountValues = accountSheet.getRange(1, 1, accountSheet.getLastRow(), accountSheet.getLastColumn()).getValues();
  var accountColumns = accountColumns_(accountValues[0]);
  var accounts = [];
  for (var accountRow = 1; accountRow < accountValues.length; accountRow += 1) {
    var account = accountValues[accountRow];
    if (populated_(account[accountColumns.name])
        && Boolean(account[accountColumns.isReal])
        && liveFlag_(account[accountColumns.isLive])) {
      accounts.push({
        name: String(account[accountColumns.name]),
        type: nullable_(account[accountColumns.type]),
        currency: nullable_(account[accountColumns.currency]),
        balanceNative: nullable_(account[accountColumns.balanceNative]),
        actualBalanceNative: nullable_(account[accountColumns.actualBalanceNative]),
        reconciliationNative: nullable_(account[accountColumns.reconciliationNative]),
        balanceEur: nullable_(account[accountColumns.balanceEur]),
        actualBalanceEur: nullable_(account[accountColumns.actualBalanceEur]),
      });
    }
  }
  return {
    generatedAt: new Date().toISOString(),
    asOf: asOf,
    budgetMonth: targetMonth,
    transactionCount: records.length,
    recentTransactions: records.slice(Math.max(0, records.length - recentCount)).reverse(),
    budgets: budgets,
    accounts: accounts,
  };
}

function submitTransaction(payload) {
  var caller = assertAllowedCaller_();
  payload = payload || {};
  var transactionId = requiredString_(payload.transactionId, 'transactionId').toLowerCase();
  if (!/^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/.test(transactionId)) {
    throw new Error('transactionId must be UUIDv7.');
  }
  var lock = LockService.getScriptLock();
  lock.waitLock(30000);
  try {
    var spreadsheet = configuredSpreadsheet_();
    var sheet = spreadsheet.getSheetByName('Transactions');
    assertTransactionHeaders_(sheet);
    var duplicate = sheet.getRange(2, 18, sheet.getMaxRows() - 1, 1)
      .createTextFinder(transactionId)
      .matchEntireCell(true)
      .findNext();
    if (duplicate) {
      return {
        duplicate: true,
        transaction: transactionResult_(
          sheet.getRange(duplicate.getRow(), 1, 1, 20).getValues()[0],
          duplicate.getRow(),
          spreadsheet.getSpreadsheetTimeZone(),
          sheet.getRange(duplicate.getRow(), 1, 1, 20).getFormulas()[0]
        ),
      };
    }

    var normalized = normalizeTransaction_(payload, spreadsheet);
    var rowNumber = nextPreparedRow_(sheet);
    var formulaRange = sheet.getRange(rowNumber, 1, 1, 17);
    var formulasBefore = formulaRange.getFormulas()[0];
    assertFormulaTemplate_(formulasBefore, rowNumber);
    var validationCount = sheet.getRange(rowNumber, 1, 1, 9).getDataValidations()[0]
      .filter(function (rule) { return rule !== null; }).length;
    if (validationCount < 5) throw new Error('Prepared row has fewer than five validation rules.');

    var createdAt = new Date().toISOString();
    var createdBy = createdByFor_(caller);
    var values = [
      [0, serialDate_(normalized.date)],
      [1, normalized.description],
      [2, normalized.amount],
      [5, normalized.fromCategory],
      [6, normalized.toCategory],
      [7, normalized.fromAccount],
      [8, normalized.toAccount],
      [17, transactionId],
      [18, createdAt],
      [19, createdBy],
    ];
    if (normalized.currency) values.push([3, normalized.currency]);
    if (normalized.toAmount !== null) values.push([4, normalized.toAmount]);
    Sheets.Spreadsheets.Values.batchUpdate({
      valueInputOption: 'RAW',
      data: values.map(function (entry) {
        return {
          range: "'Transactions'!" + columnName_(entry[0] + 1) + rowNumber,
          majorDimension: 'ROWS',
          values: [[entry[1]]],
        };
      }),
    }, spreadsheet.getId());
    SpreadsheetApp.flush();

    var formulasAfter = formulaRange.getFormulas()[0];
    var intentionallyReplaced = {};
    if (normalized.currency) intentionallyReplaced[3] = true;
    if (normalized.toAmount !== null) intentionallyReplaced[4] = true;
    FORMULA_COLUMNS_.forEach(function (column) {
      if (!intentionallyReplaced[column] && formulasAfter[column] !== formulasBefore[column]) {
        throw new Error('Formula integrity failed in ' + columnName_(column + 1) + rowNumber + '.');
      }
    });
    return {
      duplicate: false,
      transaction: transactionResult_(
        sheet.getRange(rowNumber, 1, 1, 20).getValues()[0],
        rowNumber,
        spreadsheet.getSpreadsheetTimeZone(),
        sheet.getRange(rowNumber, 1, 1, 20).getFormulas()[0]
      ),
    };
  } finally {
    lock.releaseLock();
  }
}

function updateTransaction(payload) {
  var caller = assertAllowedCaller_();
  payload = payload || {};
  var transactionId = requiredString_(payload.transactionId, 'transactionId').toLowerCase();
  if (!/^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/.test(transactionId)) {
    throw new Error('transactionId must be UUIDv7.');
  }
  var rowNumber = Number(payload.row);
  if (!Number.isInteger(rowNumber) || rowNumber < 2) throw new Error('row must identify a transaction row.');
  var lock = LockService.getScriptLock();
  lock.waitLock(30000);
  try {
    var spreadsheet = configuredSpreadsheet_();
    var sheet = spreadsheet.getSheetByName('Transactions');
    assertTransactionHeaders_(sheet);
    if (rowNumber > sheet.getMaxRows()) throw new Error('Transaction row no longer exists.');
    var currentRange = sheet.getRange(rowNumber, 1, 1, 20);
    var current = currentRange.getValues()[0];
    var currentFormulas = currentRange.getFormulas()[0];
    if (!INPUT_COLUMNS_.some(function (column) { return populated_(current[column]); })) {
      throw new Error('Transaction row is now empty. Refresh before editing.');
    }
    var existingId = String(current[17] || '').toLowerCase();
    var expectedId = String(payload.expectedTransactionId || '').toLowerCase();
    if (expectedId && existingId !== expectedId) throw new Error('Transaction changed before this edit. Refresh and try again.');
    if (!expectedId && existingId) throw new Error('Manual transaction was claimed by another edit. Refresh and try again.');
    if (existingId && existingId !== transactionId) throw new Error('Transaction ID cannot be changed.');
    var timezone = spreadsheet.getSpreadsheetTimeZone();
    var expectedRevision = String(payload.expectedRevision || '');
    if (expectedRevision) {
      if (transactionRevision_(current, currentFormulas, timezone) !== expectedRevision) {
        throw new Error('Transaction changed before this edit. Refresh and try again.');
      }
    } else {
    if (dateText_(current[0], timezone) !== String(payload.expectedDate || '')) {
      throw new Error('Transaction date changed before this edit. Refresh and try again.');
    }
    if (String(current[1] || '') !== String(payload.expectedDescription || '')) {
      throw new Error('Transaction description changed before this edit. Refresh and try again.');
    }
    if (Number(current[2] || 0) !== Number(payload.expectedAmount || 0)) {
      throw new Error('Transaction amount changed before this edit. Refresh and try again.');
    }
    }

    var normalized = normalizeTransaction_(payload, spreadsheet);
    var templateRow = nextPreparedRow_(sheet);
    var templateFormulas = sheet.getRange(templateRow, 1, 1, 17).getFormulas()[0];
    assertFormulaTemplate_(templateFormulas, templateRow);
    FORMULA_COLUMNS_.forEach(function (column) {
      sheet.getRange(templateRow, column + 1).copyTo(
        sheet.getRange(rowNumber, column + 1),
        SpreadsheetApp.CopyPasteType.PASTE_FORMULA,
        false
      );
    });

    var values = [
      [0, serialDate_(normalized.date)],
      [1, normalized.description],
      [2, normalized.amount],
      [5, normalized.fromCategory],
      [6, normalized.toCategory],
      [7, normalized.fromAccount],
      [8, normalized.toAccount],
      [17, transactionId],
      [18, current[18] || new Date().toISOString()],
      [19, current[19] || createdByFor_(caller)],
    ];
    if (normalized.currency) values.push([3, normalized.currency]);
    if (normalized.toAmount !== null) values.push([4, normalized.toAmount]);
    Sheets.Spreadsheets.Values.batchUpdate({
      valueInputOption: 'RAW',
      data: values.map(function (entry) {
        return {
          range: "'Transactions'!" + columnName_(entry[0] + 1) + rowNumber,
          majorDimension: 'ROWS',
          values: [[entry[1]]],
        };
      }),
    }, spreadsheet.getId());
    SpreadsheetApp.flush();

    var formulasAfter = sheet.getRange(rowNumber, 1, 1, 17).getFormulas()[0];
    var intentionallyReplaced = {};
    if (normalized.currency) intentionallyReplaced[3] = true;
    if (normalized.toAmount !== null) intentionallyReplaced[4] = true;
    FORMULA_COLUMNS_.forEach(function (column) {
      if (!intentionallyReplaced[column] && (!formulasAfter[column] || formulasAfter[column].charAt(0) !== '=')) {
        throw new Error('Formula integrity failed in ' + columnName_(column + 1) + rowNumber + '.');
      }
    });
    return {
      updated: true,
      transaction: transactionResult_(
        sheet.getRange(rowNumber, 1, 1, 20).getValues()[0],
        rowNumber,
        timezone,
        sheet.getRange(rowNumber, 1, 1, 20).getFormulas()[0]
      ),
    };
  } finally {
    lock.releaseLock();
  }
}

function normalizeTransaction_(payload, spreadsheet) {
  var kind = requiredString_(payload.kind, 'kind');
  if (['expense', 'income', 'transfer', 'reallocate'].indexOf(kind) === -1) {
    throw new Error('Unsupported transaction kind.');
  }
  var date = requiredString_(payload.date, 'date');
  if (!/^\d{4}-\d{2}-\d{2}$/.test(date)) throw new Error('date must use YYYY-MM-DD.');
  var amount = Number(payload.amount);
  if (!Number.isFinite(amount) || amount <= 0) throw new Error('amount must be greater than zero.');
  var description = requiredString_(payload.description, 'description');
  if (description.length > 500) throw new Error('description must not exceed 500 characters.');
  var currency = payload.currency ? String(payload.currency).toUpperCase() : null;
  if (currency && CURRENCIES_.indexOf(currency) === -1) throw new Error('Unsupported currency.');
  var toAmount = payload.toAmount === undefined || payload.toAmount === null || payload.toAmount === ''
    ? null : Number(payload.toAmount);
  if (toAmount !== null && (!Number.isFinite(toAmount) || toAmount <= 0)) {
    throw new Error('toAmount must be greater than zero.');
  }

  var budgetSheet = spreadsheet.getSheetByName('Expenses v3');
  var budgetRows = budgetSheet.getRange(3, 1, budgetSheet.getLastRow() - 2, 42).getValues();
  var categories = {};
  budgetRows.forEach(function (row) {
    if (row[0]) categories[String(row[0])] = { isReal: Boolean(row[41]) };
  });
  var accountSheet = spreadsheet.getSheetByName('Accounts');
  var accountValues = accountSheet.getRange(1, 1, accountSheet.getLastRow(), accountSheet.getLastColumn()).getValues();
  var accountColumns = accountColumns_(accountValues[0]);
  var accountRows = accountValues.slice(1);
  var accounts = {};
  accountRows.forEach(function (row) {
    if (row[accountColumns.name]) {
      accounts[String(row[accountColumns.name])] = {
        isReal: Boolean(row[accountColumns.isReal]),
        currency: String(row[accountColumns.currency] || ''),
        isLive: liveFlag_(row[accountColumns.isLive]),
      };
    }
  });
  function realCategory(value, field) {
    var name = requiredString_(value, field);
    if (!categories[name] || !categories[name].isReal) throw new Error(field + ' must be a real category.');
    return name;
  }
  function realAccount(value, field) {
    var name = requiredString_(value, field);
    if (!accounts[name] || !accounts[name].isReal || !accounts[name].isLive) {
      throw new Error(field + ' must be a live real account.');
    }
    return name;
  }

  var result = { kind: kind, date: date, description: description, amount: amount, currency: currency, toAmount: toAmount };
  if (kind === 'expense') {
    result.fromCategory = realCategory(payload.fromCategory, 'fromCategory');
    result.toCategory = SYSTEM_.expenseCategory;
    result.fromAccount = realAccount(payload.fromAccount, 'fromAccount');
    result.toAccount = SYSTEM_.expenseAccount;
    if (toAmount !== null) throw new Error('Expense does not accept toAmount.');
  } else if (kind === 'income') {
    result.fromCategory = SYSTEM_.incomeCategory;
    result.toCategory = realCategory(payload.toCategory, 'toCategory');
    result.fromAccount = SYSTEM_.incomeAccount;
    result.toAccount = realAccount(payload.toAccount, 'toAccount');
    if (toAmount !== null) throw new Error('Income does not accept toAmount.');
  } else if (kind === 'transfer') {
    result.fromCategory = SYSTEM_.transferCategory;
    result.toCategory = SYSTEM_.transferCategory;
    result.fromAccount = realAccount(payload.fromAccount, 'fromAccount');
    result.toAccount = realAccount(payload.toAccount, 'toAccount');
    if (result.fromAccount === result.toAccount) throw new Error('Transfer accounts must differ.');
    var crossCurrency = accounts[result.fromAccount].currency !== accounts[result.toAccount].currency;
    if (crossCurrency && toAmount === null) throw new Error('Cross-currency transfer requires toAmount.');
    if (!crossCurrency && toAmount !== null) throw new Error('Same-currency transfer must not include toAmount.');
  } else {
    result.fromCategory = realCategory(payload.fromCategory, 'fromCategory');
    result.toCategory = realCategory(payload.toCategory, 'toCategory');
    if (result.fromCategory === result.toCategory) throw new Error('Reallocation categories must differ.');
    result.fromAccount = SYSTEM_.reallocationAccount;
    result.toAccount = SYSTEM_.reallocationAccount;
    if (toAmount !== null || currency) throw new Error('Reallocation does not accept currency overrides.');
  }
  if (result.currency && accounts[result.fromAccount]
      && result.currency === accounts[result.fromAccount].currency) {
    result.currency = null;
  }
  return result;
}

function nextPreparedRow_(sheet) {
  var maxRows = sheet.getMaxRows();
  var values = sheet.getRange(2, 1, maxRows - 1, 9).getValues();
  var lastInputOffset = -1;
  for (var index = 0; index < values.length; index += 1) {
    if (INPUT_COLUMNS_.some(function (column) { return populated_(values[index][column]); })) {
      lastInputOffset = index;
    }
  }
  var rowNumber = lastInputOffset + 3;
  if (rowNumber > maxRows) throw new Error('No prepared transaction rows remain.');
  return rowNumber;
}

function assertTransactionHeaders_(sheet) {
  if (!sheet) throw new Error('Transactions sheet was not found.');
  var actual = sheet.getRange(1, 1, 1, 20).getValues()[0].map(String);
  if (JSON.stringify(actual) !== JSON.stringify(TRANSACTION_HEADERS_)) {
    throw new Error('Transactions header fingerprint changed.');
  }
}

function assertFormulaTemplate_(formulas, rowNumber) {
  FORMULA_COLUMNS_.forEach(function (column) {
    if (!formulas[column] || formulas[column].charAt(0) !== '=') {
      throw new Error('Prepared row ' + rowNumber + ' is missing formula ' + columnName_(column + 1) + '.');
    }
  });
  var exact = {};
  exact[4] = '=C' + rowNumber;
  exact[11] = '=C' + rowNumber + '*K' + rowNumber;
  exact[13] = '=C' + rowNumber + '*M' + rowNumber;
  exact[16] = '=P' + rowNumber + '*E' + rowNumber;
  Object.keys(exact).forEach(function (column) {
    if (formulas[Number(column)] !== exact[column]) {
      throw new Error('Prepared row formula contract changed in ' + columnName_(Number(column) + 1) + '.');
    }
  });
}

function explicitToAmount_(row, formulas) {
  return formulas && formulas[4] ? null : nullable_(row[4]);
}

function transactionRevision_(row, formulas, timezone) {
  return JSON.stringify([
    dateText_(row[0], timezone),
    String(row[1] || ''),
    nullable_(row[2]),
    nullable_(row[3]),
    explicitToAmount_(row, formulas),
    nullable_(row[5]),
    nullable_(row[6]),
    nullable_(row[7]),
    nullable_(row[8]),
    nullable_(row[17]),
  ]);
}

function transactionResult_(row, rowNumber, timezone, formulas) {
  return {
    row: rowNumber,
    date: dateText_(row[0], timezone),
    description: String(row[1] || ''),
    amount: nullable_(row[2]),
    currency: nullable_(row[3]),
    toAmount: explicitToAmount_(row, formulas),
    fromCategory: nullable_(row[5]),
    toCategory: nullable_(row[6]),
    fromAccount: nullable_(row[7]),
    toAccount: nullable_(row[8]),
    fromAmountEur: nullable_(row[11]),
    fromAmountNative: nullable_(row[13]),
    toAmountEur: nullable_(row[16]),
    transactionId: nullable_(row[17]),
    createdAt: nullable_(row[18]),
    createdBy: nullable_(row[19]),
    revision: transactionRevision_(row, formulas, timezone),
  };
}

function requiredHeaderIndex_(headers, name) {
  var index = headers.indexOf(name);
  if (index === -1) throw new Error('Accounts header "' + name + '" was not found.');
  return index;
}

function accountColumns_(headerRow) {
  var headers = headerRow.map(String);
  var isLive = headers.indexOf('IsLive');
  if (isLive === -1) isLive = headers.indexOf('IsActive');
  if (isLive === -1) throw new Error('Accounts header "IsLive" was not found.');
  return {
    name: requiredHeaderIndex_(headers, 'Name'),
    type: requiredHeaderIndex_(headers, 'Type'),
    isReal: requiredHeaderIndex_(headers, 'IsReal'),
    isLive: isLive,
    currency: requiredHeaderIndex_(headers, 'Currency'),
    balanceNative: requiredHeaderIndex_(headers, 'Balance'),
    actualBalanceNative: requiredHeaderIndex_(headers, 'Actual Balance'),
    reconciliationNative: requiredHeaderIndex_(headers, 'Reconciliation Amount'),
    balanceEur: requiredHeaderIndex_(headers, 'Balance (EUR)'),
    actualBalanceEur: requiredHeaderIndex_(headers, 'Actual Balance (EUR)'),
  };
}

function liveFlag_(value) {
  return value === true || value === 1 || String(value).trim() === '1';
}

function configuredSpreadsheet_() {
  var id = PropertiesService.getScriptProperties().getProperty('SPREADSHEET_ID');
  if (!id) throw new Error('Gateway is not configured.');
  return SpreadsheetApp.openById(id);
}

function assertAllowedCaller_() {
  var email = activeEmail_();
  var raw = PropertiesService.getScriptProperties().getProperty('ALLOWED_USERS');
  if (!raw) throw new Error('Gateway is not configured.');
  var allowed = JSON.parse(raw).map(function (value) { return String(value).toLowerCase(); });
  if (allowed.indexOf(email) === -1) throw new Error('Caller is not allowed.');
  return email;
}

function activeEmail_() {
  var email = String(Session.getActiveUser().getEmail() || '').trim().toLowerCase();
  if (!email) throw new Error('Google did not expose the signed-in caller email.');
  return email;
}

function createdByFor_(email) {
  var raw = PropertiesService.getScriptProperties().getProperty('CREATED_BY_MAP') || '{}';
  var labels = JSON.parse(raw);
  return requiredString_(labels[email], 'Created By mapping');
}

function requiredString_(value, name) {
  if (value === undefined || value === null || String(value).trim() === '') {
    throw new Error(name + ' is required.');
  }
  return String(value).trim();
}

function populated_(value) {
  return value !== undefined && value !== null && value !== '';
}

function nullable_(value) {
  if (value instanceof Date) return value.toISOString();
  return populated_(value) ? value : null;
}

function serialDate_(dateText) {
  return (Date.parse(dateText + 'T00:00:00Z') - Date.UTC(1899, 11, 30)) / 86400000;
}

function dateText_(value, timezone) {
  if (value instanceof Date) return Utilities.formatDate(value, timezone, 'yyyy-MM-dd');
  if (typeof value === 'number') {
    return Utilities.formatDate(new Date(Date.UTC(1899, 11, 30) + value * 86400000), 'UTC', 'yyyy-MM-dd');
  }
  return value ? String(value) : null;
}

function monthKey_(value, timezone) {
  var text = dateText_(value, timezone);
  return text ? text.slice(0, 7) : '';
}

function columnName_(columnNumber) {
  var value = columnNumber;
  var result = '';
  while (value > 0) {
    value -= 1;
    result = String.fromCharCode(65 + (value % 26)) + result;
    value = Math.floor(value / 26);
  }
  return result;
}
