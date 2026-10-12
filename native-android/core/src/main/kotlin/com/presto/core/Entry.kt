package com.presto.core

import java.math.BigDecimal
import java.text.Normalizer
import java.time.LocalDate

fun normalize(text: String) =
    Normalizer.normalize(text, Normalizer.Form.NFD)
        .replace(Regex("[\\u0300-\\u036f]"), "")
        .lowercase()
        .replace(Regex("[^a-z0-9]+"), " ")
        .trim()

fun mostUsedAccount(
    accounts: List<Account>,
    transactions: List<Transaction>,
    category: String? = null,
): Account? {
    val accountIds = accounts.mapTo(hashSetOf()) { it.id }
    val matching = if (category == null) emptyList() else transactions.filter {
        it.fromCategoryId == category || it.toCategoryId == category
    }
    val source = if (matching.any {
        it.fromAccountId in accountIds || it.toAccountId in accountIds
    }) matching else transactions
    val counts = mutableMapOf<String, Int>()
    val firstUse = mutableMapOf<String, Int>()
    source.forEachIndexed { index, transaction ->
        fun record(id: String?) {
            if (id == null || id !in accountIds) return
            counts[id] = (counts[id] ?: 0) + 1
            firstUse.putIfAbsent(id, index)
        }
        record(transaction.fromAccountId)
        if (transaction.toAccountId != transaction.fromAccountId) record(transaction.toAccountId)
    }
    return accounts.minWithOrNull(
        compareByDescending<Account> { counts[it.id] ?: 0 }
            .thenBy { firstUse[it.id] ?: Int.MAX_VALUE }
            .thenBy { it.name }
    )
}

/** Resolve automatic entry links off the UI thread, also immediately before saving. */
fun suggestDraftLinks(draft: Draft, snapshot: Snapshot): Draft {
    val category = if (!draft.manualCategory && draft.kind == "expense" &&
        (draft.original == null || draft.description != draft.original?.description))
        suggestCategory(draft.description, snapshot.budgets, snapshot.recentTransactions,
            draft.original?.id)?.id ?: draft.categoryFrom else draft.categoryFrom
    val from = if (!draft.manualFromAccount && draft.original == null)
        mostUsedAccount(snapshot.accounts, snapshot.recentTransactions,
            category.takeIf { draft.kind == "expense" }) else null
    val to = if (!draft.manualToAccount && draft.original == null)
        mostUsedAccount(snapshot.accounts, snapshot.recentTransactions,
            draft.categoryTo.takeIf { draft.kind == "income" }) else null
    return draft.copy(
        categoryFrom = category,
        accountFrom = from?.id ?: draft.accountFrom,
        accountTo = to?.id ?: draft.accountTo,
        currency = when (draft.kind) {
            "expense", "transfer" -> from?.currency ?: draft.currency
            "income" -> to?.currency ?: draft.currency
            else -> draft.currency
        },
    )
}

private val stopWords =
    setOf("a", "an", "and", "at", "aug", "for", "from", "in", "of", "on", "the", "to")

private fun words(text: String) =
    normalize(text).split(' ').filter { it.length > 1 && it !in stopWords }.toSet()

private val hints =
    listOf(
        listOf("cleaner", "cleaning", "housekeeping") to
            listOf("clean", "home", "house", "household"),
        listOf("grocery", "groceries", "market", "supermarket") to
            listOf("food", "grocer", "market"),
        listOf("cafe", "coffee", "dinner", "lunch", "restaurant") to
            listOf("dining", "eating", "restaurant", "food"),
        listOf("bus", "metro", "parking", "taxi", "train", "uber") to
            listOf("car", "transport", "travel"),
        listOf("doctor", "dentist", "medicine", "pharmacy") to listOf("health", "medical"),
        listOf("electric", "internet", "phone", "utility", "water") to
            listOf("bill", "home", "house", "utility"),
        listOf("flight", "hotel", "holiday", "trip") to listOf("holiday", "travel", "vacation"),
        listOf("salary", "payroll") to listOf("buffer", "income", "saving"),
    )

data class CategorySuggestion(val id: String, val reason: String)

