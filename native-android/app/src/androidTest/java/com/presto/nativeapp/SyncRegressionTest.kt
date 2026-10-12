package com.presto.nativeapp

import android.content.Context
import androidx.room.Room
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import com.presto.core.*
import java.io.File
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import kotlin.test.*
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.first
import okhttp3.mockwebserver.*
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class SyncRegressionTest {
    @Test fun acceptedSaveSurvivesCallerCancellation(): Unit = runBlocking {
        val context = ApplicationProvider.getApplicationContext<Context>()
        val namespace = "cancelled-save-${uuidV7()}"
        val repo = FinanceRepository(context, FinanceApi("http://127.0.0.1:1"), namespace, false)
        val seed = Room.databaseBuilder(context, FinanceDatabase::class.java, "$namespace.db").build()
        seed.finance().putDraft(DraftRow("demo|${repo.api.origin}", codec.encodeToString(Draft.serializer(),
            defaultDraft(demoSnapshot()).copy(amount = "7.50", description = "After cancelled caller"))))
        seed.close()
        try {
            // Hold initialization so cancellation deterministically happens before disk work.
            val caller = launch(start = CoroutineStart.UNDISPATCHED) { repo.save(false) }
            assertTrue(repo.state.value.saving, "An accepted tap signals progress before initialization")
            caller.cancelAndJoin()
            assertTrue(repo.state.value.saving, "Cancelling the screen must not cancel its accepted write")
            repo.start()
            repo.awaitReady()
            withTimeout(5_000) { repo.state.first { !it.saving } }
            assertNull(repo.state.value.error)
            assertEquals(1, repo.state.value.snapshot.recentTransactions.count {
                it.description == "After cancelled caller"
            })
            repo.updateDraft(repo.state.value.draft.copy(amount = "3.50", description = "Next accepted entry"))
            assertTrue(repo.save(false), "The write gate must be released after the cancelled caller's save")
            repo.close()
            val reopened = FinanceRepository(context, repo.api, namespace, false)
            try {
                reopened.start()
                reopened.awaitReady()
                assertEquals(1, reopened.state.value.snapshot.recentTransactions.count {
                    it.description == "After cancelled caller"
                })
            } finally { reopened.close() }
        } finally {
            repo.close()
            context.deleteDatabase("$namespace.db")
            File(context.noBackupFilesDir, "$namespace-diagnostics.log").delete()
        }
    }

    @Test fun saveCompletesOnMainWhileNetworkIsStalled(): Unit = runBlocking {
        val context = ApplicationProvider.getApplicationContext<Context>()
        for (another in listOf(false, true)) {
            val namespace = "queued-save-${uuidV7()}"
            val session = Session("a".repeat(64), "queued-save", "Test")
            val snapshot = demoSnapshot()
            val requested = CountDownLatch(1)
            val release = CountDownLatch(1)
            val server = MockWebServer()
            server.dispatcher = object : Dispatcher() {
                override fun dispatch(request: RecordedRequest): MockResponse {
                    requested.countDown()
                    release.await(30, TimeUnit.SECONDS)
                    return MockResponse().setResponseCode(503)
                }
            }
            server.start()
            val api = FinanceApi(server.url("/").toString())
            val scope = "${session.owner}|${api.origin}"
            val seed = Room.databaseBuilder(context, FinanceDatabase::class.java, "$namespace.db").build()
            seed.finance().putCache(CacheRow(scope, codec.encodeToString(Snapshot.serializer(), snapshot)))
            seed.close()
            Vault(context, namespace).saveSession(session)
            val repo = FinanceRepository(context, api, namespace, false)
            try {
                repo.start()
                repo.awaitReady()
                assertTrue(requested.await(5, TimeUnit.SECONDS))
                repo.updateDraft(defaultDraft(snapshot).copy(
                    amount = "7.50", description = "Immediate queue", date = "2020-01-15",
                ))
                val done = CompletableDeferred<Unit>()
                withContext(Dispatchers.Main) {
                    PrestoViewModel(repo).save(another) { done.complete(Unit) }
                    assertTrue(repo.state.value.saving, "The tap must immediately signal saving")
                }
                withTimeout(3_000) { done.await() }
                assertEquals(1L, release.count, "Save must finish before the network responds")
                assertEquals(1, repo.state.value.pending)
                assertFalse(repo.state.value.saving)
                assertEquals("", repo.state.value.draft.amount)
                if (another) assertEquals("2020-01-15", repo.state.value.draft.date)
                val stored = Room.databaseBuilder(context, FinanceDatabase::class.java, "$namespace.db").build()
                try {
                    assertEquals(1, stored.finance().outbox(scope).size)
                    assertEquals("", codec.decodeFromString<Draft>(stored.finance().draft(scope)!!.draft).amount)
                } finally { stored.close() }
            } finally {
                release.countDown()
                repo.close()
                server.shutdown()
                context.deleteDatabase("$namespace.db")
                File(context.noBackupFilesDir, "$namespace-pocket-session.sealed").delete()
                File(context.noBackupFilesDir, "$namespace-diagnostics.log").delete()
            }
        }
    }

    @Test fun delayedSuggestionsCannotOverwriteNewerInput(): Unit = runBlocking {
        val context = ApplicationProvider.getApplicationContext<Context>()
        val namespace = "stale-suggestion-${uuidV7()}"
        val repo = FinanceRepository(context, FinanceApi("http://127.0.0.1:1"), namespace, false)
        try {
            repo.start()
            repo.awaitReady()
            val previous = repo.state.value.draft
            repo.updateDraft(previous.copy(description = "New description", amount = "15"))
            val current = repo.state.value.draft
            repo.updateDraft(previous.copy(categoryFrom = "stale"), userEdited = false, expected = previous)
            assertEquals(current, repo.state.value.draft)
            withContext(Dispatchers.Main) { assertTrue(repo.persistDraft()) }
            repo.close()
            val reopened = FinanceRepository(context, repo.api, namespace, false)
            try {
                reopened.start()
                reopened.awaitReady()
                assertEquals(current, reopened.state.value.draft)
            } finally { reopened.close() }
        } finally {
            repo.close()
            context.deleteDatabase("$namespace.db")
        }
    }

    @androidx.test.filters.SdkSuppress(minSdkVersion = 28)
    @Test fun syncFromMainAvoidsUiThreadDiskWork(): Unit = runBlocking {
        val context = ApplicationProvider.getApplicationContext<Context>()
        val namespace = "main-safe-sync-${uuidV7()}"
        val session = Session("a".repeat(64), "main-safe-sync", "Test")
        val requested = CountDownLatch(1)
        val release = CountDownLatch(1)
        val server = MockWebServer()
        server.dispatcher = object : Dispatcher() {
            override fun dispatch(request: RecordedRequest): MockResponse {
                if (request.path == "/api/session") {
                    requested.countDown()
                    release.await(10, TimeUnit.SECONDS)
                    return MockResponse().setBody("{\"pocketAuthenticated\":true}")
                }
                return MockResponse().setResponseCode(503)
            }
        }
        server.start()
        Vault(context, namespace).saveSession(session)
        val repo = FinanceRepository(context, FinanceApi(server.url("/").toString()), namespace, false)
        try {
            repo.start()
            repo.awaitReady()
            assertTrue(requested.await(5, TimeUnit.SECONDS))
            release.countDown()
            repo.sync() // Drain startup sync before installing the UI thread disk policy.
            val violations = java.util.concurrent.CopyOnWriteArrayList<android.os.strictmode.Violation>()
            withContext(Dispatchers.Main) {
                val oldPolicy = android.os.StrictMode.getThreadPolicy()
                android.os.StrictMode.setThreadPolicy(android.os.StrictMode.ThreadPolicy.Builder()
                    .detectDiskReads().detectDiskWrites()
                    .penaltyListener(java.util.concurrent.Executor { it.run() }) { violations.add(it) }
                    .build())
                try {
                    repo.sync()
                } finally { android.os.StrictMode.setThreadPolicy(oldPolicy) }
            }
            assertTrue(violations.isEmpty(), "Sync performed UI thread disk work: $violations")
        } finally {
            release.countDown()
            repo.close()
            server.shutdown()
            context.deleteDatabase("$namespace.db")
            File(context.noBackupFilesDir, "$namespace-pocket-session.sealed").delete()
            File(context.noBackupFilesDir, "$namespace-diagnostics.log").delete()
        }
    }

    @Test fun optimisticBudgetsUseTheLatestConfirmedTransaction(): Unit = runBlocking {
        val context = ApplicationProvider.getApplicationContext<Context>()
        for ((editing, deleted) in listOf(false to false, true to false, true to true)) {
            val namespace = "budget-overlay-${uuidV7()}"
            val api = FinanceApi("http://127.0.0.1:1")
            val base = demoSnapshot()
            val confirmed = base.recentTransactions.first().copy(amount = "20".toBigDecimal(), revision = 2)
            val optimistic = confirmed.copy(amount = (if (editing) "15" else "20").toBigDecimal(), pending = true)
            val snapshot = base.copy(recentTransactions = if (deleted) emptyList() else listOf(confirmed),
                transactionCount = if (deleted) 0 else 1)
            val original = confirmed.copy(amount = "10".toBigDecimal(), revision = 1)
            val scope = "demo|${api.origin}"
            val seed = Room.databaseBuilder(context, FinanceDatabase::class.java, "$namespace.db").build()
            seed.finance().putCache(CacheRow(scope, codec.encodeToString(Snapshot.serializer(), snapshot)))
            seed.finance().enqueue(OutboxRow(uuidV7(), scope, confirmed.id, "{}",
                codec.encodeToString(Transaction.serializer(), optimistic),
                if (editing) codec.encodeToString(Transaction.serializer(), original) else null,
                editing, status = "blocked"))
            seed.close()
            val repo = FinanceRepository(context, api, namespace, false)
            try {
                repo.start()
                repo.awaitReady()
                val category = repo.state.value.snapshot.budgets.first { it.categoryId == confirmed.fromCategoryId }
                val before = base.budgets.first { it.categoryId == confirmed.fromCategoryId }
                val previousAmount = if (deleted) ZERO else confirmed.amount
                assertEquals(before.remainingEur!! + previousAmount - optimistic.amount, category.remainingEur)
                assertEquals(before.expenseEur!! - previousAmount + optimistic.amount, category.expenseEur)
            } finally {
                repo.close()
                context.deleteDatabase("$namespace.db")
            }
        }
    }

    @Test fun completeRefreshRemovesDeletedRowsButRetainsPendingWrites(): Unit = runBlocking {
        val context = ApplicationProvider.getApplicationContext<Context>()
        val namespace = "deleted-cache-${uuidV7()}"
        val session = Session("a".repeat(64), "deleted-test", "Test")
        val cached = demoSnapshot()
        val pending = cached.recentTransactions.first().copy(id = uuidV7(), description = "Offline entry", pending = true)
        val fresh = cached.copy(recentTransactions = emptyList(), transactionCount = 0)
        val server = MockWebServer()
        server.dispatcher = object : Dispatcher() {
            override fun dispatch(request: RecordedRequest): MockResponse = when {
                request.path == "/api/session" -> MockResponse().setBody("{\"pocketAuthenticated\":true}")
                request.path!!.startsWith("/api/db/finance?") -> MockResponse().setBody(codec.encodeToString(Snapshot.serializer(), fresh))
                else -> MockResponse().setResponseCode(503)
            }
        }
        server.start()
        val api = FinanceApi(server.url("/").toString().trimEnd('/'))
        val scope = "${session.owner}|${api.origin}"
        val seed = Room.databaseBuilder(context, FinanceDatabase::class.java, "$namespace.db").build()
        seed.finance().putCache(CacheRow(scope, codec.encodeToString(Snapshot.serializer(), cached)))
        seed.finance().enqueue(OutboxRow(uuidV7(), scope, pending.id, "{}",
            codec.encodeToString(Transaction.serializer(), pending), null, false))
        seed.close()
        Vault(context, namespace).saveSession(session)
        val repo = FinanceRepository(context, api, namespace, false)
        try {
            repo.start()
            repo.awaitReady()
            repo.sync()
            assertEquals(listOf(pending.id), repo.state.value.snapshot.recentTransactions.map { it.id })
            assertEquals(1, repo.state.value.pending)
            assertTrue(repo.state.value.snapshot.recentTransactions.single().pending)
        } finally {
            repo.close()
            server.shutdown()
            context.deleteDatabase("$namespace.db")
            File(context.noBackupFilesDir, "$namespace-pocket-session.sealed").delete()
        }
    }

    @Test fun concurrentSavesAndRapidDraftChangesSurviveReopening(): Unit = runBlocking {
        val context = ApplicationProvider.getApplicationContext<Context>()
        val namespace = "concurrent-save-${uuidV7()}"
        val api = FinanceApi("http://127.0.0.1:1")
        var repo = FinanceRepository(context, api, namespace, false)
        try {
            repo.start()
            repo.awaitReady()
            repeat(100) { repo.updateDraft(repo.state.value.draft.copy(description = "Draft $it", amount = "10")) }
            repo.persistDraft()
            val saves = (1..20).map { async(Dispatchers.IO) { repo.save(false) } }.awaitAll()
            assertEquals(1, saves.count { it })
            assertEquals(1, repo.state.value.snapshot.recentTransactions.count { it.description == "Draft 99" })
            val next = repo.state.value.draft.copy(description = "Next unfinished entry", amount = "3.50")
            repo.updateDraft(next)
            repo.persistDraft()
            repo.close()
            repo = FinanceRepository(context, api, namespace, false)
            repo.start()
            repo.awaitReady()
            assertEquals(next.copy(userTouched = true), repo.state.value.draft)
            assertEquals(1, repo.state.value.snapshot.recentTransactions.count { it.description == "Draft 99" })
        } finally {
            repo.close()
            context.deleteDatabase("$namespace.db")
        }
    }

    @Test fun rejectedBookkeepingRevisionIsRepairedAndEditSyncs() = exerciseRepair(false)
    @Test fun repairedEditStillRetainsRealAccountConflicts() = exerciseRepair(true)

    private fun exerciseRepair(conflict: Boolean): Unit = runBlocking {
        val context = ApplicationProvider.getApplicationContext<Context>()
        val namespace = "revision-repair-${uuidV7()}"
        val session = Session("a".repeat(64), "repair-test", "Test")
        val original = demoSnapshot().recentTransactions.first().copy(
            toAccountId = "bookkeeping-expense", revision = 1, canEdit = true,
        )
        var snapshot = demoSnapshot().copy(
            accounts = demoSnapshot().accounts.map { it.copy(ledgerRevision = 10) },
            recentTransactions = listOf(original),
        )
        val optimistic = original.copy(description = "Corrected description", pending = true)
        val payload = writeEntry(editDraft(original, snapshot).copy(description = optimistic.description), snapshot).first
            .copy(expectedAccountRevisions = listOf(AccountRevision("Cash", 10), AccountRevision("bookkeeping-expense", 0)))
        val sent = mutableListOf<WritePayload>()
        val server = MockWebServer()
        server.dispatcher = object : Dispatcher() {
            override fun dispatch(request: RecordedRequest): MockResponse {
                fun json(body: String) = MockResponse().setHeader("Content-Type", "application/json").setBody(body)
                return when {
                    request.path == "/api/session" -> json("{\"pocketAuthenticated\":true}")
                    request.path!!.startsWith("/api/db/finance?") -> json(codec.encodeToString(Snapshot.serializer(), snapshot))
                    request.path == "/api/db/transactions/edit" -> {
                        val received = codec.decodeFromString<WritePayload>(request.body.readUtf8())
                        sent.add(received)
                        if (received.expectedAccountRevisions != listOf(AccountRevision("Cash", 10)))
                            MockResponse().setResponseCode(400).setBody("{\"error\":\"Wrong revisions\"}")
                        else if (conflict)
                            MockResponse().setResponseCode(409).setBody("{\"error\":\"An account changed after it was loaded. Refresh and retry the edit.\"}")
                        else {
                            snapshot = snapshot.copy(recentTransactions = listOf(optimistic.copy(pending = false, revision = 2)))
                            json("{\"revision\":2}")
                        }
                    }
                    else -> MockResponse().setResponseCode(503)
                }
            }
        }
        server.start()
        val api = FinanceApi(server.url("/").toString().trimEnd('/'))
        val scope = "${session.owner}|${api.origin}"
        val seed = Room.databaseBuilder(context, FinanceDatabase::class.java, "$namespace.db").build()
        seed.finance().putCache(CacheRow(scope, codec.encodeToString(Snapshot.serializer(), snapshot)))
        val rejected = OutboxRow(
            payload.operationId, scope, original.id, codec.encodeToString(WritePayload.serializer(), payload),
            codec.encodeToString(Transaction.serializer(), optimistic), codec.encodeToString(Transaction.serializer(), original),
            editing = true, status = "blocked",
            error = "ApiFailure (HTTP 400): Provide the current ledger revision for every real account affected by this edit.",
        )
        seed.finance().enqueue(rejected)
        seed.close()
        Vault(context, namespace).saveSession(session)
        val repo = FinanceRepository(context, api, namespace, false)
        try {
            repo.start()
            repo.awaitReady()
            repo.sync()
            assertEquals(1, sent.size)
            assertEquals(payload.operationId, sent.single().operationId)
            assertEquals(payload.expectedRevision, sent.single().expectedRevision)
            assertTrue(repo.logs.value.any { it.contains("Repaired bookkeeping revision") })
            assertTrue(repo.logs.value.none { it.contains(session.accessToken) })
            val transaction = repo.state.value.snapshot.recentTransactions.first { it.id == original.id }
            if (conflict) {
                assertEquals("blocked", repo.state.value.outbox.single().status)
                assertTrue(syncErrorSummary(repo.state.value.outbox.single()).contains("Review"))
                assertTrue(transaction.pending)
                assertEquals(10L, codec.decodeFromString<WritePayload>(repo.state.value.outbox.single().payload)
                    .expectedAccountRevisions!!.single().revision)
            } else {
                assertTrue(repo.state.value.outbox.isEmpty())
                assertFalse(transaction.pending)
                assertTrue(transaction.canEdit)
                assertEquals(2L, transaction.revision)
                assertTrue(repo.logs.value.any { it.contains("acknowledged") })
            }
        } finally {
            repo.close()
            server.shutdown()
            context.deleteDatabase("$namespace.db")
            File(context.noBackupFilesDir, "$namespace-pocket-session.sealed").delete()
            File(context.noBackupFilesDir, "$namespace-diagnostics.log").delete()
        }
    }

    @Test fun diagnosticsRedactCredentialsAndRepairDoesNotInventMissingRevisions() {
        assertEquals("Bearer [redacted] [redacted]", redactDiagnostics("Bearer secret ${"b".repeat(64)}"))
        val snapshot = demoSnapshot()
        val original = snapshot.recentTransactions.first()
        val payload = writeEntry(editDraft(original, snapshot), snapshot).first.copy(expectedAccountRevisions = emptyList())
        val row = OutboxRow(payload.operationId, "test", original.id,
            codec.encodeToString(WritePayload.serializer(), payload), codec.encodeToString(Transaction.serializer(), original),
            codec.encodeToString(Transaction.serializer(), original), true, status = "blocked",
            error = "HTTP 400: Provide the current ledger revision for every real account affected by this edit.")
        assertNull(repairBookkeepingRevisions(row))
    }
}
