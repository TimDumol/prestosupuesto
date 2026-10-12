package com.presto.core

import java.math.BigDecimal
import java.time.Instant
import java.time.LocalDate
import java.util.UUID
import kotlin.test.*

class CoreTest {
    @Test
    fun selectionOnlyDrafts() {
        val snapshot = demoSnapshot()
        val draft = defaultDraft(snapshot)
        assertFalse(draft.touched)
        assertTrue(draft.copy(userTouched = true, date = "2020-01-01").touched)
        assertTrue(draft.copy(note = "Reference").touched)
    }

    @Test
    fun localizedAndFormulaAmounts() {
        val cases =
            mapOf(
                "1.234,56" to "1234.56",
                "1,234.56" to "1234.56",
                "1 234,56" to "1234.56",
                "₱ 20.50" to "20.50",
                "=2*(3+4)-1" to "13.00",
                "=0.1+0.2" to "0.30",
                "=12÷4" to "3.00",
                "=1/3*3" to "1.00",
            )
        cases.forEach { (input, expected) ->
            assertEquals(BigDecimal(expected), parseMoney(input), input)
        }
        listOf("1,23,4", "1,2.34", "1.234", "=1/0", "=1/3", "=1+", "=foo", "NaN").forEach {
            assertNull(parseMoney(it), it)
        }
        assertEquals(BigDecimal("1.234"), parseAmount("1.234"))
    }

    @Test
    fun uuidAndDateSemantics() {
        repeat(100) {
            val uuid = UUID.fromString(uuidV7())
            assertEquals(7, uuid.version())
            assertEquals(2, uuid.variant())
        }
        assertEquals("2024-02-29", monthEnd("2024-02-01"))
        assertEquals("2026-02-28", monthEnd("2026-02-01"))
        assertNull(captureTimestamp("2000-01-01", now = Instant.now()))
        val original =
            Transaction(
                "id",
                "expense",
                "2026-02-02",
                "Test",
                BigDecimal.ONE,
                occurredAt = "2026-02-02T10:00:00Z",
            )
        assertEquals(original.occurredAt, captureTimestamp(original.effectiveDate, original))
    }

    @Test
    fun categorySuggestions() {
        val categories = listOf(Category("home", "Household"), Category("food", "Food & groceries"))
        assertEquals("home", suggestCategory("Cleaner for Aug 30", categories, emptyList())?.id)
        val t =
            Transaction(
                "old",
                "expense",
                today(),
                "Weekend provisions",
                BigDecimal.TEN,
                fromCategoryId = "food",
            )
        assertEquals("food", suggestCategory(t.description, categories, listOf(t))?.id)
        assertNull(suggestCategory(t.description, categories, listOf(t), "old"))
    }

    @Test
    fun accountRankingAndDescriptionHistory() {
        val accounts = listOf(Account("a", "A"), Account("b", "B"))
        fun t(
            id: String,
            account: String,
            cat: String,
            date: String = "2026-08-20",
            description: String = "Weekly groceries",
        ) =
            Transaction(
                id,
                "expense",
                date,
                description,
                BigDecimal.ONE,
                fromAccountId = account,
                fromCategoryId = cat,
            )
        val rows =
            listOf(
                t("1", "a", "food"),
                t("2", "b", "food"),
                t("3", "b", "food"),
                t("4", "a", "home"),
            )
        assertEquals("b", mostUsedAccount(accounts, rows, "food")?.id)
        assertEquals("a", mostUsedAccount(accounts, rows, "travel")?.id)
        assertEquals(
            listOf("Weekly groceries"),
            recentDescriptions(
                listOf(
                    t("1", "a", "food"),
                    t("2", "a", "food", "2026-07-20", "weekly groceries"),
                    t("3", "a", "food", "2026-01-20", "Old rent"),
                ),
                LocalDate.of(2026, 8, 28),
            ),
        )
        assertEquals(
            listOf("Old rent"),
            recentDescriptions(
                listOf(t("3", "a", "food", "2026-01-20", "Old rent")),
                LocalDate.of(2026, 8, 28),
            ),
        )
        assertTrue(fuzzyScore("Food & groceries", "fg")!! > 0)
        assertNull(fuzzyScore("Food", "xyz"))
    }