fun suggestCategory(
    description: String,
    categories: List<Category>,
    transactions: List<Transaction>,
    excluded: String? = null,
): CategorySuggestion? {
    val key = normalize(description)
    val tokens = words(description)
    if (key.isBlank() || tokens.isEmpty()) return null
    val scores = linkedMapOf<String, Pair<Double, String>>()
    fun add(id: String, score: Double, reason: String) {
        if (score > (scores[id]?.first ?: 0.0)) scores[id] = score to reason
    }
    categories
        .filter { !it.system }
        .forEach { c ->
            if (words(c.name).any { it in tokens })
                add(c.categoryId, 65.0, "description matches the category name")
            hints.forEach { (keywords, terms) ->
                if (
                    keywords.any { key.contains(it) } &&
                        terms.any { normalize(c.name).contains(it) }
                )
                    add(c.categoryId, 75.0, "description keyword")
            }
        }
    transactions
        .filter { it.kind == "expense" && it.id != excluded && it.fromCategoryId != null }
        .forEach { t ->
            val old = normalize(t.description)
            val other = words(t.description)
            val overlap = tokens.intersect(other).size
            val union = (tokens + other).size
            if (old == key) add(t.fromCategoryId!!, 120.0, "same description used before")
            else if (overlap >= 2 || (overlap == 1 && union <= 3))
                add(
                    t.fromCategoryId!!,
                    70.0 + overlap.toDouble() / union * 30,
                    "similar description used before",
                )
        }
    return scores
        .maxByOrNull { it.value.first }
        ?.takeIf { (id, v) -> v.first >= 60 && categories.any { it.categoryId == id } }
        ?.let { CategorySuggestion(it.key, it.value.second) }
}

fun recentDescriptions(
    transactions: List<Transaction>,
    now: LocalDate = LocalDate.now(),
): List<String> {
    val recent =
        transactions.filter {
            it.description.isNotBlank() && it.effectiveDate >= now.minusMonths(3).toString()
        }
    val source = recent.ifEmpty { transactions.filter { it.description.isNotBlank() } }
    return source
        .withIndex()
        .groupBy { normalize(it.value.description) }
        .filterKeys { it.isNotBlank() }
        .values
        .sortedWith(
            compareByDescending<List<IndexedValue<Transaction>>> { rows ->
                    rows.maxOf { it.value.effectiveDate }
                }
                .thenByDescending { it.size }
                .thenBy { rows -> rows.minOf { it.index } }
                .thenBy { it.first().value.description }
        )
        .take(8)
        .map { it.first().value.description.trim() }
}

fun fuzzyScore(label: String, query: String): Double? {
    val candidate =
        Normalizer.normalize(label, Normalizer.Form.NFD)
            .replace(Regex("[\\u0300-\\u036f]"), "")
            .lowercase()
            .trim()
    val search =
        Normalizer.normalize(query, Normalizer.Form.NFD)
            .replace(Regex("[\\u0300-\\u036f]"), "")
            .lowercase()
            .trim()
    if (search.isBlank()) return 0.0
    if (candidate == search) return 120.0
    if (candidate.startsWith(search)) return 100 - candidate.length * .01
    val at = candidate.indexOf(search)
    if (at >= 0) return 80.0 - at
    val initials =
        candidate
            .split(Regex("[^a-z0-9]+"))
            .filter { it.isNotEmpty() }
            .joinToString("") { it.take(1) }
    if (initials.startsWith(search)) return 70.0
    var next = 0
    var last = -1
    var gap = 0
    candidate.forEachIndexed { index, c ->
        if (next < search.length && c == search[next]) {
            if (last >= 0) gap += index - last - 1
            last = index
            next++
        }
    }
    return if (next == search.length) 50.0 - gap else null
}

data class Entity(val kind: String, val id: String, val name: String)

fun entities(s: Snapshot) =
    s.accounts.map { Entity("account", it.id, it.name) } +
        s.budgets.map { Entity("category", it.categoryId, it.name) }

