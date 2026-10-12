@file:OptIn(
    androidx.compose.material3.ExperimentalMaterial3Api::class,
    androidx.compose.foundation.ExperimentalFoundationApi::class,
)

package com.presto.nativeapp

import androidx.compose.foundation.*
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.*
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.presto.core.*
import java.time.*
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

@Composable
internal fun PickerSheet(
    target: String,
    s: AppState,
    choose: (String) -> Unit,
    dismiss: () -> Unit,
) {
    var query by rememberSaveable(target) { mutableStateOf("") }
    val isCategory = target.startsWith("category")
    val isAccount = target.startsWith("account")
    val d = s.draft
    val ranked by produceState<List<Pair<Triple<String, String, String>, Double>>?>(
        null, target, query, s.snapshot, d.kind, d.balancing,
    ) {
        value = null
        value = withContext(Dispatchers.Default) {
            val options: List<Triple<String, String, String>> =
                when {
                    isCategory ->
                        s.snapshot.budgets
                            .filter {
                                !it.system &&
                                    (d.kind == "reallocate" || it.categoryId != "budget-unallocated") &&
                                    (!d.balancing ||
                                        if (target == "categoryFrom") (it.remainingEur ?: ZERO) > ZERO
                                        else (it.remainingEur ?: ZERO) < ZERO)
                            }
                            .map { Triple(it.categoryId, it.name, "${money(it.remainingEur)} available") }
                    isAccount ->
                        s.snapshot.accounts.map {
                            Triple(
                                it.id,
                                it.name,
                                "${money(it.balanceNative, it.currency ?: "EUR")} · calculated balance",
                            )
                        }
                    target == "currency" -> listOf("EUR", "PHP", "USD").map { Triple(it, it, "") }
                    else ->
                        listOf(Triple("", "New activity", "No correction link")) +
                            s.snapshot.recentTransactions
                                .filter { !it.canEdit && it.effectiveDate.take(7) < today().take(7) }
                                .map { Triple(it.id, it.description, it.effectiveDate) }
                }
            val usage = mutableMapOf<String, Double>()
            if (isCategory || isAccount) {
                s.snapshot.recentTransactions.forEachIndexed { index, transaction ->
                    val from = if (isCategory) transaction.fromCategoryId else transaction.fromAccountId
                    val to = if (isCategory) transaction.toCategoryId else transaction.toAccountId
                    val weight = Math.pow(0.92, index.toDouble())
                    fun record(id: String?) {
                        if (id != null) usage[id] = (usage[id] ?: 0.0) + weight
                    }
                    record(from)
                    if (to != from) record(to)
                }
            }
            options
                .mapNotNull { option ->
                    fuzzyScore(option.second, query)?.let { score -> option to score }
                }
                .sortedWith(
                    compareByDescending<Pair<Triple<String, String, String>, Double>> { it.second }
                        .thenByDescending { usage[it.first.first] ?: 0.0 }
                        .thenBy { it.first.second }
                )
        }
    }
    ModalBottomSheet(onDismissRequest = dismiss) {
        Column(Modifier.fillMaxWidth().padding(horizontal = 16.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text(
                    if (isCategory) "Choose category"
                    else if (isAccount) "Choose account"
                    else if (target == "currency") "Choose currency" else "Choose correction link",
                    fontSize = 20.sp,
                    fontWeight = FontWeight.Bold,
                    modifier = Modifier.weight(1f),
                )
                TextButton(onClick = dismiss) { Text("Close") }
            }
            if (target != "currency") CompactSearch(query, { query = it }, "picker-search")
            LazyColumn(
                Modifier.heightIn(max = 420.dp),
                contentPadding = PaddingValues(vertical = 12.dp),
                verticalArrangement = Arrangement.spacedBy(4.dp),
            ) {
                items(ranked.orEmpty(), key = { it.first.first }) { (option, _) ->
                    OutlinedCard(
                        onClick = { choose(option.first) },
                        modifier = Modifier.fillMaxWidth().testTag("picker-option-${option.first}"),
                    ) {
                        Row(
                            Modifier.padding(horizontal = 12.dp, vertical = 8.dp)
                                .heightIn(min = 40.dp),
                            verticalAlignment = Alignment.CenterVertically,
                        ) {
                            Column(Modifier.weight(1f)) {
                                Text(
                                    option.second,
                                    fontWeight = FontWeight.Medium,
                                    fontSize = 15.sp,
                                    maxLines = 1,
                                    overflow = androidx.compose.ui.text.style.TextOverflow.Ellipsis,
                                )
                                if (option.third.isNotBlank())
                                    Text(option.third, fontSize = 13.sp, color = Muted)
                            }
                            val selected =
                                when (target) {
                                    "categoryFrom" -> d.categoryFrom
                                    "categoryTo" -> d.categoryTo
                                    "accountFrom" -> d.accountFrom
                                    "accountTo" -> d.accountTo
                                    "currency" -> d.currency
                                    else -> d.correctionOf
                                }
                            if (option.first == selected) StatusBadge("Selected")
                        }
                    }
                }
                if (ranked == null)
                    item { CircularProgressIndicator(Modifier.padding(24.dp)) }
                else if (ranked.orEmpty().isEmpty())
                    item { Text("No matching options", modifier = Modifier.padding(24.dp)) }
            }
        }
    }
}
