@file:OptIn(
    androidx.compose.material3.ExperimentalMaterial3Api::class,
    androidx.compose.foundation.layout.ExperimentalLayoutApi::class,
)

package com.presto.nativeapp

import androidx.compose.foundation.*
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.*
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.presto.core.*
import java.time.*
import java.time.format.DateTimeFormatter

@Composable
internal fun RecentScreen(
    s: AppState,
    query: String,
    changeQuery: (String) -> Unit,
    month: String,
    changeMonth: (String) -> Unit,
    type: String,
    changeType: (String) -> Unit,
    entityId: String?,
    entityKind: String?,
    chooseEntity: (Entity) -> Unit,
    clear: () -> Unit,
    edit: (Transaction) -> Unit,
    refresh: () -> Unit,
) {
    val allEntities = entities(s.snapshot)
    val selected = allEntities.find { it.id == entityId && it.kind == entityKind }
    var filters by remember { mutableStateOf(false) }
    val rows =
        s.snapshot.recentTransactions
            .filter {
                it.effectiveDate.startsWith(month) &&
                    (type == "all" || it.kind == type) &&
                    matchesSearch(it, query, allEntities, selected)
            }
            .sortedWith(
                compareByDescending<Transaction> { it.effectiveDate }
                    .thenByDescending { it.occurredAt ?: "" }
                    .thenByDescending { it.id }
            )
    Column(Modifier.fillMaxSize().padding(horizontal = 16.dp)) {
        Row(
            Modifier.padding(top = 8.dp, bottom = 4.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            CompactSearch(
                query,
                changeQuery,
                "transaction-search",
                Modifier.weight(1f),
                "Search activity",
            )
            IconButton(
                onClick = { filters = true },
                modifier =
                    Modifier.testTag("activity-filters").semantics {
                        contentDescription = "Filter activity"
                    },
            ) {
                PrestoIcon(
                    "setup",
                    color = if (type != "all" || month != today().take(7)) Primary else Muted,
                )
            }
        }
        if (selected != null) {
            InputChip(
                selected = true,
                onClick = clear,
                label = { Text(selected.name, maxLines = 1, overflow = TextOverflow.Ellipsis) },
                trailingIcon = { Text("×") },
            )
        } else if (query.isNotBlank()) {
            val suggestions =
                allEntities
                    .filter { it.name.contains(query.trim(), ignoreCase = true) }
                    .sortedWith(
                        compareByDescending<Entity> { it.name.equals(query.trim(), true) }
                            .thenBy { it.name }
                    )
                    .take(8)
            if (suggestions.isNotEmpty())
                Row(
                    Modifier.horizontalScroll(rememberScrollState()),
                    horizontalArrangement = Arrangement.spacedBy(6.dp),
                ) {
                    suggestions.forEach { e ->
                        SuggestionChip(
                            onClick = { chooseEntity(e) },
                            label = { Text("${e.name} · ${e.kind}", maxLines = 1) },
                        )
                    }
                }
        }
        if (type != "all" || month != today().take(7)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text(
                    "${monthLabel(month)} · ${kindNames[type] ?: "All types"}",
                    Modifier.weight(1f),
                    fontSize = 12.sp,
                    color = Muted,
                )
                TextButton(
                    onClick = {
                        changeType("all")
                        changeMonth(today().take(7))
                        clear()
                    }
                ) {
                    Text("Reset")
                }
            }
        }
        LazyColumn(
            Modifier.weight(1f).testTag("activity-list"),
            verticalArrangement = Arrangement.spacedBy(4.dp),
            contentPadding = PaddingValues(bottom = 12.dp),
        ) {
            if (rows.isEmpty())
                item {
                    Text(
                        "No matching transactions. Change the filters or search.",
                        Modifier.padding(vertical = 20.dp),
                        color = Muted,
                    )
                }
            rows
                .groupBy { it.effectiveDate }
                .forEach { (date, transactions) ->
                    item(key = "day-$date") {
                        Text(
                            dateLabel(date),
                            fontSize = 12.sp,
                            fontWeight = FontWeight.SemiBold,
                            color = Muted,
                            modifier = Modifier.padding(top = 8.dp, bottom = 2.dp),
                        )
                    }
                    items(transactions, key = { it.id }) { t ->
                        TransactionRow(t, allEntities, s.snapshot.accounts, edit, s.outbox.find { it.transactionId == t.id })
                    }
                }
        }
    }
    if (filters)
        ModalBottomSheet(onDismissRequest = { filters = false }) {
            Column(
                Modifier.fillMaxWidth().padding(16.dp),
                verticalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                Text("Activity filters", style = MaterialTheme.typography.titleLarge)
                Row(verticalAlignment = Alignment.CenterVertically) {
                    TextButton(
                        onClick = { changeMonth(YearMonth.parse(month).minusMonths(1).toString()) }
                    ) {
                        Text("‹", fontSize = 24.sp)
                    }
                    Text(monthLabel(month), Modifier.weight(1f), fontWeight = FontWeight.SemiBold)
                    TextButton(
                        onClick = { changeMonth(YearMonth.parse(month).plusMonths(1).toString()) }
                    ) {
                        Text("›", fontSize = 24.sp)
                    }
                }
                FlowRow(horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                    (listOf("all" to "All types") + kindNames.toList()).forEach { (id, name) ->
                        FilterChip(
                            selected = id == type,
                            onClick = { changeType(id) },
                            label = { Text(name) },
                        )
                    }
                }
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    OutlinedButton(
                        onClick = refresh,
                        enabled = !s.syncing,
                        modifier = Modifier.weight(1f),
                    ) {
                        Text(if (s.syncing) "Syncing…" else "Refresh")
                    }
                    Button(onClick = { filters = false }, modifier = Modifier.weight(1f)) {
                        Text("Done")
                    }
                }
            }
        }
}

