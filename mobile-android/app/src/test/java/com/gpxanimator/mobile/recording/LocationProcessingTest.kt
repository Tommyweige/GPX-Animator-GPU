package com.gpxanimator.mobile.recording

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class LocationProcessingTest {
    @Test
    fun `filter accepts a fresh accurate valid sample`() {
        val sample = sample(elapsedRealtimeNanos = 15_000_000_000L)

        assertTrue(
            LocationSampleFilter.accepts(
                sample = sample,
                nowElapsedRealtimeNanos = 20_000_000_000L,
                previous = null,
            ),
        )
    }

    @Test
    fun `filter rejects invalid stale and inaccurate samples`() {
        val now = 20_000_000_000L

        assertFalse(
            LocationSampleFilter.accepts(
                sample = sample(latitude = 91.0, elapsedRealtimeNanos = 15_000_000_000L),
                nowElapsedRealtimeNanos = now,
                previous = null,
            ),
        )
        assertFalse(
            LocationSampleFilter.accepts(
                sample = sample(elapsedRealtimeNanos = 9_999_999_999L),
                nowElapsedRealtimeNanos = now,
                previous = null,
            ),
        )
        assertFalse(
            LocationSampleFilter.accepts(
                sample =
                    sample(
                        horizontalAccuracyMeters = 50.01f,
                        elapsedRealtimeNanos = 15_000_000_000L,
                    ),
                nowElapsedRealtimeNanos = now,
                previous = null,
            ),
        )
    }

    @Test
    fun `filter rejects a greater than 250 kilometer per hour isolated jump`() {
        val previous = sample(longitude = 121.0, elapsedRealtimeNanos = 10_000_000_000L)
        val jump = sample(longitude = 121.001, elapsedRealtimeNanos = 11_000_000_000L)

        assertFalse(
            LocationSampleFilter.accepts(
                sample = jump,
                nowElapsedRealtimeNanos = 11_000_000_000L,
                previous = previous,
            ),
        )
    }

    @Test
    fun `filter rejects an implausible jump after a long GPS gap`() {
        val previous = sample(longitude = 121.0, elapsedRealtimeNanos = 10_000_000_000L)
        val jump = sample(longitude = 121.1, elapsedRealtimeNanos = 71_000_000_000L)

        assertFalse(
            LocationSampleFilter.accepts(
                sample = jump,
                nowElapsedRealtimeNanos = 71_000_000_000L,
                previous = previous,
            ),
        )
    }

    @Test
    fun `filter permits a plausible cycling movement`() {
        val previous = sample(longitude = 121.0, elapsedRealtimeNanos = 10_000_000_000L)
        val nearby = sample(longitude = 121.00005, elapsedRealtimeNanos = 11_000_000_000L)

        assertTrue(
            LocationSampleFilter.accepts(
                sample = nearby,
                nowElapsedRealtimeNanos = 11_000_000_000L,
                previous = previous,
            ),
        )
    }

    @Test
    fun `haversine returns the expected distance for one longitude degree at equator`() {
        val distanceMeters = GeoDistance.haversineMeters(0.0, 0.0, 0.0, 1.0)

        assertEquals(111_195.08, distanceMeters, 0.1)
        assertEquals(0.0, GeoDistance.haversineMeters(25.0, 121.0, 25.0, 121.0), 0.0)
    }

    @Test
    fun `monotonic clock derives wall time from elapsed realtime`() {
        val timestamp =
            MonotonicRideClock.timestampEpochMillis(
                startEpochMillis = 1_700_000_000_000L,
                startElapsedRealtimeNanos = 10_000_000_000L,
                sampleElapsedRealtimeNanos = 11_500_000_000L,
            )

        assertEquals(1_700_000_001_500L, timestamp)
        assertEquals(1_500L, MonotonicRideClock.durationMillis(10_000_000_000L, 11_500_000_000L))
    }

    private fun sample(
        latitude: Double = 25.0,
        longitude: Double = 121.0,
        horizontalAccuracyMeters: Float = 5f,
        elapsedRealtimeNanos: Long,
    ): LocationSample =
        LocationSample(
            latitude = latitude,
            longitude = longitude,
            altitudeMeters = 10.0,
            horizontalAccuracyMeters = horizontalAccuracyMeters,
            verticalAccuracyMeters = 3f,
            speedMetersPerSecond = 5f,
            bearingDegrees = 90f,
            elapsedRealtimeNanos = elapsedRealtimeNanos,
        )
}
