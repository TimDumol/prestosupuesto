package com.presto.nativeapp

import android.content.Context
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import com.presto.core.*
import java.io.File
import kotlin.test.*
import kotlinx.coroutines.runBlocking
import okhttp3.mockwebserver.*
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class AuthRegressionTest {
    @Test fun expiredCredentialsAreClearedEvenWhileOffline(): Unit = runBlocking {
        val context = ApplicationProvider.getApplicationContext<Context>()
        val namespace = "auth-expired-${uuidV7()}"
        val session = Session("a".repeat(64), "expired-owner", "Test", 1)
        val vault = Vault(context, namespace)
        vault.saveSession(session)
        val repo = FinanceRepository(context, FinanceApi("http://127.0.0.1:1"), namespace, false)
        try {
            repo.start()
            repo.awaitReady()
            repo.sync()
            assertNull(repo.state.value.session)
            assertNull(vault.session())
            assertEquals(session.owner, repo.state.value.owner)
            assertTrue(repo.state.value.error.orEmpty().contains("session expired"))
        } finally {
            repo.close()
            context.deleteDatabase("$namespace.db")
            File(context.noBackupFilesDir, "$namespace-pocket-session.sealed").delete()
        }
    }

    @Test fun failedLogoutRetainsCredentialsUntilServerRevocationSucceeds(): Unit = runBlocking {
        val context = ApplicationProvider.getApplicationContext<Context>()
        val namespace = "auth-logout-${uuidV7()}"
        val session = Session("a".repeat(64), "logout-owner", "Test")
        val vault = Vault(context, namespace)
        vault.saveSession(session)
        var logoutFails = true
        val server = MockWebServer()
        server.dispatcher = object : Dispatcher() {
            override fun dispatch(request: RecordedRequest): MockResponse = when {
                request.path == "/api/session" -> MockResponse().setBody("{\"pocketAuthenticated\":true}")
                request.path!!.startsWith("/api/db/finance?") -> MockResponse().setBody(codec.encodeToString(Snapshot.serializer(), demoSnapshot()))
                request.path == "/api/mobile/logout" -> if (logoutFails) MockResponse().setResponseCode(503)
                    else MockResponse().setBody("{\"signedOut\":true}")
                else -> MockResponse().setResponseCode(503)
            }
        }
        server.start()
        val repo = FinanceRepository(context, FinanceApi(server.url("/").toString()), namespace, false)
        try {
            repo.start()
            repo.awaitReady()
            repo.sync()
            repo.logout()
            assertEquals(session, repo.state.value.session)
            assertEquals(session, vault.session())
            assertTrue(repo.state.value.error.orEmpty().contains("could not be revoked"))
            logoutFails = false
            repo.logout()
            assertNull(repo.state.value.session)
            assertNull(vault.session())
            assertTrue(repo.state.value.demo)
        } finally {
            repo.close()
            server.shutdown()
            context.deleteDatabase("$namespace.db")
            File(context.noBackupFilesDir, "$namespace-pocket-session.sealed").delete()
        }
    }
}
