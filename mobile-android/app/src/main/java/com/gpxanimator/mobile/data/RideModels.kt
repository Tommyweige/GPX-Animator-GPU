package com.gpxanimator.mobile.data

import androidx.room.Entity
import androidx.room.ForeignKey
import androidx.room.Index

enum class TripState {
    RECORDING,
    FINALIZING,
    READY,
    INTERRUPTED,
    EXPORT_FAILED,
}

enum class SyncState {
    LOCAL_ONLY,
    PENDING,
    UPLOADING,
    SYNCED,
    FAILED,
    AUTH_REQUIRED,
}

@Entity(tableName = "trips")
data class TripEntity(
    @androidx.room.PrimaryKey val id: String,
    val name: String,
    val state: TripState,
    val syncState: SyncState,
    val startedAtEpochMillis: Long,
    val startElapsedRealtimeNanos: Long,
    val endedAtEpochMillis: Long? = null,
    val distanceMeters: Double = 0.0,
    val durationMillis: Long = 0,
    val pointCount: Int = 0,
    val localGpxPath: String? = null,
    val localGpxSha256: String? = null,
    val driveFileId: String? = null,
    val driveFolderId: String? = null,
    val lastError: String? = null,
    val updatedAtEpochMillis: Long,
)

@Entity(
    tableName = "track_points",
    primaryKeys = ["tripId", "sequence"],
    foreignKeys = [
        ForeignKey(
            entity = TripEntity::class,
            parentColumns = ["id"],
            childColumns = ["tripId"],
            onDelete = ForeignKey.CASCADE,
        ),
    ],
    indices = [Index("tripId")],
)
data class TrackPointEntity(
    val tripId: String,
    val sequence: Int,
    val latitude: Double,
    val longitude: Double,
    val altitudeMeters: Double? = null,
    val horizontalAccuracyMeters: Float,
    val verticalAccuracyMeters: Float? = null,
    val speedMetersPerSecond: Float? = null,
    val bearingDegrees: Float? = null,
    val timestampEpochMillis: Long,
    val elapsedRealtimeNanos: Long,
)
