package com.gpxanimator.mobile.sync

import android.content.Context
import androidx.work.BackoffPolicy
import androidx.work.Constraints
import androidx.work.ExistingWorkPolicy
import androidx.work.NetworkType
import androidx.work.OneTimeWorkRequestBuilder
import androidx.work.Operation
import androidx.work.WorkManager
import androidx.work.workDataOf
import java.util.concurrent.TimeUnit

object DriveSyncCoordinator {
    fun enqueue(context: Context, tripId: String): Operation {
        require(tripId.isNotBlank()) { "tripId must not be blank." }
        val request =
            OneTimeWorkRequestBuilder<DriveUploadWorker>()
                .setInputData(
                    workDataOf(
                        DriveUploadWorker.INPUT_TRIP_ID to tripId,
                        DriveUploadWorker.INPUT_SYNC_EPOCH to DriveSyncEpoch.current(context),
                    ),
                )
                .setConstraints(
                    Constraints.Builder()
                        .setRequiredNetworkType(NetworkType.CONNECTED)
                        .build(),
                )
                .setBackoffCriteria(BackoffPolicy.EXPONENTIAL, 30, TimeUnit.SECONDS)
                .addTag(UPLOAD_TAG)
                .addTag(tripTag(tripId))
                .build()

        return WorkManager.getInstance(context.applicationContext)
            .enqueueUniqueWork(workName(tripId), ExistingWorkPolicy.KEEP, request)
    }

    fun workName(tripId: String): String = "drive-upload-$tripId"

    fun cancelAll(context: Context): Operation =
        WorkManager.getInstance(context.applicationContext).cancelAllWorkByTag(UPLOAD_TAG)

    private fun tripTag(tripId: String): String = "drive-upload-trip-$tripId"

    private const val UPLOAD_TAG = "drive-upload"
}
