package com.presto.nativeapp

import com.presto.core.*
import java.io.File
import java.util.concurrent.TimeUnit
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import kotlinx.serialization.json.*
import okhttp3.*
import okhttp3.MediaType.Companion.toMediaType
import okhttp3.RequestBody.Companion.asRequestBody
import okhttp3.RequestBody.Companion.toRequestBody

class ApiFailure(val status: Int, message: String) : Exception(message)

class FinanceApi(
    private val baseUrl: String,
    private val client: OkHttpClient =
        OkHttpClient.Builder()
            .connectTimeout(20, TimeUnit.SECONDS)
            .readTimeout(90, TimeUnit.SECONDS)
            .build(),
) {
    val origin
        get() = baseUrl.trimEnd('/')

    private suspend fun response(
        path: String,
        token: String?,
        body: String? = null,
        digest: String? = null,
    ): Pair<String, String?> =
        withContext(Dispatchers.IO) {
            val request =
                Request.Builder()
                    .url(origin + path)
                    .apply {
                        token?.let { header("Authorization", "Bearer $it") }
                        digest?.let { header("X-Finance-Snapshot-SHA256", it) }
                        if (body != null) post(body.toRequestBody("application/json".toMediaType()))
                    }
                    .build()
            client.newCall(request).execute().use { response ->
                val text = response.body?.string() ?: ""
                if (!response.isSuccessful) {
                    val message =
                        runCatching {
                                codec
                                    .parseToJsonElement(text)
                                    .jsonObject["error"]
                                    ?.jsonPrimitive
                                    ?.content
                            }
                            .getOrNull()
                    throw ApiFailure(
                        response.code,
                        "${request.method} ${request.url}: ${message ?: "Finance server returned ${response.code}."}",
                    )
                }
                text to response.header("x-finance-snapshot-sha256")
            }
        }

    suspend fun exchange(code: String, verifier: String): Session {
        require(code.matches(Regex("[0-9a-fA-F]{64}"))) { "Invalid Pocket ID handoff." }
        val session =
            codec.decodeFromString<Session>(
                response(
                        "/api/mobile/session/exchange",
                        null,
                        buildJsonObject { put("handoffCode", code); put("codeVerifier", verifier) }.toString(),
                    )
                    .first
            )
        require(
            session.accessToken.matches(Regex("[0-9a-fA-F]{64}")) && session.subject.isNotBlank()
        ) {
            "Invalid finance session."
        }
        return session
    }

    suspend fun validate(session: Session): Boolean =
        codec
            .parseToJsonElement(response("/api/session", session.accessToken).first)
            .jsonObject["pocketAuthenticated"]
            ?.jsonPrimitive
            ?.booleanOrNull == true

    suspend fun logout(session: Session) {
        response("/api/mobile/logout", session.accessToken, "{}")
    }

    suspend fun registerPush(session: Session, installation: String, token: String) {
        response("/api/mobile/push-device", session.accessToken,
            buildJsonObject { put("installationId", installation); put("token", token) }.toString())
    }

    suspend fun removePush(session: Session, installation: String) {
        response("/api/mobile/push-device/remove", session.accessToken,
            buildJsonObject { put("installationId", installation) }.toString())
    }

    suspend fun transaction(session: Session, id: String): Transaction = codec.decodeFromString(
        response("/api/db/transactions/detail?id=$id", session.accessToken).first)

    suspend fun snapshot(session: Session, count: Int = 200): Snapshot =
        withContext(Dispatchers.IO) {
            codec.decodeFromString(
                response("/api/db/finance?asOf=${today()}&recentCount=$count", session.accessToken).first
            )
        }

    suspend fun write(session: Session, payload: String, editing: Boolean): JsonObject =
        codec
            .parseToJsonElement(
                response(
                        if (editing) "/api/db/transactions/edit" else "/api/db/transactions",
                        session.accessToken,
                        payload,
                    )
                    .first
            )
            .jsonObject

    suspend fun deleteTransaction(session: Session, payload: String): JsonObject =
        codec.parseToJsonElement(
            response("/api/db/transactions/delete", session.accessToken, payload).first
        ).jsonObject

    suspend fun completeIncome(session: Session, month: String) {
        response(
            "/api/budget/income-complete",
            session.accessToken,
            buildJsonObject {
                    put("expectedMonth", month)
                    put("operationId", uuidV7())
                }
                .toString(),
        )
    }

    suspend fun backupFile(session: Session, target: File): String =
        withContext(Dispatchers.IO) {
            val request =
                Request.Builder()
                    .url(origin + "/api/db/finance/backup")
                    .header("Authorization", "Bearer ${session.accessToken}")
                    .build()
            client.newCall(request).execute().use { response ->
                if (!response.isSuccessful)
                    throw ApiFailure(response.code, "${request.method} ${request.url}: Recovery server returned ${response.code}.")
                val digest =
                    response.header("x-finance-snapshot-sha256")
                        ?: error("Missing recovery digest.")
                response.body!!.byteStream().use { input ->
                    java.io.FileOutputStream(target).use { output ->
                        input.copyTo(output)
                        output.fd.sync()
                    }
                }
                digest
            }
        }

    suspend fun restoreFile(session: Session, file: File, digest: String) =
        withContext(Dispatchers.IO) {
            val request =
                Request.Builder()
                    .url(origin + "/api/db/finance/restore")
                    .header("Authorization", "Bearer ${session.accessToken}")
                    .header("X-Finance-Snapshot-SHA256", digest)
                    .post(file.asRequestBody("application/json".toMediaType()))
                    .build()
            client.newCall(request).execute().use { response ->
                if (!response.isSuccessful) {
                    val message =
                        runCatching {
                                codec
                                    .parseToJsonElement(response.body!!.string())
                                    .jsonObject["error"]
                                    ?.jsonPrimitive
                                    ?.content
                            }
                            .getOrNull()
                    throw ApiFailure(
                        response.code,
                        "${request.method} ${request.url}: ${message ?: "Recovery server returned ${response.code}."}",
                    )
                }
            }
        }
}
