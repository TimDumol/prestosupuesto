package com.presto.core

import kotlin.test.*
import kotlinx.serialization.json.*

class EntryFixturesTest {
    @Test
    fun amountFixtures() {
        val fixture =
            codec
                .parseToJsonElement(javaClass.getResource("/entry-cases.json")!!.readText())
                .jsonObject
        fixture.getValue("amounts").jsonArray.forEach { value ->
            val row = value.jsonObject
            val input = row.getValue("input").jsonPrimitive.content
            val expected = row["money"]?.jsonPrimitive?.contentOrNull?.toBigDecimal()
            assertEquals(expected, parseMoney(input), input)
            val amount = row["amount"]?.jsonPrimitive?.contentOrNull?.toBigDecimal()
            val actual = parseAmount(input)
            if (amount == null) assertNull(actual, input)
            else {
                assertNotNull(actual, input)
                // Decimal fixtures include representation noise from formula
                // intermediates.
                assertTrue((amount - actual).abs() < "0.00000000000001".toBigDecimal(), input)
            }
        }
    }
}
