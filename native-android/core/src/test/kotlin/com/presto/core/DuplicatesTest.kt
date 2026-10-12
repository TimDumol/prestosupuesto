package com.presto.core

import java.math.BigDecimal
import kotlin.test.*

class DuplicatesTest {
    private val entry = Transaction("new", "expense", "2026-10-11", "Café groceries", BigDecimal("12.30"), fromAccountId = "joint", fromCategoryId = "food")
    private val other = entry.copy(id = "other", description = "Completely different words", pending = true)

    @Test fun amountBoundaryAndOptionalDescription() {
        for (amount in listOf("12.27", "12.30", "12.33"))
            assertEquals(1, possibleDuplicates(entry, listOf(other.copy(amount = BigDecimal(amount)))).size)
        for (amount in listOf("12.26", "12.34"))
            assertTrue(possibleDuplicates(entry, listOf(other.copy(amount = BigDecimal(amount)))).isEmpty())
    }

    @Test fun identitiesDatesCurrencyAndDirectionalLinks() {
        for (row in listOf(entry, other.copy(kind = "income"), other.copy(effectiveDate = "2026-10-12"),
            other.copy(inputCurrency = "USD"), other.copy(fromAccountId = "personal"),
            other.copy(toAccountId = "joint", fromAccountId = null), other.copy(fromCategoryId = "travel")))
            assertTrue(possibleDuplicates(entry, listOf(row)).isEmpty(), row.toString())
    }

    @Test fun syncedSystemEndpointsDoNotHideExpenseDuplicates() {
        val synced = other.copy(toAccountId = "system-expense", toCategoryId = "system-expense", toCurrency = null)
        assertEquals(1, possibleDuplicates(entry.copy(toCurrency = "EUR"), listOf(synced)).size)
    }

    @Test fun incomeAndReallocationUseTheirRelevantLinks() {
        val income = entry.copy(kind = "income", fromAccountId = null, fromCategoryId = null, toAccountId = "joint", toCategoryId = "food", incomeTreatment = "refund")
        val synced = income.copy(id = "synced", fromAccountId = "system-income", fromCategoryId = "system-income")
        assertEquals(1, possibleDuplicates(income, listOf(synced)).size)
        for (row in listOf(synced.copy(toAccountId = "personal"), synced.copy(toCategoryId = "travel"), synced.copy(incomeTreatment = "salary")))
            assertTrue(possibleDuplicates(income, listOf(row)).isEmpty())
        val move = entry.copy(kind = "reallocate", fromAccountId = null, toCategoryId = "travel")
        assertEquals(1, possibleDuplicates(move, listOf(move.copy(id = "synced", fromAccountId = "system", toAccountId = "system"))).size)
        assertTrue(possibleDuplicates(move, listOf(move.copy(id = "different", toCategoryId = "food"))).isEmpty())
    }

    @Test fun similarityRanksAndRetryRowsCollapse() {
        val similar = other.copy(id = "similar", description = "Cafe groceries")
        assertEquals(listOf("similar", "other"), possibleDuplicates(entry, listOf(other, similar, similar)).map { it.id })
    }

    @Test fun transfersCompareDestinationAmounts() {
        val transfer = entry.copy(kind = "transfer", fromCategoryId = null, toAccountId = "usd", toCurrency = "USD", toAmount = BigDecimal("14.00"))
        assertEquals(1, possibleDuplicates(transfer, listOf(transfer.copy(id = "other", toAmount = BigDecimal("14.03")))).size)
        assertTrue(possibleDuplicates(transfer, listOf(transfer.copy(id = "other", toAmount = BigDecimal("14.04")))).isEmpty())
    }
}
