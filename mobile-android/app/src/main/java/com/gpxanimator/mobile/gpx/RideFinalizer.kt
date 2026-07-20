package com.gpxanimator.mobile.gpx

import android.content.Context
import androidx.work.CoroutineWorker
import androidx.work.Data
import androidx.work.ExistingWorkPolicy
import androidx.work.OneTimeWorkRequestBuilder
import androidx.work.Operation
import androidx.work.WorkManager
import androidx.work.WorkerParameters
import androidx.work.await
import com.gpxanimator.mobile.GpxAnimatorRideApplication
import com.gpxanimator.mobile.data.SyncState
import com.gpxanimator.mobile.data.TripState
import com.gpxanimator.mobile.sync.DriveSyncCoordinator
import java.util.concurrent.CancellationException

object RideFinalizer {
    private const val INPUT_TRIP_ID = "trip_id"

    fun enqueue(context: Context, tripId: String): Operation {
        val request =
            OneTimeWorkRequestBuilder<FinalizeRideWorker>()
                .setInputData(Data.Builder().putString(INPUT_TRIP_ID, tripId).build())
                .build()
        return WorkManager.getInstance(context).enqueueUniqueWork(
            "finalize-ride-$tripId",
            ExistingWorkPolicy.KEEP,
            request,
        )
    }

    internal fun tripId(parameters: WorkerParameters): String? =
        parameters.inputData.getString(INPUT_TRIP_ID)
}

class FinalizeRideWorker(
    appContext: Context,
    private val parameters: WorkerParameters,
) : CoroutineWorker(appContext, parameters) {
    override suspend fun doWork(): Result {
        val tripId = RideFinalizer.tripId(parameters) ?: return Result.failure()
        val application =
            applicationContext as? GpxAnimatorRideApplication ?: return Result.failure()
        val repository = application.container.rideRepository
        val trip = repository.getTrip(tripId) ?: return Result.failure()
        return try {
            if (trip.state != TripState.FINALIZING) {
                repository.updateTrip(
                    trip.copy(
                        state = TripState.FINALIZING,
                        updatedAtEpochMillis = System.currentTimeMillis(),
                    ),
                )
            }
            val exported = GpxExporter(applicationContext, repository).export(tripId)
            val latest = repository.getTrip(tripId) ?: return Result.failure()
            val readyTrip =
                latest.copy(
                    state = TripState.READY,
                    syncState = SyncState.PENDING,
                    endedAtEpochMillis = latest.endedAtEpochMillis ?: System.currentTimeMillis(),
                    localGpxPath = exported.file.absolutePath,
                    localGpxSha256 = exported.sha256,
                    pointCount = exported.pointCount,
                    lastError = null,
                    updatedAtEpochMillis = System.currentTimeMillis(),
                )
            repository.updateTrip(readyTrip)
            try {
                DriveSyncCoordinator.enqueue(applicationContext, tripId).await()
            } catch (error: CancellationException) {
                throw error
            } catch (error: Exception) {
                repository.updateTrip(
                    readyTrip.copy(
                        syncState = SyncState.FAILED,
                        lastError = error.message ?: "Unable to queue Google Drive sync",
                        updatedAtEpochMillis = System.currentTimeMillis(),
                    ),
                )
            }
            Result.success()
        } catch (error: CancellationException) {
            throw error
        } catch (error: GpxExportException) {
            repository.updateTrip(
                trip.copy(
                    state = TripState.EXPORT_FAILED,
                    syncState = SyncState.LOCAL_ONLY,
                    lastError = error.message,
                    updatedAtEpochMillis = System.currentTimeMillis(),
                ),
            )
            Result.failure()
        } catch (error: Exception) {
            runCatching {
                repository.updateTrip(
                    trip.copy(
                        state = TripState.EXPORT_FAILED,
                        syncState = SyncState.LOCAL_ONLY,
                        lastError = error.message ?: "Unable to finalize the ride",
                        updatedAtEpochMillis = System.currentTimeMillis(),
                    ),
                )
            }
            if (runAttemptCount < MAX_FINALIZE_ATTEMPTS - 1) Result.retry() else Result.failure()
        }
    }

    private companion object {
        const val MAX_FINALIZE_ATTEMPTS = 3
    }
}
