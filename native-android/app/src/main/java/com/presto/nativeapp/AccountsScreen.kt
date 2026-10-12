package com.presto.nativeapp

import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.*
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.presto.core.*

@Composable
internal fun AccountsScreen(snapshot: Snapshot) {
    LazyColumn(
        Modifier.fillMaxSize().padding(horizontal = 16.dp),
        verticalArrangement = Arrangement.spacedBy(6.dp),
        contentPadding = PaddingValues(vertical = 8.dp),
    ) {
        items(snapshot.accounts, key = { it.id }) { a ->
            Section {
                Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                    Text(
                        a.name,
                        Modifier.weight(1f),
                        fontSize = 16.sp,
                        fontWeight = FontWeight.Medium,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                    )
                    Text(
                        money(a.balanceNative, a.currency ?: "EUR"),
                        fontSize = 18.sp,
                        fontWeight = FontWeight.SemiBold,
                        color = if ((a.balanceNative ?: ZERO) < ZERO) ExpenseColor else Ink,
                    )
                }
                Text(
                    "${a.accountType ?: "Account"} · ${a.currency ?: "EUR"} · calculated balance",
                    fontSize = 12.sp,
                    color = Muted,
                )
                if (a.actualBalanceNative != null || a.reconciliationNative != null) {
                    HorizontalDivider(color = MaterialTheme.colorScheme.outlineVariant)
                    Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                        Column(Modifier.weight(1f)) {
                            Text("Recorded actual", fontSize = 12.sp, color = Muted)
                            Text(
                                money(a.actualBalanceNative, a.currency ?: "EUR"),
                                fontSize = 14.sp,
                            )
                        }
                        Column(Modifier.weight(1f)) {
                            Text("Difference", fontSize = 12.sp, color = Muted)
                            Text(
                                money(a.reconciliationNative, a.currency ?: "EUR"),
                                fontSize = 14.sp,
                                color =
                                    if (
                                        a.reconciliationNative != null &&
                                            a.reconciliationNative != ZERO
                                    )
                                        PendingColor
                                    else Muted,
                            )
                        }
                    }
                }
            }
        }
        if (snapshot.accounts.isEmpty()) item { Text("No accounts available.") }
        else
            item {
                Text(
                    "Actual balances are last recorded values, not live bank balances.",
                    Modifier.padding(vertical = 8.dp),
                    fontSize = 12.sp,
                    color = Muted,
                )
            }
    }
}
