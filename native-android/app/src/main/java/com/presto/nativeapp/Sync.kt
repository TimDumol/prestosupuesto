package com.presto.nativeapp

import android.Manifest
import android.app.*
import android.content.Context
import android.content.pm.PackageManager
import android.os.Build
import androidx.core.app.NotificationCompat
import androidx.core.content.ContextCompat
import androidx.work.*
import java.util.concurrent.TimeUnit

object SyncScheduler {
    private fun name(scope: String) = "finance-sync-${sha256(scope.encodeToByteArray())}"

    fun schedule(context: Context, scope: String) {
        val constraints =
            Constraints.Builder().setRequiredNetworkType(NetworkType.CONNECTED).build()
        val data = workDataOf("scope" to scope)
        WorkManager.getInstance(context)
            .enqueueUniqueWork(
                name(scope),
                // A save arriving as a worker finishes must leave a durable successor.
                ExistingWorkPolicy.APPEND_OR_REPLACE,
                OneTimeWorkRequestBuilder<FinanceWorker>()
                    .setConstraints(constraints)
                    .setInputData(data)
                    .setBackoffCriteria(BackoffPolicy.EXPONENTIAL, 30, TimeUnit.SECONDS)
                    .build(),
            )
        ensureSafetyWork(context, scope)
    }

    fun maintain(context: Context, scope: String, hasPending: Boolean = false) {
        if (hasPending) schedule(context, scope) else ensureSafetyWork(context, scope)
    }

    private fun ensureSafetyWork(context: Context, scope: String) {
        WorkManager.getInstance(context).cancelUniqueWork(name(scope) + "-periodic")
        // Keep one infrequent fallback; saved entries get immediate one-time work above.
        WorkManager.getInstance(context).enqueueUniquePeriodicWork(
            name(scope) + "-safety",
            ExistingPeriodicWorkPolicy.KEEP,
            PeriodicWorkRequestBuilder<FinanceWorker>(6, TimeUnit.HOURS)
                .setConstraints(Constraints.Builder().setRequiredNetworkType(NetworkType.CONNECTED).build())
                .setInputData(workDataOf("scope" to scope))
                .setInitialDelay(6, TimeUnit.HOURS)
                .build(),
        )
    }

    fun cancel(context: Context, scope: String) {
        WorkManager.getInstance(context).cancelUniqueWork(name(scope))
        WorkManager.getInstance(context).cancelUniqueWork(name(scope) + "-periodic")
        WorkManager.getInstance(context).cancelUniqueWork(name(scope) + "-safety")
    }

    fun notification(context: Context, count: Int) {
        val manager = context.getSystemService(NotificationManager::class.java)
        if (count == 0) {
            manager.cancel(71)
            return
        }
        if (
            Build.VERSION.SDK_INT >= 33 &&
                ContextCompat.checkSelfPermission(
                    context,
                    Manifest.permission.POST_NOTIFICATIONS,
                ) != PackageManager.PERMISSION_GRANTED
        )
            return
        if (Build.VERSION.SDK_INT >= 26)
            manager.createNotificationChannel(
                NotificationChannel(
                    "finance-sync",
                    "Pending finance entries",
                    NotificationManager.IMPORTANCE_LOW,
                )
            )
        val intent =
            PendingIntent.getActivity(
                context,
                0,
                android.content.Intent(context, MainActivity::class.java),
                PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
            )
        manager.notify(
            71,
            NotificationCompat.Builder(context, "finance-sync")
                .setSmallIcon(android.R.drawable.stat_notify_sync)
                .setContentTitle("Presto Presupuesto")
                .setContentText("$count ${if (count == 1) "entry" else "entries"} waiting to sync")
                .setOngoing(true)
                .setOnlyAlertOnce(true)
                .setContentIntent(intent)
                .build(),
        )
    }
}

class FinanceWorker(context: Context, parameters: WorkerParameters) :
    CoroutineWorker(context, parameters) {
    override suspend fun doWork(): Result {
        val repo = (applicationContext as PrestoApplication).repository
        repo.awaitReady()
        val state = repo.state.value
        if (
            state.session == null ||
                inputData.getString("scope") != "${state.owner}|${repo.api.origin}"
        )
            return Result.success()
        return if (repo.syncInBackground()) Result.success() else Result.retry()
    }
}
