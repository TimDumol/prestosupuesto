package com.presto.core

import java.math.BigDecimal

/** Advisory matching uses posting dates and directional links, including pending entries. */
fun possibleDuplicates(entry: Transaction, history: List<Transaction>): List<Transaction> {
    fun words(text: String) = normalize(text).split(Regex("[^\\p{L}\\p{N}]+" )).filter { it.isNotBlank() }.toSet()
    val description = words(entry.description)
    fun similarity(row: Transaction): Double {
        val other = words(row.description)
        return if (description.isEmpty() || other.isEmpty()) 0.0
        else description.intersect(other).size.toDouble() / maxOf(description.size, other.size)
    }
    return history.filter { row ->
        row.id != entry.id && row.kind == entry.kind && row.effectiveDate == entry.effectiveDate &&
            (row.inputCurrency ?: "EUR") == (entry.inputCurrency ?: "EUR") &&
            // Synced rows include system bookkeeping endpoints absent from optimistic rows.
            (when (entry.kind) {
                "expense" -> row.fromAccountId == entry.fromAccountId && row.fromCategoryId == entry.fromCategoryId
                "income" -> row.toAccountId == entry.toAccountId && row.toCategoryId == entry.toCategoryId && row.incomeTreatment == entry.incomeTreatment
                "transfer" -> row.fromAccountId == entry.fromAccountId && row.toAccountId == entry.toAccountId
                "reallocate" -> row.fromCategoryId == entry.fromCategoryId && row.toCategoryId == entry.toCategoryId
                else -> false
            }) && row.amount.subtract(entry.amount).abs() <= BigDecimal("0.03") &&
            (entry.kind != "transfer" || (row.toCurrency == entry.toCurrency &&
                ((row.toAmount == null && entry.toAmount == null) ||
                    (row.toAmount != null && entry.toAmount != null &&
                        row.toAmount.subtract(entry.toAmount).abs() <= BigDecimal("0.03")))))
    }.distinctBy { it.id }.sortedWith(
        compareByDescending<Transaction> { similarity(it) }.thenBy { it.amount.subtract(entry.amount).abs() }
    )
}
