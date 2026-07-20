package com.gpxanimator.mobile.recovery

import android.content.Context
import com.gpxanimator.mobile.data.RideRepository
import com.gpxanimator.mobile.data.SyncState
import com.gpxanimator.mobile.data.TripEntity
import com.gpxanimator.mobile.data.TripState
import com.gpxanimator.mobile.gpx.RideFinalizer
import com.gpxanimator.mobile.sync.DriveSyncCoordinator
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.CancellationException
import androidx.work.await

internal data class StartupRecoveryPlan(
    val tripsToFinalize: List<String>,
    val tripsToSync: List<String>,
)

internal fun startupRecoveryPlan(trips: List<TripEntity>): StartupRecoveryPlan =
    StartupRecoveryPlan(
        tripsToFinalize =
            trips.filter { it.state == TripState.FINALIZING }.map(TripEntity::id),
        tripsToSync =
            trips.filter {
                it.state == TripState.READY &&
                        it.localGpxPath != null &&
                        it.syncState in setOf(SyncState.PENDING, SyncState.UPLOADING)
            }.map(TripEntity::id),
    )

object StartupRecoveryCoordinator {
    suspend fun recover(context: Context, repository: RideRepository) {
        val plan = startupRecoveryPlan(repository.trips.first())
        plan.tripsToFinalize.forEach { tripId ->
            try {
                RideFinalizer.enqueue(context, tripId).await()
            } catch (error: CancellationException) {
                throw error
            } catch (_: Exception) {
                // The durable state remains eligible for the next app startup.
            }
        }
        plan.tripsToSync.forEach { tripId ->
            try {
                DriveSyncCoordinator.enqueue(context, tripId).await()
            } catch (error: CancellationException) {
                throw error
            } catch (_: Exception) {
                // The durable state remains eligible for the next app startup.
            }
        }
    }
}
