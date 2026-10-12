@file:OptIn(
    androidx.compose.material3.ExperimentalMaterial3Api::class,
    androidx.compose.foundation.ExperimentalFoundationApi::class,
)

package com.presto.nativeapp

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.*
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.*
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import com.presto.core.*
import java.time.*

internal val Primary = Color(0xff17675e)
internal val Danger = Color(0xffad4038)
internal val Positive = Color(0xff267457)
internal val Background = Color(0xfff6f5f0)
internal val chartColors =
    listOf(
        Color(0xff17675e),
        Color(0xffe39640),
        Color(0xff4ba58b),
        Color(0xff9e6fc0),
        Color(0xffd5656d),
        Color(0xff64a6c8),
        Color(0xff86845b),
        Color(0xffaf738c),
    )
internal val kindNames =
    mapOf(
        "expense" to "Expense",
        "income" to "Income",
        "transfer" to "Transfer",
        "reallocate" to "Budget move",
        "reconciliation" to "Reconciliation",
    )

@Composable
fun PrestoApp(model: PrestoViewModel, login: () -> Unit, requestNotification: () -> Unit) {
    val state by model.state.collectAsStateWithLifecycle()
    val logs by model.repository.logs.collectAsStateWithLifecycle()
    var inspected by remember { mutableStateOf<Transaction?>(null) }
    val notificationTransaction by model.notificationTransaction
    LaunchedEffect(notificationTransaction) {
        notificationTransaction?.let { inspected = it; model.notificationTransaction.value = null }
    }
    LaunchedEffect(state.session?.subject) { inspected = null }

    var screen by rememberSaveable { mutableStateOf("add") }
    var picker by rememberSaveable { mutableStateOf<String?>(null) }
    var confirmation by remember { mutableStateOf<(() -> Unit)?>(null) }
    var confirmationTitle by remember { mutableStateOf("") }
    var confirmationText by remember { mutableStateOf("") }
    var query by rememberSaveable { mutableStateOf("") }
    var entityId by rememberSaveable { mutableStateOf<String?>(null) }
    var entityKind by rememberSaveable { mutableStateOf<String?>(null) }
    var recentMonth by rememberSaveable { mutableStateOf(today().take(7)) }
    var type by rememberSaveable { mutableStateOf("all") }
    fun confirm(title: String, text: String, action: () -> Unit) {
        confirmationTitle = title
        confirmationText = text
        confirmation = action
    }
    fun replaceDraft(next: Draft) {
        if (next.original != null && next.original?.id == state.draft.original?.id) {
            screen = "add"
            return
        }
        if (state.draft.touched)
            confirm("Replace unfinished entry?", "Your saved draft will be replaced.") {
                model.update(next, userEdited = false)
                screen = "add"
            }
        else {
            model.update(next, userEdited = false)
            screen = "add"
        }
    }
    fun drill(category: Category, month: String) {
        entityId = category.categoryId
        entityKind = "category"
        query = category.name
        recentMonth = month
        type = "all"
        screen = "recent"
    }
    MaterialTheme(
        colorScheme =
            lightColorScheme(
                primary = Primary,
                background = Background,
                surface = Color(0xfffffefa),
                surfaceContainerLowest = Color(0xfffffefa),
                surfaceContainerLow = Color(0xfffffefa),
                surfaceContainer = Color(0xfff1f2eb),
                surfaceContainerHigh = Color(0xffeeeee7),
                surfaceContainerHighest = Color(0xffe8eae1),
                onSurface = Ink,
                onBackground = Ink,
                secondary = Primary,
                secondaryContainer = SoftAccent,
                onSecondaryContainer = Ink,
                tertiary = Primary,
                surfaceTint = Primary,
                primaryContainer = Color(0xffdceee7),
                onPrimaryContainer = Ink,
                surfaceVariant = Color(0xffeeeee7),
                onSurfaceVariant = Muted,
                outline = Color(0xff858c85),
                outlineVariant = Color(0xffdedfd6),
                error = Danger,
            ),
        shapes =
            Shapes(
                small = RoundedCornerShape(12.dp),
                medium = RoundedCornerShape(18.dp),
                large = RoundedCornerShape(24.dp),
            ),
    ) {
        Scaffold(
            // Edge-to-edge windows do not resize Compose content above the IME automatically.
            modifier = Modifier.imePadding(),
            containerColor = Background,
            topBar = {
                Column(
                    Modifier.fillMaxWidth()
                        .background(MaterialTheme.colorScheme.surface)
                        .statusBarsPadding()
                        .padding(horizontal = 16.dp, vertical = 10.dp)
                ) {
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        Column(Modifier.weight(1f)) {
                            Text(
                                mapOf(
                                    "add" to "New entry",
                                    "recent" to "Activity",
                                    "budget" to "Budget",
                                    "accounts" to "Accounts",
                                    "setup" to "Settings",
                                )[screen] ?: "Presto",
                                fontSize = 23.sp,
                                fontWeight = FontWeight.Bold,
                            )
                        }
                        Surface(
                            onClick = { screen = "setup" },
                            color = if (state.pending > 0) Color(0xffffedcf) else SoftAccent,
                            shape = RoundedCornerShape(24.dp),
                        ) {
                            Text(
                                if (state.demo) "Demo"
                                else if (state.syncing) "Syncing…"
                                else if (state.session == null) "Sign in"
                                else if (state.pending > 0) "${state.pending} pending"
                                else "Synced",
                                modifier = Modifier.padding(horizontal = 12.dp, vertical = 10.dp),
                                color = Ink,
                                style = MaterialTheme.typography.labelMedium,
                            )
                        }
                    }
                    if (state.syncing)
                        LinearProgressIndicator(Modifier.fillMaxWidth().padding(top = 4.dp))
                }
            },
            bottomBar = {
                NavigationBar(containerColor = MaterialTheme.colorScheme.surface) {
                    listOf(
                            "recent" to "Activity",
                            "add" to "Add",
                            "budget" to "Budget",
                            "accounts" to "Accounts",
                            "setup" to "Settings",
                        )
                        .forEach { (id, label) ->
                            NavigationBarItem(
                                selected = screen == id,
                                onClick = {
                                    screen = id
                                    if (id in listOf("recent", "budget", "accounts"))
                                        model.refreshIfStale()
                                },
                                colors =
                                    NavigationBarItemDefaults.colors(
                                        selectedIconColor = Primary,
                                        selectedTextColor = Primary,
                                        unselectedIconColor = Muted,
                                        unselectedTextColor = Muted,
                                        indicatorColor = Color.Transparent,
                                    ),
                                icon = {
                                    Box(
                                        Modifier.background(
                                                if (screen == id) SoftAccent else Color.Transparent,
                                                androidx.compose.foundation.shape.CircleShape,
                                            )
                                            .padding(6.dp)
                                    ) {
                                        PrestoIcon(id)
                                    }
                                },
                                label = { Text(label, fontSize = 11.sp, maxLines = 1) },
                                modifier = Modifier.testTag("tab-$id"),
                            )
                        }
                }
            },
        ) { insets ->
            if (!state.ready)
                Box(Modifier.fillMaxSize().padding(insets), contentAlignment = Alignment.Center) {
                    CircularProgressIndicator()
                }
            else
                Column(Modifier.fillMaxSize().padding(insets)) {
                    val failedOperation = state.outbox.firstOrNull { it.error != null }
                    val error = model.localError.value ?: state.error ?: failedOperation?.let(::syncErrorSummary)
                    if (error != null)
                        Row(
                            Modifier.fillMaxWidth().background(Color(0xffffecea)).padding(12.dp)
                                .semantics { liveRegion = LiveRegionMode.Assertive },
                            verticalAlignment = Alignment.CenterVertically,
                        ) {
                            Text(error, color = Danger, modifier = Modifier.weight(1f))
                            if (failedOperation != null)
                                TextButton(onClick = { model.clearError(); screen = "setup" }) { Text("Review") }
                            else TextButton(onClick = model::clearError) { Text("Dismiss") }
                        }
                    state.notice?.takeIf { failedOperation == null }?.let { notice ->
                        Row(
                            Modifier.fillMaxWidth()
                                .background(Color(0xffeaf5ef))
                                .padding(horizontal = 12.dp),
                            verticalAlignment = Alignment.CenterVertically,
                        ) {
                            Text(notice, modifier = Modifier.weight(1f), fontSize = 14.sp)
                            TextButton(onClick = model::clearError) { Text("OK") }
                        }
                    }
                    when (screen) {
                        "add" ->
                            EntryScreen(
                                state,
                                model,
                                { picker = it },
                                { another, review ->
                                    model.save(another, review) {
                                        if (!another) screen = "recent"
                                        else if (
                                            state.draft.balancing &&
                                                !model.state.value.draft.balancing
                                        )
                                            screen = "budget"
                                        if (!state.demo) requestNotification()
                                    }
                                },
                                { replaceDraft(defaultDraft(state.snapshot)) },
                            )
                        "recent" ->
                            RecentScreen(
                                state,
                                query,
                                {
                                    query = it
                                    entityId = null
                                    entityKind = null
                                },
                                recentMonth,
                                { recentMonth = it },
                                type,
                                { type = it },
                                entityId,
                                entityKind,
                                { e ->
                                    entityId = e.id
                                    entityKind = e.kind
                                    query = e.name
                                },
                                {
                                    entityId = null
                                    entityKind = null
                                    query = ""
                                },
                                { t -> inspected = t },
                                model::refresh,
                            )
                        "budget" ->
                            BudgetScreen(
                                state,
                                ::drill,
                                {
                                    val suggestion = suggestBalance(state.snapshot.budgets)
                                    replaceDraft(
                                        defaultDraft(state.snapshot)
                                            .copy(
                                                kind = "reallocate",
                                                amount = suggestion.amount.toPlainString(),
                                                description = "Month-end budget balancing",
                                                date = previousMonthEnd(),
                                                currency = "EUR",
                                                categoryFrom = suggestion.source?.categoryId,
                                                categoryTo = suggestion.destination?.categoryId,
                                                balancing = true,
                                            )
                                    )
                                },
                                { e ->
                                    confirm(
                                        "Complete expected income?",
                                        "Remove ${money(e.outstandingEur)} of anticipated funding from Unallocated? Category allocations stay as planned.",
                                    ) {
                                        model.complete(e.month)
                                    }
                                },
                            )
                        "accounts" -> AccountsScreen(state.snapshot)
                        "setup" ->
                            SetupScreen(
                                state,
                                login,
                                {
                                    confirm(
                                        "Disconnect Pocket ID?",
                                        "Pending entries and unfinished drafts remain on this device for this owner.",
                                    ) {
                                        model.logout()
                                    }
                                },
                                model::refresh,
                                {
                                    confirm(
                                        "Restore this phone's latest snapshot?",
                                        "The server accepts recovery only when every finance table is empty and administrator recovery mode is enabled. Resolve pending writes first.",
                                    ) {
                                        model.restore()
                                    }
                                },
                                model::retry,
                                { id ->
                                    confirm(
                                        "Review failed entry?",
                                        "Load saved values against current confirmed revisions? Review them before saving. Your current draft will be replaced; the failed operation stays queued until a replacement is saved.",
                                    ) {
                                        model.review(id) { screen = "add" }
                                    }
                                },
                                requestNotification,
                                logs,
                                model::partnerNotifications,
                            )
                    }
                }
        }
        inspected?.let { inspectedTransaction ->
            val transaction = state.snapshot.recentTransactions.find { it.id == inspectedTransaction.id } ?: inspectedTransaction
            TransactionDetails(
                transaction,
                state.snapshot,
                { inspected = null },
                {
                    inspected = null
                    replaceDraft(editDraft(transaction, state.snapshot))
                },
                operation = state.outbox.find { it.transactionId == transaction.id },
                syncing = state.syncing,
                retry = model::refresh,
                review = { inspected = null; screen = "setup" },
                deleting = state.saving,
                delete = {
                    confirm(
                        "Delete entry?",
                        "Delete “${transaction.description}” for ${money(transaction.amount, transaction.inputCurrency ?: "EUR")}? This reverses its effect on balances and budgets and cannot be undone.",
                    ) {
                        model.delete(transaction.id) { inspected = null }
                    }
                },
            )
        }
        if (picker != null)
            PickerSheet(
                picker!!,
                state,
                { value ->
                    val d = state.draft
                    model.update(
                        when (picker) {
                            "categoryFrom" -> d.copy(categoryFrom = value, manualCategory = true)
                            "categoryTo" -> d.copy(categoryTo = value, manualCategory = true)
                            "accountFrom" ->
                                d.copy(
                                    accountFrom = value,
                                    currency =
                                        state.snapshot.accounts.find { it.id == value }?.currency
                                            ?: d.currency,
                                    manualFromAccount = true,
                                )
                            "accountTo" ->
                                d.copy(
                                    accountTo = value,
                                    currency =
                                        if (d.kind == "income")
                                            state.snapshot.accounts
                                                .find { it.id == value }
                                                ?.currency ?: d.currency
                                        else d.currency,
                                    manualToAccount = true,
                                )
                            "currency" -> d.copy(currency = value)
                            else -> d.copy(correctionOf = value)
                        }
                    )
                    picker = null
                },
                { picker = null },
            )
        confirmation?.let { action ->
            AlertDialog(
                onDismissRequest = { confirmation = null },
                title = { Text(confirmationTitle) },
                text = { Text(confirmationText) },
                confirmButton = {
                    TextButton(
                        onClick = {
                            confirmation = null
                            action()
                        }
                    ) {
                        Text("Confirm")
                    }
                },
                dismissButton = { TextButton(onClick = { confirmation = null }) { Text("Cancel") } },
            )
        }
        BackHandler(
            enabled = screen != "add" && picker == null && confirmation == null && inspected == null
        ) {
            screen = "add"
        }
    }
}

