package com.presto.core

/** Search all available history, rather than filtering an already truncated recent list. */
data class DescriptionSuggestion(val transaction: Transaction, val score: Double)

fun descriptionSuggestions(
    query: String,
    transactions: List<Transaction>,
    kind: String,
    excluded: String? = null,
): List<DescriptionSuggestion> {
    val key = normalize(query)
    return transactions
        .filter { it.kind == kind && it.id != excluded && normalize(it.description).isNotBlank() }
        .groupBy { normalize(it.description) }
        .mapNotNull { (description, matches) ->
            val score = descriptionMatch(description, key) ?: return@mapNotNull null
            val latest =
                matches.maxWith(
                    compareBy<Transaction> { it.effectiveDate }
                        .thenBy { it.occurredAt ?: "" }
                        .thenBy { it.id }
                )
            DescriptionSuggestion(latest, score) to matches.size
        }
        .sortedWith(
            compareByDescending<Pair<DescriptionSuggestion, Int>> { it.first.score }
                .thenByDescending { it.second }
                .thenByDescending { it.first.transaction.effectiveDate }
                .thenBy { it.first.transaction.description }
        )
        .take(5)
        .map { it.first }
}

private fun descriptionMatch(candidate: String, query: String): Double? {
    if (query.isBlank()) return 0.0
    val direct = fuzzyScore(candidate, query)
    val tokens = query.split(' ').filter { it.isNotBlank() }
    val words = candidate.split(' ')
    val tokenScores =
        tokens.map { token ->
            words
                .mapNotNull { word ->
                    when {
                        word == token -> 100.0
                        word.startsWith(token) -> 90.0
                        token.length >= 3 &&
                            editDistance(word, token) <= (if (token.length >= 6) 2 else 1) -> 75.0
                        else -> null
                    }
                }
                .maxOrNull()
        }
    val reordered =
        if (tokenScores.all { it != null }) tokenScores.filterNotNull().average() else null
    return listOfNotNull(direct, reordered).maxOrNull()?.takeIf { it >= 25 }
}

private fun editDistance(a: String, b: String): Int {
    var previous = IntArray(b.length + 1) { it }
    var beforePrevious = previous
    for (i in a.indices) {
        val current = IntArray(b.length + 1)
        current[0] = i + 1
        for (j in b.indices) {
            current[j + 1] =
                minOf(current[j] + 1, previous[j + 1] + 1, previous[j] + if (a[i] == b[j]) 0 else 1)
            if (i > 0 && j > 0 && a[i] == b[j - 1] && a[i - 1] == b[j])
                current[j + 1] = minOf(current[j + 1], beforePrevious[j - 1] + 1)
        }
        beforePrevious = previous
        previous = current
    }
    return previous[b.length]
}

/** Choosing a suggestion explicitly fills its valid links; never reuses amounts or dates. */
fun applyDescriptionSuggestion(
    draft: Draft,
    snapshot: Snapshot,
    suggestion: DescriptionSuggestion,
): Draft {
    val t = suggestion.transaction
    if (t.kind != draft.kind) return draft
    val from = snapshot.accounts.find { it.id == t.fromAccountId }
    val to = snapshot.accounts.find { it.id == t.toAccountId }
    fun category(id: String?) =
        snapshot.budgets.find { it.categoryId == id && !it.system }?.categoryId
    val fromCategory = category(t.fromCategoryId)
    val toCategory = category(t.toCategoryId)
    val useFrom = draft.kind in listOf("expense", "transfer") && from != null
    val useTo = draft.kind in listOf("income", "transfer") && to != null
    val useCategoryFrom = draft.kind in listOf("expense", "reallocate") && fromCategory != null
    val treatment = if (draft.kind == "income")
        t.incomeTreatment?.takeIf { it in listOf("salary", "unexpected", "refund", "legacy") }
            ?: if (toCategory != null) "legacy" else draft.incomeTreatment
    else draft.incomeTreatment
    val useCategoryTo =
        (draft.kind == "reallocate" ||
            (draft.kind == "income" && treatment in listOf("refund", "legacy"))) &&
            toCategory != null
    return draft.copy(
        description = t.description.trim(),
        incomeTreatment = treatment,
        toAmount = if (draft.kind == "transfer" &&
            (useTo && to!!.id != draft.accountTo || useFrom && from!!.id != draft.accountFrom))
            "" else draft.toAmount,
        accountFrom = if (useFrom) from!!.id else draft.accountFrom,
        accountTo = if (useTo) to!!.id else draft.accountTo,
        categoryFrom = if (useCategoryFrom) fromCategory else draft.categoryFrom,
        categoryTo = if (useCategoryTo) toCategory else draft.categoryTo,
        currency =
            if (useFrom) from!!.currency ?: draft.currency
            else if (useTo && draft.kind == "income") to!!.currency ?: draft.currency
            else draft.currency,
        manualFromAccount = draft.manualFromAccount || useFrom,
        manualToAccount = draft.manualToAccount || useTo,
        manualCategory = draft.manualCategory || useCategoryFrom || useCategoryTo,
    )
}
