package com.presto.nativeapp

import androidx.test.ext.junit.runners.AndroidJUnit4
import kotlin.test.*
import kotlinx.coroutines.*
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class DiagnosticsRegressionTest {
    @Test fun historyIsBoundedSanitizedAndDoesNotSurviveANewSession(): Unit = runBlocking {
        val diagnostics = Diagnostics()
        withContext(Dispatchers.Default) {
            repeat(250) { diagnostics.record("Entry $it") }
        }
        assertEquals(200, diagnostics.logs.value.size)
        assertTrue(diagnostics.logs.value.first().endsWith("Entry 50"))
        assertTrue(diagnostics.logs.value.last().endsWith("Entry 249"))
        diagnostics.record("Bearer secret\n${"a".repeat(64)}")
        assertTrue(diagnostics.logs.value.last().endsWith("Bearer [redacted] [redacted]"))
        diagnostics.record("x".repeat(3_000))
        assertEquals(2_000, diagnostics.logs.value.last().substringAfter(' ').length)
        assertEquals(emptyList(), Diagnostics().logs.value)
    }

    @Test fun concurrentEventsAreNotLost(): Unit = runBlocking {
        val diagnostics = Diagnostics()
        coroutineScope {
            repeat(100) { number ->
                launch(Dispatchers.Default) { diagnostics.record("Event $number") }
            }
        }
        assertEquals(100, diagnostics.logs.value.size)
        assertEquals((0 until 100).map { "Event $it" }.toSet(),
            diagnostics.logs.value.map { it.substringAfter(' ') }.toSet())
    }
}
