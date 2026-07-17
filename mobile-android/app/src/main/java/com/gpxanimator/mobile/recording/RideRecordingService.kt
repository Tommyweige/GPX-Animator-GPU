package com.gpxanimator.mobile.recording

import android.Manifest
import android.annotation.SuppressLint
import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Intent
import android.content.pm.PackageManager
import android.content.pm.ServiceInfo
import android.location.Location
import android.os.IBinder
import android.os.Looper
import android.os.SystemClock
import android.util.Log
import androidx.core.app.NotificationCompat
import androidx.core.content.ContextCompat
import androidx.work.await
import com.google.android.gms.location.FusedLocationProviderClient
import com.google.android.gms.location.LocationCallback
import com.google.android.gms.location.LocationRequest
import com.google.android.gms.location.LocationResult
import com.google.android.gms.location.LocationServices
import com.google.android.gms.location.Priority
import com.gpxanimator.mobile.GpxAnimatorRideApplication
import com.gpxanimator.mobile.MainActivity
import com.gpxanimator.mobile.R
import com.gpxanimator.mobile.data.RideRepository
import com.gpxanimator.mobile.data.SyncState
import com.gpxanimator.mobile.data.TrackPointEntity
import com.gpxanimator.mobile.data.TripEntity
import com.gpxanimator.mobile.data.TripState
import com.gpxanimator.mobile.gpx.RideFinalizer
import java.time.Instant
import java.time.ZoneId
import java.time.format.DateTimeFormatter
import java.util.Locale
import java.util.UUID
import java.util.concurrent.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock

class RideRecordingService : Service() {
    private val serviceScope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    private val recordingMutex = Mutex()

    private lateinit var repository: RideRepository
    private lateinit var fusedLocationClient: FusedLocationProviderClient
    private lateinit var recordingLease: RecordingLease

    private var activeTrip: TripEntity? = null
    private var lastPoint: TrackPointEntity? = null
    private var locationUpdatesActive = false
    private var notificationJob: Job? = null
    private var foregroundStarted = false

    private val locationCallback =
        object : LocationCallback() {
            override fun onLocationResult(result: LocationResult) {
                val orderedLocations = result.locations.sortedBy(Location::getElapsedRealtimeNanos)
                launchSerialized("persist location batch") {
                    activeTrip?.let { recordingLease.refresh(it.id) }
                    orderedLocations.forEach { location -> persistLocationLocked(location) }
                }
            }
        }

    override fun onCreate() {
        super.onCreate()
        isRunning = true
        val app = application as GpxAnimatorRideApplication
        repository = app.container.rideRepository
        fusedLocationClient = LocationServices.getFusedLocationProviderClient(this)
        recordingLease = RecordingLease(this)
        createNotificationChannel()
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        when (intent?.action) {
            ACTION_FINISH -> {
                launchSerialized("finish ride") { finishRecordingLocked() }
            }

            ACTION_START -> {
                if (!promoteToForeground(startId)) return START_NOT_STICKY
                launchSerialized("start ride") { startOrRecoverLocked(createIfMissing = true) }
            }

            ACTION_RECOVER -> {
                if (!promoteToForeground(startId)) return START_NOT_STICKY
                launchSerialized("recover ride") { startOrRecoverLocked(createIfMissing = false) }
            }

            else -> {
                if (!promoteToForeground(startId)) return START_NOT_STICKY
                launchSerialized("recover ride") { startOrRecoverLocked(createIfMissing = false) }
            }
        }
        return START_STICKY
    }

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onDestroy() {
        isRunning = false
        locationUpdatesActive = false
        fusedLocationClient.removeLocationUpdates(locationCallback)
        notificationJob?.cancel()
        serviceScope.cancel()
        super.onDestroy()
    }

    private fun promoteToForeground(startId: Int): Boolean {
        if (!hasPreciseLocationPermission()) {
            stopSelf(startId)
            return false
        }

        return try {
            startForeground(
                NOTIFICATION_ID,
                buildNotification(null),
                ServiceInfo.FOREGROUND_SERVICE_TYPE_LOCATION,
            )
            foregroundStarted = true
            true
        } catch (error: SecurityException) {
            Log.e(TAG, "Unable to start the location foreground service", error)
            stopSelf(startId)
            false
        }
    }

