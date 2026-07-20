package com.gpxanimator.mobile.recording

import kotlin.math.asin
import kotlin.math.cos
import kotlin.math.sin
import kotlin.math.sqrt

internal data class LocationSample(
    val latitude: Double,
    val longitude: Double,
    val altitudeMeters: Double?,
    val horizontalAccuracyMeters: Float,
    val verticalAccuracyMeters: Float?,
    val speedMetersPerSecond: Float?,
    val bearingDegrees: Float?,
    val elapsedRealtimeNanos: Long,
)

internal object LocationSampleFilter {
    private const val MAX_AGE_NANOS = 10_000_000_000L
    private const val MAX_HORIZONTAL_ACCURACY_METERS = 50f
    private const val MAX_JUMP_SPEED_METERS_PER_SECOND = 250.0 / 3.6

    fun accepts(
        sample: LocationSample,
        nowElapsedRealtimeNanos: Long,
        previous: LocationSample?,
    ): Boolean {
        if (!sample.latitude.isFinite() || sample.latitude !in -90.0..90.0) return false
        if (!sample.longitude.isFinite() || sample.longitude !in -180.0..180.0) return false
        if (!sample.horizontalAccuracyMeters.isFinite() ||
            sample.horizontalAccuracyMeters < 0f ||
            sample.horizontalAccuracyMeters > MAX_HORIZONTAL_ACCURACY_METERS
        ) {
            return false
        }

        val ageNanos = nowElapsedRealtimeNanos - sample.elapsedRealtimeNanos
        if (sample.elapsedRealtimeNanos <= 0L || ageNanos !in 0L..MAX_AGE_NANOS) return false

        if (previous != null) {
            val elapsedNanos = sample.elapsedRealtimeNanos - previous.elapsedRealtimeNanos
            if (elapsedNanos <= 0L) return false
            val elapsedSeconds = elapsedNanos / 1_000_000_000.0
            val calculatedSpeed =
                GeoDistance.haversineMeters(
                    previous.latitude,
                    previous.longitude,
                    sample.latitude,
                    sample.longitude,
                ) / elapsedSeconds
            if (calculatedSpeed > MAX_JUMP_SPEED_METERS_PER_SECOND) return false
        }

        return true
    }
}

internal object GeoDistance {
    private const val EARTH_RADIUS_METERS = 6_371_008.8

    fun haversineMeters(
        latitude1: Double,
        longitude1: Double,
        latitude2: Double,
        longitude2: Double,
    ): Double {
        val latitudeDelta = Math.toRadians(latitude2 - latitude1)
        val longitudeDelta = Math.toRadians(longitude2 - longitude1)
        val latitude1Radians = Math.toRadians(latitude1)
        val latitude2Radians = Math.toRadians(latitude2)

        val haversine =
            sin(latitudeDelta / 2.0).let { it * it } +
                    cos(latitude1Radians) *
                    cos(latitude2Radians) *
                    sin(longitudeDelta / 2.0).let { it * it }
        return 2.0 * EARTH_RADIUS_METERS * asin(sqrt(haversine.coerceIn(0.0, 1.0)))
    }
}

internal object MonotonicRideClock {
    fun timestampEpochMillis(
        startEpochMillis: Long,
        startElapsedRealtimeNanos: Long,
        sampleElapsedRealtimeNanos: Long,
    ): Long {
        require(sampleElapsedRealtimeNanos >= startElapsedRealtimeNanos) {
            "A location sample cannot predate the recording start"
        }
        return startEpochMillis +
                (sampleElapsedRealtimeNanos - startElapsedRealtimeNanos) / 1_000_000L
    }

    fun durationMillis(
        startElapsedRealtimeNanos: Long,
        endElapsedRealtimeNanos: Long,
    ): Long =
        if (endElapsedRealtimeNanos < startElapsedRealtimeNanos) {
            0L
        } else {
            (endElapsedRealtimeNanos - startElapsedRealtimeNanos) / 1_000_000L
        }
}