    @Test
    fun accountRankingHandlesTransfersAndMissingCategoryAccounts() {
        val accounts = listOf(Account("a", "Alpha"), Account("b", "Beta"))
        fun row(id: String, from: String, to: String? = null, category: String? = null) =
            Transaction(id, "transfer", today(), "Transfer", BigDecimal.ONE,
                fromAccountId = from, toAccountId = to, fromCategoryId = category)
        // A transfer uses both accounts once, even if both links name the same account.
        assertEquals("b", mostUsedAccount(accounts,
            listOf(row("1", "b", "b"), row("2", "a", "b"), row("3", "a")))?.id)
        // History linked only to removed accounts must fall back to general usage.
        assertEquals("b", mostUsedAccount(accounts,
            listOf(row("1", "removed", category = "food"), row("2", "b")), "food")?.id)
        assertEquals("a", mostUsedAccount(accounts, emptyList())?.id)
        assertNull(mostUsedAccount(emptyList(), emptyList()))
    }

    @Test
    fun draftSuggestionsPreserveExplicitChoicesAndEntryValues() {
        val snapshot = demoSnapshot()
        val draft = defaultDraft(snapshot).copy(
            description = "Cleaner", amount = "12.50", date = "2020-01-15",
            note = "Keep this", manualCategory = true, manualFromAccount = true,
            manualToAccount = true, currency = "PHP",
        )
        assertEquals(draft, suggestDraftLinks(draft, snapshot))
        val automatic = draft.copy(manualCategory = false, manualFromAccount = false)
        val resolved = suggestDraftLinks(automatic, snapshot)
        assertEquals(suggestCategory("Cleaner", snapshot.budgets, snapshot.recentTransactions)?.id,
            resolved.categoryFrom)
        assertEquals(draft.amount, resolved.amount)
        assertEquals(draft.date, resolved.date)
        assertEquals(draft.note, resolved.note)
        val original = snapshot.recentTransactions.first { it.kind == "expense" }
        val editing = editDraft(original, snapshot)
        assertEquals(editing, suggestDraftLinks(editing, snapshot))
    }

    @Test
    fun entitySearchIsExactWhenNamed() {
        val entities = listOf(Entity("account", "cash", "Cash"), Entity("category", "food", "Food"))
        val t =
            Transaction(
                "id",
                "expense",
                today(),
                "Cashback",
                BigDecimal.ONE,
                fromAccountId = "bank",
                fromCategoryId = "food",
            )
        assertFalse(matchesSearch(t, "Cash", entities))
        assertTrue(matchesSearch(t, "cashb", entities))
        assertTrue(matchesSearch(t, "Food", entities))
        assertFalse(matchesSearch(t, "Food", entities, entities.first()))
    }

    @Test
    fun multiDonorBalancingAndSalaryPreview() {
        val categories =
            listOf(
                Category("a", "A", remainingEur = BigDecimal(20)),
                Category("b", "B", remainingEur = BigDecimal(100)),
                Category("c", "C", remainingEur = BigDecimal(-150)),
            )
        val suggestion = suggestBalance(categories)
        assertEquals("b", suggestion.source?.categoryId)
        assertEquals("c", suggestion.destination?.categoryId)
        assertEquals(BigDecimal(100), suggestion.amount)
        val next =
            budgetOverlay(
                categories,
                Transaction(
                    "id",
                    "reallocate",
                    today(),
                    "Balance",
                    BigDecimal(100),
                    fromCategoryId = "b",
                    toCategoryId = "c",
                ),
            )
        assertEquals(BigDecimal(20), suggestBalance(next, to = "c").amount)
        val salary =
            Transaction(
                "id",
                "income",
                today(),
                "Salary",
                BigDecimal(3000),
                incomeTreatment = "salary",
                toCategoryId = "a",
            )
        assertEquals(categories, budgetOverlay(categories, salary))
        assertTrue(
            preview(
                    Draft(kind = "income", amount = "3000", incomeTreatment = "salary"),
                    demoSnapshot(),
                )
                .any { it.contains("Category allocations stay unchanged") }
        )
    }

