package com.presto.nativeapp

import androidx.activity.compose.setContent
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import org.junit.Rule
import androidx.test.core.app.ApplicationProvider
import android.content.Context
import androidx.test.ext.junit.runners.AndroidJUnit4
import com.presto.core.*
import kotlinx.coroutines.runBlocking
import kotlin.test.*
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class DuplicateRegressionTest {
    @get:Rule val compose = createAndroidComposeRule<MainActivity>()

    @Test fun saveButtonShowsCancelableWarning() = warningUi(false)
    @Test fun saveAnotherShowsCancelableWarning() = warningUi(true)

    private fun warningUi(another: Boolean) {
        val repo = FinanceRepository(compose.activity, namespace = "duplicate-ui-${uuidV7()}", scheduleWork = false)
        repo.start()
        runBlocking { repo.awaitReady() }
        val draft = defaultDraft(repo.state.value.snapshot).copy(amount = "8.50", description = "Duplicate UI", date = "2020-01-15", manualCategory = true, manualFromAccount = true)
        runBlocking {
            repo.updateDraft(draft)
            assertTrue(repo.save(false))
        }
        val count = repo.state.value.snapshot.recentTransactions.size
        compose.runOnIdle {
            repo.updateDraft(draft.copy(amount = "8.52"))
            compose.activity.setContent { PrestoApp(PrestoViewModel(repo), {}, {}) }
        }
        val tag = if (another) "save-another" else "save-entry"
        compose.onNodeWithTag(tag).apply { if (another) performScrollTo() }.performClick()
        compose.waitUntil(5_000) { repo.state.value.duplicateReview != null }
        compose.onNodeWithText("Possible duplicate").assertIsDisplayed()
        compose.onNodeWithText("Keep editing").performClick()
        compose.runOnIdle {
            assertNull(repo.state.value.duplicateReview)
            assertEquals("8.52", repo.state.value.draft.amount)
            assertEquals(count, repo.state.value.snapshot.recentTransactions.size)
        }
        compose.onNodeWithTag(tag).apply { if (another) performScrollTo() }.performClick()
        compose.waitUntil(5_000) { repo.state.value.duplicateReview != null }
        compose.onNodeWithText("Save anyway").performClick()
        compose.waitUntil(5_000) { repo.state.value.snapshot.recentTransactions.size == count + 1 && !repo.state.value.saving }
        compose.onNodeWithText("Possible duplicate").assertDoesNotExist()
        if (another) compose.runOnIdle { assertEquals("", repo.state.value.draft.amount) }
    }

    @Test fun cancelConfirmAndSaveAnother() = runBlocking {
        val context = ApplicationProvider.getApplicationContext<Context>()
        val repo = FinanceRepository(context, namespace = "duplicate-${uuidV7()}", scheduleWork = false)
        repo.start()
        repo.awaitReady()
        val draft = defaultDraft(repo.state.value.snapshot).copy(
            amount = "7.50", description = "Duplicate regression", date = "2020-01-15",
            manualCategory = true, manualFromAccount = true,
        )
        repo.updateDraft(draft)
        assertTrue(repo.save(false))
        val count = repo.state.value.snapshot.recentTransactions.size
        repo.updateDraft(draft.copy(amount = "7.52"))
        assertFalse(repo.save(true))
        val review = assertNotNull(repo.state.value.duplicateReview)
        assertEquals(count, repo.state.value.snapshot.recentTransactions.size)
        repo.dismissDuplicateReview()
        assertFalse(repo.save(true, review))
        val refreshed = assertNotNull(repo.state.value.duplicateReview)
        assertTrue(repo.save(true, refreshed))
        assertNull(repo.state.value.duplicateReview)
        assertEquals(count + 1, repo.state.value.snapshot.recentTransactions.size)
        assertEquals("", repo.state.value.draft.amount)
        assertEquals("2020-01-15", repo.state.value.draft.date)
    }

    @Test fun approvalDoesNotCoverChangedDraft() = runBlocking {
        val context = ApplicationProvider.getApplicationContext<Context>()
        val repo = FinanceRepository(context, namespace = "duplicate-change-${uuidV7()}", scheduleWork = false)
        repo.start()
        repo.awaitReady()
        val draft = defaultDraft(repo.state.value.snapshot).copy(amount = "9.50", description = "First", date = "2020-01-15", manualCategory = true, manualFromAccount = true)
        repo.updateDraft(draft)
        assertTrue(repo.save(false))
        repo.updateDraft(draft)
        assertFalse(repo.save(false))
        val review = assertNotNull(repo.state.value.duplicateReview)
        repo.updateDraft(repo.state.value.draft.copy(description = "Changed"))
        assertFalse(repo.save(false, review))
        val fresh = assertNotNull(repo.state.value.duplicateReview)
        assertEquals("Changed", fresh.draft.description)
        assertTrue(repo.save(false, fresh))
    }
}
