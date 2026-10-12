package com.presto.nativeapp

import android.app.Application
import android.content.Context
import androidx.room.Room
import androidx.room.withTransaction
import com.presto.core.*
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.*
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.serialization.json.*

class PrestoApplication : Application() {
    val repository by lazy { FinanceRepository(this) }

    override fun onCreate() {
        super.onCreate()
        PartnerPush.initialize(this)
        repository.start()
    }
}

data class DuplicateReview(val owner: String, val draft: Draft, val matches: List<Transaction>, val addAnother: Boolean)
private class DuplicateReviewRequired : Exception()

data class AppState(
    val duplicateReview: DuplicateReview? = null,
    val ready: Boolean = false,
    val session: Session? = null,
    val owner: String = "demo",
    val snapshot: Snapshot = demoSnapshot(),
    val draft: Draft = defaultDraft(demoSnapshot()),
    val outbox: List<OutboxRow> = emptyList(),
    val syncing: Boolean = false,
    val saving: Boolean = false,
    val error: String? = null,
    val notice: String? = null,
    val recovery: RecoveryInfo? = null,
    val recoveryIssue: String? = null,
    val partnerNotifications: Boolean = false,
    val partnerNotificationStatus: String? = null,
) {
    val demo
        get() = owner == "demo"

    val pending
        get() = outbox.count { it.status != "acknowledged" }
}