fun matchesSearch(
    t: Transaction,
    query: String,
    entities: List<Entity>,
    selected: Entity? = null,
): Boolean {
    fun searchKey(s: String) = s.trim().lowercase().replace(Regex("\\s+"), " ")
    fun linked(e: Entity) =
        if (e.kind == "account") e.id == t.fromAccountId || e.id == t.toAccountId
        else e.id == t.fromCategoryId || e.id == t.toCategoryId
    val search = searchKey(query)
    val exact = selected?.let { listOf(it) } ?: entities.filter { searchKey(it.name) == search }
    if (exact.isNotEmpty()) return exact.any(::linked)
    return (listOf(t.description) + entities.filter(::linked).map { it.name }).any {
        searchKey(it).contains(search)
    }
}

data class BalanceSuggestion(
    val source: Category?,
    val destination: Category?,
    val amount: BigDecimal,
)

fun suggestBalance(
    categories: List<Category>,
    from: String? = null,
    to: String? = null,
): BalanceSuggestion {
    val budgets = categories.filter { !it.system }
    val destination =
        budgets.firstOrNull { it.categoryId == to && (it.remainingEur ?: ZERO) < ZERO }
            ?: budgets
                .filter { (it.remainingEur ?: ZERO) < ZERO }
                .minByOrNull { it.remainingEur ?: ZERO }
    val source =
        budgets.firstOrNull {
            it.categoryId == from &&
                it.categoryId != destination?.categoryId &&
                (it.remainingEur ?: ZERO) > ZERO
        }
            ?: budgets
                .filter {
                    it.categoryId != destination?.categoryId && (it.remainingEur ?: ZERO) > ZERO
                }
                .maxByOrNull { it.remainingEur ?: ZERO }
    return BalanceSuggestion(
        source,
        destination,
        if (source != null && destination != null)
            source.remainingEur!!.min(-destination.remainingEur!!)
        else ZERO,
    )
}

/** Bookkeeping endpoints returned by the API are not real bank/card accounts. */
fun realAccountIds(t: Transaction): Set<String> = when (t.kind) {
    "expense" -> listOfNotNull(t.fromAccountId).toSet()
    "income" -> listOfNotNull(t.toAccountId).toSet()
    "transfer" -> listOfNotNull(t.fromAccountId, t.toAccountId).toSet()
    else -> emptySet()
}

