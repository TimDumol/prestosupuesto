@file:OptIn(
    androidx.compose.material3.ExperimentalMaterial3Api::class,
    androidx.compose.foundation.ExperimentalFoundationApi::class,
)

package com.presto.nativeapp

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
import com.presto.core.*
import java.math.BigDecimal
import java.time.*

@Composable
internal fun BudgetScreen(
    s: AppState,
    drill: (Category, String) -> Unit,
    balance: () -> Unit,
    complete: (Expectation) -> Unit,
) {
    var history by rememberSaveable { mutableStateOf(false) }
    var funding by rememberSaveable { mutableStateOf(false) }
    val categories = s.snapshot.budgets.filter { !it.system }
    val suggestion = suggestBalance(categories)
    Column(
        Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(16.dp),
        verticalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        Text(monthLabel(s.snapshot.budgetMonth), fontSize = 20.sp, fontWeight = FontWeight.Bold)
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            FilterChip(
                selected = !history,
                onClick = { history = false },
                label = { Text("Overview") },
                modifier = Modifier.testTag("budget-subtab-overview"),
            )
            FilterChip(
                selected = history,
                onClick = { history = true },
                label = { Text("History") },
                modifier = Modifier.testTag("budget-subtab-history"),
            )
        }
        if (history) BudgetHistory(s.snapshot, drill)
        else {
            Card(
                colors = CardDefaults.cardColors(containerColor = Primary),
                modifier = Modifier.fillMaxWidth(),
            ) {
                Column(Modifier.padding(18.dp)) {
                    Text("Available in your budget plan", color = Color.White)
                    Text(
                        money(categories.fold(ZERO) { a, c -> a + (c.remainingEur ?: ZERO) }),
                        color = Color.White,
                        fontSize = 36.sp,
                        fontWeight = FontWeight.Bold,
                    )
                    Text(
                        "Includes rollover and budget moves · not your bank balance",
                        color = Color.White,
                        fontSize = 13.sp,
                    )
                }
            }
            s.snapshot.budgetCoverage?.let { coverage ->
                Section {
                    Text("Household cash coverage", fontWeight = FontWeight.Bold)
                    Text(
                        if (coverage.currentGapEur < ZERO) "Cash shortfall today"
                        else "Cash above assigned budgets"
                    )
                    Text(
                        money(coverage.currentGapEur.abs()),
                        fontSize = 28.sp,
                        color = if (coverage.currentGapEur < ZERO) Danger else Positive,
                    )
                    if (s.outbox.isNotEmpty())
                        Text(
                            "Coverage reflects confirmed server data; entries are waiting to sync.",
                            fontSize = 13.sp,
                        )
                    TextButton(onClick = { funding = !funding }) {
                        Text(if (funding) "Hide funding details" else "Show funding details")
                    }
                    if (funding) {
                        listOf(
                                "Cash on hand" to coverage.actualCashEur,
                                "Assigned budgets" to coverage.assignedRemainingEur,
                                "Unallocated plan" to coverage.unallocatedEur,
                                "Expected income" to coverage.outstandingExpectedEur,
                                "Projected difference" to coverage.projectedGapEur,
                            )
                            .forEach { (label, value) ->
                                Row {
                                    Text(label, Modifier.weight(1f), fontSize = 14.sp)
                                    Text(
                                        money(value),
                                        fontSize = 14.sp,
                                        color = if (value < ZERO) Danger else Color.Unspecified,
                                    )
                                }
                            }
                        coverage.expectations
                            .filter { it.outstandingEur > ZERO }
                            .forEach { expectation ->
                                Row(verticalAlignment = Alignment.CenterVertically) {
                                    Column(Modifier.weight(1f)) {
                                        Text("${expectation.month} salary")
                                        Text(
                                            "${money(expectation.outstandingEur)} outstanding",
                                            fontSize = 13.sp,
                                        )
                                    }
                                    TextButton(
                                        onClick = { complete(expectation) },
                                        enabled =
                                            s.session != null &&
                                                !s.syncing &&
                                                s.outbox.none {
                                                    codec
                                                        .decodeFromString<Transaction>(
                                                            it.optimistic
                                                        )
                                                        .incomeTreatment == "salary"
                                                },
                                    ) {
                                        Text("Mark complete")
                                    }
                                }
                            }
                        Text(
                            "Unallocated may include expected income. Adjust budgets if cash is short.",
                            fontSize = 13.sp,
                        )
                    }
                }
            }
            Text("Spending categories", fontSize = 20.sp, fontWeight = FontWeight.Bold)
            categories.forEach { c ->
                OutlinedCard(
                    onClick = { drill(c, s.snapshot.budgetMonth) },
                    modifier = Modifier.fillMaxWidth(),
                ) {
                    Column(Modifier.padding(12.dp)) {
                        Row {
                            Text(c.name, Modifier.weight(1f), fontWeight = FontWeight.SemiBold)
                            Text(
                                money(c.remainingEur),
                                color =
                                    if ((c.remainingEur ?: ZERO) < ZERO) Danger
                                    else Color.Unspecified,
                                fontWeight = FontWeight.Bold,
                            )
                        }
                        val rollover =
                            (c.remainingEur ?: ZERO) - (c.monthlyEur ?: ZERO) +
                                (c.expenseEur ?: ZERO)
                        Text(
                            if ((c.remainingEur ?: ZERO) < ZERO) "Overspent" else "Remaining",
                            color = if ((c.remainingEur ?: ZERO) < ZERO) Danger else Muted,
                            fontSize = 13.sp,
                        )
                        val available = (c.monthlyEur ?: ZERO) + rollover
                        val spent = c.expenseEur ?: ZERO
                        if (available > ZERO && spent >= ZERO) {
                            LinearProgressIndicator(
                                progress = {
                                    (spent.toFloat() / available.toFloat()).coerceIn(0f, 1f)
                                },
                                modifier =
                                    Modifier.fillMaxWidth().padding(vertical = 6.dp).height(4.dp),
                                color = if ((c.remainingEur ?: ZERO) < ZERO) Danger else Primary,
                                trackColor = SoftAccent,
                            )
                        }
                        Text(
                            "${money(c.expenseEur)} spent · ${money(c.monthlyEur)} monthly · ${money(rollover)} rollover & moves",
                            fontSize = 12.sp,
                            color = Muted,
                        )
                    }
                }
            }
            Section {
                Text(
                    "Cover overspent categories",
                    color = Primary,
                    fontSize = 13.sp,
                    fontWeight = FontWeight.Bold,
                )
                val deficits = categories.count { (it.remainingEur ?: ZERO) < ZERO }
                Text(
                    if (deficits > 0) "$deficits categories below zero"
                    else "No categories are below zero",
                    fontWeight = FontWeight.Bold,
                )
                Text(
                    "Move positive remaining budget into negative categories. Each move is recorded on the selected month-end.",
                    fontSize = 14.sp,
                )
                Button(
                    onClick = balance,
                    enabled = suggestion.amount > ZERO,
                    modifier = Modifier.fillMaxWidth().testTag("balance-categories"),
                ) {
                    Text(
                        if (suggestion.amount > ZERO) "Balance categories" else "Nothing to balance"
                    )
                }
            }
            Section {
                Text("SYSTEM CATEGORIES", fontSize = 13.sp)
                Text("Income · receipts and expected funding")
                Text("Expense · recorded spending")
            }
        }
    }
}

