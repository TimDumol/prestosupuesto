@file:OptIn(
    androidx.compose.material3.ExperimentalMaterial3Api::class,
    androidx.compose.foundation.ExperimentalFoundationApi::class,
)

package com.presto.nativeapp

import androidx.compose.foundation.*
import androidx.compose.foundation.layout.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.platform.LocalClipboardManager
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.ui.semantics.*
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.presto.core.*
import java.time.*

@Composable
internal fun SetupScreen(
    s: AppState,
    login: () -> Unit,
    logout: () -> Unit,
    refresh: () -> Unit,
    restore: () -> Unit,
    retry: (String) -> Unit,
    review: (String) -> Unit,
    requestNotification: () -> Unit,
    logs: List<String> = emptyList(),
    setPartnerNotifications: (Boolean) -> Unit = {},
) {
    var showLogs by remember { mutableStateOf(false) }
    val clipboard = LocalClipboardManager.current
    if (showLogs) {
        val text = logs.asReversed().joinToString("\n\n").ifBlank { "No diagnostic events recorded yet." }
        AlertDialog(
            onDismissRequest = { showLogs = false },
            title = { Text("App logs (this session)") },
            text = {
                SelectionContainer {
                    Text(text, fontSize = 12.sp,
                        modifier = Modifier.heightIn(max = 440.dp).verticalScroll(rememberScrollState())
                            .testTag("diagnostic-logs"))
                }
            },
            confirmButton = { TextButton(onClick = { showLogs = false }) { Text("Close") } },
            dismissButton = { TextButton(onClick = { clipboard.setText(AnnotatedString(text)) }) { Text("Copy logs") } },
        )
    }
    Column(
        Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(16.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Section {
            Text(
                "Presto Presupuesto · ${BuildConfig.VERSION_NAME}",
                fontSize = 13.sp,
                fontWeight = FontWeight.Medium,
            )
            Text(
                "Built: ${runCatching {
                    java.time.Instant.parse(BuildConfig.BUILD_TIMESTAMP).atZone(java.time.ZoneOffset.UTC)
                        .format(java.time.format.DateTimeFormatter.ofPattern("yyyy-MM-dd HH:mm:ss 'UTC'"))
                }.getOrDefault(BuildConfig.BUILD_TIMESTAMP)}",
                fontSize = 12.sp,
                color = Muted,
                modifier = Modifier.testTag("build-timestamp"),
            )
        }
        Section {
            Text(
                if (s.session != null) "Connected to your household" else "Connect your household",
                fontWeight = FontWeight.Bold,
            )
            Text(
                s.session?.let { "Signed in as ${it.username}" }
                    ?: if (s.demo) "Sign in with Pocket ID to manage household finances."
                    else "Your cached data and pending entries are retained. Sign in to sync."
            )
            Button(
                onClick = if (s.session != null) logout else login,
                modifier = Modifier.fillMaxWidth(),
            ) {
                Text(if (s.session != null) "Disconnect" else "Connect Pocket ID")
            }
            OutlinedButton(
                onClick = refresh,
                enabled = s.session != null && !s.syncing,
                modifier = Modifier.fillMaxWidth(),
            ) {
                Text("Refresh and retry sync")
            }
        }
        if (!s.demo && BuildConfig.FCM_ENABLED) Section {
            Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween) {
                Text("Partner transactions", fontWeight = FontWeight.Bold)
                Switch(checked = s.partnerNotifications, enabled = s.session != null,
                    onCheckedChange = { enabled ->
                        setPartnerNotifications(enabled)
                        if (enabled) requestNotification()
                    }, modifier = Modifier.testTag("partner-notifications"))
            }
            Text("Receive an alert with the amount and description when your partner adds a transaction.", fontSize = 14.sp)
            s.partnerNotificationStatus?.let { Text(it, fontSize = 13.sp, color = Muted) }
        }
        Section {
            Text("Sync · ${s.pending} entries waiting", fontWeight = FontWeight.Bold)
            val blocked = s.outbox.count { it.status == "blocked" }
            if (blocked > 0) Text("$blocked failed ${if (blocked == 1) "entry needs" else "entries need"} review before syncing.", color = Danger)
            Text(
                "Entries are saved on this phone and sync automatically when a connection is available.",
                fontSize = 14.sp,
            )
            TextButton(onClick = requestNotification) { Text("Enable sync notifications") }
            s.outbox
                .filter { it.error != null }
                .forEach { operation ->
                    Text(
                        codec.decodeFromString<Transaction>(operation.optimistic).description,
                        fontWeight = FontWeight.SemiBold,
                    )
                    Text(syncErrorSummary(operation), color = Danger, fontSize = 14.sp)
                    if (operation.status == "blocked") {
                        TextButton(onClick = { review(operation.id) }) { Text("Review as draft") }
                        TextButton(onClick = { retry(operation.id) }) { Text("Try again") }
                    }
                }
        }
        if (!s.demo)
            Section {
                Text("Backup & recovery", fontWeight = FontWeight.Bold)
                s.recovery?.let { info ->
                    Text("Latest verified backup · ${info.createdAt}", fontSize = 14.sp)
                    OutlinedButton(
                        onClick = restore,
                        enabled = s.session != null && !s.syncing && s.outbox.isEmpty(),
                        modifier = Modifier.fillMaxWidth(),
                    ) {
                        Text("Restore to an empty database")
                    }
                } ?: Text("No verified phone snapshot yet.", fontSize = 14.sp)
                Text(
                    "Backups every six hours while using the app; eight verified generations retained. Restore requires an empty server and administrator enabled recovery mode.",
                    fontSize = 13.sp,
                )
                s.recoveryIssue?.let { Text(it, color = Danger, fontSize = 14.sp) }
            }
        Section {
            Text("Preferences & app information", fontWeight = FontWeight.Bold)
            OutlinedButton(onClick = { showLogs = true }, modifier = Modifier.fillMaxWidth().testTag("show-logs")) {
                Text("Show logs")
            }
            Text("Smart suggestions · prior descriptions and category keywords")
            Text("Timezone · ${zone().id}")
            Text("Currencies · EUR, PHP, USD")
            Text("Reconciliation and budget administration · browser workspace", fontSize = 14.sp)
        }
    }
}