    private suspend fun startOrRecoverLocked(createIfMissing: Boolean) {
        if (activeTrip?.state == TripState.RECORDING) {
            if (!startLocationUpdatesLocked()) {
                interruptRecordingLocked("Precise location permission is unavailable")
                return
            }
            startNotificationUpdatesLocked()
            return
        }

        val nowElapsedRealtimeNanos = SystemClock.elapsedRealtimeNanos()
        var trip = repository.getActiveTrip()
        val currentBootCount = RecordingLease.currentBootCount(this)
        val bootChanged =
            trip != null &&
                    trip.startBootCount != RecordingLease.UNKNOWN_BOOT &&
                    currentBootCount != RecordingLease.UNKNOWN_BOOT &&
                    trip.startBootCount != currentBootCount
        val monotonicClockReset =
            trip != null &&
                    (trip.startBootCount == RecordingLease.UNKNOWN_BOOT ||
                            currentBootCount == RecordingLease.UNKNOWN_BOOT) &&
                    nowElapsedRealtimeNanos < trip.startElapsedRealtimeNanos
        val staleRecoveryLease =
            trip != null && !createIfMissing && !recordingLease.isFresh(trip.id)
        if (trip != null && (bootChanged || monotonicClockReset || staleRecoveryLease)) {
            val interruptionReason =
                when {
                    bootChanged || monotonicClockReset ->
                        "Device rebooted while the ride was recording"

                    else -> "Recording service restarted after its recovery lease expired"
                }
            repository.updateTrip(
                trip.copy(
                    state = TripState.INTERRUPTED,
                    endedAtEpochMillis = System.currentTimeMillis(),
                    lastError = interruptionReason,
                    updatedAtEpochMillis = System.currentTimeMillis(),
                ),
            )
            recordingLease.clear(trip.id)
            trip = null
        }

        if (trip == null && createIfMissing) {
            val nowEpochMillis = System.currentTimeMillis()
            trip =
                TripEntity(
                    id = UUID.randomUUID().toString(),
                    name = defaultRideName(nowEpochMillis),
                    state = TripState.RECORDING,
                    syncState = SyncState.LOCAL_ONLY,
                    startedAtEpochMillis = nowEpochMillis,
                    startElapsedRealtimeNanos = nowElapsedRealtimeNanos,
                    startBootCount = currentBootCount,
                    startZoneId = ZoneId.systemDefault().id,
                    updatedAtEpochMillis = nowEpochMillis,
                )
            repository.create(trip)
        }

        if (trip == null) {
            stopServiceLocked()
            return
        }

        activeTrip = trip
        lastPoint = repository.lastPoint(trip.id)
        recordingLease.refresh(trip.id)
        if (!startLocationUpdatesLocked()) {
            interruptRecordingLocked("Precise location permission is unavailable")
            return
        }
        updateNotificationLocked()
        startNotificationUpdatesLocked()
    }

    @SuppressLint("MissingPermission")
    private fun startLocationUpdatesLocked(): Boolean {
        if (locationUpdatesActive) return true
        if (!hasPreciseLocationPermission()) return false

        val locationRequest =
            LocationRequest.Builder(Priority.PRIORITY_HIGH_ACCURACY, LOCATION_INTERVAL_MILLIS)
                .setMinUpdateIntervalMillis(LOCATION_INTERVAL_MILLIS)
                .setMinUpdateDistanceMeters(MIN_UPDATE_DISTANCE_METERS)
                .setMaxUpdateDelayMillis(MAX_UPDATE_DELAY_MILLIS)
                .build()

        return try {
            fusedLocationClient
                .requestLocationUpdates(locationRequest, locationCallback, Looper.getMainLooper())
                .addOnFailureListener { error ->
                    launchSerialized("handle location failure") {
                        interruptRecordingLocked(
                            error.message ?: "Fused location updates failed",
                        )
                    }
                }
            locationUpdatesActive = true
            true
        } catch (error: SecurityException) {
            Log.e(TAG, "Location permission was revoked", error)
            false
        }
    }

