package com.presto.nativeapp

import android.util.Log
import java.time.Instant
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.asStateFlow

/** Session-only debug history; never record request bodies or session credentials. No disk IO. */
internal class Diagnostics {
    private val entries = MutableStateFlow<List<String>>(emptyList())
    val logs = entries.asStateFlow()

    @Synchronized fun record(message: String) {
        val line = "${Instant.now()} ${redactDiagnostics(message).replace('\n', ' ').take(2000)}"
        entries.value = (entries.value + line).takeLast(200)
        if (BuildConfig.DEBUG) Log.d("PrestoSync", line)
    }
}

private val bearerPattern = Regex("(?i)Bearer\\s+\\S+")
private val credentialPattern = Regex("(?i)\\b[0-9a-f]{64}\\b")

internal fun redactDiagnostics(text: String): String = text
    .replace(bearerPattern, "Bearer [redacted]")
    .replace(credentialPattern, "[redacted]")

internal fun syncErrorSummary(operation: OutboxRow): String {
    val detail = operation.error.orEmpty().substringAfterLast(": ").substringBefore("\n")
    val action = if (operation.status == "blocked") "Review this entry in Settings." else
        "Saved on this phone. Retry sync when connected."
    return "${if (operation.status == "blocked") "Sync failed" else "Waiting to sync"}: $detail $action"
}

/** Repair only the known rejected revision-list shape; preserve every real revision and ID. */
internal fun repairBookkeepingRevisions(row: OutboxRow): OutboxRow? {
    if (!row.editing || row.status != "blocked" ||
        row.error?.contains("HTTP 400") != true ||
        row.error.contains("Provide the current ledger revision for every real account affected by this edit.") != true
    ) return null
    return runCatching {
        val original = com.presto.core.codec.decodeFromString<com.presto.core.Transaction>(row.original ?: return null)
        val optimistic = com.presto.core.codec.decodeFromString<com.presto.core.Transaction>(row.optimistic)
        val payload = com.presto.core.codec.decodeFromString<com.presto.core.WritePayload>(row.payload)
        val needed = com.presto.core.realAccountIds(original) + com.presto.core.realAccountIds(optimistic)
        val before = payload.expectedAccountRevisions ?: return null
        val after = before.filter { it.accountId in needed }
        if (after.size == before.size || after.map { it.accountId }.toSet() != needed) return null
        val repaired = payload.copy(expectedAccountRevisions = after)
        row.copy(payload = com.presto.core.codec.encodeToString(com.presto.core.WritePayload.serializer(), repaired),
            status = "pending", error = null)
    }.getOrNull()
}