@Composable
internal fun Section(content: @Composable ColumnScope.() -> Unit) {
    Card(
        Modifier.fillMaxWidth(),
        shape = RoundedCornerShape(14.dp),
        border = BorderStroke(1.dp, MaterialTheme.colorScheme.outlineVariant),
        colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.surface),
    ) {
        Column(
            Modifier.padding(12.dp),
            verticalArrangement = Arrangement.spacedBy(6.dp),
            content = content,
        )
    }
}

@Composable
internal fun Field(
    label: String,
    value: String,
    change: (String) -> Unit,
    tag: String = label,
    modifier: Modifier = Modifier,
    keyboardOptions: androidx.compose.foundation.text.KeyboardOptions =
        androidx.compose.foundation.text.KeyboardOptions.Default,
    error: String? = null,
) {
    OutlinedTextField(
        value = value,
        onValueChange = change,
        label = {
            Text(
                label,
                maxLines = 1,
                overflow = androidx.compose.ui.text.style.TextOverflow.Ellipsis,
            )
        },
        modifier = modifier.fillMaxWidth().testTag(tag),
        singleLine = true,
        keyboardOptions = keyboardOptions,
        isError = error != null,
        supportingText = error?.let { message -> { Text(message) } },
        shape = RoundedCornerShape(16.dp),
    )
}

@Composable
internal fun Choice(label: String, value: String?, detail: String? = null, choose: () -> Unit) {
    OutlinedCard(
        onClick = choose,
        modifier = Modifier.fillMaxWidth().testTag("choose-${normalize(label).replace(' ', '-')}"),
    ) {
        Row(
            Modifier.padding(12.dp).heightIn(min = 40.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
                Text(label, fontSize = 12.sp, color = Muted, maxLines = 1)
                Row(
                    horizontalArrangement = Arrangement.spacedBy(8.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Text(
                        value ?: "Choose…",
                        modifier = Modifier.weight(1f),
                        fontWeight = FontWeight.Medium,
                        fontSize = 15.sp,
                        maxLines = 1,
                        overflow = androidx.compose.ui.text.style.TextOverflow.Ellipsis,
                    )
                    detail?.let {
                        Text(
                            it,
                            fontSize = 12.sp,
                            color = Muted,
                            maxLines = 1,
                            overflow = androidx.compose.ui.text.style.TextOverflow.Ellipsis,
                        )
                    }
                }
            }
            PrestoIcon("next", color = Muted)
        }
    }
}