    private suspend fun persistLocationLocked(location: Location) {
        val trip = activeTrip ?: return
        if (!locationUpdatesActive || trip.state != TripState.RECORDING) return

        val sample = location.toSample()
        if (sample.elapsedRealtimeNanos < trip.startElapsedRealtimeNanos) return
        val previousSample = lastPoint?.toSample()
        if (!LocationSampleFilter.accepts(
                sample = sample,
                nowElapsedRealtimeNanos = SystemClock.elapsedRealtimeNanos(),
                previous = previousSample,
            )
        ) {
            return
        }

        val previousPoint = lastPoint
        val addedDistanceMeters =
            previousPoint?.let {
                GeoDistance.haversineMeters(
                    it.latitude,
                    it.longitude,
                    sample.latitude,
                    sample.longitude,
                )
            } ?: 0.0
        val derivedTimestamp =
            MonotonicRideClock.timestampEpochMillis(
                startEpochMillis = trip.startedAtEpochMillis,
                startElapsedRealtimeNanos = trip.startElapsedRealtimeNanos,
                sampleElapsedRealtimeNanos = sample.elapsedRealtimeNanos,
            )
        val timestampEpochMillis =
            previousPoint?.timestampEpochMillis?.plus(1L)?.let(derivedTimestamp::coerceAtLeast)
                ?: derivedTimestamp
        val sequence = (previousPoint?.sequence ?: -1) + 1
        val point =
            TrackPointEntity(
                tripId = trip.id,
                sequence = sequence,
                latitude = sample.latitude,
                longitude = sample.longitude,
                altitudeMeters = sample.altitudeMeters?.takeIf(Double::isFinite),
                horizontalAccuracyMeters = sample.horizontalAccuracyMeters,
                verticalAccuracyMeters = sample.verticalAccuracyMeters?.takeIf(Float::isFinite),
                speedMetersPerSecond =
                    sample.speedMetersPerSecond?.takeIf { it.isFinite() && it >= 0f },
                bearingDegrees =
                    sample.bearingDegrees?.takeIf { it.isFinite() && it in 0f..360f },
                timestampEpochMillis = timestampEpochMillis,
                elapsedRealtimeNanos = sample.elapsedRealtimeNanos,
            )
        val updatedTrip =
            trip.copy(
                distanceMeters = trip.distanceMeters + addedDistanceMeters,
                durationMillis =
                    MonotonicRideClock.durationMillis(
                        trip.startElapsedRealtimeNanos,
                        sample.elapsedRealtimeNanos,
                    ).coerceAtLeast(trip.durationMillis),
                pointCount = maxOf(trip.pointCount, sequence + 1),
                lastError = null,
                updatedAtEpochMillis = timestampEpochMillis,
            )

        repository.appendPoint(point, updatedTrip)
        activeTrip = updatedTrip
        lastPoint = point
        recordingLease.refresh(updatedTrip.id)
        updateNotificationLocked()
    }

    private suspend fun finishRecordingLocked() {
        locationUpdatesActive = false
        fusedLocationClient.removeLocationUpdates(locationCallback)
        notificationJob?.cancel()

        val trip = activeTrip ?: repository.getActiveTrip()
        if (trip != null) {
            val nowElapsedRealtimeNanos = SystemClock.elapsedRealtimeNanos()
            val durationMillis =
                MonotonicRideClock.durationMillis(
                    trip.startElapsedRealtimeNanos,
                    nowElapsedRealtimeNanos,
                ).coerceAtLeast(trip.durationMillis)
            val finalizedTrip =
                trip.copy(
                    state = TripState.FINALIZING,
                    endedAtEpochMillis = trip.startedAtEpochMillis + durationMillis,
                    durationMillis = durationMillis,
                    updatedAtEpochMillis = System.currentTimeMillis(),
                )
            repository.updateTrip(finalizedTrip)
            activeTrip = finalizedTrip
            try {
                RideFinalizer.enqueue(applicationContext, finalizedTrip.id).await()
            } catch (error: RuntimeException) {
                Log.e(TAG, "Unable to enqueue GPX finalization for ${finalizedTrip.id}", error)
                repository.updateTrip(
                    finalizedTrip.copy(
                        state = TripState.EXPORT_FAILED,
                        lastError = error.message ?: "Unable to enqueue GPX finalization",
                        updatedAtEpochMillis = System.currentTimeMillis(),
                    ),
                )
            }
        }

        activeTrip = null
        lastPoint = null
        stopServiceLocked()
    }

