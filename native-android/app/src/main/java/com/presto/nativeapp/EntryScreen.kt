@file:OptIn(
    androidx.compose.material3.ExperimentalMaterial3Api::class,
    androidx.compose.foundation.ExperimentalFoundationApi::class,
    androidx.compose.foundation.layout.ExperimentalLayoutApi::class,
)

package com.presto.nativeapp

import androidx.compose.foundation.*
import androidx.compose.foundation.layout.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.onFocusChanged
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.platform.LocalFocusManager
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.*
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.window.Popup
import androidx.compose.ui.window.PopupProperties
import com.presto.core.*
import java.time.*
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

@Composable
internal fun EntryScreen(
    s: AppState,
    model: PrestoViewModel,
    picker: (String) -> Unit,
    save: (Boolean, DuplicateReview?) -> Unit,
    discard: () -> Unit,
) {
    val focusManager = LocalFocusManager.current
    var attemptedSave by remember { mutableStateOf(false) }
    val d = s.draft
    LaunchedEffect(d.amount, d.description) {
        if (d.amount.isBlank() && d.description.isBlank()) attemptedSave = false
    }
    fun submit(another: Boolean) {
        if (BuildConfig.DEBUG) android.util.Log.d("PrestoSave", "Save tap received; add another=$another")
        attemptedSave = true
        focusManager.clearFocus()
        // The repository owns validation and the durable queue commit for every entry type.
        save(another, null)
    }
    s.duplicateReview?.let { review ->
        AlertDialog(
            onDismissRequest = model.repository::dismissDuplicateReview,
            title = { Text("Possible duplicate") },
            text = {
                Column(Modifier.verticalScroll(rememberScrollState()), verticalArrangement = Arrangement.spacedBy(12.dp)) {
                    Text("Similar transactions are already recorded. Save this as a separate transaction?")
                    review.matches.take(5).forEach { row ->
                        Text("${row.effectiveDate} · ${row.description}\n${money(row.amount, row.inputCurrency ?: "EUR")} · ${listOfNotNull(row.fromAccount, row.toAccount, row.fromCategory, row.toCategory).joinToString(" → ")}${if (row.pending) " · Pending sync" else ""}")
                    }
                    if (review.matches.size > 5) Text("And ${review.matches.size - 5} more similar transactions.")
                }
            },
            dismissButton = { TextButton(onClick = model.repository::dismissDuplicateReview) { Text("Keep editing") } },
            confirmButton = { TextButton(enabled = !s.saving, onClick = { save(review.addAnother, review) }) { Text("Save anyway") } },
        )
    }
    val parsedAmount = remember(d.amount) { parseMoney(d.amount) }
    val amountError = when {
        d.amount.isBlank() -> if (attemptedSave) "Enter an amount." else null
        parsedAmount == null -> "Use a valid amount with at most two decimals."
        parsedAmount <= ZERO -> "Amount must be greater than zero."
        else -> null
    }
    val snapshot = s.snapshot
    val fromCategory = snapshot.budgets.find { it.categoryId == d.categoryFrom }
    val toCategory = snapshot.budgets.find { it.categoryId == d.categoryTo }
    val from = snapshot.accounts.find { it.id == d.accountFrom }
    val to = snapshot.accounts.find { it.id == d.accountTo }
    fun change(next: Draft) = model.update(next)
    val latestDraft by rememberUpdatedState(d)
    LaunchedEffect(
        d.description, d.kind, d.categoryFrom, d.categoryTo, d.original,
        d.manualCategory, d.manualFromAccount, d.manualToAccount,
        snapshot.accounts, snapshot.budgets, snapshot.recentTransactions,
    ) {
        // History searches must not run in composition or in text input callbacks.
        val inferred = withContext(Dispatchers.Default) { suggestDraftLinks(d, snapshot) }
        // Preserve amount/date edits made while the background search was running.
        val current = latestDraft
        val next = current.copy(
            categoryFrom = inferred.categoryFrom,
            accountFrom = inferred.accountFrom,
            accountTo = inferred.accountTo,
            currency = inferred.currency,
        )
        if (next != current) model.repository.updateDraft(next, userEdited = false, expected = current)
    }
    Column(Modifier.fillMaxSize()) {
        Column(
            Modifier.weight(1f).verticalScroll(rememberScrollState()).padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            if (d.original != null)
                Text(
                    "Editing ${d.original?.description}",
                    color = Primary,
                    fontWeight = FontWeight.Bold,
                )
            FlowRow(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(4.dp)) {
                listOf("expense", "income", "transfer", "reallocate").forEach { kind ->
                    FilterChip(
                        selected = d.kind == kind,
                        onClick = {
                            change(
                                d.copy(
                                    kind = kind,
                                    showMore = false,
                                    balancing = false,
                                    manualCategory = false,
                                    manualFromAccount = false,
                                    manualToAccount = false,
                                    currency = if (kind == "reallocate") "EUR" else d.currency,
                                    date = if (kind == "reallocate") previousMonthEnd() else d.date,
                                )
                            )
                        },
                        label = { Text(kindNames[kind] ?: kind, fontSize = 14.sp, maxLines = 1) },
                        modifier = Modifier.testTag("kind-$kind"),
                    )
                }
            }
            var formulaKeyboard by remember { mutableStateOf(d.amount.trim().startsWith("=")) }
            var amountFocused by remember { mutableStateOf(false) }
            var descriptionFocused by remember { mutableStateOf(false) }
            var suggestionsDismissed by remember { mutableStateOf(false) }
            Box(Modifier.fillMaxWidth()) {
                OutlinedTextField(
                    value = d.amount,
                    onValueChange = {
                        if (it.trim().startsWith("=")) formulaKeyboard = true
                        change(d.copy(amount = it))
                    },
                    label = { Text("Amount") },
                    trailingIcon = {
                        TextButton(
                            onClick = { picker("currency") },
                            enabled = d.kind != "reallocate",
                        ) {
                            Text(d.currency, fontWeight = FontWeight.SemiBold)
                        }
                    },
                    textStyle = MaterialTheme.typography.titleLarge,
                    singleLine = true,
                    keyboardOptions = KeyboardOptions(keyboardType = if (formulaKeyboard) KeyboardType.Text else KeyboardType.Decimal),
                    supportingText = amountError?.let { message -> { Text(message) } },
                    shape = androidx.compose.foundation.shape.RoundedCornerShape(12.dp),
                    modifier =
                        Modifier.fillMaxWidth().testTag("entry-amount").onFocusChanged {
                            amountFocused = it.isFocused
                        },
                    isError = amountError != null,
                )
                if (amountFocused) {
                    val offset = with(LocalDensity.current) { 38.dp.roundToPx() }
                    Popup(
                        alignment = Alignment.TopEnd,
                        offset = IntOffset(0, -offset),
                        properties = PopupProperties(focusable = false),
                    ) {
                        Surface(
                            color = MaterialTheme.colorScheme.surfaceContainerHigh,
                            shape = androidx.compose.foundation.shape.RoundedCornerShape(10.dp),
                            shadowElevation = 3.dp,
                        ) {
                            Text(
                                if (d.amount.isNotBlank() && parsedAmount == null)
                                    "Use up to two decimals. Formulas start with =."
                                else if (d.amount.startsWith("="))
                                    parsedAmount?.let { "= ${money(it, d.currency)}" }
                                        ?: "Formula example: =12+8"
                                else "Formula example: =12+8",
                                modifier = Modifier.padding(horizontal = 10.dp, vertical = 6.dp),
                                color =
                                    if (d.amount.isNotBlank() && parsedAmount == null)
                                        Danger
                                    else Muted,
                                fontSize = 12.sp,
                            )
                        }
                    }
                }
            }
            TextButton(
                onClick = { formulaKeyboard = !formulaKeyboard },
                modifier = Modifier.testTag("amount-keyboard-toggle"),
            ) { Text(if (formulaKeyboard) "Use numeric keyboard" else "Use formula keyboard (=12+8)") }
            Field(
                "Description",
                d.description,
                { text ->
                    suggestionsDismissed = false
                    change(d.copy(description = text))
                },
                "entry-description",
                Modifier.onFocusChanged { descriptionFocused = it.isFocused },
                error = if (attemptedSave && d.description.isBlank()) "Add a description." else null,
            )
            val suggestions by produceState<List<DescriptionSuggestion>>(
                emptyList(), d.description, d.kind, snapshot.recentTransactions, d.original?.id,
            ) {
                value = emptyList()
                value = withContext(Dispatchers.Default) {
                    descriptionSuggestions(d.description, snapshot.recentTransactions,
                        d.kind, d.original?.id)
                }
            }
            if (descriptionFocused && !suggestionsDismissed && suggestions.isNotEmpty()) {
                OutlinedCard(shape = androidx.compose.foundation.shape.RoundedCornerShape(12.dp)) {
                    suggestions.forEachIndexed { index, suggestion ->
                        val t = suggestion.transaction
                        val category =
                            snapshot.budgets.find {
                                it.categoryId ==
                                    (if (d.kind == "income") t.toCategoryId else t.fromCategoryId)
                            }
                        val account =
                            snapshot.accounts.find {
                                it.id ==
                                    (if (d.kind == "income") t.toAccountId else t.fromAccountId)
                            }
                        Column(
                            Modifier.fillMaxWidth()
                                .clickable {
                                    change(applyDescriptionSuggestion(d, snapshot, suggestion))
                                    suggestionsDismissed = true
                                    focusManager.clearFocus()
                                }
                                .padding(horizontal = 12.dp, vertical = 8.dp)
                                .testTag("description-suggestion-${t.id}")
                        ) {
                            Text(
                                t.description,
                                fontSize = 14.sp,
                                fontWeight = FontWeight.Medium,
                                maxLines = 1,
                                overflow = TextOverflow.Ellipsis,
                            )
                            val detail =
                                listOfNotNull(
                                        category?.name?.takeUnless { category.system },
                                        account?.name,
                                    )
                                    .joinToString(" · ")
                            if (detail.isNotBlank())
                                Text(
                                    detail,
                                    fontSize = 12.sp,
                                    color = Muted,
                                    maxLines = 1,
                                    overflow = TextOverflow.Ellipsis,
                                )
                        }
                        if (index != suggestions.lastIndex)
                            HorizontalDivider(color = MaterialTheme.colorScheme.outlineVariant)
                    }
                }
            }
            if (d.kind == "expense") {
                Choice(
                    "Budget category",
                    fromCategory?.name,
                    "${money(fromCategory?.remainingEur)} available",
                    { picker("categoryFrom") },
                )
                Choice("Pay from account", from?.name, from?.currency, { picker("accountFrom") })
            }
            if (d.kind == "income") {
                Row(horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                    listOf(
                            "salary" to "Salary",
                            "unexpected" to "Extra income",
                            "refund" to "Refund",
                        )
                        .forEach { (id, label) ->
                            FilterChip(
                                selected = d.incomeTreatment == id,
                                onClick = { change(d.copy(incomeTreatment = id)) },
                                label = { Text(label) },
                            )
                        }
                }
                if (d.incomeTreatment == "salary")
                    Field(
                        "Expected salary month (YYYY-MM)",
                        d.expectedIncomeMonth,
                        { change(d.copy(expectedIncomeMonth = it)) },
                        "expected-income-month",
                    )
                if (d.incomeTreatment in listOf("refund", "legacy"))
                    Choice(
                        "Restore spending category",
                        toCategory?.name,
                        choose = { picker("categoryTo") },
                    )
                if (d.incomeTreatment == "legacy")
                    Text("Preserves the existing receipt's category credit.", fontSize = 13.sp)
                Choice("Receive in account", to?.name, to?.currency, { picker("accountTo") })
            }
            if (d.kind == "transfer") {
                Choice("From account", from?.name, from?.currency, { picker("accountFrom") })
                Choice("To account", to?.name, to?.currency, { picker("accountTo") })
                if (from?.currency != to?.currency)
                    Field(
                        "Amount received (${to?.currency ?: "destination currency"})",
                        d.toAmount,
                        { change(d.copy(toAmount = it)) },
                        "entry-to-amount",
                        keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Decimal),
                        error = if ((attemptedSave || d.toAmount.isNotBlank()) &&
                            (parseMoney(d.toAmount)?.let { it > ZERO } != true))
                            "Enter a positive received amount with at most two decimals." else null,
                    )
            }
            if (d.kind == "reallocate") {
                if (d.balancing)
                    Section {
                        Text("MONTH-END BALANCING", color = Primary, fontWeight = FontWeight.Bold)
                        val suggestion =
                            suggestBalance(snapshot.budgets, d.categoryFrom, d.categoryTo)
                        Text(
                            "Use successive positive categories to cover negative remaining budgets."
                        )
                        if (suggestion.amount > ZERO)
                            TextButton(
                                onClick = {
                                    change(
                                        d.copy(
                                            categoryFrom = suggestion.source?.categoryId,
                                            categoryTo = suggestion.destination?.categoryId,
                                            amount = suggestion.amount.toPlainString(),
                                        )
                                    )
                                }
                            ) {
                                Text(
                                    "Suggest ${money(suggestion.amount)}: ${suggestion.source?.name} → ${suggestion.destination?.name}"
                                )
                            }
                    }
                Choice(
                    "From category",
                    fromCategory?.name,
                    "${money(fromCategory?.remainingEur)} available",
                    { picker("categoryFrom") },
                )
                Choice(
                    "To category",
                    toCategory?.name,
                    "${money(toCategory?.remainingEur)} available",
                    { picker("categoryTo") },
                )
            }
            Row(
                Modifier.horizontalScroll(rememberScrollState()),
                horizontalArrangement = Arrangement.spacedBy(6.dp),
            ) {
                listOf(
                        "Today" to today(),
                        "Yesterday" to LocalDate.now().minusDays(1).toString(),
                        "Month-end" to previousMonthEnd(),
                    )
                    .forEach { (label, date) ->
                        SuggestionChip(
                            onClick = {
                                change(
                                    d.copy(
                                        date = if (d.kind == "reallocate") monthEnd(date) else date
                                    )
                                )
                            },
                            label = { Text(label) },
                        )
                    }
            }
            var showDate by remember { mutableStateOf(false) }
            Choice("Date", d.date, "${zone().id} · posting date", { showDate = true })
            if (showDate) {
                val initial =
                    runCatching {
                            LocalDate.parse(d.date)
                                .atStartOfDay(ZoneOffset.UTC)
                                .toInstant()
                                .toEpochMilli()
                        }
                        .getOrDefault(System.currentTimeMillis())
                val dateState = rememberDatePickerState(initialSelectedDateMillis = initial)
                DatePickerDialog(
                    onDismissRequest = { showDate = false },
                    confirmButton = {
                        TextButton(
                            onClick = {
                                dateState.selectedDateMillis?.let { millis ->
                                    val date =
                                        Instant.ofEpochMilli(millis)
                                            .atZone(ZoneOffset.UTC)
                                            .toLocalDate()
                                            .toString()
                                    change(
                                        d.copy(
                                            date =
                                                if (d.kind == "reallocate") monthEnd(date) else date
                                        )
                                    )
                                }
                                showDate = false
                            }
                        ) {
                            Text("Choose")
                        }
                    },
                    dismissButton = {
                        TextButton(onClick = { showDate = false }) { Text("Cancel") }
                    },
                ) {
                    DatePicker(dateState)
                }
            }
            TextButton(
                onClick = { change(d.copy(showMore = !d.showMore)) },
                modifier = Modifier.align(Alignment.CenterHorizontally),
            ) {
                Text(if (d.showMore) "Hide more" else "More options")
            }
            if (d.showMore) {
                Field("Optional note", d.note, { change(d.copy(note = it)) }, "entry-note")
                if (d.kind in listOf("expense", "income") && !s.demo)
                    Choice(
                        "Correction link",
                        snapshot.recentTransactions.find { it.id == d.correctionOf }?.description
                            ?: "New activity",
                        choose = { picker("correction") },
                    )
            }
            val lines = preview(d, snapshot)
            if (lines.isNotEmpty())
                Section {
                    Text("What this entry changes", fontWeight = FontWeight.Bold)
                    lines.forEach { Text(it, fontSize = 14.sp) }
                }
            OutlinedButton(
                onClick = { submit(true) },
                enabled = !s.saving && d.original == null,
                modifier = Modifier.fillMaxWidth().testTag("save-another"),
            ) {
                Text(if (d.balancing) "Save and continue balancing" else "Save and add another")
            }
            if (d.touched)
                TextButton(
                    onClick = { attemptedSave = false; discard() },
                    modifier = Modifier.align(Alignment.CenterHorizontally),
                ) {
                    Text(if (d.original != null) "Cancel edit" else "Clear draft")
                }
            if (s.demo)
                Text(
                    "Local demo entries stay on this device.",
                    fontSize = 13.sp,
                    modifier = Modifier.align(Alignment.CenterHorizontally),
                )
            if (s.pending > 0)
                Text(
                    "${s.pending} pending entries. Refresh or reopen to retry sync.",
                    fontSize = 13.sp,
                )
            s.recoveryIssue?.let { Text("Recovery backup: $it", fontSize = 13.sp, color = Danger) }
        }
        Surface(color = MaterialTheme.colorScheme.surface, shadowElevation = 3.dp) {
            Column(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 10.dp)) {
                s.error?.let { message ->
                    Text(
                        message, color = Danger,
                        modifier = Modifier.fillMaxWidth().padding(bottom = 8.dp)
                            .testTag("entry-validation-error")
                            .semantics { liveRegion = LiveRegionMode.Assertive },
                    )
                }
                Button(
                    onClick = { submit(false) },
                    enabled = !s.saving,
                    modifier = Modifier.fillMaxWidth().heightIn(min = 52.dp).testTag("save-entry"),
                ) {
                    Text(
                        if (s.saving) "Saving…"
                        else if (d.original != null) "Save changes" else "Save entry"
                    )
                }
            }
        }
    }
}
