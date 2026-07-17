package com.gpxanimator.mobile.ui

import com.gpxanimator.mobile.data.SyncState
import com.gpxanimator.mobile.data.TripState

data class RidePermissionState(
    val hasFineLocation: Boolean = false,
    val hasCoarseLocation: Boolean = false,
    val hasNotifications: Boolean = false,
    val locationServicesEnabled: Boolean = false,
) {
    val canRecord: Boolean
        get() = hasFineLocation && hasNotifications && locationServicesEnabled
}

data class BackgroundProtectionState(
    val ignoresBatteryOptimizations: Boolean = false,
)

enum class DriveConnectionUiState {
    NotConnected,
    Connected,
    AuthorizationRequired,
}

data class DriveUiState(
    val connection: DriveConnectionUiState = DriveConnectionUiState.NotConnected,
    val accountLabel: String? = null,
    val canStartAuthorization: Boolean = false,
)

data class TripListItemUi(
    val id: String,
    val name: String,
    val state: TripState,
    val syncState: SyncState,
    val startedAtEpochMillis: Long,
    val endedAtEpochMillis: Long?,
    val distanceMeters: Double,
    val durationMillis: Long,
    val pointCount: Int,
    val hasLocalGpx: Boolean,
)

data class ActiveRideUi(
    val id: String,
    val name: String,
    val startedAtEpochMillis: Long,
    val elapsedMillis: Long,
    val distanceMeters: Double,
    val currentSpeedMetersPerSecond: Float?,
    val averageSpeedMetersPerSecond: Double,
    val gpsAccuracyMeters: Float?,
    val gpsFixAgeMillis: Long?,
    val pointCount: Int,
    val syncState: SyncState,
)

data class TrackCoordinateUi(
    val latitude: Double,
    val longitude: Double,
)

data class RideDetailUi(
    val trip: TripListItemUi,
    val track: List<TrackCoordinateUi>,
    val localGpxPath: String?,
    val lastError: String?,
)

data class RideUiState(
    val isLoading: Boolean = true,
    val hasDataError: Boolean = false,
    val trips: List<TripListItemUi> = emptyList(),
    val activeRide: ActiveRideUi? = null,
    val selectedRide: RideDetailUi? = null,
)

data class RideAppCallbacks(
    val onStartRecording: () -> Unit,
    val onFinishRecording: () -> Unit,
    val onRequestPermissions: () -> Unit,
    val onOpenBatterySettings: () -> Unit,
    val onOpenAppSettings: () -> Unit,
    val onOpenLocationSettings: () -> Unit = {},
    val onConnectDrive: () -> Unit = {},
    val onDisconnectDrive: () -> Unit = {},
    val onShareRide: (String) -> Unit = {},
    val onRetryDriveSync: (String) -> Unit = {},
    val onFinalizeInterruptedRide: (String) -> Unit = {},
)