    private suspend fun interruptRecordingLocked(reason: String) {
        val trip = activeTrip ?: return
        locationUpdatesActive = false
        fusedLocationClient.removeLocationUpdates(locationCallback)
        notificationJob?.cancel()
        val nowElapsedRealtimeNanos = SystemClock.elapsedRealtimeNanos()
        val durationMillis =
            MonotonicRideClock.durationMillis(
                trip.startElapsedRealtimeNanos,
                nowElapsedRealtimeNanos,
            ).coerceAtLeast(trip.durationMillis)
        repository.updateTrip(
            trip.copy(
                state = TripState.INTERRUPTED,
                endedAtEpochMillis = trip.startedAtEpochMillis + durationMillis,
                durationMillis = durationMillis,
                lastError = reason,
                updatedAtEpochMillis = System.currentTimeMillis(),
            ),
        )
        activeTrip = null
        lastPoint = null
        stopServiceLocked()
    }

    private fun startNotificationUpdatesLocked() {
        if (notificationJob?.isActive == true) return
        notificationJob =
            serviceScope.launch {
                while (isActive) {
                    delay(NOTIFICATION_UPDATE_INTERVAL_MILLIS)
                    recordingMutex.withLock {
                        activeTrip?.let { recordingLease.refresh(it.id) }
                        updateNotificationLocked()
                    }
                }
            }
    }

    @SuppressLint("MissingPermission")
    private fun updateNotificationLocked() {
        if (!foregroundStarted) return
        getSystemService(NotificationManager::class.java)
            .notify(NOTIFICATION_ID, buildNotification(activeTrip))
    }

    private fun buildNotification(trip: TripEntity?): Notification {
        val openAppIntent =
            PendingIntent.getActivity(
                this,
                0,
                Intent(this, MainActivity::class.java)
                    .addFlags(Intent.FLAG_ACTIVITY_CLEAR_TOP or Intent.FLAG_ACTIVITY_SINGLE_TOP),
                PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
            )
        val contentText =
            if (trip == null) {
                getString(R.string.recording_notification_acquiring)
            } else {
                val durationMillis =
                    MonotonicRideClock.durationMillis(
                        trip.startElapsedRealtimeNanos,
                        SystemClock.elapsedRealtimeNanos(),
                    ).coerceAtLeast(trip.durationMillis)
                getString(
                    R.string.recording_notification_summary,
                    formatDuration(durationMillis),
                    formatDistance(trip.distanceMeters),
                )
            }

        return NotificationCompat.Builder(this, NOTIFICATION_CHANNEL_ID)
            .setSmallIcon(R.drawable.ic_stat_ride)
            .setContentTitle(getString(R.string.recording_notification_title))
            .setContentText(contentText)
            .setContentIntent(openAppIntent)
            .setCategory(Notification.CATEGORY_SERVICE)
            .setOngoing(true)
            .setOnlyAlertOnce(true)
            .setSilent(true)
            .setForegroundServiceBehavior(NotificationCompat.FOREGROUND_SERVICE_IMMEDIATE)
            .build()
    }

    private fun createNotificationChannel() {
        val channel =
            NotificationChannel(
                NOTIFICATION_CHANNEL_ID,
                getString(R.string.recording_channel_name),
                NotificationManager.IMPORTANCE_LOW,
            ).apply {
                description = getString(R.string.recording_channel_description)
                setShowBadge(false)
            }
        getSystemService(NotificationManager::class.java).createNotificationChannel(channel)
    }

    private fun stopServiceLocked() {
        recordingLease.clear()
        if (foregroundStarted) {
            stopForeground(STOP_FOREGROUND_REMOVE)
            foregroundStarted = false
        }
        stopSelf()
    }

    private fun hasPreciseLocationPermission(): Boolean =
        ContextCompat.checkSelfPermission(this, Manifest.permission.ACCESS_FINE_LOCATION) ==
                PackageManager.PERMISSION_GRANTED

    private fun Location.toSample(): LocationSample =
        LocationSample(
            latitude = latitude,
            longitude = longitude,
            altitudeMeters = if (hasAltitude()) altitude else null,
            horizontalAccuracyMeters = if (hasAccuracy()) accuracy else Float.POSITIVE_INFINITY,
            verticalAccuracyMeters =
                if (hasVerticalAccuracy()) verticalAccuracyMeters else null,
            speedMetersPerSecond = if (hasSpeed()) speed else null,
            bearingDegrees = if (hasBearing()) bearing else null,
            elapsedRealtimeNanos = elapsedRealtimeNanos,
        )

