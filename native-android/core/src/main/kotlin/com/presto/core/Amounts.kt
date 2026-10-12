package com.presto.core

import java.math.BigDecimal
import java.math.MathContext

/** Parses localized separators and bounded arithmetic expressions. */
fun parseLocalized(value: String): BigDecimal? {
    var text =
        value
            .trim()
            .replace(Regex("^(?:EUR|USD|PHP|[€$₱])\\s*", RegexOption.IGNORE_CASE), "")
            .replace(Regex("\\s*(?:EUR|USD|PHP|[€$₱])$", RegexOption.IGNORE_CASE), "")
    var sign = ""
    if (text.startsWith('+') || text.startsWith('-')) {
        sign = text.take(1)
        text = text.drop(1)
    }
    val dot = text.lastIndexOf('.')
    val comma = text.lastIndexOf(',')
    if (dot >= 0 && comma >= 0) {
        val separator = if (dot > comma) '.' else ','
        val grouping = if (separator == '.') ',' else '.'
        val parts = text.split(separator)
        if (parts.size != 2 || !parts[1].matches(Regex("\\d+"))) return null
        val groups = parts[0].split(grouping)
        if (
            !groups[0].matches(Regex("\\d{1,3}")) ||
                !groups.drop(1).all { it.matches(Regex("\\d{3}")) }
        )
            return null
        text = groups.joinToString("") + "." + parts[1]
    } else {
        val separator = if (dot >= 0) '.' else if (comma >= 0) ',' else null
        if (separator != null && text.split(separator).size > 2) {
            val groups = text.split(separator)
            if (
                !groups[0].matches(Regex("\\d{1,3}")) ||
                    !groups.drop(1).all { it.matches(Regex("\\d{3}")) }
            )
                return null
            text = groups.joinToString("")
        } else text = text.replace(',', '.')
        if (text.any { it.isWhitespace() || it == '\u00a0' }) {
            if (!text.matches(Regex("\\d{1,3}(?:[ \\u00a0\\u202f]\\d{3})+(?:\\.\\d+)?")))
                return null
            text = text.replace(Regex("[ \\u00a0\\u202f]"), "")
        }
    }
    return if (text.matches(Regex("(?:\\d+(?:\\.\\d*)?|\\.\\d+)")))
        (sign + text).toBigDecimalOrNull()
    else null
}

fun parseAmount(text: String): BigDecimal? =
    if (text.trim().startsWith('='))
        runCatching { Formula(text.trim().drop(1)).parse() }.getOrNull()
    else parseLocalized(text)

fun parseMoney(text: String): BigDecimal? =
    parseAmount(text)?.let { amount ->
        val rounded = amount.setScale(2, java.math.RoundingMode.HALF_UP)
        val exact =
            amount.stripTrailingZeros().scale() <= 2 ||
                (text.trim().startsWith('=') &&
                    (amount - rounded).abs() < BigDecimal("0.000000001"))
        if (!exact || amount.abs() > BigDecimal("90071992547409.91")) null else rounded
    }

private class Formula(private val text: String) {
    private var index = 0

    private fun spaces() {
        while (index < text.length && text[index].isWhitespace()) index++
    }

    fun parse(): BigDecimal {
        require(text.length <= 256)
        val value = expression()
        spaces()
        require(index == text.length)
        return value
    }

    private fun expression(): BigDecimal {
        var result = term()
        while (true) {
            spaces()
            val op = text.getOrNull(index)
            if (op != '+' && op != '-') return result
            index++
            val right = term()
            result = if (op == '+') result + right else result - right
        }
    }

    private fun term(): BigDecimal {
        var result = unary()
        while (true) {
            spaces()
            val op = text.getOrNull(index)
            if (op !in listOf('*', '/', '×', '÷')) return result
            index++
            val right = unary()
            result =
                if (op == '*' || op == '×') result * right
                else result.divide(right, MathContext.DECIMAL128)
        }
    }

    private fun unary(): BigDecimal {
        spaces()
        val op = text.getOrNull(index)
        if (op == '+' || op == '-') {
            index++
            return unary().let { if (op == '-') -it else it }
        }
        if (op == '(') {
            index++
            val value = expression()
            spaces()
            require(text.getOrNull(index) == ')')
            index++
            return value
        }
        val match =
            Regex("^(?:\\d[\\d.,]*|[.,]\\d+)").find(text.drop(index)) ?: error("Expected a number")
        index += match.value.length
        return parseLocalized(match.value) ?: error("Invalid number")
    }
}
