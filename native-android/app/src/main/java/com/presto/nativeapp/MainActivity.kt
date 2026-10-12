package com.presto.nativeapp

import android.Manifest
import android.content.Intent
import android.net.Uri
import android.os.Build
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.activity.result.contract.ActivityResultContracts
import androidx.browser.customtabs.CustomTabsIntent
import androidx.lifecycle.ViewModel
import androidx.lifecycle.ViewModelProvider
import androidx.lifecycle.lifecycleScope
import androidx.lifecycle.viewModelScope
import com.presto.core.*
import kotlinx.coroutines.launch

class MainActivity : ComponentActivity() {
    private lateinit var model: PrestoViewModel
    private val permission =
        registerForActivityResult(ActivityResultContracts.RequestPermission()) {
            SyncScheduler.notification(this, model.repository.state.value.pending)
            model.repository.refreshPushRegistration()
        }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        val repo = (application as PrestoApplication).repository
        model =
            ViewModelProvider(
                this,
                object : ViewModelProvider.Factory {
                    @Suppress("UNCHECKED_CAST")
                    override fun <T : ViewModel> create(modelClass: Class<T>): T =
                        PrestoViewModel(repo) as T
                },
            )[PrestoViewModel::class.java]
        setContent { PrestoApp(model, ::beginLogin, ::notificationPermission) }
        handleCallback(intent)
        handlePartnerTransaction(intent)
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        setIntent(intent)
        handleCallback(intent)
        handlePartnerTransaction(intent)
    }

    override fun onResume() {
        super.onResume()
        if (::model.isInitialized) model.refresh()
    }

    override fun onStop() {
        if (::model.isInitialized) lifecycleScope.launch { model.repository.persistDraft() }
        super.onStop()
    }

    private fun handlePartnerTransaction(intent: Intent) {
        val id = intent.getStringExtra("partnerTransactionId") ?: return
        val subject = intent.getStringExtra("partnerSubject") ?: return
        intent.removeExtra("partnerTransactionId")
        intent.removeExtra("partnerSubject")
        model.openNotificationTransaction(id, subject)
    }

    private fun beginLogin() {
        val verifier = newLoginVerifier()
        // The callback is usable only with this app's saved verifier.
        val persisted = getSharedPreferences("native-login", MODE_PRIVATE)
            .edit()
            .putLong("started", System.currentTimeMillis())
            .putString("verifier", verifier)
            .commit()
        if (!persisted) {
            model.report("Could not save the sign-in request. Please try again.")
            return
        }
        CustomTabsIntent.Builder()
            .build()
            .launchUrl(
                this,
                Uri.parse(
                    model.repository.api.origin + "/auth/pocket-id/mobile?client=native-android&code_challenge=" + loginChallenge(verifier)
                ),
            )
    }

    private fun handleCallback(intent: Intent) {
        val uri = intent.data ?: return
        if (
            uri.scheme != "prestosupuesto-native" ||
                uri.host != "pocket-id" ||
                uri.path != "/callback"
        )
            return
        val prefs = getSharedPreferences("native-login", MODE_PRIVATE)
        val started = prefs.getLong("started", 0)
        val verifier = prefs.getString("verifier", null)
        if (!validLoginAttempt(started, System.currentTimeMillis()) || verifier == null) {
            model.report("Sign-in expired. Open Pocket ID again.")
            return
        }
        val handoff = uri.getQueryParameter("handoff")
        if (handoff == null || !handoff.matches(Regex("[0-9a-fA-F]{64}"))) {
            model.report("Pocket ID returned an invalid callback.")
            return
        }
        intent.data = null
        model.login(handoff, verifier) {
            // An invalid callback must not erase the proof needed for the legitimate callback.
            if (prefs.getString("verifier", null) == verifier)
                prefs.edit().remove("started").remove("verifier").commit()
        }
    }

    private fun notificationPermission() {
        if (Build.VERSION.SDK_INT >= 33) permission.launch(Manifest.permission.POST_NOTIFICATIONS)
    }
}

class PrestoViewModel(val repository: FinanceRepository) : ViewModel() {
    val state = repository.state
    val notificationTransaction = androidx.compose.runtime.mutableStateOf<Transaction?>(null)

    fun openNotificationTransaction(id: String, subject: String) {
        viewModelScope.launch {
            repository.awaitReady()
            repository.sync()
            notificationTransaction.value = repository.notificationTransaction(id, subject)
        }
    }

    fun partnerNotifications(enabled: Boolean) = repository.setPartnerNotifications(enabled)
    var localError = androidx.compose.runtime.mutableStateOf<String?>(null)
        private set

    fun report(error: String) {
        localError.value = error
    }

    fun clearError() {
        localError.value = null
        repository.dismissMessage()
    }

    fun update(draft: Draft, userEdited: Boolean = true) = repository.updateDraft(draft, userEdited)

    fun refreshIfStale() = repository.refreshIfStale()

    fun refresh() {
        viewModelScope.launch {
            repository.awaitReady()
            repository.sync()
        }
    }

    fun login(handoff: String, verifier: String, signedIn: () -> Unit) {
        viewModelScope.launch { if (repository.login(handoff, verifier)) signedIn() }
    }

    fun logout() {
        viewModelScope.launch { repository.logout() }
    }

    fun save(another: Boolean, approvedReview: DuplicateReview? = null, done: () -> Unit) {
        viewModelScope.launch(start = kotlinx.coroutines.CoroutineStart.UNDISPATCHED) {
            if (repository.save(another, approvedReview)) {
                if (BuildConfig.DEBUG) android.util.Log.d("PrestoSave", "Local save completed; updating destination")
                done()
            }
        }
    }

    fun delete(transactionId: String, done: () -> Unit) {
        viewModelScope.launch(start = kotlinx.coroutines.CoroutineStart.UNDISPATCHED) {
            if (repository.deleteTransaction(transactionId)) done()
        }
    }

    fun complete(month: String) {
        viewModelScope.launch { repository.completeIncome(month) }
    }

    fun restore() {
        viewModelScope.launch { repository.restore() }
    }

    fun review(id: String, done: () -> Unit) {
        viewModelScope.launch { if (repository.reviewBlocked(id)) done() }
    }

    fun retry(id: String) {
        viewModelScope.launch { repository.retryBlocked(id) }
    }
}
