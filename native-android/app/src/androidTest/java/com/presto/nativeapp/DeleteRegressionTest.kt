package com.presto.nativeapp

import android.content.Context
import androidx.room.Room
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import com.presto.core.*
import java.io.File
import kotlinx.coroutines.runBlocking
import kotlinx.serialization.json.*
import kotlin.test.*
import okhttp3.mockwebserver.*
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class DeleteRegressionTest {
    @Test fun deletionRetriesExactRequestAfterFailureAndKeepsConflicts(): Unit = runBlocking {
        for (status in listOf(503, 409)) {
            val context = ApplicationProvider.getApplicationContext<Context>()
            val namespace = "delete-test-${uuidV7()}"
            val session = Session("a".repeat(64), "delete-test", "Test")
            val base = demoSnapshot()
            val entry = base.recentTransactions.first().copy(id = uuidV7(), canEdit = true, revision = 3)
            var snapshot = base.copy(recentTransactions = listOf(entry), transactionCount = 1)
            val sent = mutableListOf<String>()
            val server = MockWebServer()
            server.dispatcher = object : Dispatcher() {
                override fun dispatch(request: RecordedRequest): MockResponse = when {
                    request.path == "/api/session" -> MockResponse().setBody("{\"pocketAuthenticated\":true}")
                    request.path!!.startsWith("/api/db/finance?") -> MockResponse().setBody(codec.encodeToString(Snapshot.serializer(), snapshot))
                    request.path == "/api/db/transactions/delete" -> {
                        sent.add(request.body.readUtf8())
                        if (sent.size == 1) MockResponse().setResponseCode(status).setBody("{\"error\":\"Retry deletion\"}")
                        else {
                            snapshot = snapshot.copy(recentTransactions = emptyList(), transactionCount = 0)
                            MockResponse().setBody("{\"deleted\":true,\"transactionId\":\"${entry.id}\"}")
                        }
                    }
                    else -> MockResponse().setResponseCode(503)
                }
            }
            server.start()
            val api = FinanceApi(server.url("/").toString())
            val key = "${session.owner}|${api.origin}"
            val seed = Room.databaseBuilder(context, FinanceDatabase::class.java, "$namespace.db").build()
            seed.finance().putCache(CacheRow(key, codec.encodeToString(Snapshot.serializer(), snapshot)))
            seed.close()
            Vault(context, namespace).saveSession(session)
            var repo = FinanceRepository(context, api, namespace, false)
            try {
                repo.start()
                repo.awaitReady()
                assertFalse(repo.deleteTransaction(entry.id))
                assertEquals(entry.id, repo.state.value.snapshot.recentTransactions.single().id)
                assertNotNull(repo.state.value.error)
                val payload = codec.parseToJsonElement(sent.single()).jsonObject
                assertEquals(3, payload["expectedRevision"]!!.jsonPrimitive.int)
                assertEquals(base.accounts.filter { it.id == entry.fromAccountId || it.id == entry.toAccountId }.size,
                    payload["expectedAccountRevisions"]!!.jsonArray.size)
                repo.close()
                repo = FinanceRepository(context, api, namespace, false)
                repo.start()
                repo.awaitReady()
                assertTrue(repo.deleteTransaction(entry.id))
                assertTrue(repo.state.value.snapshot.recentTransactions.isEmpty())
                if (status == 503) assertEquals(sent[0], sent[1], "Retry must retain operation ID and revisions across restart")
                else assertNotEquals(payload["operationId"], codec.parseToJsonElement(sent[1]).jsonObject["operationId"])
                val stored = Room.databaseBuilder(context, FinanceDatabase::class.java, "$namespace.db").build()
                try {
                    assertTrue(codec.decodeFromString<Snapshot>(stored.finance().cache(key)!!.snapshot).recentTransactions.isEmpty())
                } finally { stored.close() }
            } finally {
                repo.close()
                server.shutdown()
                context.deleteDatabase("$namespace.db")
                File(context.noBackupFilesDir, "$namespace-pocket-session.sealed").delete()
                File(context.noBackupFilesDir, "$namespace-diagnostics.log").delete()
            }
        }
    }

    @Test fun demoDeletionReversesBudgetAndRemovesEntry(): Unit = runBlocking {
        val context = ApplicationProvider.getApplicationContext<Context>()
        val namespace = "delete-demo-${uuidV7()}"
        val repo = FinanceRepository(context, FinanceApi("http://127.0.0.1:1"), namespace, false)
        try {
            repo.start()
            repo.awaitReady()
            val before = repo.state.value.snapshot
            val entry = before.recentTransactions.first()
            assertTrue(repo.deleteTransaction(entry.id))
            assertEquals(budgetOverlay(before.budgets, entry, -1, before.budgetMonth), repo.state.value.snapshot.budgets)
            assertFalse(repo.state.value.snapshot.recentTransactions.any { it.id == entry.id })
            assertTrue(repo.state.value.outbox.isEmpty())
            assertFalse(repo.deleteTransaction(entry.id))
        } finally {
            repo.close()
            context.deleteDatabase("$namespace.db")
        }
    }
}
