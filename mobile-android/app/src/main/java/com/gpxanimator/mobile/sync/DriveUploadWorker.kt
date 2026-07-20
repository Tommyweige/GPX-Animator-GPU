package com.gpxanimator.mobile.sync

import android.content.Context
import androidx.work.CoroutineWorker
import androidx.work.WorkerParameters
import com.gpxanimator.mobile.GpxAnimatorRideApplication
import com.gpxanimator.mobile.data.RideDatabase
import com.gpxanimator.mobile.data.RideRepository
import com.gpxanimator.mobile.data.SyncState
import com.gpxanimator.mobile.data.TripEntity
import java.io.File
import java.io.IOException
import java.net.URL
import java.security.MessageDigest
import java.time.Instant
import java.time.ZoneId
import java.time.ZoneOffset
import java.util.concurrent.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.runInterruptible
import kotlinx.coroutines.withContext

class DriveUploadWorker(
    appContext: Context,
    workerParameters: WorkerParameters,
) : CoroutineWorker(appContext, workerParameters) {
    @Volatile
    private var activeClient: DriveRestClient? = null

    private val expectedSyncEpoch: Long by lazy {
        inputData.getLong(INPUT_SYNC_EPOCH, DriveSyncEpoch.current(applicationContext))
    }

    private val repository: RideRepository by lazy {
        val application = applicationContext as? GpxAnimatorRideApplication
        application?.container?.rideRepository
            ?: RideRepository(RideDatabase.create(applicationContext))
    }

    override suspend fun doWork(): Result {
        if (!isCurrentSyncEpoch()) return Result.failure()
        val tripId = inputData.getString(INPUT_TRIP_ID)?.takeIf(String::isNotBlank)
            ?: return Result.failure()
        val trip = repository.getTrip(tripId) ?: return Result.failure()
        val localFile = trip.localGpxPath?.let(::File)
        if (localFile == null || !localFile.isFile) {
            updateSyncState(
                tripId = tripId,
                syncState = SyncState.FAILED,
                errorMessage = "The local GPX file is missing.",
            )
            return Result.failure()
        }

        updateSyncState(tripId, SyncState.UPLOADING, null)
        return when (val authorization =
            DriveAuthorizationGateway(applicationContext).acquireTokenSilently()) {
            is SilentAuthorizationResult.Authorized ->
                uploadAuthorized(tripId, localFile, authorization.accessToken)

            SilentAuthorizationResult.UserActionRequired -> {
                updateSyncState(
                    tripId,
                    SyncState.AUTH_REQUIRED,
                    "Connect Google Drive to continue syncing.",
                )
                Result.failure()
            }

            is SilentAuthorizationResult.RetryableFailure -> {
                updateSyncState(tripId, SyncState.PENDING, authorization.message)
                Result.retry()
            }

            is SilentAuthorizationResult.PermanentFailure -> {
                updateSyncState(tripId, SyncState.FAILED, authorization.message)
                Result.failure()
            }
        }
    }

    private suspend fun uploadAuthorized(
        tripId: String,
        localFile: File,
        accessToken: String,
    ): Result =
        withContext(Dispatchers.IO) {
            val client = DriveRestClient(accessToken)
            val cancellationHandle =
                currentCoroutineContext()[Job]?.invokeOnCompletion { client.cancel() }
            activeClient = client
            try {
                var trip = repository.getTrip(tripId) ?: return@withContext Result.failure()
                val sha256 = localFile.sha256()
                if (trip.localGpxSha256 != sha256) {
                    trip = trip.copy(localGpxSha256 = sha256, updatedAtEpochMillis = now())
                    repository.updateTrip(trip)
                }

                val year =
                    Instant.ofEpochMilli(trip.startedAtEpochMillis)
                        .atZone(
                            runCatching { ZoneId.of(trip.startZoneId) }
                                .getOrDefault(ZoneOffset.UTC),
                        )
                        .year
                val folderId =
                    DriveFolderResolver(applicationContext, client)
                        .resolveYearFolder(year, trip.driveFolderId)
                if (trip.driveFolderId != folderId) {
                    trip = trip.copy(driveFolderId = folderId, updatedAtEpochMillis = now())
                    repository.updateTrip(trip)
                }

                val knownRemote = trip.driveFileId?.let(client::getFile)
                if (knownRemote.matches(tripId, sha256)) {
                    ensureUploadActive()
                    markSynced(tripId, knownRemote!!.id, folderId)
                    return@withContext Result.success()
                }

                val existingRemote = client.findUploadedFile(folderId, tripId, sha256)
                if (existingRemote != null) {
                    ensureUploadActive()
                    markSynced(tripId, existingRemote.id, folderId)
                    return@withContext Result.success()
                }

                val remoteFileId =
                    if (knownRemote == null && !trip.driveFileId.isNullOrBlank()) {
                        trip.driveFileId
                    } else {
                        client.generateFileId()
                    }
                persistRemoteIdentity(tripId, folderId, remoteFileId)

                val uploaded =
                    uploadWithCollisionRecovery(
                        client = client,
                        tripId = tripId,
                        folderId = folderId,
                        sha256 = sha256,
                        localFile = localFile,
                        remoteFileId = remoteFileId,
                    )
                ensureUploadActive()
                markSynced(tripId, uploaded.id, folderId)
                Result.success()
            } catch (error: CancellationException) {
                throw error
            } catch (error: DriveHttpException) {
                handleHttpFailure(tripId, error, accessToken)
            } catch (_: IOException) {
                if (isStopped) throw CancellationException("Drive upload was stopped.")
                updateSyncState(
                    tripId,
                    SyncState.PENDING,
                    "Google Drive could not be reached. Sync will retry.",
                )
                Result.retry()
            } catch (_: Exception) {
                updateSyncState(
                    tripId,
                    SyncState.FAILED,
                    "Google Drive sync failed permanently.",
                )
                Result.failure()
            } finally {
                cancellationHandle?.dispose()
                activeClient?.cancel()
                activeClient = null
            }
        }

    private suspend fun uploadWithCollisionRecovery(
        client: DriveRestClient,
        tripId: String,
        folderId: String,
        sha256: String,
        localFile: File,
        remoteFileId: String,
    ): DriveFile {
        val metadata =
            DriveUploadMetadata(
                id = remoteFileId,
                name = localFile.name,
                parentId = folderId,
                tripId = tripId,
                sha256 = sha256,
            )
        return try {
            performUpload(client, localFile, metadata, sha256)
        } catch (error: DriveHttpException) {
            if (error.statusCode != 409) throw error

            client.findUploadedFile(folderId, tripId, sha256)?.let { return it }
            val replacementId = client.generateFileId()
            persistRemoteIdentity(tripId, folderId, replacementId)
            performUpload(client, localFile, metadata.copy(id = replacementId), sha256)
        }
    }

    private suspend fun performUpload(
        client: DriveRestClient,
        localFile: File,
        metadata: DriveUploadMetadata,
        sha256: String,
    ): DriveFile {
        ensureUploadActive()
        if (client.shouldUseMultipart(localFile)) {
            return driveCall(client) { client.uploadMultipart(localFile, metadata) }
        }

        var retriedExpiredSession = false
        while (true) {
            var sessionInUse: URL? = null
            try {
                val trip =
                    repository.getTrip(metadata.tripId) ?: throw IOException("Trip not found.")
                val savedSessionUrl =
                    trip.driveUploadSessionUrl?.takeIf {
                        trip.driveUploadSessionSha256 == sha256 &&
                                trip.driveUploadSessionLength == localFile.length() &&
                                trip.driveFileId == metadata.id
                    }
                if (savedSessionUrl != null) {
                    val sessionUrl = runCatching { URL(savedSessionUrl) }.getOrNull()
                    if (sessionUrl != null) {
                        sessionInUse = sessionUrl
                        when (val status =
                            driveCall(client) {
                                client.queryResumableSession(sessionUrl, localFile.length())
                            }) {
                            is ResumableSessionStatus.Complete -> {
                                clearUploadSession(metadata.tripId)
                                return status.file
                            }

                            is ResumableSessionStatus.InProgress ->
                                return driveCall(client) {
                                    client.uploadResumable(
                                        sessionUrl = sessionUrl,
                                        file = localFile,
                                        startOffset = status.nextOffset,
                                        shouldContinue = { !isStopped && isCurrentSyncEpoch() },
                                    )
                                }

                            ResumableSessionStatus.Expired -> clearUploadSession(metadata.tripId)
                        }
                    } else {
                        clearUploadSession(metadata.tripId)
                    }
                }

                ensureUploadActive()
                val newSessionUrl = driveCall(client) {
                    client.initiateResumable(localFile, metadata)
                }
                sessionInUse = newSessionUrl
                persistUploadSession(metadata.tripId, newSessionUrl, sha256, localFile.length())
                return driveCall(client) {
                    client.uploadResumable(
                        sessionUrl = newSessionUrl,
                        file = localFile,
                        startOffset = 0L,
                        shouldContinue = { !isStopped && isCurrentSyncEpoch() },
                    )
                }
            } catch (error: DriveHttpException) {
                if (shouldRestartExpiredResumableSession(
                        error,
                        sessionInUse,
                        retriedExpiredSession
                    )
                ) {
                    clearUploadSession(metadata.tripId)
                    retriedExpiredSession = true
                    continue
                }
                throw error
            }
        }
    }

    private suspend fun <T> driveCall(client: DriveRestClient, block: () -> T): T {
        val cancellationHandle =
            currentCoroutineContext()[Job]?.invokeOnCompletion { client.cancel() }
        return try {
            runInterruptible { block() }
        } finally {
            cancellationHandle?.dispose()
        }
    }

    private suspend fun handleHttpFailure(
        tripId: String,
        error: DriveHttpException,
        accessToken: String,
    ): Result {
        val message =
            buildString {
                append("Google Drive returned HTTP ")
                append(error.statusCode)
                error.driveReason?.takeIf(String::isNotBlank)?.let {
                    append(" (")
                    append(it)
                    append(").")
                }
            }
        return when (DriveErrorClassifier.classifyHttp(error.statusCode, error.driveReason)) {
            DriveFailureKind.AUTH_REQUIRED -> {
                val cleared =
                    runAttemptCount == 0 &&
                            DriveAuthorizationGateway(applicationContext).clearToken(accessToken)
                if (cleared) {
                    updateSyncState(
                        tripId,
                        SyncState.PENDING,
                        "Google authorization will be refreshed before retrying.",
                    )
                    Result.retry()
                } else {
                    updateSyncState(
                        tripId,
                        SyncState.AUTH_REQUIRED,
                        "Reconnect Google Drive to continue syncing.",
                    )
                    Result.failure()
                }
            }

            DriveFailureKind.RETRYABLE -> {
                updateSyncState(tripId, SyncState.PENDING, message)
                Result.retry()
            }

            DriveFailureKind.PERMANENT -> {
                updateSyncState(tripId, SyncState.FAILED, message)
                Result.failure()
            }
        }
    }

    private suspend fun persistRemoteIdentity(
        tripId: String,
        folderId: String,
        remoteFileId: String,
    ) {
        updateTrip(tripId) {
            val keepSession = it.driveFileId == remoteFileId
            it.copy(
                driveFolderId = folderId,
                driveFileId = remoteFileId,
                syncState = SyncState.UPLOADING,
                driveUploadSessionUrl = it.driveUploadSessionUrl.takeIf { keepSession },
                driveUploadSessionSha256 = it.driveUploadSessionSha256.takeIf { keepSession },
                driveUploadSessionLength = it.driveUploadSessionLength.takeIf { keepSession },
                lastError = null,
                updatedAtEpochMillis = now(),
            )
        }
    }

    private suspend fun persistUploadSession(
        tripId: String,
        sessionUrl: URL,
        sha256: String,
        length: Long,
    ) {
        updateTrip(tripId) {
            it.copy(
                driveUploadSessionUrl = sessionUrl.toExternalForm(),
                driveUploadSessionSha256 = sha256,
                driveUploadSessionLength = length,
                updatedAtEpochMillis = now(),
            )
        }
    }

    private suspend fun clearUploadSession(tripId: String) {
        updateTrip(tripId) {
            it.copy(
                driveUploadSessionUrl = null,
                driveUploadSessionSha256 = null,
                driveUploadSessionLength = null,
                updatedAtEpochMillis = now(),
            )
        }
    }

    private suspend fun markSynced(
        tripId: String,
        remoteFileId: String,
        folderId: String,
    ) {
        updateTrip(tripId) {
            it.copy(
                driveFolderId = folderId,
                driveFileId = remoteFileId,
                syncState = SyncState.SYNCED,
                driveUploadSessionUrl = null,
                driveUploadSessionSha256 = null,
                driveUploadSessionLength = null,
                lastError = null,
                updatedAtEpochMillis = now(),
            )
        }
    }

    private suspend fun updateSyncState(
        tripId: String,
        syncState: SyncState,
        errorMessage: String?,
    ) {
        updateTrip(tripId) {
            it.copy(
                syncState = syncState,
                lastError = errorMessage,
                updatedAtEpochMillis = now(),
            )
        }
    }

    private suspend fun updateTrip(
        tripId: String,
        transform: (TripEntity) -> TripEntity,
    ) {
        if (!isCurrentSyncEpoch()) return
        repository.getTrip(tripId)?.let { repository.updateTrip(transform(it)) }
    }

    private suspend fun ensureUploadActive() {
        currentCoroutineContext().ensureActive()
        if (isStopped || !isCurrentSyncEpoch()) {
            throw CancellationException("Drive upload was stopped.")
        }
    }

    private fun isCurrentSyncEpoch(): Boolean =
        expectedSyncEpoch == DriveSyncEpoch.current(applicationContext)

    companion object {
        const val INPUT_TRIP_ID = "tripId"
        const val INPUT_SYNC_EPOCH = "syncEpoch"
    }
}

internal fun shouldRestartExpiredResumableSession(
    error: DriveHttpException,
    sessionUrl: URL?,
    alreadyRetried: Boolean,
): Boolean =
    sessionUrl != null && !alreadyRetried && error.statusCode in setOf(404, 410)

private fun DriveFile?.matches(tripId: String, sha256: String): Boolean =
    this != null &&
            !trashed &&
            appProperties["tripId"] == tripId &&
            appProperties["sha256"] == sha256

private fun File.sha256(): String {
    val digest = MessageDigest.getInstance("SHA-256")
    inputStream().buffered().use { input ->
        val buffer = ByteArray(DEFAULT_BUFFER_SIZE)
        while (true) {
            val count = input.read(buffer)
            if (count < 0) break
            if (count > 0) digest.update(buffer, 0, count)
        }
    }
    return digest.digest().joinToString("") { byte ->
        (byte.toInt() and 0xff).toString(16).padStart(2, '0')
    }
}

private fun now(): Long = System.currentTimeMillis()
