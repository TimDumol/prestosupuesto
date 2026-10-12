package com.presto.nativeapp

import android.content.Context
import androidx.room.Room
import java.io.File
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import okhttp3.mockwebserver.*
import androidx.activity.compose.setContent
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.test.ext.junit.runners.AndroidJUnit4
import com.presto.core.*
import kotlinx.coroutines.runBlocking
import kotlin.test.*
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class EntryRegressionTest {
    @get:Rule val compose = createAndroidComposeRule<MainActivity>()

    @Test fun expenseSaveNavigatesWhileSyncIsStalled() = saveWhileSyncIsStalled(false, "expense")
    @Test fun expenseSaveWithAmountKeyboardOpen() = saveWhileSyncIsStalled(false, "expense", focusField = "entry-amount")
    @Test fun expenseSaveWithDescriptionKeyboardOpen() = saveWhileSyncIsStalled(false, "expense", focusField = "entry-description")
    @Test fun expenseSaveAnotherResetsWhileSyncIsStalled() = saveWhileSyncIsStalled(true, "expense")
    @Test fun salarySaveNavigatesWhileSyncIsStalled() = saveWhileSyncIsStalled(false, "income", "salary")
    @Test fun salarySaveAnotherResetsWhileSyncIsStalled() = saveWhileSyncIsStalled(true, "income", "salary")
    @Test fun extraIncomeSaveNavigatesWhileSyncIsStalled() = saveWhileSyncIsStalled(false, "income", "unexpected")
    @Test fun extraIncomeSaveAnotherResetsWhileSyncIsStalled() = saveWhileSyncIsStalled(true, "income", "unexpected")
    @Test fun refundSaveNavigatesWhileSyncIsStalled() = saveWhileSyncIsStalled(false, "income", "refund")
    @Test fun refundSaveAnotherResetsWhileSyncIsStalled() = saveWhileSyncIsStalled(true, "income", "refund")
    @Test fun transferSaveNavigatesWhileSyncIsStalled() = saveWhileSyncIsStalled(false, "transfer")
    @Test fun transferSaveAnotherResetsWhileSyncIsStalled() = saveWhileSyncIsStalled(true, "transfer")
    @Test fun budgetMoveSaveNavigatesWhileSyncIsStalled() = saveWhileSyncIsStalled(false, "reallocate")
    @Test fun budgetMoveSaveAnotherResetsWhileSyncIsStalled() = saveWhileSyncIsStalled(true, "reallocate")

    private fun saveWhileSyncIsStalled(another: Boolean, kind: String, treatment: String = "salary", focusField: String? = null) {
        val context = compose.activity
        val namespace = "save-feedback-${uuidV7()}"
        val session = Session("a".repeat(64), "save-feedback", "Test")
        val snapshot = demoSnapshot()
        val requested = CountDownLatch(1)
        val release = CountDownLatch(1)
        val server = MockWebServer()
        server.dispatcher = object : Dispatcher() {
            override fun dispatch(request: RecordedRequest): MockResponse {
                if (request.path == "/api/session") {
                    requested.countDown()
                    release.await(30, TimeUnit.SECONDS)
                }
                return MockResponse().setResponseCode(503)
            }
        }
        server.start()
        val api = FinanceApi(server.url("/").toString())
        val scope = "${session.owner}|${api.origin}"
        val seed = Room.databaseBuilder(context, FinanceDatabase::class.java, "$namespace.db").build()
        runBlocking {
            seed.finance().putCache(CacheRow(scope, codec.encodeToString(Snapshot.serializer(), snapshot)))
        }
        seed.close()
        Vault(context, namespace).saveSession(session)
        val repo = FinanceRepository(context, api, namespace, false)
        repo.start()
        runBlocking { repo.awaitReady() }
        try {
            assertTrue(requested.await(5, TimeUnit.SECONDS))
            compose.runOnIdle {
                repo.updateDraft(defaultDraft(snapshot).copy(
                    kind = kind, incomeTreatment = treatment,
                    amount = "7.50", description = "Queued without waiting",
                    date = if (kind == "reallocate") "2020-01-31" else "2020-01-15",
                    toAmount = "8.50", manualFromAccount = true, manualToAccount = true,
                ))
                context.setContent { PrestoApp(PrestoViewModel(repo), {}, {}) }
            }
            val saveButton = compose.onNodeWithTag(if (another) "save-another" else "save-entry")
            if (focusField != null) {
                compose.onNodeWithTag(focusField).performScrollTo().performClick()
                compose.waitUntil(5_000) {
                    androidx.core.view.ViewCompat.getRootWindowInsets(context.window.decorView)
                        ?.isVisible(androidx.core.view.WindowInsetsCompat.Type.ime()) == true
                }
                compose.waitForIdle()
                val keyboard = androidx.core.view.ViewCompat.getRootWindowInsets(context.window.decorView)!!
                    .getInsets(androidx.core.view.WindowInsetsCompat.Type.ime()).bottom
                assertTrue(saveButton.fetchSemanticsNode().boundsInWindow.bottom <= context.window.decorView.height - keyboard,
                    "Save Entry must be above the keyboard and reachable by touch")
            }
            if (another) saveButton.performScrollTo()
            saveButton.performClick()
            compose.waitUntil(3_000) { repo.state.value.pending == 1 && !repo.state.value.saving }
            if (another) {
                compose.onNodeWithTag("entry-amount").assertExists()
                assertEquals("", repo.state.value.draft.amount)
                assertEquals(if (kind == "reallocate") "2020-01-31" else "2020-01-15", repo.state.value.draft.date)
            } else {
                compose.onNodeWithTag("activity-list").assertExists()
            }
            assertEquals(1L, release.count)
            assertTrue(repo.state.value.snapshot.recentTransactions.single {
                it.description == "Queued without waiting"
            }.pending)
            // Check durable storage while the network is still blocked.
            val stored = Room.databaseBuilder(context, FinanceDatabase::class.java, "$namespace.db").build()
            try {
                runBlocking {
                    assertEquals(1, stored.finance().outbox(scope).size)
                    val draft = codec.decodeFromString<Draft>(stored.finance().draft(scope)!!.draft)
                    assertEquals("", draft.amount)
                }
            } finally { stored.close() }
        } finally {
            release.countDown()
            runBlocking { repo.close() }
            server.shutdown()
            context.deleteDatabase("$namespace.db")
            File(context.noBackupFilesDir, "$namespace-pocket-session.sealed").delete()
            File(context.noBackupFilesDir, "$namespace-diagnostics.log").delete()
            context.getSharedPreferences("$namespace-owner", Context.MODE_PRIVATE).edit().clear().commit()
        }
    }

    @Test
    fun structuredEntryAndBackdatedContinuationUseIsolatedDemo() {
        val namespace = "entry-regression-${uuidV7()}"
        val repo = FinanceRepository(compose.activity, FinanceApi("http://127.0.0.1:1"), namespace, false)
        repo.start()
        runBlocking { repo.awaitReady() }
        try {
            compose.runOnIdle {
                compose.activity.setContent { PrestoApp(PrestoViewModel(repo), {}, {}) }
            }
            compose.onNodeWithText("More options").performScrollTo().performClick()
            compose.onNodeWithTag("quick-entry").assertDoesNotExist()
            compose.onNodeWithTag("apply-quick-entry").assertDoesNotExist()
            compose.onNodeWithTag("entry-amount").performScrollTo().performTextReplacement("2287.43")
            compose.onNodeWithTag("entry-description").performScrollTo().performTextReplacement("Cleaner")
            compose.runOnIdle {
                repo.updateDraft(repo.state.value.draft.copy(date = "2020-01-15"))
            }
            compose.onNodeWithTag("save-another").performScrollTo().performClick()
            compose.waitUntil(10_000) { repo.state.value.draft.amount.isEmpty() }
            compose.runOnIdle {
                assertEquals("2020-01-15", repo.state.value.draft.date)
                assertTrue(repo.state.value.snapshot.recentTransactions.any {
                    it.description == "Cleaner" && it.effectiveDate == "2020-01-15"
                })
            }
        } finally {
            runBlocking { repo.close() }
            compose.activity.deleteDatabase("$namespace.db")
        }
    }
    @Test
    fun suggestionCanBeSavedAndInvalidAmountShowsPersistentError() {
        val namespace = "suggestion-regression-${uuidV7()}"
        val repo = FinanceRepository(compose.activity, FinanceApi("http://127.0.0.1:1"), namespace, false)
        repo.start()
        runBlocking { repo.awaitReady() }
        try {
            val historical = repo.state.value.snapshot.recentTransactions.first {
                it.kind == "expense" && it.fromCategoryId != null && it.fromAccountId != null
            }
            compose.runOnIdle {
                compose.activity.setContent { PrestoApp(PrestoViewModel(repo), {}, {}) }
            }
            compose.onNodeWithTag("save-entry").performClick()
            compose.waitUntil(3_000) { repo.state.value.error != null && !repo.state.value.saving }
            compose.onNodeWithTag("entry-validation-error").assertIsDisplayed()
            compose.onNodeWithTag("entry-amount").performTextReplacement("0")
            compose.onNodeWithText("Amount must be greater than zero.").assertExists()
            compose.onNodeWithTag("entry-amount").performTextReplacement("7.50")
            compose.onNodeWithTag("entry-description").performScrollTo()
                .performTextReplacement(historical.description)
            compose.onNodeWithTag("description-suggestion-${historical.id}")
                .performScrollTo().performClick()
            compose.onNodeWithTag("save-entry").performClick()
            compose.waitUntil(10_000) { repo.state.value.draft.amount.isEmpty() }
            compose.runOnIdle {
                assertNull(repo.state.value.error)
                assertTrue(repo.state.value.snapshot.recentTransactions.any {
                    it.id != historical.id && it.description == historical.description &&
                        it.amount.compareTo("7.50".toBigDecimal()) == 0 &&
                        it.fromAccountId == historical.fromAccountId &&
                        it.fromCategoryId == historical.fromCategoryId
                })
            }
            compose.onNodeWithTag("tab-setup").performClick()
            compose.onNodeWithTag("show-logs").performScrollTo().performClick()
            compose.onNodeWithTag("diagnostic-logs").assertIsDisplayed()
            compose.onNodeWithText("Copy logs").assertIsDisplayed()
        } finally {
            runBlocking { repo.close() }
            compose.activity.deleteDatabase("$namespace.db")
        }
    }

    @Test
    fun invalidEntriesKeepDraftAndShowErrorForEveryKind() {
        val namespace = "invalid-entry-${uuidV7()}"
        val repo = FinanceRepository(compose.activity, FinanceApi("http://127.0.0.1:1"), namespace, false)
        repo.start()
        runBlocking { repo.awaitReady() }
        try {
            compose.runOnIdle {
                compose.activity.setContent { PrestoApp(PrestoViewModel(repo), {}, {}) }
            }
            for (kind in listOf("expense", "income", "transfer", "reallocate")) {
                for (another in listOf(false, true)) {
                    compose.runOnIdle {
                        repo.updateDraft(defaultDraft(repo.state.value.snapshot).copy(
                            kind = kind, amount = "7.50", description = "",
                            manualFromAccount = true, manualToAccount = true,
                        ))
                    }
                    val button = compose.onNodeWithTag(if (another) "save-another" else "save-entry")
                    if (another) button.performScrollTo()
                    button.performClick()
                    compose.waitUntil(3_000) { repo.state.value.error != null && !repo.state.value.saving }
                    compose.onNodeWithTag("entry-validation-error").assertIsDisplayed()
                    compose.onNodeWithTag("entry-amount").assertExists()
                    compose.runOnIdle {
                        assertEquals("7.50", repo.state.value.draft.amount)
                        assertEquals(kind, repo.state.value.draft.kind)
                        assertEquals(0, repo.state.value.pending)
                    }
                }
            }
        } finally {
            runBlocking { repo.close() }
            compose.activity.deleteDatabase("$namespace.db")
        }
    }

    @Test
    fun blockedTransactionShowsCauseAndReviewAction() {
        val snapshot = demoSnapshot()
        val transaction = snapshot.recentTransactions.first().copy(pending = true)
        val operation = OutboxRow("test-operation", "demo", transaction.id, "{}", "{}", null,
            false, status = "blocked", error = "ApiFailure (HTTP 409): POST https://example.test/edit: An account changed after it was loaded.")
        var reviewed = false
        compose.runOnIdle {
            compose.activity.setContent {
                androidx.compose.material3.MaterialTheme {
                    TransactionDetails(transaction, snapshot, {}, {}, operation = operation,
                        review = { reviewed = true })
                }
            }
        }
        compose.onNodeWithTag("transaction-sync-error").assertIsDisplayed()
        compose.onNodeWithText("Sync failed: An account changed after it was loaded. Review this entry in Settings.")
            .assertIsDisplayed()
        compose.onNodeWithText("Review sync failure").performClick()
        compose.runOnIdle { assertTrue(reviewed) }
        compose.onNodeWithTag("edit-inspected-entry").assertDoesNotExist()
    }

}
