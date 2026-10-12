package com.presto.core

import kotlin.test.*

class AuthTest {
    @Test fun pkceUsesFreshVerifiersAndTheS256TestVector() {
        assertEquals("E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM",
            loginChallenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"))
        val first = newLoginVerifier()
        assertTrue(first.matches(Regex("[A-Za-z0-9_-]{43}")))
        assertNotEquals(first, newLoginVerifier())
    }
    @Test fun attemptsRejectExpiryAndClockRollback() {
        assertTrue(validLoginAttempt(1_000, 601_000))
        assertFalse(validLoginAttempt(1_000, 601_001))
        assertFalse(validLoginAttempt(1_000, 999))
        assertFalse(validLoginAttempt(0, 1_000))
    }
}
