package com.gpxanimator.mobile.ui

import android.app.Application
import android.os.SystemClock
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import com.gpxanimator.mobile.GpxAnimatorRideApplication
import com.gpxanimator.mobile.data.TrackPointEntity
import com.gpxanimator.mobile.data.TripEntity
import com.gpxanimator.mobile.data.TripState
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharingStarted
import kotlinx.coroutines.flow.catch
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.flow
import kotlinx.coroutines.flow.mapLatest
import kotlinx.coroutines.flow.stateIn
import kotlin.math.max

@OptIn(ExperimentalCoroutinesApi::class)
class RideViewModel(application: Application) : AndroidViewModel(application) {
    private val repository =
        (application as GpxAnimatorRideApplication).container.rideRepository
    private val selectedTripId = MutableStateFlow<String?>(null)

    private val clock: Flow<Long> =
        flow {
            while (currentCoroutineContext().isActive) {
                emit(SystemClock.elapsedRealtime())
                delay(1_000)
            }
        }

    private val tripSnapshot =
        repository.trips
            .mapLatest { trips ->
                val activeTrip = trips.firstOrNull { it.state == TripState.RECORDING }
                val lastPoint = activeTrip?.let { repository.lastPoint(it.id) }
                TripsSnapshot(
                    isLoading = false,
                    hasDataError = false,
                    trips = trips.map(TripEntity::toListItemUi),
                    activeTrip = activeTrip,
                    activeSpeedMetersPerSecond = lastPoint?.speedMetersPerSecond,
                    activeAccuracyMeters = lastPoint?.horizontalAccuracyMeters,
                    activePointElapsedRealtimeNanos = lastPoint?.elapsedRealtimeNanos,
                )
            }.catch {
                emit(TripsSnapshot(isLoading = false, hasDataError = true))
            }.stateIn(
                scope = viewModelScope,
                started = SharingStarted.WhileSubscribed(5_000),
                initialValue = TripsSnapshot(),
            )

    private val selectedRide =
        combine(repository.trips, selectedTripId) { trips, selectedId ->
            trips.firstOrNull { it.id == selectedId }
        }.mapLatest { trip ->
            trip?.let {
                val points = runCatching { repository.points(it.id) }.getOrDefault(emptyList())
                RideDetailUi(
                    trip = it.toListItemUi(),
                    track = points.toPreviewTrack(),
                    localGpxPath = it.localGpxPath,
                    lastError = it.lastError,
                )
            }
        }.catch { emit(null) }
            .stateIn(
                scope = viewModelScope,
                started = SharingStarted.WhileSubscribed(5_000),
                initialValue = null,
            )

    val uiState =
        combine(tripSnapshot, selectedRide, clock) { snapshot, detail, now ->
            RideUiState(
                isLoading = snapshot.isLoading,
                hasDataError = snapshot.hasDataError,
                trips = snapshot.trips,
                activeRide = snapshot.activeTrip?.toActiveRideUi(snapshot, now),
                selectedRide = detail,
            )
        }.stateIn(
            scope = viewModelScope,
            started = SharingStarted.WhileSubscribed(5_000),
            initialValue = RideUiState(),
        )

    fun selectTrip(tripId: String?) {
        selectedTripId.value = tripId
    }

    private fun TripEntity.toActiveRideUi(
        snapshot: TripsSnapshot,
        nowElapsedRealtimeMillis: Long
    ): ActiveRideUi {
        val elapsed =
            max(
                durationMillis,
                (nowElapsedRealtimeMillis - startElapsedRealtimeNanos / 1_000_000L)
                    .coerceAtLeast(0),
            )
        val elapsedSeconds = elapsed / 1_000.0
        val gpsFixAgeMillis =
            snapshot.activePointElapsedRealtimeNanos?.let { pointElapsedRealtimeNanos ->
                (nowElapsedRealtimeMillis - pointElapsedRealtimeNanos / 1_000_000L)
                    .coerceAtLeast(0L)
            }
        val hasFreshFix = gpsFixAgeMillis != null && gpsFixAgeMillis <= FRESH_FIX_MAX_AGE_MILLIS
        return ActiveRideUi(
            id = id,
            name = name,
            startedAtEpochMillis = startedAtEpochMillis,
            elapsedMillis = elapsed,
            distanceMeters = distanceMeters,
            currentSpeedMetersPerSecond =
                snapshot.activeSpeedMetersPerSecond.takeIf { hasFreshFix },
            averageSpeedMetersPerSecond =
                if (elapsedSeconds > 0) distanceMeters / elapsedSeconds else 0.0,
            gpsAccuracyMeters = snapshot.activeAccuracyMeters,
            gpsFixAgeMillis = gpsFixAgeMillis,
            pointCount = pointCount,
            syncState = syncState,
        )
    }
}

private data class TripsSnapshot(
    val isLoading: Boolean = true,
    val hasDataError: Boolean = false,
    val trips: List<TripListItemUi> = emptyList(),
    val activeTrip: TripEntity? = null,
    val activeSpeedMetersPerSecond: Float? = null,
    val activeAccuracyMeters: Float? = null,
    val activePointElapsedRealtimeNanos: Long? = null,
)

private fun TripEntity.toListItemUi() =
    TripListItemUi(
        id = id,
        name = name,
        state = state,
        syncState = syncState,
        startedAtEpochMillis = startedAtEpochMillis,
        endedAtEpochMillis = endedAtEpochMillis,
        distanceMeters = distanceMeters,
        durationMillis = durationMillis,
        pointCount = pointCount,
        hasLocalGpx = localGpxPath != null,
    )

private fun List<TrackPointEntity>.toPreviewTrack(): List<TrackCoordinateUi> {
    if (isEmpty()) return emptyList()
    val outputSize = minOf(size, MAX_PREVIEW_POINTS)
    return List(outputSize) { outputIndex ->
        val sourceIndex =
            if (outputSize == 1) 0 else outputIndex * (size - 1) / (outputSize - 1)
        this[sourceIndex].let { point ->
            TrackCoordinateUi(
                latitude = point.latitude,
                longitude = point.longitude,
            )
        }
    }
}

private const val MAX_PREVIEW_POINTS = 1_200
private const val FRESH_FIX_MAX_AGE_MILLIS = 15_000L
