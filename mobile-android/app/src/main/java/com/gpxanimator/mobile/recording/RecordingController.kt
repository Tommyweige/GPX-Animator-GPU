package com.gpxanimator.mobile.recording

import android.content.Context
import android.content.Intent
import android.app.ActivityManager
import android.app.ApplicationExitInfo
import android.os.Build
import android.os.Process
import androidx.core.content.ContextCompat
import com.gpxanimator.mobile.GpxAnimatorRideApplication
import com.gpxanimator.mobile.data.RideRepository
import com.gpxanimator.mobile.data.SyncState
import com.gpxanimator.mobile.data.TripEntity
import com.gpxanimator.mobile.data.TripState

/** Entry point used by UI surfaces to control the durable recording service. */
object RecordingController {
    fun start(context: Context) {
        val intent =
            Intent(context.applicationContext, RideRecordingService::class.java)
                .setAction(RideRecordingService.ACTION_START)
        ContextCompat.startForegroundService(context.applicationContext, intent)
    }

    fun finish(context: Context) {
        val intent =
            Intent(context.applicationContext, RideRecordingService::class.java)
                .setAction(RideRecordingService.ACTION_FINISH)
        context.applicationContext.startService(intent)
    }

    private fun recover(context: Context) {
        val intent =
            Intent(context.applicationContext, RideRecordingService::class.java)
                .setAction(RideRecordingService.ACTION_RECOVER)
        ContextCompat.startForegroundService(context.applicationContext, intent)
    }

    /**
     * Reconciles a ride left in RECORDING after a true process stop or force-stop.
     * A fresh persisted service lease is recovered; a stale lease is finalized as interrupted.
     */
    suspend fun reconcileInterruptedRide(context: Context) {
        val application = context.applicationContext as? GpxAnimatorRideApplication ?: return
        val repository = application.container.rideRepository
        val trip = repository.getActiveTrip() ?: return
        val currentBootCount = RecordingLease.currentBootCount(application)
        val sameBoot =
            trip.startBootCount == RecordingLease.UNKNOWN_BOOT ||
                    currentBootCount == RecordingLease.UNKNOWN_BOOT ||
                    trip.startBootCount == currentBootCount
        val userRequestedStop =
            !RideRecordingService.isRunning &&
                    wasUserRequestedExitAfter(context, trip.startedAtEpochMillis)
        if (userRequestedStop) {
            markInterrupted(repository, trip)
            return
        }
        when (
            RecordingRecoveryPolicy.decide(
                serviceRunning = RideRecordingService.isRunning,
                sameBoot = sameBoot,
                leaseFresh = RecordingLease(application).isFresh(trip.id),
            )
        ) {
            RecordingRecoveryAction.KEEP_RUNNING -> return
            RecordingRecoveryAction.RECOVER_SERVICE -> {
                recover(application)
                return
            }

            RecordingRecoveryAction.MARK_INTERRUPTED -> Unit
        }
        if (RideRecordingService.isRunning) return
        markInterrupted(repository, trip)
    }

    private suspend fun markInterrupted(
        repository: RideRepository,
        trip: TripEntity,
    ) {
        val lastPoint = repository.lastPoint(trip.id)

        val durationMillis =
            maxOf(
                trip.durationMillis,
                lastPoint?.let {
                    MonotonicRideClock.durationMillis(
                        trip.startElapsedRealtimeNanos,
                        it.elapsedRealtimeNanos,
                    )
                } ?: 0L,
            )
        repository.updateTrip(
            trip.copy(
                state = TripState.INTERRUPTED,
                syncState = SyncState.LOCAL_ONLY,
                endedAtEpochMillis = trip.startedAtEpochMillis + durationMillis,
                durationMillis = durationMillis,
                lastError = "Recording stopped outside the app; saved points were preserved.",
                updatedAtEpochMillis = System.currentTimeMillis(),
            ),
        )
    }

    private fun wasUserRequestedExitAfter(context: Context, startedAtEpochMillis: Long): Boolean {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.R) return false
        val activityManager = context.getSystemService(ActivityManager::class.java) ?: return false
        val latestExit =
            activityManager
                .getHistoricalProcessExitReasons(context.packageName, Process.myUid(), 1)
                .firstOrNull()
                ?: return false
        return latestExit.reason == ApplicationExitInfo.REASON_USER_REQUESTED &&
                latestExit.timestamp >= startedAtEpochMillis
    }
}