    @Test
    fun writesAreDecimalAndRevisionGuarded() {
        val snapshot = demoSnapshot()
        val d = defaultDraft(snapshot).copy(amount = "=10+2.50", description = "Test expense")
        val (payload, t) = writeEntry(d, snapshot)
        assertEquals(BigDecimal("12.50"), payload.amount)
        val json = codec.encodeToString(WritePayload.serializer(), payload)
        assertTrue(json.contains("\"amount\":\"12.50\""))
        assertFalse(json.contains("expectedRevision"))
        assertEquals(payload.transactionId, t.id)
        val editing = d.copy(original = t.copy(pending = false, revision = 3))
        val (edit, _) = writeEntry(editing, snapshot)
        assertEquals(3, edit.expectedRevision)
        assertEquals(t.id, edit.transactionId)
        assertNotEquals(payload.operationId, edit.operationId)
        assertTrue(edit.expectedAccountRevisions!!.isNotEmpty())
        assertFailsWith<IllegalArgumentException> {
            writeEntry(d.copy(kind = "transfer", accountTo = d.accountFrom), snapshot)
        }
        assertFailsWith<IllegalArgumentException> {
            writeEntry(d.copy(kind = "transfer", accountTo = "H GoTyme", toAmount = ""), snapshot)
        }
        assertFailsWith<IllegalArgumentException> { writeEntry(d.copy(original = t), snapshot) }
        assertFailsWith<IllegalArgumentException> {
            writeEntry(d.copy(kind = "reallocate", date = "2026-08-15"), snapshot)
        }
        assertFailsWith<IllegalArgumentException> {
            writeEntry(d.copy(kind = "income", expectedIncomeMonth = "bad"), snapshot)
        }
    }

    @Test
    fun historicalAndFutureWritesWaitForAuthoritativeBudgetRecalculation() {
        val categories =
            listOf(
                Category("food", "Food", expenseEur = BigDecimal(30), remainingEur = BigDecimal(70))
            )
        val transaction =
            Transaction(
                "id",
                "expense",
                "2026-09-30",
                "Backdated",
                BigDecimal(10),
                fromCategoryId = "food",
            )
        assertEquals(categories, budgetOverlay(categories, transaction, month = "2026-10"))
        assertEquals(
            categories,
            budgetOverlay(
                categories,
                transaction.copy(effectiveDate = "2026-11-01"),
                month = "2026-10",
            ),
        )
        val current =
            budgetOverlay(
                    categories,
                    transaction.copy(effectiveDate = "2026-10-03"),
                    month = "2026-10",
                )
                .single()
        assertEquals(BigDecimal(40), current.expenseEur)
        assertEquals(BigDecimal(60), current.remainingEur)
    }

    @Test
    fun refundAndReallocationOverlaysDoNotInventSpending() {
        val category =
            Category("food", "Food", expenseEur = BigDecimal(30), remainingEur = BigDecimal(70))
        val refund =
            Transaction(
                "id",
                "income",
                today(),
                "Refund",
                BigDecimal(10),
                incomeTreatment = "refund",
                toCategoryId = "food",
            )
        val result = budgetOverlay(listOf(category), refund).single()
        assertEquals(BigDecimal(30), result.expenseEur)
        assertEquals(BigDecimal(80), result.remainingEur)
        val historical =
            Transaction(
                "id",
                "expense",
                today(),
                "PHP 2,287.43 / th gotyme / cleaner",
                BigDecimal.ONE,
                canEdit = true,
            )
        val draft = editDraft(historical, demoSnapshot())
        assertEquals(historical.description, draft.description)
        assertEquals(defaultDraft(demoSnapshot()).accountFrom, draft.accountFrom)
        assertEquals("1", draft.amount)
    }