fun writeEntry(d: Draft, snapshot: Snapshot): Pair<WritePayload, Transaction> {
    val amount = parseMoney(d.amount)
    require(amount != null && amount > ZERO) {
        "Enter a positive amount with at most two decimal places."
    }
    require(d.description.isNotBlank()) { "Add a description. Categories are selected separately." }
    val description = listOf(d.description.trim(), d.note.trim())
        .filter { it.isNotEmpty() }.joinToString("\n")
    require(description.codePointCount(0, description.length) <= 500) {
        "Keep the description and note within 500 characters."
    }
    val date =
        runCatching { LocalDate.parse(d.date) }
            .getOrElse { error("Choose a valid date.") }
            .toString()
    val fromCategory = snapshot.budgets.find { it.categoryId == d.categoryFrom }
    val toCategory =
        if (d.kind == "income" && d.incomeTreatment in listOf("salary", "unexpected"))
            snapshot.budgets.find { it.categoryId == "budget-unallocated" }
        else snapshot.budgets.find { it.categoryId == d.categoryTo }
    val from = snapshot.accounts.find { it.id == d.accountFrom }
    val to = snapshot.accounts.find { it.id == d.accountTo }
    when (d.kind) {
        "expense" ->
            require(fromCategory != null && from != null) {
                "Choose a budget category and account."
            }
        "income" ->
            require(
                to != null &&
                    (toCategory != null || d.incomeTreatment in listOf("salary", "unexpected"))
            ) {
                "Choose a budget category and account."
            }
        "transfer" ->
            require(from != null && to != null && from.id != to.id) {
                "Choose two different accounts for a transfer."
            }
        "reallocate" -> {
            require(
                fromCategory != null &&
                    toCategory != null &&
                    fromCategory.categoryId != toCategory.categoryId
            ) {
                "Choose two different budget categories."
            }
            require(date == monthEnd(date)) { "Budget moves must be dated on month-end." }
            if (d.balancing) {
                require(
                    (fromCategory.remainingEur ?: ZERO) > ZERO &&
                        (toCategory.remainingEur ?: ZERO) < ZERO
                ) {
                    "Choose a positive donor and negative category."
                }
                require(amount <= fromCategory.remainingEur) {
                    "The donor does not have enough budget left."
                }
            }
        }
        else -> error("Unsupported transaction type.")
    }
    val crossCurrency = d.kind == "transfer" && from?.currency != to?.currency
    val toAmount = if (crossCurrency) parseMoney(d.toAmount) else null
    require(!crossCurrency || (toAmount != null && toAmount > ZERO)) {
        "Enter the amount received in ${to?.currency}."
    }
    require(
        d.original == null ||
            (d.original.canEdit && !d.original.pending && d.original.kind != "reconciliation")
    ) {
        "This transaction is read only or has a pending write."
    }
    require(
        d.correctionOf.isBlank() ||
            (d.kind in listOf("expense", "income") && date.take(7) == today().take(7))
    ) {
        "Corrections must be expense or income entries in the current month."
    }
    val expectedMonth =
        if (d.kind == "income" && d.incomeTreatment == "salary")
            d.expectedIncomeMonth.ifBlank { date.take(7) }
        else null
    require(
        expectedMonth == null || runCatching { java.time.YearMonth.parse(expectedMonth) }.isSuccess
    ) {
        "Enter the expected salary month as YYYY-MM."
    }
    val id = d.original?.id ?: d.queuedTransactionId ?: uuidV7()
    val captured = captureTimestamp(date, d.original)
    val fromAccountId = from?.id.takeIf { d.kind in listOf("expense", "transfer") }
    val toAccountId = to?.id.takeIf { d.kind in listOf("income", "transfer") }
    val fromCategoryId =
        fromCategory?.categoryId.takeIf { d.kind in listOf("expense", "reallocate") }
    val toCategoryId = toCategory?.categoryId.takeIf { d.kind in listOf("income", "reallocate") }
    val revisions =
        if (d.original != null)
            (realAccountIds(d.original) + listOfNotNull(fromAccountId, toAccountId))
                .map { accountId ->
                    AccountRevision(
                        accountId,
                        snapshot.accounts.find { it.id == accountId }?.ledgerRevision
                            ?: error("An affected account is unavailable. Refresh before editing."),
                    )
                }
        else null
    val payload =
        WritePayload(
            uuidV7(),
            id,
            d.kind,
            date,
            description,
            amount,
            captured,
            captured == null,
            d.currency.takeUnless { d.kind == "reallocate" },
            toAmount,
            fromCategoryId,
            toCategoryId,
            fromAccountId,
            toAccountId,
            d.incomeTreatment.takeIf { d.kind == "income" && it != "legacy" },
            expectedMonth,
            d.correctionOf.takeIf { it.isNotBlank() },
            d.original?.revision,
            revisions,
        )
    val transaction =
        Transaction(
            id,
            d.kind,
            date,
            payload.description,
            amount,
            if (d.kind == "reallocate") "EUR" else d.currency,
            captured,
            toAmount,
            to?.currency,
            fromCategory?.name.takeIf { fromCategoryId != null },
            toCategory?.name.takeIf { toCategoryId != null },
            fromCategoryId,
            toCategoryId,
            from?.name.takeIf { fromAccountId != null },
            to?.name.takeIf { toAccountId != null },
            fromAccountId,
            toAccountId,
            revision = d.original?.revision ?: 1,
            canEdit = true,
            correctsTransactionId = payload.correctsTransactionId,
            incomeTreatment = d.incomeTreatment.takeIf { d.kind == "income" },
            expectedIncomeMonth = expectedMonth,
            pending = true,
        )
    return payload to transaction
}

