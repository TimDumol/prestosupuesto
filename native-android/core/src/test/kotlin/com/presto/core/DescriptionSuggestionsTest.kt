package com.presto.core

import kotlin.test.*

class DescriptionSuggestionsTest {
    private fun transaction(
        id: String = "old",
        description: String = "Corner café",
        kind: String = "expense",
    ) =
        Transaction(
            id,
            kind,
            "2020-01-01",
            description,
            "25".toBigDecimal(),
            fromCategoryId = "food",
            fromAccountId = "php",
        )

    private val snapshot =
        Snapshot(
            asOf = "2026-10-03",
            budgetMonth = "2026-10",
            accounts = listOf(Account("php", "GoTyme", currency = "PHP")),
            budgets = listOf(Category("food", "Food")),
        )

    @Test
    fun searchesOlderHistoryAndHandlesTyposAccentsAndWordOrder() {
        val rows = (1..20).map { transaction("new-$it", "Unrelated item $it") } + transaction()
        for (query in listOf("cornr", "cafe corner", "cnr", "café")) {
            assertEquals(
                "old",
                descriptionSuggestions(query, rows, "expense").first().transaction.id,
                query,
            )
        }
        assertTrue(descriptionSuggestions("zzzzzz", rows, "expense").isEmpty())
        assertTrue(descriptionSuggestions("corner", rows, "income").isEmpty())
        assertTrue(
            descriptionSuggestions("corner", listOf(transaction()), "expense", "old").isEmpty()
        )
    }

    @Test
    fun choosingSuggestionFillsLinksAndPreservesAmountAndDate() {
        val d =
            Draft(
                amount = "7.50",
                date = "2026-10-03",
                accountFrom = "other",
                categoryFrom = "other",
                manualCategory = true,
                manualFromAccount = true,
            )
        val result =
            applyDescriptionSuggestion(d, snapshot, DescriptionSuggestion(transaction(), 100.0))
        assertEquals("Corner café", result.description)
        assertEquals("food", result.categoryFrom)
        assertEquals("php", result.accountFrom)
        assertEquals("PHP", result.currency)
        assertEquals(d.amount, result.amount)
        assertEquals(d.date, result.date)
        assertTrue(result.manualCategory)
        assertTrue(result.manualFromAccount)
    }

    @Test
    fun deletedLinksAndOtherKindsDoNotOverwriteValidSelections() {
        val d = Draft(accountFrom = "current", categoryFrom = "current", currency = "USD")
        val result =
            applyDescriptionSuggestion(d, Snapshot("2026-10-03", "2026-10"), DescriptionSuggestion(transaction(), 100.0))
        assertEquals(d.accountFrom, result.accountFrom)
        assertEquals(d.categoryFrom, result.categoryFrom)
        assertEquals(d.currency, result.currency)
        assertEquals(
            d,
            applyDescriptionSuggestion(
                d,
                snapshot,
                DescriptionSuggestion(transaction(kind = "income"), 100.0),
            ),
        )
    }
    @Test
    fun suggestedExpenseCanBeSavedAndRefundKeepsItsTreatment() {
        val expense = applyDescriptionSuggestion(
            Draft(amount = "7.50", date = "2026-10-03"), snapshot,
            DescriptionSuggestion(transaction(), 100.0),
        )
        assertEquals("php", writeEntry(expense, snapshot).first.fromAccountId)
        val refund = transaction(kind = "income").copy(
            fromCategoryId = null, fromAccountId = null,
            toCategoryId = "food", toAccountId = "php", incomeTreatment = "refund",
        )
        val income = applyDescriptionSuggestion(
            Draft(kind = "income", amount = "7.50", date = "2026-10-03"), snapshot,
            DescriptionSuggestion(refund, 100.0),
        )
        assertEquals("refund", income.incomeTreatment)
        assertEquals("food", writeEntry(income, snapshot).first.toCategoryId)
    }

}
