package com.presto.nativeapp

import android.content.Context
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import com.presto.core.uuidV7
import java.io.File
import kotlin.test.*
import kotlinx.serialization.json.*
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class RecoveryFixtureTest {
    @Test
    fun actualServerExportCanBeVerifiedEncryptedAndMaterializedExactly() {
        val target = ApplicationProvider.getApplicationContext<Context>()
        val assets =
            androidx.test.platform.app.InstrumentationRegistry.getInstrumentation().context.assets
        val source = File.createTempFile("recovery-fixture-", ".json", target.cacheDir)
        val scope = "server-export-${uuidV7()}"
        val root =
            File(target.noBackupFilesDir, "finance-recovery/${sha256(scope.encodeToByteArray())}")
        val store = RecoveryStore(target, Vault(target))
        try {
            assets.open("finance-snapshot.json").use { input ->
                source.outputStream().use { input.copyTo(it) }
            }
            val digest = sha256(source.readBytes())
            val info = store.saveFile(scope, source, digest)
            assertEquals(digest, store.latestInfo(scope)?.sha256)
            assertTrue(info.tableCounts.values.sumOf { it.jsonPrimitive.int } > 0)
            val materialized = store.materialize(scope, info)
            try {
                assertTrue(source.readBytes().contentEquals(materialized.readBytes()))
            } finally {
                materialized.delete()
            }
            // A table count change invalidates the interchange even with a matching digest.
            val bad =
                source.readText().replaceFirst("\"finance_accounts\": 6", "\"finance_accounts\": 7")
            if (bad != source.readText())
                assertFails { store.save(scope, bad, sha256(bad.encodeToByteArray())) }
        } finally {
            source.delete()
            root.deleteRecursively()
        }
    }
}
