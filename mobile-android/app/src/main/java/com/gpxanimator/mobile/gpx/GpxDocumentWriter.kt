package com.gpxanimator.mobile.gpx

import com.gpxanimator.mobile.data.TrackPointEntity
import com.gpxanimator.mobile.data.TripEntity
import java.time.Instant
import java.time.ZoneOffset
import java.time.ZoneId
import java.time.format.DateTimeFormatter
import java.util.Locale

class GpxExportException(message: String, cause: Throwable? = null) : Exception(message, cause)

object GpxDocumentWriter {
    private const val GPX_NAMESPACE = "http://www.topografix.com/GPX/1/1"
    private const val EXTENSION_NAMESPACE =
        "https://gpxanimator.app/xmlschemas/TrackPointExtension/1"

    fun write(trip: TripEntity, points: List<TrackPointEntity>): String {
        if (points.size < 2) {
            throw GpxExportException("A GPX track requires at least two valid points")
        }

        return buildString(points.size * 220 + 512) {
            append("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n")
            append("<gpx version=\"1.1\" creator=\"GPX Animator Ride\"")
            append(" xmlns=\"").append(GPX_NAMESPACE).append('"')
            append(" xmlns:gpxa=\"").append(EXTENSION_NAMESPACE).append("\">\n")
            append("  <metadata>\n")
            append("    <name>").appendXml(trip.name).append("</name>\n")
            append("    <time>").append(formatInstant(trip.startedAtEpochMillis))
                .append("</time>\n")
            append("  </metadata>\n")
            append("  <trk>\n")
            append("    <name>").appendXml(trip.name).append("</name>\n")
            append("    <trkseg>\n")
            points.forEach { point -> appendPoint(point) }
            append("    </trkseg>\n")
            append("  </trk>\n")
            append("</gpx>\n")
        }
    }

    fun safeFileStem(value: String): String {
        val cleaned =
            value
                .replace(Regex("[<>:\"/\\\\|?*\\p{Cc}]"), "_")
                .replace(Regex("\\s+"), " ")
                .trim(' ', '.')
                .take(60)
        return cleaned.ifBlank { "Ride" }
    }

    fun fileName(trip: TripEntity): String {
        val timestamp =
            DateTimeFormatter
                .ofPattern("yyyy-MM-dd_HH-mm", Locale.US)
                .withZone(runCatching { ZoneId.of(trip.startZoneId) }.getOrDefault(ZoneOffset.UTC))
                .format(Instant.ofEpochMilli(trip.startedAtEpochMillis))
        val id = trip.id.replace("-", "").take(8).ifBlank { "unknown" }
        return "${timestamp}_${safeFileStem(trip.name)}_$id.gpx"
    }

    private fun StringBuilder.appendPoint(point: TrackPointEntity) {
        append("      <trkpt lat=\"")
            .append(decimal(point.latitude, 8))
            .append("\" lon=\"")
            .append(decimal(point.longitude, 8))
            .append("\">\n")
        point.altitudeMeters?.takeIf { it.isFinite() }?.let {
            append("        <ele>").append(decimal(it, 2)).append("</ele>\n")
        }
        append("        <time>")
            .append(formatInstant(point.timestampEpochMillis))
            .append("</time>\n")
        append("        <extensions>\n")
        append("          <gpxa:accuracy>")
            .append(decimal(point.horizontalAccuracyMeters.toDouble(), 1))
            .append("</gpxa:accuracy>\n")
        point.verticalAccuracyMeters?.takeIf { it.isFinite() }?.let {
            append("          <gpxa:verticalAccuracy>")
                .append(decimal(it.toDouble(), 1))
                .append("</gpxa:verticalAccuracy>\n")
        }
        point.speedMetersPerSecond?.takeIf { it.isFinite() }?.let {
            append("          <gpxa:speed>")
                .append(decimal(it.toDouble(), 3))
                .append("</gpxa:speed>\n")
        }
        point.bearingDegrees?.takeIf { it.isFinite() }?.let {
            append("          <gpxa:bearing>")
                .append(decimal(it.toDouble(), 1))
                .append("</gpxa:bearing>\n")
        }
        append("        </extensions>\n")
        append("      </trkpt>\n")
    }

    private fun StringBuilder.appendXml(value: String): StringBuilder =
        append(
            value
                .filter { it == '\t' || it == '\n' || it == '\r' || it >= ' ' }
                .replace("&", "&amp;")
                .replace("<", "&lt;")
                .replace(">", "&gt;")
                .replace("\"", "&quot;")
                .replace("'", "&apos;"),
        )

    private fun formatInstant(epochMillis: Long): String =
        DateTimeFormatter.ISO_INSTANT.format(Instant.ofEpochMilli(epochMillis))

    private fun decimal(value: Double, precision: Int): String =
        String.format(Locale.US, "%.${precision}f", value)
}