@Composable
private fun BudgetHistory(snapshot: Snapshot, drill: (Category, String) -> Unit) {
    var month by rememberSaveable {
        mutableStateOf(snapshot.budgetHistory.lastOrNull()?.month ?: snapshot.budgetMonth)
    }
    var ignore by rememberSaveable { mutableStateOf(false) }
    var selected by rememberSaveable { mutableStateOf<String?>(null) }
    var isolated by rememberSaveable { mutableStateOf<String?>(null) }
    val months = snapshot.budgetHistory.sortedBy { it.month }
    val names = months.flatMap { it.categories }.map { it.name }.distinct()
    fun use(c: Category): BigDecimal =
        (c.expenseEur ?: ZERO) - (if (ignore) c.reallocationImpactEur ?: ZERO else ZERO)
    val active = months.find { it.month == month } ?: months.lastOrNull()
    Section {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Column(Modifier.weight(1f)) {
                Text(
                    if (isolated != null) "$isolated over time" else "Monthly spending",
                    fontWeight = FontWeight.Bold,
                    modifier = Modifier.testTag("budget-history-chart-title"),
                )
                Text("Tap a segment to highlight · hold to isolate", fontSize = 13.sp)
            }
            if (isolated != null)
                TextButton(
                    onClick = { isolated = null },
                    modifier = Modifier.testTag("chart-show-all"),
                ) {
                    Text("Show all")
                }
        }
        if (months.isEmpty()) Text("No budget history to chart yet.")
        else {
            selected?.let { name ->
                Text(
                    "$name · ${money(active?.categories?.find { it.name==name }?.let(::use))} · ${active?.month}",
                    color = Primary,
                    modifier = Modifier.testTag("budget-history-chart-selection"),
                )
            }
            val visibleNames = if (isolated != null) listOf(isolated!!) else names
            val maxPositive =
                months
                    .maxOf { m ->
                        m.categories
                            .filter { it.name in visibleNames }
                            .map(::use)
                            .filter { it > ZERO }
                            .fold(ZERO, BigDecimal::add)
                    }
                    .max(BigDecimal.ONE)
                    .toFloat()
            val maxNegative =
                months
                    .maxOf { m ->
                        m.categories
                            .filter { it.name in visibleNames }
                            .map(::use)
                            .filter { it < ZERO }
                            .fold(ZERO, BigDecimal::add)
                            .abs()
                    }
                    .toFloat()
            val total = maxPositive + maxNegative
            val topHeight = 200f * maxPositive / total
            val bottomHeight = 200f * maxNegative / total
            var segmentIndex = 0
            Row(
                Modifier.horizontalScroll(rememberScrollState()).testTag("budget-history-chart"),
                horizontalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                months.forEach { m ->
                    Column(
                        Modifier.width(72.dp),
                        horizontalAlignment = Alignment.CenterHorizontally,
                    ) {
                        Column(
                            Modifier.height(topHeight.dp).fillMaxWidth(),
                            verticalArrangement = Arrangement.Bottom,
                        ) {
                            m.categories
                                .filter { it.name in visibleNames && use(it) > ZERO }
                                .asReversed()
                                .forEach { c ->
                                    val index = segmentIndex++
                                    val color =
                                        chartColors[
                                            names.indexOf(c.name).coerceAtLeast(0) %
                                                chartColors.size]
                                    Box(
                                        Modifier.fillMaxWidth()
                                            .height((200f * use(c).toFloat() / total).dp)
                                            .background(
                                                if (selected == null || selected == c.name) color
                                                else color.copy(alpha = .35f)
                                            )
                                            .combinedClickable(
                                                onClick = {
                                                    selected = c.name
                                                    month = m.month
                                                },
                                                onLongClick = {
                                                    selected = c.name
                                                    isolated = c.name
                                                    month = m.month
                                                },
                                            )
                                            .testTag("budget-history-segment-$index")
                                            .semantics {
                                                contentDescription =
                                                    "${c.name}, ${m.month}, ${money(use(c))}"
                                            }
                                    )
                                }
                        }
                        HorizontalDivider(thickness = 1.dp)
                        Column(Modifier.height(bottomHeight.dp).fillMaxWidth()) {
                            m.categories
                                .filter { it.name in visibleNames && use(it) < ZERO }
                                .forEach { c ->
                                    val index = segmentIndex++
                                    val color =
                                        chartColors[
                                            names.indexOf(c.name).coerceAtLeast(0) %
                                                chartColors.size]
                                    Box(
                                        Modifier.fillMaxWidth()
                                            .height((200f * use(c).abs().toFloat() / total).dp)
                                            .background(color)
                                            .combinedClickable(
                                                onClick = {
                                                    selected = c.name
                                                    month = m.month
                                                },
                                                onLongClick = {
                                                    selected = c.name
                                                    isolated = c.name
                                                    month = m.month
                                                },
                                            )
                                            .testTag("budget-history-segment-$index")
                                            .semantics {
                                                contentDescription =
                                                    "${c.name}, ${m.month}, ${money(use(c))}"
                                            }
                                    )
                                }
                        }
                        Text(
                            m.month,
                            fontSize = 14.sp,
                            modifier = Modifier.clickable { month = m.month }.padding(top = 8.dp),
                        )
                    }
                }
            }
            Text("EUR · positive use above zero; negative use below", fontSize = 14.sp)
            names.forEachIndexed { index, name ->
                Row(
                    Modifier.fillMaxWidth()
                        .heightIn(min = 48.dp)
                        .combinedClickable(
                            onClick = { selected = name },
                            onLongClick = {
                                isolated = name
                                selected = name
                            },
                        )
                        .testTag("budget-history-legend-$index"),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Box(
                        Modifier.size(12.dp)
                            .background(
                                chartColors[index % chartColors.size],
                                RoundedCornerShape(3.dp),
                            )
                    )
                    Spacer(Modifier.width(8.dp))
                    Text(name, fontSize = 14.sp)
                }
            }
        }
    }
    Row(
        Modifier.horizontalScroll(rememberScrollState()),
        horizontalArrangement = Arrangement.spacedBy(6.dp),
    ) {
        months.forEach { m ->
            FilterChip(
                selected = m.month == active?.month,
                onClick = { month = m.month },
                label = { Text(monthLabel(m.month)) },
            )
        }
    }
    Row(
        Modifier.fillMaxWidth().clickable { ignore = !ignore },
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Checkbox(checked = ignore, onCheckedChange = { ignore = it })
        Text("Ignore reallocation")
    }
    active?.let { m ->
        Section {
            Text("${monthLabel(m.month)} use", fontWeight = FontWeight.Bold)
            Text(money(m.categories.map(::use).fold(ZERO, BigDecimal::add)), fontSize = 24.sp)
            Text(
                "${money(m.categories.fold(ZERO) { a, c -> a+(c.monthlyEur ?: ZERO) })} total monthly budgets",
                fontSize = 13.sp,
            )
        }
        m.categories.forEach { c ->
            Choice(
                c.name,
                "${money(use(c))} used",
                "${money(c.monthlyEur)} monthly budget",
                { drill(c, m.month) },
            )
        }
    }
}
