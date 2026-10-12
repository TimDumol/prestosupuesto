package com.presto.nativeapp

import android.content.Context
import androidx.room.Room
import androidx.room.withTransaction
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import com.presto.core.*
import java.io.File
import java.util.concurrent.TimeUnit
import kotlin.test.*
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.launch
import kotlinx.serialization.json.*
import okhttp3.mockwebserver.*
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class PersistenceTest {
    private val context
        get() = ApplicationProvider.getApplicationContext<Context>()

    @Test
    fun selectionOnlyDraftSurvivesRefreshReopenAndSaveAnother(): Unit = runBlocking {
        val server = MockWebServer()
        val namespace = "draft-test-${uuidV7()}"
        val snapshot = demoSnapshot()
        server.dispatcher = object : Dispatcher() {
            override fun dispatch(request: RecordedRequest): MockResponse {
                val body = when {
                    request.path == "/api/mobile/session/exchange" -> codec.encodeToString(
                        Session.serializer(), Session("b".repeat(64), "draft-owner", "Test"))
                    request.path == "/api/session" -> "{\"pocketAuthenticated\":true}"
                    request.path!!.startsWith("/api/db/finance?") -> codec.encodeToString(Snapshot.serializer(), snapshot)
                    request.path == "/api/db/transactions" -> "{\"revision\":1}"
                    else -> return MockResponse().setResponseCode(503)
                }
                return MockResponse().setHeader("Content-Type", "application/json").setBody(body)
            }
        }
        server.start()
        var repo = FinanceRepository(context, FinanceApi(server.url("/").toString()), namespace, false)
        try {
            repo.start()
            repo.awaitReady()
            repo.login("a".repeat(64), "v".repeat(64))
            for (selection in listOf(
                defaultDraft(snapshot).copy(date = "2020-01-15"),
                defaultDraft(snapshot).copy(accountFrom = "H GoTyme", manualFromAccount = true),
                defaultDraft(snapshot).copy(categoryFrom = "Travel", manualCategory = true),
                defaultDraft(snapshot).copy(note = "Reference only"),
            )) {
                repo.updateDraft(selection)
                val expected = repo.state.value.draft
                assertTrue(expected.touched)
                repo.sync()
                assertEquals(expected, repo.state.value.draft)
                repo.persistDraft()
                repo.close()
                repo = FinanceRepository(context, FinanceApi(server.url("/").toString()), namespace, false)
                repo.start()
                repo.awaitReady()
                assertEquals(expected, repo.state.value.draft)
            }
            repo.updateDraft(defaultDraft(snapshot).copy(date = "2020-01-15", amount = "10", description = "Backdated"))
            assertTrue(repo.save(true))
            repo.sync()
            assertEquals("2020-01-15", repo.state.value.draft.date)
        } finally {
            repo.close()
            context.deleteDatabase("$namespace.db")
            context.getSharedPreferences("$namespace-owner", Context.MODE_PRIVATE).edit().clear().commit()
            server.shutdown()
        }
    }

    @Test
    fun atomicOutboxSurvivesReopenAndIsOwnerIsolated(): Unit = runBlocking {
        val name = "outbox-test-${uuidV7()}.db"
        var db = Room.databaseBuilder(context, FinanceDatabase::class.java, name).build()
        val (payload, t) =
            writeEntry(
                defaultDraft(demoSnapshot())
                    .copy(amount = "12.50", description = "Offline expense"),
                demoSnapshot(),
            )
        val row =
            OutboxRow(
                payload.operationId,
                "owner-a",
                t.id,
                codec.encodeToString(WritePayload.serializer(), payload),
                codec.encodeToString(Transaction.serializer(), t),
                null,
                false,
            )
        val draft = DraftRow("owner-a", codec.encodeToString(Draft.serializer(), Draft()))
        try {
            assertFailsWith<IllegalStateException> {
                db.withTransaction {
                    db.finance().enqueue(row)
                    error("Crash before commit")
                }
            }
            assertTrue(db.finance().outbox("owner-a").isEmpty())
            db.withTransaction {
                db.finance().enqueue(row)
                db.finance().putDraft(draft)
            }
            db.close()
            db = Room.databaseBuilder(context, FinanceDatabase::class.java, name).build()
            assertEquals(row, db.finance().outbox("owner-a").single())
            assertEquals(draft, db.finance().draft("owner-a"))
            assertTrue(db.finance().outbox("owner-b").isEmpty())
            assertFailsWith<android.database.sqlite.SQLiteConstraintException> {
                db.finance().enqueue(row)
            }
        } finally {
            db.close()
            context.deleteDatabase(name)
        }
    }

    private fun interchange(): String {
        val fixtureTables =
            setOf(
                "finance_accounts",
                "finance_account_opening_balances",
                "finance_categories",
                "category_budget_periods",
                "finance_transactions",
                "finance_transaction_revisions",
                "transaction_valuations",
                "transaction_valuation_source_cells",
                "budget_month_snapshots",
                "budget_month_closures",
                "budget_opening_state",
                "finance_exchange_rate_source_rows",
                "exchange_rates",
                "finance_account_actuals",
                "finance_account_balance_checkpoints",
                "finance_operations",
                "finance_category_corrections",
                "finance_category_correction_items",
                "reconciliation_batches",
                "reconciliation_items",
                "finance_import_runs",
                "finance_quarantined_rows",
                "finance_source_artifacts",
                "budget_income_expectations",
                "budget_ledger_state",
                "budget_plan_income",
                "budget_income_receipts",
                "budget_ledger_movements",
                "budget_coverage_closures",
            )
        return buildJsonObject {
                put("interchangeFormatVersion", 1)
                put("databaseSchemaVersion", 14)
                put("exportedAt", "2026-10-02T00:00:00Z")
                put("operationWatermark", JsonNull)
                put(
                    "tables",
                    buildJsonObject { fixtureTables.forEach { put(it, JsonArray(emptyList())) } },
                )
                put("tableCounts", buildJsonObject { fixtureTables.forEach { put(it, 0) } })
            }
            .toString()
    }

    @Test
    fun recoveryRejectsDamageAndFallsBackAndRotates() {
        val scope = "recovery-test-${uuidV7()}"
        val store = RecoveryStore(context, Vault(context))
        val raw = interchange()
        val digest = sha256(raw.encodeToByteArray())
        val root =
            File(context.noBackupFilesDir, "finance-recovery/${sha256(scope.encodeToByteArray())}")
        try {
            assertTrue(store.reserve(scope))
            assertFalse(store.reserve(scope))
            assertFailsWith<IllegalArgumentException> { store.save(scope, raw, "0".repeat(64)) }
            val first = store.save(scope, raw, digest)
            assertEquals(raw, store.latest(scope)?.raw)
            Thread.sleep(2)
            val second = store.save(scope, raw, digest)
            File(root, "${second.id}/0.sealed").writeBytes(byteArrayOf(1, 2, 3))
            assertEquals(first.id, store.latest(scope)?.info?.id)
            assertNull(store.latest("different-owner"))
            repeat(9) {
                Thread.sleep(2)
                store.save(scope, raw, digest)
            }
            assertEquals(
                8,
                root.listFiles()!!.count { it.isDirectory && !it.name.endsWith(".partial") },
            )
            val bad = raw.replace("\"databaseSchemaVersion\":14", "\"databaseSchemaVersion\":999")
            assertFailsWith<IllegalArgumentException> {
                store.save(scope, bad, sha256(bad.encodeToByteArray()))
            }
        } finally {
            root.deleteRecursively()
        }
    }

    @Test
    fun lostAcknowledgementRetriesSameOperationAndConflictIsRetained(): Unit = runBlocking {
        val server = MockWebServer()
        val namespace = "sync-test-${uuidV7()}"
        var snapshot = demoSnapshot()
        var firstLost = true
        var offline = false
        var conflict = false
        val delayValidation = java.util.concurrent.atomic.AtomicBoolean(false)
        val validationStarted = java.util.concurrent.CountDownLatch(1)
        val operationIds = mutableListOf<String>()
        val committed = mutableSetOf<String>()
        server.dispatcher =
            object : Dispatcher() {
                override fun dispatch(request: RecordedRequest): MockResponse {
                    fun json(body: String) =
                        MockResponse().setHeader("Content-Type", "application/json").setBody(body)
                    return when {
                        request.path == "/api/mobile/session/exchange" ->
                            json(
                                codec.encodeToString(
                                    Session.serializer(),
                                    Session("a".repeat(64), "test-owner", "Test"),
                                )
                            )
                        request.path == "/api/session" -> {
                            val response = json("{\"pocketAuthenticated\":true}")
                            if (delayValidation.getAndSet(false)) {
                                validationStarted.countDown()
                                response.setBodyDelay(500, TimeUnit.MILLISECONDS)
                            }
                            response
                        }
                        request.path!!.startsWith("/api/db/finance?") ->
                            if (offline) MockResponse().setResponseCode(503)
                            else json(codec.encodeToString(Snapshot.serializer(), snapshot))
                        request.path == "/api/db/finance/backup" ->
                            MockResponse().setResponseCode(503)
                        request.path == "/api/db/transactions" ||
                            request.path == "/api/db/transactions/edit" -> {
                            if (offline) MockResponse().setResponseCode(503)
                            else {
                                val payload =
                                    codec.decodeFromString<WritePayload>(request.body.readUtf8())
                                operationIds.add(payload.operationId)
                                if (conflict)
                                    MockResponse()
                                        .setResponseCode(409)
                                        .setBody("{\"error\":\"Revision changed\"}")
                                else {
                                    if (committed.add(payload.operationId)) {
                                        val t =
                                            Transaction(
                                                payload.transactionId,
                                                payload.kind,
                                                payload.date,
                                                payload.description,
                                                payload.amount,
                                                fromCategoryId = payload.fromCategoryId,
                                                fromAccountId = payload.fromAccountId,
                                                canEdit = true,
                                            )
                                        snapshot =
                                            snapshot.copy(
                                                recentTransactions =
                                                    listOf(t) + snapshot.recentTransactions
                                            )
                                    }
                                    if (firstLost) {
                                        firstLost = false
                                        MockResponse()
                                            .setSocketPolicy(SocketPolicy.DISCONNECT_AFTER_REQUEST)
                                    } else json("{\"duplicate\":true,\"revision\":1}")
                                }
                            }
                        }
                        else -> MockResponse().setResponseCode(404)
                    }
                }
            }
        server.start()
        val client =
            okhttp3.OkHttpClient.Builder()
                .retryOnConnectionFailure(false)
                .readTimeout(2, TimeUnit.SECONDS)
                .build()
        val repo =
            FinanceRepository(
                context,
                FinanceApi(server.url("/").toString().trimEnd('/'), client),
                namespace,
                scheduleWork = false,
            )
        try {
            repo.start()
            repo.awaitReady()
            repo.login("b".repeat(64), "v".repeat(64))
            repo.dismissMessage()
            delayValidation.set(true)
            val cancelledSync = launch(kotlinx.coroutines.Dispatchers.IO) { repo.sync() }
            assertTrue(validationStarted.await(5, TimeUnit.SECONDS))
            cancelledSync.cancel()
            cancelledSync.join()
            assertTrue(cancelledSync.isCancelled)
            assertNull(repo.state.value.error)
            assertFalse(repo.state.value.syncing)
            // Cancellation must release the sync lock and permit the next refresh.
            assertTrue(repo.sync())
            repo.updateDraft(
                defaultDraft(repo.state.value.snapshot)
                    .copy(amount = "10", description = "Lost response")
            )
            assertTrue(repo.save(false))
            // Save triggers an asynchronous drain. A sequential sync will finish it or retry on the
            // next call.
            repeat(5) {
                kotlinx.coroutines.delay(200)
                repo.sync()
            }
            assertEquals(1, committed.size)
            assertTrue(operationIds.size >= 2)
            assertEquals(1, operationIds.toSet().size)
            assertEquals(0, repo.state.value.pending)
            assertTrue(repo.state.value.outbox.isEmpty())
            assertEquals("Synced successfully", repo.state.value.notice)
            offline = true
            repo.updateDraft(
                defaultDraft(repo.state.value.snapshot)
                    .copy(amount = "20", description = "Offline preserved")
            )
            assertTrue(repo.save(false))
            kotlinx.coroutines.delay(300)
            repo.sync()
            assertEquals(1, repo.state.value.pending)
            assertTrue(
                repo.state.value.snapshot.recentTransactions.any {
                    it.description == "Offline preserved" && it.pending
                }
            )
            conflict = true
            offline = false
            repo.sync()
            assertEquals("blocked", repo.state.value.outbox.single().status)
            assertTrue(repo.state.value.outbox.single().error!!.contains("Revision changed"))
            assertTrue(repo.state.value.outbox.single().error!!.contains("HTTP 409"))
            assertTrue(repo.state.value.outbox.single().error!!.contains(repo.state.value.outbox.single().id))
            assertTrue(
                repo.state.value.snapshot.recentTransactions.any {
                    it.description == "Offline preserved"
                }
            )
            val blocked = repo.state.value.outbox.single()
            assertTrue(repo.reviewBlocked(blocked.id))
            assertEquals(blocked.id, repo.state.value.draft.replacesOperation)
            assertEquals(blocked.id, repo.state.value.outbox.single().id)
            conflict = false
            assertTrue(repo.save(false))
            repeat(5) {
                kotlinx.coroutines.delay(100)
                repo.sync()
            }
            assertTrue(repo.state.value.outbox.isEmpty())
            assertEquals(2, committed.size)
            assertNull(repo.state.value.draft.replacesOperation)
        } finally {
            repo.close()
            server.shutdown()
            context.deleteDatabase("$namespace.db")
            File(context.noBackupFilesDir, "$namespace-pocket-session.sealed").delete()
        }
    }
}