fun budgetOverlay(
    categories: List<Category>,
    t: Transaction,
    direction: Int = 1,
    month: String = today().take(7),
): List<Category> {
    // Historical writes can change rollover and frozen periods. Let the server recalculate them.
    if (
        t.inputCurrency != "EUR" ||
            t.correctsTransactionId != null ||
            t.effectiveDate.take(7) != month
    )
        return categories
    val amount = t.amount * BigDecimal(direction)
    return categories.map { c ->
        val delta =
            when {
                t.kind == "expense" && c.categoryId == t.fromCategoryId -> -amount
                t.kind == "income" &&
                    t.incomeTreatment !in listOf("salary", "unexpected") &&
                    c.categoryId == t.toCategoryId -> amount
                t.kind == "reallocate" && c.categoryId == t.fromCategoryId -> -amount
                t.kind == "reallocate" && c.categoryId == t.toCategoryId -> amount
                else -> ZERO
            }
        c.copy(
            remainingEur = c.remainingEur?.plus(delta),
            expenseEur = if (t.kind == "expense") c.expenseEur?.minus(delta) else c.expenseEur,
        )
    }
}

fun preview(d: Draft, s: Snapshot): List<String> {
    val amount = parseAmount(d.amount)?.takeIf { it > ZERO } ?: return emptyList()
    val value = money(amount, d.currency)
    val budgetValue = if (d.currency == "EUR") value else "the EUR value of $value"
    val from = s.accounts.find { it.id == d.accountFrom }
    val to = s.accounts.find { it.id == d.accountTo }
    val fromCategory = s.budgets.find { it.categoryId == d.categoryFrom }?.name
    val toCategory = s.budgets.find { it.categoryId == d.categoryTo }?.name
    fun accountValue(a: Account) =
        if (a.currency == d.currency) value else "$value converted to ${a.currency}"
    return buildList {
        if (d.original != null)
            add(
                "This replaces the original entry. Effects describe the replacement, not the difference."
            )
        if (d.kind in listOf("expense", "transfer") && from != null)
            add("${from.name} decreases by ${accountValue(from)}.")
        if (d.kind == "income" && to != null) add("${to.name} increases by ${accountValue(to)}.")
        when (d.kind) {
            "transfer" -> {
                if (to != null)
                    add(
                        if (from?.currency != to.currency)
                            "${to.name} receives ${money(parseAmount(d.toAmount), to.currency ?: "EUR")}."
                        else "${to.name} increases by ${accountValue(to)}."
                    )
                add("Category budgets stay unchanged.")
            }
            "reallocate" -> {
                add("$fromCategory decreases by ${money(amount)}.")
                add("$toCategory increases by ${money(amount)}.")
                add("Account balances stay unchanged.")
            }
            else ->
                if (d.correctionOf.isNotBlank())
                    add("Linked to an earlier entry; budget effects follow its correction rules.")
                else if (d.kind == "expense")
                    add("$fromCategory remaining budget decreases by $budgetValue.")
                else
                    when (d.incomeTreatment) {
                        "salary" -> {
                            add("Category allocations stay unchanged.")
                            add(
                                "Settles the selected month's expected salary; any excess goes to Unallocated."
                            )
                        }
                        "unexpected" ->
                            add(
                                "Unallocated increases by $budgetValue; category allocations stay unchanged."
                            )
                        else -> add("$toCategory remaining budget increases by $budgetValue.")
                    }
        }
        if (d.currency != "EUR" && d.kind != "reallocate")
            add("Conversions use the rate for the transaction date when saved.")
    }
}

fun editDraft(t: Transaction, snapshot: Snapshot): Draft {
    val defaults = defaultDraft(snapshot)
    return Draft(
        kind = t.kind,
        amount = t.amount.toPlainString(),
        currency = t.inputCurrency ?: "EUR",
        description = t.description,
        date = t.effectiveDate,
        toAmount = t.toAmount?.toPlainString() ?: "",
        categoryFrom = t.fromCategoryId ?: defaults.categoryFrom,
        categoryTo = t.toCategoryId ?: defaults.categoryTo,
        accountFrom = t.fromAccountId ?: defaults.accountFrom,
        accountTo = t.toAccountId ?: defaults.accountTo,
        correctionOf = t.correctsTransactionId ?: "",
        incomeTreatment = t.incomeTreatment ?: "legacy",
        expectedIncomeMonth = t.expectedIncomeMonth ?: "",
        original = t,
        showMore = t.toAmount != null || t.correctsTransactionId != null,
        manualCategory = t.fromCategoryId != null,
        manualFromAccount = t.fromAccountId != null,
        manualToAccount = t.toAccountId != null,
    )
}