    private fun TrackPointEntity.toSample(): LocationSample =
        LocationSample(
            latitude = latitude,
            longitude = longitude,
            altitudeMeters = altitudeMeters,
            horizontalAccuracyMeters = horizontalAccuracyMeters,
            verticalAccuracyMeters = verticalAccuracyMeters,
            speedMetersPerSecond = speedMetersPerSecond,
            bearingDegrees = bearingDegrees,
            elapsedRealtimeNanos = elapsedRealtimeNanos,
        )

    private fun launchSerialized(
        operation: String,
        block: suspend () -> Unit,
    ) {
        serviceScope.launch {
            try {
                recordingMutex.withLock { block() }
            } catch (error: CancellationException) {
                throw error
            } catch (error: Exception) {
                Log.e(TAG, "Recorder operation failed: $operation", error)
                recordingMutex.withLock { handleUnexpectedFailureLocked(error) }
            }
        }
    }

    private suspend fun handleUnexpectedFailureLocked(error: Exception) {
        locationUpdatesActive = false
        fusedLocationClient.removeLocationUpdates(locationCallback)
        notificationJob?.cancel()
        val trip = activeTrip ?: runCatching { repository.getActiveTrip() }.getOrNull()
        if (trip != null) {
            val durationMillis =
                MonotonicRideClock.durationMillis(
                    trip.startElapsedRealtimeNanos,
                    SystemClock.elapsedRealtimeNanos(),
                ).coerceAtLeast(trip.durationMillis)
            runCatching {
                repository.updateTrip(
                    trip.copy(
                        state =
                            if (trip.state == TripState.RECORDING) {
                                TripState.INTERRUPTED
                            } else {
                                trip.state
                            },
                        endedAtEpochMillis =
                            trip.endedAtEpochMillis
                                ?: trip.startedAtEpochMillis + durationMillis,
                        durationMillis = durationMillis,
                        lastError = error.message ?: "The recorder stopped unexpectedly",
                        updatedAtEpochMillis = System.currentTimeMillis(),
                    ),
                )
            }.onFailure { databaseError ->
                Log.e(TAG, "Unable to persist recorder failure", databaseError)
            }
        }
        activeTrip = null
        lastPoint = null
        stopServiceLocked()
    }

    private fun defaultRideName(nowEpochMillis: Long): String =
        getString(
            R.string.default_ride_name,
            RIDE_NAME_FORMATTER.format(Instant.ofEpochMilli(nowEpochMillis)),
        )

    private fun formatDuration(durationMillis: Long): String {
        val totalSeconds = durationMillis.coerceAtLeast(0L) / 1_000L
        val hours = totalSeconds / 3_600L
        val minutes = (totalSeconds % 3_600L) / 60L
        val seconds = totalSeconds % 60L
        return String.format(Locale.ROOT, "%02d:%02d:%02d", hours, minutes, seconds)
    }

    private fun formatDistance(distanceMeters: Double): String =
        if (distanceMeters < 1_000.0) {
            String.format(Locale.getDefault(), "%.0f m", distanceMeters)
        } else {
            String.format(Locale.getDefault(), "%.2f km", distanceMeters / 1_000.0)
        }

    companion object {
        @Volatile
        internal var isRunning: Boolean = false
            private set

        internal const val ACTION_START = "com.gpxanimator.mobile.recording.action.START"
        internal const val ACTION_RECOVER = "com.gpxanimator.mobile.recording.action.RECOVER"
        internal const val ACTION_FINISH = "com.gpxanimator.mobile.recording.action.FINISH"

        private const val TAG = "RideRecordingService"
        private const val NOTIFICATION_CHANNEL_ID = "ride_recording"
        private const val NOTIFICATION_ID = 1_001
        private const val LOCATION_INTERVAL_MILLIS = 1_000L
        private const val MIN_UPDATE_DISTANCE_METERS = 3f
        private const val MAX_UPDATE_DELAY_MILLIS = 2_000L
        private const val NOTIFICATION_UPDATE_INTERVAL_MILLIS = 5_000L
        private val RIDE_NAME_FORMATTER =
            DateTimeFormatter.ofPattern("yyyy-MM-dd HH:mm")
                .withZone(ZoneId.systemDefault())
    }
}
