package com.presto.nativeapp

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.os.Build
import androidx.core.app.NotificationCompat
import androidx.core.app.NotificationManagerCompat
import com.google.firebase.FirebaseApp
import com.google.firebase.FirebaseOptions
import com.google.firebase.messaging.FirebaseMessaging
import com.google.firebase.messaging.FirebaseMessagingService
import com.google.firebase.messaging.RemoteMessage
import com.presto.core.Session
import kotlinx.coroutines.suspendCancellableCoroutine
import kotlinx.coroutines.withTimeout
import java.util.UUID
import kotlin.coroutines.resume
import kotlin.coroutines.resumeWithException

internal interface PartnerNotifications {
    fun enabled(): Boolean
    fun post(event: String, notification: Notification)
    fun clear()
}

private class AndroidPartnerNotifications(private val context: Context) : PartnerNotifications {
    private val manager = context.getSystemService(NotificationManager::class.java)
    override fun enabled(): Boolean {
        if (Build.VERSION.SDK_INT >= 26) manager.createNotificationChannel(
            NotificationChannel(PartnerPush.CHANNEL, "Partner transactions", NotificationManager.IMPORTANCE_DEFAULT))
        return NotificationManagerCompat.from(context).areNotificationsEnabled() &&
            (Build.VERSION.SDK_INT < 26 || manager.getNotificationChannel(PartnerPush.CHANNEL)?.importance != NotificationManager.IMPORTANCE_NONE)
    }
    override fun post(event: String, notification: Notification) {
        manager.notify("partner:$event", 72, notification)
    }
    override fun clear() {
        manager.activeNotifications.filter { it.tag?.startsWith("partner:") == true }
            .forEach { manager.cancel(it.tag, it.id) }
    }
}

internal class PartnerPush(private val context: Context, namespace: String,
    private val available: Boolean = BuildConfig.FCM_ENABLED,
    private val notifications: PartnerNotifications = AndroidPartnerNotifications(context)) {
    private val prefs = context.getSharedPreferences("$namespace-push", Context.MODE_PRIVATE)
    val installation: String = synchronized(ownerLock) {
        prefs.getString("installation", null) ?: UUID.randomUUID().toString().also {
            check(prefs.edit().putString("installation", it).commit())
        }
    }
    val enabled get() = prefs.getBoolean("enabled", false)

    fun setEnabled(enabled: Boolean) = synchronized(ownerLock) {
        check(prefs.edit().putBoolean("enabled", enabled).commit())
        if (!enabled) clearOwner()
    }

    fun clearOwner() = synchronized(ownerLock) {
        prefs.edit().remove("subject").remove("expires").remove("sessionHash").commit()
        notifications.clear()
    }

    fun bind(session: Session) = synchronized(ownerLock) {
        check(prefs.edit().putString("subject", session.subject)
            .putString("sessionHash", sha256(session.accessToken.encodeToByteArray()))
            .putLong("expires", session.expiresAt ?: 0).commit())
    }

    suspend fun token(): String = withTimeout(20_000) {
        suspendCancellableCoroutine { continuation ->
            FirebaseMessaging.getInstance().token.addOnCompleteListener { task ->
                if (continuation.isActive) {
                    if (task.isSuccessful) continuation.resume(task.result)
                    else continuation.resumeWithException(task.exception ?: IllegalStateException("Push registration failed"))
                }
            }
        }
    }

    fun canReceive(subject: String): Boolean = synchronized(ownerLock) {
        available && enabled && prefs.getString("subject", null) == subject &&
            prefs.getLong("expires", 0) > System.currentTimeMillis() / 1000
    }

    fun receive(message: Map<String, String>): Unit = synchronized(ownerLock) {
        val subject = message["recipientSubject"] ?: return
        val event = message["eventId"] ?: return
        val transaction = message["transactionId"] ?: return
        if (prefs.getString("sessionHash", null) != message["recipientSession"]) return
        if (!canReceive(subject) || !validId(event) || !validId(transaction)) return
        if (!notifications.enabled()) return
        val seen = prefs.getString("seen", "")!!.split(',').filter { it.isNotBlank() }
        if (event in seen) return
        val intent = Intent(context, MainActivity::class.java)
            .setAction("com.presto.nativeapp.TRANSACTION.$event")
            .putExtra("partnerTransactionId", transaction).putExtra("partnerSubject", subject)
            .addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP or Intent.FLAG_ACTIVITY_CLEAR_TOP)
        val pending = PendingIntent.getActivity(context, 0, intent,
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE)
        val notification = NotificationCompat.Builder(context, CHANNEL)
            .setSmallIcon(R.drawable.ic_partner_notification)
            .setContentTitle(message["title"]?.take(100) ?: "Partner added a transaction")
            .setContentText(message["body"]?.take(250) ?: "Tap to view transaction")
            .setStyle(NotificationCompat.BigTextStyle().bigText(message["body"]?.take(250)))
            .setContentIntent(pending).setAutoCancel(true).setOnlyAlertOnce(true)
            .setVisibility(NotificationCompat.VISIBILITY_PRIVATE).build()
        try {
            notifications.post(event, notification)
            // Bounded persisted deduplication, including across process restarts.
            prefs.edit().putString("seen", (seen.takeLast(255) + event).joinToString(",")).commit()
        } catch (_: SecurityException) { /* Permission may change between checking and posting. */ }
    }

    companion object {
        // Shared by repository and Firebase service instances; clearing waits for posting.
        private val ownerLock = Any()
        const val CHANNEL = "partner-transactions"
        fun validId(value: String) = runCatching { UUID.fromString(value) }.isSuccess
        fun initialize(context: Context) {
            if (!BuildConfig.FCM_ENABLED) return
            if (FirebaseApp.getApps(context).none { it.name == FirebaseApp.DEFAULT_APP_NAME })
                FirebaseApp.initializeApp(context, FirebaseOptions.Builder()
                    .setApplicationId(BuildConfig.FCM_APP_ID).setApiKey(BuildConfig.FCM_API_KEY)
                    .setProjectId(BuildConfig.FCM_PROJECT_ID).setGcmSenderId(BuildConfig.FCM_SENDER_ID).build())
        }
    }
}

class PartnerMessagingService : FirebaseMessagingService() {
    override fun onNewToken(token: String) {
        if (BuildConfig.FCM_ENABLED) (application as PrestoApplication).repository.refreshPushRegistration()
    }
    override fun onMessageReceived(message: RemoteMessage) {
        if (BuildConfig.FCM_ENABLED) PartnerPush(this, "presto-native").receive(message.data)
    }
}
