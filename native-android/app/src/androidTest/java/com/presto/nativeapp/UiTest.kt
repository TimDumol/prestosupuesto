package com.presto.nativeapp

import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.test.ext.junit.runners.AndroidJUnit4
import com.presto.core.*
import kotlin.test.assertTrue
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class UiTest {
    @get:Rule val compose = createAndroidComposeRule<MainActivity>()

    private fun ready() {
        compose.waitUntil(10_000) {
            (compose.activity.application as PrestoApplication).repository.state.value.ready
        }
    }

    @Before
    fun resetDraft() {
        ready()
        val repo = (compose.activity.application as PrestoApplication).repository
        compose.runOnIdle { repo.updateDraft(defaultDraft(repo.state.value.snapshot)) }
    }

    @Test
    fun incomeTransferAndBudgetMoveFormsSave() {
        ready()
        val repo = (compose.activity.application as PrestoApplication).repository
        fun fresh() {
            compose.runOnIdle { repo.updateDraft(defaultDraft(repo.state.value.snapshot)) }
            compose.onNodeWithTag("tab-add").performClick()
        }
        fun save(description: String) {
            compose.onNodeWithTag("entry-amount").performScrollTo().performTextReplacement("10")
            compose
                .onNodeWithTag("entry-description")
                .performScrollTo()
                .performTextReplacement(description)
            compose.onNodeWithTag("save-entry").performClick()
            compose.waitUntil(10_000) {
                repo.state.value.snapshot.recentTransactions.any { it.description == description }
            }
        }
        fresh()
        compose.onNodeWithTag("kind-income").performClick()
        compose.onNodeWithText("Extra income").performClick()
        save("UI extra income")
        fresh()
        compose.onNodeWithTag("kind-transfer").performClick()
        compose.onNodeWithTag("choose-to-account").performScrollTo().performClick()
        compose.onNodeWithTag("picker-search").performTextReplacement("gotyme")
        compose.onNodeWithText("H GoTyme").performClick()
        compose.onNodeWithTag("entry-to-amount").performScrollTo().performTextReplacement("600")
        save("UI cross-currency transfer")
        fresh()
        compose.onNodeWithTag("kind-reallocate").performClick()
        compose.onNodeWithTag("choose-from-category").performScrollTo().performClick()
        compose.onNodeWithTag("picker-search").performTextReplacement("Travel")
        compose.onNodeWithTag("picker-option-Travel").performClick()
        compose.onNodeWithTag("choose-to-category").performScrollTo().performClick()
        compose.onNodeWithTag("picker-search").performTextReplacement("Eating out")
        compose.onNodeWithTag("picker-option-Eating out").performClick()
        save("UI budget move")
        assertTrue(
            repo.state.value.snapshot.recentTransactions.any {
                it.description == "UI cross-currency transfer" &&
                    it.toAmount == "600.00".toBigDecimal()
            }
        )
    }

    @Test
    fun budgetHistorySelectionIsolationAndReset() {
        ready()
        compose.onNodeWithTag("tab-budget").performClick()
        compose.onNodeWithTag("budget-subtab-history").performClick()
        compose.onNodeWithTag("budget-history-segment-0").performClick()
        compose.onNodeWithTag("budget-history-chart-selection").assertExists()
        compose.onNodeWithTag("budget-history-legend-0").performScrollTo().performTouchInput {
            longClick()
        }
        compose.onNodeWithText("Food & groceries over time").assertExists()
        compose.onNodeWithTag("chart-show-all").performScrollTo().performClick()
        compose.onNodeWithText("Monthly spending").assertExists()
    }

    @Test
    fun demoEntrySaveAndAccountNavigation() {
        ready()
        compose.onNodeWithTag("tab-add").performClick()
        compose.onNodeWithTag("entry-amount").performTextReplacement("12.50")
        compose.onNodeWithTag("entry-description").performTextReplacement("Native UI test")
        compose.onNodeWithTag("save-entry").performClick()
        compose.waitUntil(10_000) {
            (compose.activity.application as PrestoApplication)
                .repository
                .state
                .value
                .snapshot
                .recentTransactions
                .any { it.description == "Native UI test" }
        }
        compose.onNodeWithTag("transaction-search").performTextReplacement("Native UI test")
        val saved =
            (compose.activity.application as PrestoApplication)
                .repository
                .state
                .value
                .snapshot
                .recentTransactions
                .first { it.description == "Native UI test" }
        compose.onNodeWithTag("transaction-${saved.id}").performScrollTo().performClick()
        compose.onNodeWithText("From account").assertExists()
        compose.onNodeWithTag("edit-inspected-entry").performClick()
        compose.onNodeWithTag("entry-description").assertTextContains("Native UI test")
        compose.onNodeWithTag("tab-accounts").performClick()
        compose.onNodeWithText("Checking").assertExists()
        compose.onNodeWithTag("tab-setup").performClick()
        compose.onNodeWithText("Connect Pocket ID").assertExists()
    }

    @Test
    fun activityShowsTransactionsImmediatelyAndFiltersStillWork() {
        ready()
        val repo = (compose.activity.application as PrestoApplication).repository
        val newest =
            repo.state.value.snapshot.recentTransactions
                .filter { it.effectiveDate.startsWith(today().take(7)) }
                .sortedWith(
                    compareByDescending<Transaction> { it.effectiveDate }
                        .thenByDescending { it.occurredAt ?: "" }
                        .thenByDescending { it.id }
                )
                .first()
        compose.onNodeWithTag("tab-recent").performClick()
        compose.onNodeWithTag("transaction-${newest.id}").assertIsDisplayed()
        compose
            .onAllNodesWithText("transactions on this device", substring = true)
            .assertCountEquals(0)
        compose.onNodeWithTag("activity-filters").performClick()
        compose.onNodeWithText("Income", useUnmergedTree = true).performClick()
        compose.onNodeWithText("Done").performClick()
        compose.onNodeWithTag("transaction-${newest.id}").assertDoesNotExist()
        compose.onNodeWithText("Reset").performClick()
        compose.onNodeWithTag("transaction-${newest.id}").assertIsDisplayed()
    }

    @Test
    fun descriptionAutocompleteFillsBudgetAndAccountAndAmountHintFollowsFocus() {
        ready()
        val repo = (compose.activity.application as PrestoApplication).repository
        val prior =
            repo.state.value.snapshot.recentTransactions.first { it.description == "Corner café" }
        compose.onNodeWithTag("tab-add").performClick()
        compose.onNodeWithText("Use quick entry").assertDoesNotExist()
        compose.onNodeWithText("Formula example: =12+8").assertDoesNotExist()
        val descriptionBounds = compose.onNodeWithTag("entry-description").fetchSemanticsNode().boundsInRoot
        compose.onNodeWithTag("entry-amount").performClick()
        compose.onNodeWithText("Formula example: =12+8").assertExists()
        assertTrue(compose.onNodeWithTag("entry-description").fetchSemanticsNode().boundsInRoot == descriptionBounds)
        compose.onNodeWithTag("entry-description").performTextReplacement("cornr")
        compose.onNodeWithText("Formula example: =12+8").assertDoesNotExist()
        compose.onNodeWithTag("description-suggestion-${prior.id}").performScrollTo().performClick()
        compose.runOnIdle {
            val draft = repo.state.value.draft
            assertTrue(draft.description == prior.description)
            assertTrue(draft.categoryFrom == prior.fromCategoryId)
            assertTrue(draft.accountFrom == prior.fromAccountId)
            assertTrue(draft.manualCategory && draft.manualFromAccount)
        }
    }

    @Test
    fun draftSurvivesActivityRecreation() {
        ready()
        compose.onNodeWithTag("entry-amount").performTextReplacement("17.25")
        compose.onNodeWithTag("entry-description").performTextReplacement("Draft survives")
        compose.activityRule.scenario.recreate()
        compose.onNodeWithTag("entry-description").assertTextContains("Draft survives")
        val repo = (compose.activity.application as PrestoApplication).repository
        assertTrue(repo.state.value.draft.amount == "17.25")
    }
}
