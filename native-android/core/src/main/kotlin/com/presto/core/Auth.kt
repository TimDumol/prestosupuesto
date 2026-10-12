package com.presto.core

import java.security.MessageDigest
import java.security.SecureRandom
import java.util.Base64

fun newLoginVerifier(): String = Base64.getUrlEncoder().withoutPadding()
    .encodeToString(ByteArray(32).also { SecureRandom().nextBytes(it) })

fun loginChallenge(verifier: String): String = Base64.getUrlEncoder().withoutPadding()
    .encodeToString(MessageDigest.getInstance("SHA-256").digest(verifier.toByteArray(Charsets.US_ASCII)))

fun validLoginAttempt(started: Long, now: Long): Boolean =
    started > 0 && now >= started && now - started <= 10 * 60 * 1000
