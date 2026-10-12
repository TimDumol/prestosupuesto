package com.presto.core

import java.math.BigDecimal
import java.security.SecureRandom
import java.time.*
import java.util.UUID
import kotlinx.serialization.*
import kotlinx.serialization.descriptors.*
import kotlinx.serialization.encoding.*
import kotlinx.serialization.json.Json

object DecimalSerializer : KSerializer<BigDecimal> {
    override val descriptor = PrimitiveSerialDescriptor("Decimal", PrimitiveKind.STRING)

    override fun serialize(encoder: Encoder, value: BigDecimal) =
        encoder.encodeString(value.toPlainString())

    override fun deserialize(decoder: Decoder) = decoder.decodeString().toBigDecimal()
}

typealias Decimal = @Serializable(with = DecimalSerializer::class) BigDecimal

val ZERO: BigDecimal = BigDecimal.ZERO
val codec = Json {
    ignoreUnknownKeys = true
    encodeDefaults = true
    explicitNulls = false
}

fun decimal(value: String?) = value?.toBigDecimalOrNull() ?: ZERO

fun money(value: BigDecimal?, currency: String = "EUR"): String =
    value?.let {
        val symbol = mapOf("EUR" to "€", "USD" to "$", "PHP" to "₱")[currency] ?: "$currency "
        val formatted =
            java.text
                .DecimalFormat("#,##0.##", java.text.DecimalFormatSymbols(java.util.Locale.US))
                .format(it.abs())
        (if (it.signum() < 0) "−" else "") + symbol + formatted
    } ?: "Not available"

fun today(): String = LocalDate.now().toString()

fun zone(): ZoneId = runCatching { ZoneId.systemDefault() }.getOrDefault(ZoneId.of("Europe/Madrid"))

fun monthEnd(date: String): String =
    LocalDate.parse(date).withDayOfMonth(LocalDate.parse(date).lengthOfMonth()).toString()

fun previousMonthEnd(): String = LocalDate.now().withDayOfMonth(1).minusDays(1).toString()

fun captureTimestamp(
    date: String,
    original: Transaction? = null,
    now: Instant = Instant.now(),
): String? =
    if (original?.effectiveDate == date) original.occurredAt
    else if (date == now.atZone(zone()).toLocalDate().toString()) now.toString() else null

fun uuidV7(now: Long = System.currentTimeMillis()): String {
    val random = SecureRandom()
    val high = ((now and 0xffffffffffffL) shl 16) or 0x7000 or random.nextInt(4096).toLong()
    val low = (random.nextLong() and 0x3fffffffffffffffL) or Long.MIN_VALUE
    return UUID(high, low).toString()
}

@Serializable
data class Session(val accessToken: String, val subject: String, val username: String, val expiresAt: Long? = null) {
    val owner
        get() = "pocket-id:$subject"
}

@Serializable
data class Account(
    val id: String,
    val name: String,
    val accountType: String? = null,
    val currency: String? = "EUR",
    val balanceNative: Decimal? = null,
    val actualBalanceNative: Decimal? = null,
    val reconciliationNative: Decimal? = null,
    val balanceEur: Decimal? = null,
    val actualBalanceEur: Decimal? = null,
    val ledgerRevision: Long = 0,
)

@Serializable
data class Category(
    val categoryId: String,
    val name: String,
    val monthlyEur: Decimal? = null,
    val expenseEur: Decimal? = null,
    val remainingEur: Decimal? = null,
    val reallocationImpactEur: Decimal? = null,
) {
    val system
        get() = categoryId in setOf("income", "expense")
}

@Serializable
data class Transaction(
    val id: String,
    val kind: String,
    val effectiveDate: String,
    val description: String,
    val amount: Decimal,
    val inputCurrency: String? = "EUR",
    val occurredAt: String? = null,
    val toAmount: Decimal? = null,
    val toCurrency: String? = null,
    val fromCategory: String? = null,
    val toCategory: String? = null,
    val fromCategoryId: String? = null,
    val toCategoryId: String? = null,
    val fromAccount: String? = null,
    val toAccount: String? = null,
    val fromAccountId: String? = null,
    val toAccountId: String? = null,
    val hasCategoryCorrection: Boolean = false,
    val revision: Long = 1,
    val canEdit: Boolean = false,
    val correctsTransactionId: String? = null,
    val incomeTreatment: String? = null,
    val expectedIncomeMonth: String? = null,
    val pending: Boolean = false,
)

@Serializable
data class Expectation(
    val month: String,
    val amount: Decimal = ZERO,
    val currency: String = "EUR",
    val amountEur: Decimal = ZERO,
    val receivedEur: Decimal = ZERO,
    val receivedAmount: Decimal = ZERO,
    val outstandingEur: Decimal = ZERO,
    val arrivalDay: Int = 1,
    val completedMonth: String? = null,
)