    @Test
    fun notesAreSavedAndCombinedDescriptionLengthIsValidated() {
        val snapshot = demoSnapshot()
        val draft = defaultDraft(snapshot).copy(amount = "10", description = "Purchase", note = "Receipt reference")
        val (payload, transaction) = writeEntry(draft, snapshot)
        assertEquals("Purchase\nReceipt reference", payload.description)
        assertEquals(payload.description, transaction.description)
        val reopened = editDraft(transaction.copy(pending = false), snapshot)
        assertEquals(payload.description, writeEntry(reopened, snapshot).first.description)
        assertFailsWith<IllegalArgumentException> {
            writeEntry(draft.copy(note = "x".repeat(500)), snapshot)
        }
        assertEquals(250, writeEntry(draft.copy(description = "😀".repeat(250), note = ""), snapshot)
            .first.description.codePointCount(0, 500))
    }

    @Test
    fun editingStructuredTransactionsNeverReinterpretsTheirDescription() {
        val snapshot = demoSnapshot()
        for (kind in listOf("expense", "income", "transfer", "reallocate")) {
            val transaction = snapshot.recentTransactions.first().copy(
                kind = kind, description = "PHP 2,287.43 / th gotyme / cleaner",
                amount = BigDecimal("15.25"), inputCurrency = "EUR",
            )
            val draft = editDraft(transaction, snapshot)
            assertEquals(kind, draft.kind)
            assertEquals(transaction.description, draft.description)
            assertEquals("15.25", draft.amount)
            assertEquals("EUR", draft.currency)
            assertEquals(transaction.fromAccountId, draft.accountFrom)
        }
    }

    @Test
    fun nullableValuesAndRoundTrips() {
        val snapshot = demoSnapshot()
        val json = codec.encodeToString(Snapshot.serializer(), snapshot)
        assertEquals(snapshot, codec.decodeFromString<Snapshot>(json))
        assertEquals("Not available", money(null))
        assertEquals("€0", money(ZERO))
        assertEquals("−€12.5", money(BigDecimal("-12.50")))
    }
    @Test
    fun editsGuardOnlyRealAccountsIncludingBothSidesOfAccountChanges() {
        val snapshot = demoSnapshot().copy(accounts = demoSnapshot().accounts.mapIndexed { i, a ->
            a.copy(ledgerRevision = 10L + i)
        })
        val expense = snapshot.recentTransactions.first().copy(
            toAccountId = "bookkeeping-expense", toCategoryId = "system-expense",
        )
        val expenseDraft = editDraft(expense, snapshot).copy(accountFrom = "Checking")
        val expenseRevisions = writeEntry(expenseDraft, snapshot).first.expectedAccountRevisions!!
        assertEquals(setOf("Cash", "Checking"), expenseRevisions.map { it.accountId }.toSet())
        assertEquals(12L, expenseRevisions.first { it.accountId == "Cash" }.revision)
        val income = expense.copy(kind = "income", fromAccountId = "bookkeeping-income",
            toAccountId = "Checking", incomeTreatment = "unexpected")
        val incomeDraft = editDraft(income, snapshot).copy(accountTo = "Visa")
        assertEquals(setOf("Checking", "Visa"), writeEntry(incomeDraft, snapshot).first
            .expectedAccountRevisions!!.map { it.accountId }.toSet())
        val transfer = expense.copy(kind = "transfer", fromAccountId = "Cash", toAccountId = "Visa")
        assertEquals(setOf("Cash", "Visa", "Checking"), writeEntry(
            editDraft(transfer, snapshot).copy(accountTo = "Checking"), snapshot,
        ).first.expectedAccountRevisions!!.map { it.accountId }.toSet())
        assertFailsWith<IllegalStateException> {
            writeEntry(expenseDraft, snapshot.copy(accounts = snapshot.accounts.filter { it.id != "Cash" }))
        }
    }

}
