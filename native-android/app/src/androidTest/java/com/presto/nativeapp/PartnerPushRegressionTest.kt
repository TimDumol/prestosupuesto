package com.presto.nativeapp

import android.app.Notification
import android.content.Context
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import com.google.firebase.FirebaseApp
import com.presto.core.Session
import com.presto.core.uuidV7
import kotlin.test.*
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class PartnerPushRegressionTest {
    @Test fun syntheticAppNeverInitializesHouseholdFirebase() {
        val context = ApplicationProvider.getApplicationContext<Context>()
        assertFalse(BuildConfig.FCM_ENABLED)
        PartnerPush.initialize(context)
        assertTrue(FirebaseApp.getApps(context).isEmpty())
    }

    @Test fun alertsShowDetailsDeduplicateAndRejectOtherSessions() {
        val context = ApplicationProvider.getApplicationContext<Context>()
        val namespace = "push-receive-${uuidV7()}"
        // Capture actual Android Notification objects without posting or contacting Firebase.
        val posted = mutableListOf<Pair<String, Notification>>()
        val sink = object : PartnerNotifications {
            var allowed = true
            override fun enabled() = allowed
            override fun post(event: String, notification: Notification) { posted += event to notification }
            override fun clear() { posted.clear() }
        }
        val push = PartnerPush(context, namespace, available = true, notifications = sink)
        val session = Session("a".repeat(64), "synthetic-owner", "Test", System.currentTimeMillis()/1000+3600)
        val event = uuidV7()
        val message = mapOf("eventId" to event, "transactionId" to uuidV7(),
            "recipientSubject" to session.subject, "recipientSession" to sha256(session.accessToken.encodeToByteArray()),
            "title" to "Partner added an expense", "body" to "24.50 EUR · Synthetic groceries")
        try {
            push.setEnabled(true)
            push.bind(session)
            push.receive(message)
            val notification = posted.single().second
            assertEquals(message["body"], notification.extras.getCharSequence("android.text").toString())
            push.receive(message)
            val prefs = context.getSharedPreferences("$namespace-push", Context.MODE_PRIVATE)
            assertEquals(event, prefs.getString("seen", null))
            assertEquals(1, posted.size)
            push.receive(message + mapOf("eventId" to uuidV7(), "recipientSubject" to "other-owner"))
            push.receive(message + mapOf("eventId" to uuidV7(), "recipientSession" to "old-session"))
            assertEquals(event, prefs.getString("seen", null))
            push.setEnabled(false)
            push.receive(message + mapOf("eventId" to uuidV7()))
            assertEquals(event, prefs.getString("seen", null))
            assertTrue(posted.isEmpty())
            push.setEnabled(true)
            push.bind(session)
            sink.allowed = false
            push.receive(message + mapOf("eventId" to uuidV7()))
            assertTrue(posted.isEmpty())
            push.bind(session.copy(expiresAt = 1))
            assertFalse(push.canReceive(session.subject))
        } finally {
            context.getSharedPreferences("$namespace-push", Context.MODE_PRIVATE).edit().clear().commit()
        }
    }

    @Test fun clearingOwnerWaitsForReceiveAcrossInstances() {
        val context = ApplicationProvider.getApplicationContext<Context>()
        val namespace = "push-race-${uuidV7()}"
        val entered = java.util.concurrent.CountDownLatch(1)
        val release = java.util.concurrent.CountDownLatch(1)
        val cleared = java.util.concurrent.CountDownLatch(1)
        val posted = java.util.concurrent.atomic.AtomicBoolean(false)
        val sink = object : PartnerNotifications {
            override fun enabled() = true
            override fun post(event: String, notification: Notification) {
                entered.countDown()
                check(release.await(5, java.util.concurrent.TimeUnit.SECONDS))
                posted.set(true)
            }
            override fun clear() { posted.set(false); cleared.countDown() }
        }
        val receiver = PartnerPush(context, namespace, available = true, notifications = sink)
        val owner = PartnerPush(context, namespace, available = true, notifications = sink)
        val session = Session("a".repeat(64), "owner", "Test", System.currentTimeMillis()/1000+3600)
        val executor = java.util.concurrent.Executors.newFixedThreadPool(2)
        try {
            owner.setEnabled(true)
            owner.bind(session)
            val receiving = executor.submit {
                receiver.receive(mapOf("eventId" to uuidV7(), "transactionId" to uuidV7(),
                    "recipientSubject" to session.subject,
                    "recipientSession" to sha256(session.accessToken.encodeToByteArray())))
            }
            assertTrue(entered.await(5, java.util.concurrent.TimeUnit.SECONDS))
            val clearing = executor.submit { owner.clearOwner() }
            assertFalse(cleared.await(100, java.util.concurrent.TimeUnit.MILLISECONDS))
            release.countDown()
            receiving.get(5, java.util.concurrent.TimeUnit.SECONDS)
            clearing.get(5, java.util.concurrent.TimeUnit.SECONDS)
            assertFalse(posted.get())
            assertFalse(receiver.canReceive(session.subject))
        } finally {
            release.countDown()
            executor.shutdownNow()
            context.getSharedPreferences("$namespace-push", Context.MODE_PRIVATE).edit().clear().commit()
        }
    }

    @Test fun optOutPersistsAndClearsRecipientIdentity() {
        val context = ApplicationProvider.getApplicationContext<Context>()
        val namespace = "push-test-${uuidV7()}"
        val push = PartnerPush(context, namespace)
        val session = Session("a".repeat(64), "synthetic-owner", "Test", System.currentTimeMillis()/1000+3600)
        try {
            assertFalse(push.enabled)
            push.setEnabled(true)
            push.bind(session)
            val reopened = PartnerPush(context, namespace)
            assertTrue(reopened.enabled)
            assertEquals(push.installation, reopened.installation)
            assertEquals(session.subject, context.getSharedPreferences("$namespace-push", Context.MODE_PRIVATE).getString("subject", null))
            reopened.setEnabled(false)
            assertFalse(PartnerPush(context, namespace).enabled)
            assertNull(context.getSharedPreferences("$namespace-push", Context.MODE_PRIVATE).getString("subject", null))
        } finally {
            context.getSharedPreferences("$namespace-push", Context.MODE_PRIVATE).edit().clear().commit()
        }
    }
}
