package com.presto.nativeapp

/** Keep the action and exception chain visible without including session credentials. */
internal fun failureDetails(action: String, failure: Throwable): String {
    val causes = mutableListOf<Throwable>()
    var current: Throwable? = failure
    while (current != null && causes.none { it === current }) {
        causes.add(current)
        current = current.cause
    }
    return "$action failed: " + causes.joinToString("\nCaused by: ") {
        val status = if (it is ApiFailure) " (HTTP ${it.status})" else ""
        "${it.javaClass.simpleName}$status: ${it.message ?: "No exception message"}"
    }
}
