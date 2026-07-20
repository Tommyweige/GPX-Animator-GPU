package com.gpxanimator.mobile.recovery

import com.gpxanimator.mobile.data.SyncState
import com.gpxanimator.mobile.data.TripEntity
import com.gpxanimator.mobile.data.TripState
import org.junit.Assert.assertEquals
import org.junit.Test

class StartupRecoveryCoordinatorTest {
    @Test
    fun `plans stranded finalization and durable pending uploads`() {
        val plan =
            startupRecoveryPlan(
                listOf(
                    trip("finalizing", TripState.FINALIZING, SyncState.LOCAL_ONLY),
                    trip("pending", TripState.READY, SyncState.PENDING, localGpx = true),
                    trip("uploading", TripState.READY, SyncState.UPLOADING, localGpx = true),
                    trip("auth", TripState.READY, SyncState.AUTH_REQUIRED, localGpx = true),
                    trip("synced", TripState.READY, SyncState.SYNCED, localGpx = true),
                ),
            )

        assertEquals(listOf("finalizing"), plan.tripsToFinalize)
        assertEquals(listOf("pending", "uploading"), plan.tripsToSync)
    }

    private fun trip(
        id: String,
        state: TripState,
        syncState: SyncState,
        localGpx: Boolean = false,
    ) =
        TripEntity(
            id = id,
            name = id,
            state = state,
            syncState = syncState,
            startedAtEpochMillis = 1_000L,
            startElapsedRealtimeNanos = 1_000L,
            localGpxPath = if (localGpx) "/rides/$id.gpx" else null,
            updatedAtEpochMillis = 1_000L,
        )
}