@Serializable
data class Coverage(
    val actualCashEur: Decimal,
    val assignedRemainingEur: Decimal,
    val unallocatedEur: Decimal,
    val outstandingExpectedEur: Decimal,
    val currentGapEur: Decimal,
    val projectedGapEur: Decimal,
    val expectations: List<Expectation> = emptyList(),
    val frozen: Boolean = false,
)

@Serializable data class HistoryMonth(val month: String, val categories: List<Category>)

@Serializable
data class Snapshot(
    val asOf: String,
    val budgetMonth: String,
    val transactionCount: Int = 0,
    val recentTransactions: List<Transaction> = emptyList(),
    val budgets: List<Category> = emptyList(),
    val budgetHistory: List<HistoryMonth> = emptyList(),
    val accounts: List<Account> = emptyList(),
    val budgetCoverage: Coverage? = null,
)

@Serializable data class AccountRevision(val accountId: String, val revision: Long)

@Serializable
data class WritePayload(
    val operationId: String,
    val transactionId: String,
    val kind: String,
    val date: String,
    val description: String,
    val amount: Decimal,
    val occurredAt: String? = null,
    val dateOnly: Boolean = true,
    val currency: String? = null,
    val toAmount: Decimal? = null,
    val fromCategoryId: String? = null,
    val toCategoryId: String? = null,
    val fromAccountId: String? = null,
    val toAccountId: String? = null,
    val incomeTreatment: String? = null,
    val expectedIncomeMonth: String? = null,
    val correctsTransactionId: String? = null,
    val expectedRevision: Long? = null,
    val expectedAccountRevisions: List<AccountRevision>? = null,
)

@Serializable
data class Draft(
    val kind: String = "expense",
    val amount: String = "",
    val currency: String = "EUR",
    val description: String = "",
    val date: String = today(),
    val note: String = "",
    val toAmount: String = "",
    val categoryFrom: String? = null,
    val categoryTo: String? = null,
    val accountFrom: String? = null,
    val accountTo: String? = null,
    val correctionOf: String = "",
    val incomeTreatment: String = "salary",
    val expectedIncomeMonth: String = "",
    val original: Transaction? = null,
    val balancing: Boolean = false,
    val showMore: Boolean = false,
    val replacesOperation: String? = null,
    val queuedTransactionId: String? = null,
    val userTouched: Boolean = false,
    val manualCategory: Boolean = false,
    val manualFromAccount: Boolean = false,
    val manualToAccount: Boolean = false,
) {
    val touched
        get() =
            userTouched ||
                note.isNotBlank() ||
                toAmount.isNotBlank() ||
                amount.isNotBlank() ||
                description.isNotBlank() ||
                original != null
}

fun defaultDraft(snapshot: Snapshot) =
    Draft(
        categoryFrom = snapshot.budgets.firstOrNull { !it.system }?.categoryId,
        categoryTo =
            snapshot.budgets.filter { !it.system }.getOrNull(1)?.categoryId
                ?: snapshot.budgets.firstOrNull { !it.system }?.categoryId,
        accountFrom = snapshot.accounts.firstOrNull()?.id,
        accountTo = snapshot.accounts.getOrNull(1)?.id ?: snapshot.accounts.firstOrNull()?.id,
        currency = snapshot.accounts.firstOrNull()?.currency ?: "EUR",
    )

fun demoSnapshot(): Snapshot {
    val categories =
        listOf(
                "Food & groceries",
                "Transport",
                "Eating out",
                "Home repairs",
                "Travel",
                "General buffer",
            )
            .mapIndexed { i, name ->
                Category(
                    name,
                    name,
                    BigDecimal(100 + i * 50),
                    BigDecimal(20 + i * 10),
                    BigDecimal(if (i == 2) -30 else 100 + i * 40),
                    ZERO,
                )
            }
    val accounts =
        listOf(
            Account("Checking", "Checking", "bank", "EUR", BigDecimal(2500)),
            Account("Visa", "Visa", "card", "EUR", BigDecimal(-200)),
            Account("Cash", "Cash", "cash", "EUR", BigDecimal(80)),
            Account("H GoTyme", "H GoTyme", "savings", "PHP", BigDecimal(40000)),
        )
    val transactions =
        listOf(
            Transaction(
                "demo-1",
                "expense",
                today(),
                "Corner café",
                BigDecimal("2.20"),
                fromCategoryId = "Eating out",
                fromAccountId = "Cash",
                canEdit = true,
            ),
            Transaction(
                "demo-2",
                "expense",
                LocalDate.now().minusDays(1).toString(),
                "Metro pass",
                BigDecimal("1.50"),
                fromCategoryId = "Transport",
                fromAccountId = "Visa",
                canEdit = true,
            ),
        )
    return Snapshot(
        today(),
        today().take(7),
        transactions.size,
        transactions,
        categories,
        (0..5).reversed().map {
            HistoryMonth(
                YearMonth.now().minusMonths(it.toLong()).toString(),
                categories.mapIndexed { index, c ->
                    c.copy(expenseEur = BigDecimal(20 + index * 12 + it * 3))
                },
            )
        },
        accounts,
    )
}