@Composable
private fun TransactionRow(
    t: Transaction,
    allEntities: List<Entity>,
    accounts: List<Account>,
    inspect: (Transaction) -> Unit,
    operation: OutboxRow? = null,
) {
    fun name(id: String?, kind: String, fallback: String?) =
        allEntities.find { it.id == id && it.kind == kind }?.name ?: fallback
    val fromCategory = name(t.fromCategoryId, "category", t.fromCategory)
    val toCategory = name(t.toCategoryId, "category", t.toCategory)
    val fromAccount = name(t.fromAccountId, "account", t.fromAccount)
    val toAccount = name(t.toAccountId, "account", t.toAccount)
    val context =
        when (t.kind) {
            "expense" -> listOfNotNull(fromCategory, fromAccount).distinct().joinToString(" · ")
            "income" ->
                listOfNotNull(
                        "Income",
                        toAccount,
                        toCategory?.takeUnless {
                            t.toCategoryId == "income" || it.equals("Income", true)
                        },
                    )
                    .joinToString(" · ")
            "transfer" -> "Transfer · " + listOfNotNull(fromAccount, toAccount).joinToString(" → ")
            "reallocate" ->
                "Budget move · " + listOfNotNull(fromCategory, toCategory).joinToString(" → ")
            else ->
                listOfNotNull(kindNames[t.kind], fromAccount, toAccount)
                    .distinct()
                    .joinToString(" · ")
        }
    val time =
        t.occurredAt?.let {
            runCatching {
                    Instant.parse(it).atZone(zone()).format(DateTimeFormatter.ofPattern("HH:mm"))
                }
                .getOrNull()
        }
    OutlinedCard(
        onClick = { inspect(t) },
        shape = RoundedCornerShape(12.dp),
        modifier = Modifier.fillMaxWidth().testTag("transaction-${t.id}"),
    ) {
        Row(
            Modifier.fillMaxWidth()
                .padding(horizontal = 12.dp, vertical = 8.dp)
                .heightIn(min = 40.dp),
            horizontalArrangement = Arrangement.spacedBy(10.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
                Text(
                    t.description,
                    fontSize = 15.sp,
                    fontWeight = FontWeight.Medium,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
                Text(
                    context,
                    fontSize = 12.sp,
                    color = Muted,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
                if (t.pending || t.hasCategoryCorrection || t.correctsTransactionId != null) {
                    Text(
                        listOfNotNull(
                                if (t.pending) {
                                    if (operation?.status == "blocked") "Sync failed · needs review"
                                    else if (operation?.error != null) "Sync delayed · tap for details"
                                    else "Pending sync"
                                } else null,
                                if (t.hasCategoryCorrection) "Category corrected" else null,
                                if (t.correctsTransactionId != null) "Linked correction" else null,
                            )
                            .joinToString(" · "),
                        fontSize = 11.sp,
                        color = if (operation?.status == "blocked") Danger else if (t.pending) PendingColor else Muted,
                    )
                }
            }
            Column(
                horizontalAlignment = Alignment.End,
                verticalArrangement = Arrangement.spacedBy(2.dp),
            ) {
                Text(
                    (if (t.kind == "expense") "−" else if (t.kind == "income") "+" else "") +
                        money(t.amount, t.inputCurrency ?: "EUR"),
                    color = transactionColor(t.kind),
                    fontSize = 15.sp,
                    fontWeight = FontWeight.SemiBold,
                    maxLines = 1,
                )
                if (t.toAmount != null)
                    Text(
                        "→ ${money(t.toAmount, t.toCurrency ?: accounts.find { it.id == t.toAccountId }?.currency ?: "EUR")}",
                        fontSize = 11.sp,
                        color = TransferColor,
                    )
                else if (time != null) Text(time, fontSize = 11.sp, color = Muted)
            }
        }
    }
}

private fun dateLabel(date: String): String =
    when (date) {
        today() -> "Today"
        LocalDate.now().minusDays(1).toString() -> "Yesterday"
        else ->
            runCatching { LocalDate.parse(date).format(DateTimeFormatter.ofPattern("EEE, d MMM")) }
                .getOrDefault(date)
    }
