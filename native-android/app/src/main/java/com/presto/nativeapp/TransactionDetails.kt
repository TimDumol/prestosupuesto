@file:OptIn(androidx.compose.material3.ExperimentalMaterial3Api::class)

package com.presto.nativeapp

import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.presto.core.*

@Composable
internal fun TransactionDetails(
    t: Transaction,
    snapshot: Snapshot,
    dismiss: () -> Unit,
    edit: () -> Unit,
    operation: OutboxRow? = null,
    syncing: Boolean = false,
    retry: () -> Unit = {},
    review: () -> Unit = {},
    delete: () -> Unit = {},
    deleting: Boolean = false,
) {
    val allEntities = entities(snapshot)
    fun name(id: String?, kind: String, fallback: String?) =
        allEntities.find { it.id == id && it.kind == kind }?.name ?: fallback
    ModalBottomSheet(
        onDismissRequest = dismiss,
        sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true),
    ) {
        Column(
            Modifier.fillMaxWidth().verticalScroll(rememberScrollState()).padding(24.dp),
            verticalArrangement = Arrangement.spacedBy(14.dp),
        ) {
            Text(
                kindNames[t.kind] ?: t.kind,
                color = Muted,
                style = MaterialTheme.typography.labelLarge,
            )
            Text(t.description, fontSize = 24.sp, fontWeight = FontWeight.Bold)
            Text(
                (if (t.kind == "expense") "−" else if (t.kind == "income") "+" else "") +
                    money(t.amount, t.inputCurrency ?: "EUR"),
                fontSize = 32.sp,
                fontWeight = FontWeight.Bold,
                color = transactionColor(t.kind),
            )
            if (t.pending) {
                StatusBadge(if (operation?.status == "blocked") "Sync failed · needs review" else "Waiting to sync")
                operation?.takeIf { it.error != null }?.let {
                    Text(syncErrorSummary(it), color = Danger, modifier = Modifier.testTag("transaction-sync-error"))
                }
                Text("Saved on this phone. You can edit this entry after it syncs.", color = Muted)
                if (operation?.status == "blocked") {
                    Button(onClick = review, modifier = Modifier.fillMaxWidth()) { Text("Review sync failure") }
                } else {
                    Button(onClick = retry, enabled = !syncing, modifier = Modifier.fillMaxWidth()) {
                        Text(if (syncing) "Syncing…" else "Retry sync")
                    }
                }
            } else if (t.kind == "reconciliation") {
                Text(
                    "Reconciliation adjustments are managed in the browser workspace.",
                    color = Muted,
                )
            } else if (!t.canEdit) {
                Text(
                    "This entry is read only. Use a linked correction to adjust historical activity.",
                    color = Muted,
                )
            }
            HorizontalDivider()
            DetailRow("Date", t.effectiveDate)
            name(t.fromAccountId, "account", t.fromAccount)?.let { DetailRow("From account", it) }
            name(t.toAccountId, "account", t.toAccount)?.let { DetailRow("To account", it) }
            name(t.fromCategoryId, "category", t.fromCategory)?.let {
                DetailRow("From category", it)
            }
            name(t.toCategoryId, "category", t.toCategory)?.let { DetailRow("To category", it) }
            t.toAmount?.let {
                DetailRow(
                    "Amount received",
                    money(
                        it,
                        t.toCurrency
                            ?: snapshot.accounts.find { a -> a.id == t.toAccountId }?.currency
                            ?: "EUR",
                    ),
                )
            }
            if (t.hasCategoryCorrection) StatusBadge("Category corrected")
            if (t.correctsTransactionId != null) StatusBadge("Linked correction")
            if (t.kind == "reallocate")
                Text(
                    "This move changes budget allocations without moving money between accounts.",
                    color = Muted,
                )
            if (t.canEdit && !t.pending && t.kind != "reconciliation") {
                Button(
                    onClick = edit,
                    enabled = !deleting,
                    modifier = Modifier.fillMaxWidth().testTag("edit-inspected-entry"),
                ) {
                    Text("Edit entry")
                }
            }
            if (t.canEdit && !t.pending && !t.hasCategoryCorrection &&
                t.kind in listOf("expense", "income", "transfer", "reallocate")) {
                TextButton(
                    onClick = delete,
                    enabled = !deleting && !syncing,
                    colors = ButtonDefaults.textButtonColors(contentColor = Danger),
                    modifier = Modifier.fillMaxWidth().testTag("delete-inspected-entry"),
                ) { Text(if (deleting) "Deleting…" else "Delete entry") }
            }
            TextButton(onClick = dismiss, modifier = Modifier.fillMaxWidth()) { Text("Close") }
        }
    }
}

@Composable
private fun DetailRow(label: String, value: String) {
    Row(horizontalArrangement = Arrangement.spacedBy(16.dp)) {
        Text(label, modifier = Modifier.weight(1f), color = Muted)
        Text(value, modifier = Modifier.weight(1f), fontWeight = FontWeight.Medium)
    }
}