class FinanceRepository(
    private val context: Context,
    val api: FinanceApi = FinanceApi(BuildConfig.API_URL),
    private val namespace: String = "presto-native",
    private val scheduleWork: Boolean = true,
) {
    private val diagnostics = Diagnostics()
    val logs = diagnostics.logs

    private fun failureDetails(action: String, failure: Throwable): String =
        com.presto.nativeapp.failureDetails(action, failure).also { diagnostics.record(it) }

    private val lifetime = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private val db =
        Room.databaseBuilder(context, FinanceDatabase::class.java, "$namespace.db").build()
    private val dao = db.finance()
    private val vault = Vault(context, namespace)
    private val prefs = context.getSharedPreferences("$namespace-owner", Context.MODE_PRIVATE)
    private val recoveryStore = RecoveryStore(context, vault)
    private val mutation = Mutex()
    private val entryWrite = Mutex()
    private val syncMutex = Mutex()
    private val authMutex = Mutex()
    private val partnerPush = PartnerPush(context, namespace)
    private val _state = MutableStateFlow(AppState(partnerNotifications = partnerPush.enabled))
    private var pushJob: Job? = null
    val state: StateFlow<AppState> = _state.asStateFlow()
    private var initialized = CompletableDeferred<Unit>()
    private var draftJob: Job? = null
    @Volatile private var lastReadAt: Long = 0

    fun refreshPushRegistration() {
        if (!scheduleWork || !BuildConfig.FCM_ENABLED || pushJob?.isActive == true) return
        pushJob = lifetime.launch {
            awaitReady()
            authMutex.withLock {
                val session = _state.value.session ?: return@withLock
                if (!partnerPush.enabled) {
                    try { api.removePush(session, partnerPush.installation) }
                    catch (e: Exception) {
                        if (e is CancellationException) throw e
                        _state.update { it.copy(partnerNotificationStatus = "Disabled on this phone; server cleanup will retry.") }
                    }
                    return@withLock
                }
                if (!androidx.core.app.NotificationManagerCompat.from(context).areNotificationsEnabled()) {
                    _state.update { it.copy(partnerNotificationStatus = "Allow notifications in Android settings to receive alerts.") }
                    return@withLock
                }
                try {
                    partnerPush.bind(session)
                    val token = partnerPush.token()
                    api.registerPush(session, partnerPush.installation, token)
                    if (!partnerPush.enabled) {
                        partnerPush.clearOwner()
                        api.removePush(session, partnerPush.installation)
                    } else {
                        _state.update { it.copy(partnerNotificationStatus = "Ready to receive partner transactions.") }
                    }
                } catch (e: Exception) {
                    if (e is CancellationException) throw e
                    _state.update { it.copy(partnerNotificationStatus = "Could not register notifications; will retry during sync.") }
                    diagnostics.record("Partner notification registration failed; will retry")
                }
            }
        }
    }

    fun setPartnerNotifications(enabled: Boolean) {
        partnerPush.setEnabled(enabled)
        _state.update { it.copy(partnerNotifications = enabled, partnerNotificationStatus = null) }
        // An in-flight registration sees the latest preference; queue a fresh pass if needed.
        lifetime.launch { pushJob?.join(); refreshPushRegistration() }
    }

    suspend fun notificationTransaction(id: String, subject: String): Transaction? {
        awaitReady()
        if (!PartnerPush.validId(id)) return null
        val session = _state.value.session ?: return null
        if (session.subject != subject) return null
        return try {
            val transaction = api.transaction(session, id)
            if (_state.value.session?.accessToken == session.accessToken) transaction else null
        } catch (e: Exception) {
            if (e is CancellationException) throw e
            _state.update { it.copy(error = "Could not open the transaction. It may have been deleted or the connection is unavailable.") }
            null
        }
    }

    private fun scope(owner: String = _state.value.owner) = "$owner|${api.origin}"

    private suspend fun rebuild(owner: String = _state.value.owner) {
        val key = scope(owner)
        val base =
            dao.cache(key)?.let { codec.decodeFromString<Snapshot>(it.snapshot) }
                ?: if (owner == "demo") demoSnapshot() else Snapshot(today(), today().take(7))
        val operations = dao.outbox(key)
        var categories = base.budgets
        val rows = base.recentTransactions.associateBy { it.id }.toMutableMap()
        operations.forEach { op ->
            val t = codec.decodeFromString<Transaction>(op.optimistic)
            val confirmed = rows[t.id]
            if (
                op.status == "acknowledged" &&
                    confirmed != null &&
                    confirmed.revision >= (op.confirmedRevision ?: 1)
            )
                return@forEach
            val previous = confirmed ?: op.original
                ?.takeIf { base.transactionCount > base.recentTransactions.size }
                ?.let { codec.decodeFromString<Transaction>(it) }
            previous?.let {
                categories =
                    budgetOverlay(categories, it, -1, base.budgetMonth)
            }
            categories = budgetOverlay(categories, t, month = base.budgetMonth)
            rows[t.id] = t.copy(pending = op.status != "acknowledged")
        }
        _state.update {
            it.copy(
                owner = owner,
                snapshot =
                    base.copy(
                        budgets = categories,
                        recentTransactions =
                            rows.values.sortedWith(
                                compareByDescending<Transaction> { it.effectiveDate }
                                    .thenByDescending { it.occurredAt ?: "" }
                                    .thenByDescending { it.id }
                            ),
                    ),
                outbox = operations,
            )
        }
    }

    fun start() {
        lifetime.launch {
            try {
                context.cacheDir
                    .listFiles()
                    ?.filter {
                        it.name.startsWith("recovery-") &&
                            System.currentTimeMillis() - it.lastModified() > 24 * 60 * 60 * 1000L
                    }
                    ?.forEach { it.delete() }
                val session =
                    runCatching { vault.session() }
                        .getOrElse {
                            if (it is CancellationException) throw it
                            _state.update { s ->
                                s.copy(error = "Saved session could not be opened. Sign in again. ${failureDetails("Open saved session", it)}")
                            }
                            null
                        }
                val owner = session?.owner ?: prefs.getString("last-owner", "demo") ?: "demo"
                mutation.withLock {
                    dao.outbox(scope(owner)).forEach { operation ->
                        operation.error?.let { diagnostics.record("Saved sync failure: $it") }
                        repairBookkeepingRevisions(operation)?.let { repaired ->
                            dao.update(repaired)
                            diagnostics.record("Repaired bookkeeping revision list for operation ${operation.id}; retained real revisions")
                        }
                    }
                    rebuild(owner)
                    val draft =
                        runCatching {
                                dao.draft(scope(owner))?.let {
                                    codec.decodeFromString<Draft>(it.draft)
                                }
                            }
                            .getOrNull() ?: defaultDraft(_state.value.snapshot)
                    _state.update { it.copy(ready = true, session = session, draft = draft) }
                }
                _state.update { it.copy(recovery = recoveryStore.latestInfo(scope(owner))) }
            } catch (e: Exception) {
                if (e is CancellationException) throw e
                _state.update { it.copy(ready = true, error = failureDetails("Open saved finance data", e)) }
            } finally {
                initialized.complete(Unit)
            }
            if (_state.value.session != null) {
                if (scheduleWork) SyncScheduler.schedule(context, scope())
                sync()
            }
        }
    }

    suspend fun awaitReady() {
        initialized.await()
    }

    suspend fun close() {
        draftJob?.join()
        lifetime.cancel()
        db.close()
    }

    fun refreshIfStale() {
        if (_state.value.session != null && System.currentTimeMillis() - lastReadAt > 60_000)
            lifetime.launch { sync() }
    }

    fun dismissMessage() {
        _state.update { it.copy(error = null, notice = null) }
    }

    fun updateDraft(draft: Draft, userEdited: Boolean = true, expected: Draft? = null) {
        if (_state.value.saving) return
        val next = if (userEdited) draft.copy(userTouched = true) else draft
        _state.update {
            if (it.saving || (expected != null && it.draft != expected)) it else it.copy(draft = next)
        }
        if (_state.value.draft != next) return
        val key = scope()
        draftJob?.cancel()
        draftJob =
            lifetime.launch {
                try {
                    delay(250) // Coalesce typing; persistDraft and save still flush the latest draft.
                    mutation.withLock {
                        // A save or account switch may have replaced this draft while the write waited.
                        if (scope() != key || _state.value.draft != next) return@withLock
                        dao.putDraft(DraftRow(key, codec.encodeToString(Draft.serializer(), next)))
                    }
                } catch (e: Exception) {
                    if (e is CancellationException) throw e
                    if (scope() == key) _state.update {
                        it.copy(error = failureDetails("Save unfinished draft", e))
                    }
                }
            }
    }

    suspend fun persistDraft(): Boolean = withContext(Dispatchers.IO) {
        awaitReady()
        try {
            draftJob?.cancelAndJoin()
            mutation.withLock {
                dao.putDraft(
                    DraftRow(scope(), codec.encodeToString(Draft.serializer(), _state.value.draft))
                )
            }
            true
        } catch (e: Exception) {
            if (e is CancellationException) throw e
            _state.update { it.copy(error = failureDetails("Save unfinished draft", e)) }
            false
        }
    }

    suspend fun login(handoff: String, verifier: String): Boolean =
        authMutex.withLock {
            awaitReady()
            if (!persistDraft()) return@withLock false
            _state.update { it.copy(syncing = true, error = null) }
            try {
                val session = api.exchange(handoff, verifier)
                partnerPush.clearOwner()
                vault.saveSession(session)
                prefs.edit().putString("last-owner", session.owner).commit()
                mutation.withLock {
                    // A failed cache/draft read must never leave a previous owner's token active.
                    _state.update { it.copy(session = null) }
                    rebuild(session.owner)
                    val draft =
                        dao.draft(scope(session.owner))?.let {
                            codec.decodeFromString<Draft>(it.draft)
                        } ?: defaultDraft(_state.value.snapshot)
                    _state.update {
                        it.copy(
                            session = session,
                            draft = draft,
                            recovery = null,
                            recoveryIssue = null,
                        )
                    }
                }
                if (scheduleWork) SyncScheduler.schedule(context, scope())
                sync()
                true
            } catch (e: Exception) {
                if (e is CancellationException) throw e
                _state.update { it.copy(error = failureDetails("Sign in", e)) }
                false
            } finally {
                _state.update { it.copy(syncing = false) }
            }
        }

    suspend fun logout() =
        authMutex.withLock {
            if (!persistDraft()) return@withLock
            val oldScope = scope()
            val session = _state.value.session
            try {
                session?.let { api.logout(it) }
            } catch (e: Exception) {
                if (e is CancellationException) throw e
                if (e !is ApiFailure || e.status != 401) {
                    _state.update { it.copy(error = failureDetails("Sign out; server session could not be revoked", e)) }
                    return@withLock
                }
            }
            SyncScheduler.cancel(context, oldScope)
            partnerPush.clearOwner()
            vault.clearSession()
            prefs.edit().putString("last-owner", "demo").commit()
            mutation.withLock {
                _state.update { it.copy(session = null) }
                rebuild("demo")
                _state.update {
                    it.copy(
                        session = null,
                        draft =
                            dao.draft(scope("demo"))?.let { r ->
                                codec.decodeFromString<Draft>(r.draft)
                            } ?: defaultDraft(it.snapshot),
                        recovery = null,
                        recoveryIssue = null,
                        error = null,
                    )
                }
            }
            SyncScheduler.notification(context, 0)
        }

    private fun expire(session: Session) {
        if (_state.value.session?.accessToken == session.accessToken) {
            diagnostics.record("Session expired; sign in again to sync retained entries")
            partnerPush.clearOwner()
            vault.clearSession()
            _state.update {
                it.copy(
                    session = null,
                    error = "Your session expired. Sign in to sync your saved entries.",
                    notice = null,
                )
            }
        }
    }

    fun dismissDuplicateReview() {
        _state.update { it.copy(duplicateReview = null) }
    }

    suspend fun save(addAnother: Boolean, approvedReview: DuplicateReview? = null): Boolean {
        // Admission is immediate, shared with deletion, and independent of StateFlow equality.
        if (!entryWrite.tryLock()) return false
        val acceptedOwner = _state.value.owner
        val started = android.os.SystemClock.elapsedRealtime()
        _state.update { it.copy(saving = true, error = null) }
        // An accepted disk write belongs to the repository, not the screen awaiting its result.
        return lifetime.async {
            try {
                awaitReady()
                draftJob?.cancel()
                // No join: autosave uses the same lock and checks that its draft is still current.
                val (savedScope, demo) = commitEntry(addAnother, acceptedOwner, approvedReview)
                syncSavedEntry(savedScope, demo)
                true
            } catch (e: DuplicateReviewRequired) {
                false
            } catch (e: Exception) {
                if (e is CancellationException) throw e
                _state.update { it.copy(error = failureDetails("Save entry", e)) }
                false
            } finally {
                _state.update { it.copy(saving = false) }
                entryWrite.unlock()
                diagnostics.record("Save finished in ${android.os.SystemClock.elapsedRealtime() - started} ms; ${_state.value.pending} waiting")
            }
        }.await()
    }

    /** All validation and durable writes run on the repository's IO scope. No network calls. */
    private suspend fun commitEntry(addAnother: Boolean, acceptedOwner: String, approvedReview: DuplicateReview?): Pair<String, Boolean> =
        mutation.withLock {
            val current = _state.value
            require(current.owner == acceptedOwner) { "The account changed. Review this entry before saving." }
            // A quick save can beat the UI's asynchronous history suggestions.
            val s = current.copy(draft = suggestDraftLinks(current.draft, current.snapshot))
            val (payload, optimistic) = writeEntry(s.draft, s.snapshot)
            require(
                s.outbox.none {
                    it.transactionId == payload.transactionId &&
                        it.id != s.draft.replacesOperation
                }
            ) {
                "This transaction already has a pending write."
            }
            val matches = if (s.draft.original == null && s.draft.replacesOperation == null)
                possibleDuplicates(optimistic, s.snapshot.recentTransactions) else emptyList()
            val approved = approvedReview != null && approvedReview == current.duplicateReview &&
                approvedReview.owner == acceptedOwner && approvedReview.draft == s.draft &&
                approvedReview.addAnother == addAnother &&
                approvedReview.matches.map { it.id }.toSet() == matches.map { it.id }.toSet()
            if (matches.isNotEmpty() && !approved) {
                _state.update { it.copy(duplicateReview = DuplicateReview(acceptedOwner, s.draft, matches, addAnother)) }
                throw DuplicateReviewRequired()
            }
            _state.update { it.copy(duplicateReview = null) }
            var nextDraft = defaultDraft(s.snapshot)
            db.withTransaction {
                if (s.demo) {
                    val rows =
                        listOf(
                            optimistic.copy(
                                pending = false,
                                revision = (s.draft.original?.revision ?: 0) + 1,
                            )
                        ) + s.snapshot.recentTransactions.filter { it.id != optimistic.id }
                    var categories = s.snapshot.budgets
                    s.draft.original?.let {
                        categories = budgetOverlay(categories, it, -1, s.snapshot.budgetMonth)
                    }
                    val snapshot =
                        s.snapshot.copy(
                            recentTransactions = rows,
                            budgets =
                                budgetOverlay(
                                    categories,
                                    optimistic,
                                    month = s.snapshot.budgetMonth,
                                ),
                            transactionCount = rows.size,
                        )
                    dao.putCache(
                        CacheRow(scope(), codec.encodeToString(Snapshot.serializer(), snapshot))
                    )
                } else
                    dao.enqueue(
                        OutboxRow(
                            payload.operationId,
                            scope(),
                            payload.transactionId,
                            codec.encodeToString(WritePayload.serializer(), payload),
                            codec.encodeToString(Transaction.serializer(), optimistic),
                            s.draft.original?.let {
                                codec.encodeToString(Transaction.serializer(), it)
                            },
                            s.draft.original != null,
                        )
                    )
                if (s.draft.balancing && addAnother) {
                    val categories =
                        budgetOverlay(
                            s.snapshot.budgets,
                            optimistic,
                            month = s.snapshot.budgetMonth,
                        )
                    val next =
                        suggestBalance(categories, s.draft.categoryFrom, s.draft.categoryTo)
                    nextDraft =
                        if (next.amount > ZERO)
                            s.draft.copy(
                                amount = next.amount.toPlainString(),
                                categoryFrom = next.source?.categoryId,
                                categoryTo = next.destination?.categoryId,
                                original = null,
                                correctionOf = "",
                            )
                        else defaultDraft(s.snapshot)
                } else if (addAnother)
                    nextDraft =
                        s.draft.copy(
                            amount = "",
                            description = "",
                            note = "",
                            toAmount = "",
                            original = null,
                            correctionOf = "",
                            userTouched = true,
                        )
                s.draft.replacesOperation?.let { id ->
                    require(s.outbox.any { it.id == id && it.status == "blocked" }) {
                        "The failed operation changed. Review it again."
                    }
                    dao.remove(id)
                    nextDraft =
                        nextDraft.copy(replacesOperation = null, queuedTransactionId = null)
                }
                dao.putDraft(
                    DraftRow(scope(), codec.encodeToString(Draft.serializer(), nextDraft))
                )
            }
            rebuild()
            _state.update {
                it.copy(
                    draft = nextDraft,
                    notice =
                        if (s.demo) "Saved in local demo" else "Saved on this device; syncing",
                )
            }
            scope(s.owner) to s.demo
        }

    private fun syncSavedEntry(savedScope: String, demo: Boolean) {
        lifetime.launch {
            if (!demo) {
                // A scheduler failure must not turn a committed entry into a failed save.
                if (scheduleWork) runCatching { SyncScheduler.schedule(context, savedScope) }
                    .onFailure {
                        if (it is CancellationException) throw it
                        failureDetails("Schedule saved entry sync", it)
                    }
                sync()
            }
        }
    }

    suspend fun deleteTransaction(transactionId: String): Boolean {
        awaitReady()
        if (!entryWrite.tryLock()) return false
        _state.update { it.copy(saving = true, error = null) }
        val deleted = try {
            withContext(Dispatchers.IO) {
                syncMutex.withLock {
                    mutation.withLock {
                        val s = _state.value
                        val key = scope()
                        val t = s.snapshot.recentTransactions.find { it.id == transactionId }
                            ?: error("This entry is no longer available. Refresh activity.")
                        require(t.canEdit && !t.pending && t.kind in listOf("expense", "income", "transfer", "reallocate") && !t.hasCategoryCorrection) {
                            "This entry cannot be deleted."
                        }
                        require(s.outbox.isEmpty()) { "Sync or resolve pending entries before deleting." }
                        val retryKey = "delete|$key|$transactionId"
                        if (!s.demo) {
                            val session = s.session ?: error("Sign in and connect to delete this entry.")
                            // Persist the exact request before sending so a lost response can be retried safely.
                            val payload = prefs.getString(retryKey, null) ?: buildJsonObject {
                                put("operationId", uuidV7())
                                put("transactionId", t.id)
                                put("expectedRevision", t.revision)
                                putJsonArray("expectedAccountRevisions") {
                                    s.snapshot.accounts.filter { it.id == t.fromAccountId || it.id == t.toAccountId }.forEach { account ->
                                        add(buildJsonObject {
                                            put("accountId", account.id)
                                            put("revision", account.ledgerRevision)
                                        })
                                    }
                                }
                            }.toString().also {
                                check(prefs.edit().putString(retryKey, it).commit()) { "Could not save deletion request." }
                            }
                            try {
                                val response = api.deleteTransaction(session, payload)
                                check(response["deleted"]?.jsonPrimitive?.booleanOrNull == true &&
                                    response["transactionId"]?.jsonPrimitive?.content == t.id) {
                                    "The server did not confirm deletion. Retry this entry."
                                }
                            } catch (e: ApiFailure) {
                                if (e.status in 400..499 && e.status !in listOf(408, 429, 401))
                                    prefs.edit().remove(retryKey).commit()
                                throw e
                            }
                        }
                        val snapshot = s.snapshot.copy(
                            recentTransactions = s.snapshot.recentTransactions.filter { it.id != t.id },
                            transactionCount = (s.snapshot.transactionCount - 1).coerceAtLeast(0),
                            budgets = budgetOverlay(s.snapshot.budgets, t, -1, s.snapshot.budgetMonth),
                        )
                        dao.putCache(CacheRow(key, codec.encodeToString(Snapshot.serializer(), snapshot)))
                        prefs.edit().remove(retryKey).commit()
                        rebuild()
                        _state.update { it.copy(notice = "Transaction deleted") }
                        true
                    }
                }
            }
        } catch (e: Exception) {
            if (e is CancellationException) throw e
            if (e is ApiFailure && e.status == 401) _state.value.session?.let { expire(it) }
            else _state.update { it.copy(error = failureDetails("Delete transaction $transactionId", e)) }
            false
        } finally {
            _state.update { it.copy(saving = false) }
            entryWrite.unlock()
        }
        if (deleted && !_state.value.demo) lifetime.launch { sync() }
        return deleted
    }

    // Room suspends only its queries; parsing, overlays, diagnostics and recovery also need IO.
    suspend fun sync(): Boolean = withContext(Dispatchers.IO) { syncOnIo() }

    private suspend fun syncOnIo(): Boolean {
        awaitReady()
        syncMutex.lock()
        var retry = false
        var acknowledgedWrite = false
        var hadPendingWrites = false
        val started = android.os.SystemClock.elapsedRealtime()
        val session = _state.value.session
        if (session == null) {
            syncMutex.unlock()
            return true
        }
        var stage = "validate session"
        val owner = session.owner
        val key = scope(owner)
        _state.update { it.copy(syncing = true, error = null) }
        try {
            if (session.expiresAt?.let { it <= System.currentTimeMillis() / 1000 } == true || !api.validate(session)) {
                expire(session)
                return true
            }
            refreshPushRegistration()
            stage = "load pending writes"
            val operations = dao.outbox(key)
            hadPendingWrites = operations.any { it.status != "acknowledged" }
            SyncScheduler.notification(context, operations.count { it.status != "acknowledged" })
            for (operation in operations) {
                if (_state.value.session?.accessToken != session.accessToken) break
                if (operation.status != "pending") continue
                stage = "write transaction ${operation.transactionId}; operation ${operation.id}"
                try {
                    diagnostics.record("Sending ${if (operation.editing) "edit" else "create"} operation ${operation.id}")
                    val response = api.write(session, operation.payload, operation.editing)
                    val revision =
                        response["revision"]?.jsonPrimitive?.longOrNull
                            ?: if (operation.editing)
                                (codec
                                    .decodeFromString<WritePayload>(operation.payload)
                                    .expectedRevision ?: 0) + 1
                            else 1
                    mutation.withLock {
                        dao.update(
                            operation.copy(
                                status = "acknowledged",
                                confirmedRevision = revision,
                                error = null,
                            )
                        )
                        if (_state.value.owner == owner) rebuild(owner)
                    }
                    acknowledgedWrite = true
                    diagnostics.record("Operation ${operation.id} acknowledged; revision $revision")
                } catch (e: Exception) {
                    if (e is CancellationException) throw e
                    if (e is ApiFailure && e.status == 401) {
                        expire(session)
                        break
                    }
                    val blocked =
                        e is ApiFailure && e.status in 400..499 && e.status !in listOf(408, 429)
                    mutation.withLock {
                        dao.update(
                            operation.copy(
                                status = if (blocked) "blocked" else "pending",
                                error = failureDetails("${if (operation.editing) "Edit" else "Create"} transaction ${operation.transactionId}; operation ${operation.id}", e),
                            )
                        )
                        if (_state.value.owner == owner) {
                            rebuild(owner)
                            _state.update { it.copy(notice = null) }
                        }
                    }
                    if (!blocked) {
                        retry = true
                        break
                    }
                }
            }
            if (_state.value.session?.accessToken == session.accessToken) {
                stage = "download finance snapshot"
                val fresh = api.snapshot(session)
                stage = "save finance snapshot locally"
                mutation.withLock {
                    db.withTransaction {
                        val old =
                            dao.cache(key)?.let { codec.decodeFromString<Snapshot>(it.snapshot) }
                        val latest = fresh.recentTransactions.associateBy { it.id }
                        val acknowledged = dao.outbox(key).filter { it.status == "acknowledged" }
                        val confirmedLocal =
                            acknowledged
                                .map { op ->
                                    codec
                                        .decodeFromString<Transaction>(op.optimistic)
                                        .copy(pending = false, revision = op.confirmedRevision ?: 1)
                                }
                                .associateBy { it.id }
                        val merged =
                            ((if (fresh.transactionCount <= fresh.recentTransactions.size) emptyList()
                                else old?.recentTransactions.orEmpty()).filter {
                                    it.id !in latest && it.id !in confirmedLocal
                                } +
                                    confirmedLocal.values.filter {
                                        it.id !in latest && fresh.transactionCount > fresh.recentTransactions.size
                                    } +
                                    fresh.recentTransactions)
                                .sortedByDescending { it.effectiveDate }
                        dao.putCache(
                            CacheRow(
                                key,
                                codec.encodeToString(
                                    Snapshot.serializer(),
                                    fresh.copy(recentTransactions = merged),
                                ),
                            )
                        )
                        acknowledged.forEach { dao.remove(it.id) }
                    }
                    lastReadAt = System.currentTimeMillis()
                    if (_state.value.owner == owner) {
                        rebuild(owner)
                        if (!_state.value.draft.touched) {
                            val draft = defaultDraft(_state.value.snapshot)
                            _state.update { it.copy(draft = draft) }
                        }
                    }
                }
                if (recoveryStore.reserve(key))
                    runCatching {
                            val temporary =
                                java.io.File.createTempFile(
                                    "recovery-download-",
                                    ".json",
                                    context.cacheDir,
                                )
                            val info =
                                try {
                                    val digest = api.backupFile(session, temporary)
                                    recoveryStore.saveFile(key, temporary, digest)
                                } finally {
                                    temporary.delete()
                                }
                            if (_state.value.owner == owner)
                                _state.update { it.copy(recovery = info, recoveryIssue = null) }
                        }
                        .onFailure {
                            if (it is CancellationException) throw it
                            if (_state.value.owner == owner)
                                _state.update { s -> s.copy(recoveryIssue = failureDetails("Download and store recovery snapshot; server ${api.origin}", it)) }
                        }
            }
        } catch (e: Exception) {
            if (e is CancellationException) throw e
            if (e is ApiFailure && e.status == 401) expire(session)
            else {
                retry = true
                if (_state.value.owner == owner) _state.update { it.copy(error = failureDetails("Sync: $stage; owner $owner; server ${api.origin}", e)) }
            }
        } finally {
            if (_state.value.owner == owner) _state.update {
                it.copy(syncing = false, notice =
                    if (it.pending == 0 && (acknowledgedWrite || it.notice == "Saved on this device; syncing"))
                        "Synced successfully" else it.notice)
            }
            val elapsed = android.os.SystemClock.elapsedRealtime() - started
            if (hadPendingWrites || acknowledgedWrite || retry || elapsed >= 2_000)
                diagnostics.record("Sync finished in $elapsed ms; ${_state.value.pending} waiting, ${_state.value.outbox.count { it.status == "blocked" }} need review")
            SyncScheduler.notification(context, _state.value.pending)
            syncMutex.unlock()
        }
        return !retry
    }

    suspend fun reviewBlocked(id: String): Boolean = withContext(Dispatchers.IO) {
        reviewBlockedOnIo(id)
    }

    private suspend fun reviewBlockedOnIo(id: String): Boolean =
        mutation.withLock {
            try {
                val row =
                    dao.outbox(scope()).firstOrNull { it.id == id && it.status == "blocked" }
                        ?: error("This operation is no longer blocked.")
                val optimistic = codec.decodeFromString<Transaction>(row.optimistic)
                val confirmed =
                    dao.cache(scope())?.let {
                        codec.decodeFromString<Snapshot>(it.snapshot).recentTransactions.find { t ->
                            t.id == row.transactionId
                        }
                    }
                if (row.editing)
                    require(confirmed?.canEdit == true) {
                        "This entry is no longer editable. Record a correction or review it in the browser workspace."
                    }
                val original = if (row.editing) confirmed else null
                val draft =
                    editDraft(optimistic.copy(pending = false), _state.value.snapshot)
                        .copy(
                            original = original,
                            replacesOperation = id,
                            queuedTransactionId = row.transactionId,
                            showMore = true,
                        )
                dao.putDraft(DraftRow(scope(), codec.encodeToString(Draft.serializer(), draft)))
                _state.update {
                    it.copy(
                        draft = draft,
                        error = null,
                        notice =
                            "Review saved values and current balances before saving a replacement.",
                    )
                }
                true
            } catch (e: Exception) {
                if (e is CancellationException) throw e
                _state.update { it.copy(error = failureDetails("Review blocked entry", e)) }
                false
            }
        }

    suspend fun retryBlocked(id: String) {
        mutation.withLock {
            dao.outbox(scope())
                .find { it.id == id && it.status == "blocked" }
                ?.let {
                    dao.update(it.copy(status = "pending", error = null))
                    rebuild()
                }
        }
        if (scheduleWork) SyncScheduler.schedule(context, scope())
        sync()
    }

    suspend fun completeIncome(month: String) {
        val session = _state.value.session ?: return
        _state.update { it.copy(syncing = true, error = null) }
        try {
            require(
                _state.value.outbox.none {
                    codec.decodeFromString<Transaction>(it.optimistic).incomeTreatment == "salary"
                }
            ) {
                "Sync pending salary receipts first."
            }
            api.completeIncome(session, month)
            sync()
        } catch (e: Exception) {
            if (e is CancellationException) throw e
            if (e is ApiFailure && e.status == 401) expire(session)
            else _state.update { it.copy(error = failureDetails("Complete income", e)) }
        } finally {
            _state.update { it.copy(syncing = false) }
        }
    }

    suspend fun restore() {
        val session = _state.value.session ?: return
        _state.update { it.copy(syncing = true, error = null) }
        try {
            require(_state.value.outbox.isEmpty()) {
                "Resolve pending writes before restoring the server."
            }
            val info =
                recoveryStore.latestInfo(scope())
                    ?: error("No verified recovery snapshot is available.")
            val temporary = recoveryStore.materialize(scope(), info)
            try {
                api.restoreFile(session, temporary, info.sha256)
            } finally {
                temporary.delete()
            }
            _state.update {
                it.copy(notice = "Finance database restored from verified phone snapshot")
            }
            sync()
        } catch (e: Exception) {
            if (e is CancellationException) throw e
            if (e is ApiFailure && e.status == 401) expire(session)
            else _state.update { it.copy(error = failureDetails("Restore finance database", e)) }
        } finally {
            _state.update { it.copy(syncing = false) }
        }
    }
}
