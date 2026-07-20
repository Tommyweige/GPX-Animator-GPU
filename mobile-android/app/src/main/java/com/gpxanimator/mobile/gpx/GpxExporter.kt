package com.gpxanimator.mobile.gpx

import android.content.Context
import android.util.Xml
import com.gpxanimator.mobile.data.RideRepository
import java.io.File
import java.io.FileInputStream
import java.io.FileOutputStream
import java.io.OutputStreamWriter
import java.nio.charset.StandardCharsets
import java.nio.file.AtomicMoveNotSupportedException
import java.nio.file.Files
import java.nio.file.StandardCopyOption
import java.security.MessageDigest
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import org.xmlpull.v1.XmlPullParser

data class GpxExportResult(
    val file: File,
    val sha256: String,
    val pointCount: Int,
)

class GpxExporter(
    private val context: Context,
    private val repository: RideRepository,
) {
    suspend fun export(tripId: String): GpxExportResult =
        withContext(Dispatchers.IO) {
            val trip = repository.getTrip(tripId) ?: throw GpxExportException("Trip not found")
            val points = repository.points(tripId)
            val xml = GpxDocumentWriter.write(trip, points)
            val year =
                java.time.Instant
                    .ofEpochMilli(trip.startedAtEpochMillis)
                    .atZone(
                        runCatching { java.time.ZoneId.of(trip.startZoneId) }
                            .getOrDefault(java.time.ZoneOffset.UTC),
                    )
                    .year
                    .toString()
            val directory = File(context.filesDir, "rides/$year")
            if (!directory.exists() && !directory.mkdirs()) {
                throw GpxExportException("Unable to create the ride export directory")
            }
            val destination = File(directory, GpxDocumentWriter.fileName(trip))
            val temporary = File(directory, "${destination.name}.tmp")

            try {
                FileOutputStream(temporary).use { stream ->
                    OutputStreamWriter(stream, StandardCharsets.UTF_8).use { writer ->
                        writer.write(xml)
                        writer.flush()
                        stream.fd.sync()
                    }
                }
                validate(temporary, points.size)
                moveAtomically(temporary, destination)
                GpxExportResult(
                    file = destination,
                    sha256 = sha256(destination),
                    pointCount = points.size,
                )
            } catch (error: Exception) {
                temporary.delete()
                if (error is GpxExportException) throw error
                throw GpxExportException("Unable to export the GPX file", error)
            }
        }

    private fun validate(file: File, expectedPoints: Int) {
        val parser = Xml.newPullParser()
        var rootSeen = false
        var pointCount = 0
        FileInputStream(file).use { input ->
            parser.setInput(input, StandardCharsets.UTF_8.name())
            while (parser.eventType != XmlPullParser.END_DOCUMENT) {
                if (parser.eventType == XmlPullParser.START_TAG) {
                    if (!rootSeen) {
                        rootSeen = parser.name == "gpx"
                    }
                    if (parser.name == "trkpt") pointCount += 1
                }
                parser.next()
            }
        }
        if (!rootSeen || pointCount != expectedPoints || pointCount < 2) {
            throw GpxExportException("The generated GPX file failed validation")
        }
    }

    private fun moveAtomically(source: File, destination: File) {
        try {
            Files.move(
                source.toPath(),
                destination.toPath(),
                StandardCopyOption.ATOMIC_MOVE,
                StandardCopyOption.REPLACE_EXISTING,
            )
        } catch (_: AtomicMoveNotSupportedException) {
            Files.move(source.toPath(), destination.toPath(), StandardCopyOption.REPLACE_EXISTING)
        }
    }

    private fun sha256(file: File): String {
        val digest = MessageDigest.getInstance("SHA-256")
        FileInputStream(file).use { input ->
            val buffer = ByteArray(DEFAULT_BUFFER_SIZE)
            while (true) {
                val read = input.read(buffer)
                if (read < 0) break
                digest.update(buffer, 0, read)
            }
        }
        return digest
            .digest()
            .joinToString("") { byte -> (byte.toInt() and 0xff).toString(16).padStart(2, '0') }
    }
}
