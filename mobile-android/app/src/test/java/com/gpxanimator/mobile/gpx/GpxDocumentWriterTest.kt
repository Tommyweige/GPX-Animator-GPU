package com.gpxanimator.mobile.gpx

import com.gpxanimator.mobile.data.SyncState
import com.gpxanimator.mobile.data.TrackPointEntity
import com.gpxanimator.mobile.data.TripEntity
import com.gpxanimator.mobile.data.TripState
import java.io.StringReader
import javax.xml.parsers.DocumentBuilderFactory
import org.junit.Assert.assertEquals
import org.junit.Assert.assertThrows
import org.junit.Assert.assertTrue
import org.junit.Test
import org.xml.sax.InputSource

class GpxDocumentWriterTest {
    @Test
    fun writesEscapedGpxEleTimeAndExtensions() {
        val xml = GpxDocumentWriter.write(trip(name = "East & <West>"), listOf(point(0), point(1)))
        val document =
            DocumentBuilderFactory
                .newInstance()
                .apply { isNamespaceAware = true }
                .newDocumentBuilder()
                .parse(InputSource(StringReader(xml)))

        assertEquals("gpx", document.documentElement.localName)
        assertEquals(2, document.getElementsByTagNameNS("*", "trkpt").length)
        assertEquals(
            "East & <West>",
            document.getElementsByTagNameNS("*", "name").item(0).textContent
        )
        assertEquals(2, document.getElementsByTagNameNS("*", "time").length - 1)
        assertTrue(xml.contains("<ele>12.50</ele>"))
        assertTrue(xml.contains("<gpxa:accuracy>4.0</gpxa:accuracy>"))
    }

    @Test
    fun requiresAtLeastTwoPoints() {
        assertThrows(GpxExportException::class.java) {
            GpxDocumentWriter.write(trip(), listOf(point(0)))
        }
    }

    @Test
    fun producesStableSafeFileName() {
        val name = GpxDocumentWriter.fileName(trip(name = "Taipei / Night:Ride"))
        assertEquals("2026-07-17_12-00_Taipei _ Night_Ride_12345678.gpx", name)
    }

    private fun trip(name: String = "Ride") =
        TripEntity(
            id = "12345678-90ab-cdef-1234-567890abcdef",
            name = name,
            state = TripState.FINALIZING,
            syncState = SyncState.LOCAL_ONLY,
            startedAtEpochMillis = 1_784_289_600_000,
            startElapsedRealtimeNanos = 1_000,
            updatedAtEpochMillis = 1_784_289_600_000,
        )

    private fun point(sequence: Int) =
        TrackPointEntity(
            tripId = "12345678-90ab-cdef-1234-567890abcdef",
            sequence = sequence,
            latitude = 25.0 + sequence * 0.001,
            longitude = 121.0 + sequence * 0.001,
            altitudeMeters = 12.5,
            horizontalAccuracyMeters = 4f,
            verticalAccuracyMeters = 6f,
            speedMetersPerSecond = 10f,
            bearingDegrees = 90f,
            timestampEpochMillis = 1_784_289_600_000 + sequence * 1_000,
            elapsedRealtimeNanos = 1_000 + sequence * 1_000_000_000L,
        )
}
