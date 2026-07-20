package com.gpxanimator.mobile.sync

import java.io.ByteArrayOutputStream
import java.io.File
import java.io.IOException
import java.io.RandomAccessFile
import java.net.HttpURLConnection
import java.net.URL
import java.net.URLEncoder
import java.nio.charset.StandardCharsets
import java.util.UUID
import org.json.JSONArray
import org.json.JSONObject

internal data class DriveFile(
    val id: String,
    val name: String?,
    val mimeType: String?,
    val trashed: Boolean,
    val parents: List<String>,
    val appProperties: Map<String, String>,
)

internal data class DriveUploadMetadata(
    val id: String,
    val name: String,
    val parentId: String,
    val tripId: String,
    val sha256: String,
)

internal sealed interface ResumableSessionStatus {
    data class InProgress(val nextOffset: Long) : ResumableSessionStatus

    data class Complete(val file: DriveFile) : ResumableSessionStatus

    data object Expired : ResumableSessionStatus
}

internal class DriveHttpException(
    val statusCode: Int,
    val driveReason: String?,
) : IOException(
    buildString {
        append("Drive HTTP ")
        append(statusCode)
        if (!driveReason.isNullOrBlank()) {
            append(" (")
            append(driveReason)
            append(')')
        }
    },
)

/** Minimal Drive v3 client. The bearer token is only ever written to the Authorization header. */
internal class DriveRestClient(
    private val accessToken: String,
    private val connectionFactory: (URL) -> HttpURLConnection =
        { url -> url.openConnection() as HttpURLConnection },
) {
    @Volatile
    private var activeConnection: HttpURLConnection? = null

    fun cancel() {
        activeConnection?.disconnect()
    }

    fun getFile(fileId: String): DriveFile? =
        try {
            val response =
                request(
                    method = "GET",
                    url =
                        apiUrl(
                            "/drive/v3/files/${encodePath(fileId)}",
                            mapOf("fields" to FILE_FIELDS),
                        ),
                )
            parseFile(JSONObject(response.body))
        } catch (error: DriveHttpException) {
            if (error.statusCode == 404) null else throw error
        }

    fun findFolder(parentId: String, marker: String): DriveFile? {
        val query =
            "mimeType = '$FOLDER_MIME_TYPE' and trashed = false and " +
                    "'${escapeQueryLiteral(parentId)}' in parents and " +
                    "appProperties has { key='$FOLDER_MARKER_KEY' and " +
                    "value='${escapeQueryLiteral(marker)}' }"
        return listFiles(query).firstOrNull()
    }

    fun createFolder(
        name: String,
        parentId: String,
        marker: String,
    ): DriveFile {
        val metadata =
            JSONObject()
                .put("name", name)
                .put("mimeType", FOLDER_MIME_TYPE)
                .put("parents", JSONArray().put(parentId))
                .put(
                    "appProperties",
                    JSONObject()
                        .put(FOLDER_MARKER_KEY, marker)
                        .put(SCHEMA_VERSION_KEY, SCHEMA_VERSION),
                )
        val response =
            request(
                method = "POST",
                url = apiUrl("/drive/v3/files", mapOf("fields" to FILE_FIELDS)),
                contentType = JSON_CONTENT_TYPE,
                body = metadata.toString().toByteArray(StandardCharsets.UTF_8),
            )
        return parseFile(JSONObject(response.body))
    }

    fun findUploadedFile(
        parentId: String,
        tripId: String,
        sha256: String,
    ): DriveFile? {
        val query =
            "trashed = false and '${escapeQueryLiteral(parentId)}' in parents and " +
                    "appProperties has { key='$TRIP_ID_KEY' and " +
                    "value='${escapeQueryLiteral(tripId)}' } and " +
                    "appProperties has { key='$SHA_256_KEY' and " +
                    "value='${escapeQueryLiteral(sha256)}' }"
        return listFiles(query).firstOrNull()
    }

    fun generateFileId(): String {
        val response =
            request(
                method = "GET",
                url =
                    apiUrl(
                        "/drive/v3/files/generateIds",
                        mapOf(
                            "count" to "1",
                            "space" to "drive",
                            "type" to "files",
                        ),
                    ),
            )
        val ids = JSONObject(response.body).optJSONArray("ids")
        return ids?.optString(0)?.takeIf(String::isNotBlank)
            ?: throw IOException("Drive did not return a generated file ID.")
    }

    fun shouldUseMultipart(file: File): Boolean = file.length() <= MULTIPART_LIMIT_BYTES

    fun uploadMultipart(file: File, metadata: DriveUploadMetadata): DriveFile {
        val boundary = "gpx-animator-${UUID.randomUUID()}"
        val body =
            ByteArrayOutputStream(
                (file.length() + 1024L).coerceAtMost(Int.MAX_VALUE.toLong()).toInt()
            )
                .use { output ->
                    output.write("--$boundary\r\n".utf8())
                    output.write("Content-Type: $JSON_CONTENT_TYPE\r\n\r\n".utf8())
                    output.write(metadata.toJson().toString().utf8())
                    output.write("\r\n--$boundary\r\n".utf8())
                    output.write("Content-Type: $GPX_MIME_TYPE\r\n\r\n".utf8())
                    file.inputStream().buffered().use { input -> input.copyTo(output) }
                    output.write("\r\n--$boundary--\r\n".utf8())
                    output.toByteArray()
                }
        val response =
            request(
                method = "POST",
                url =
                    apiUrl(
                        "/upload/drive/v3/files",
                        mapOf(
                            "uploadType" to "multipart",
                            "fields" to FILE_FIELDS,
                        ),
                    ),
                contentType = "multipart/related; boundary=$boundary",
                body = body,
            )
        return parseFile(JSONObject(response.body))
    }

    fun initiateResumable(file: File, metadata: DriveUploadMetadata): URL {
        val initiation =
            request(
                method = "POST",
                url =
                    apiUrl(
                        "/upload/drive/v3/files",
                        mapOf(
                            "uploadType" to "resumable",
                            "fields" to FILE_FIELDS,
                        ),
                    ),
                contentType = JSON_CONTENT_TYPE,
                body = metadata.toJson().toString().utf8(),
                extraHeaders =
                    mapOf(
                        "X-Upload-Content-Type" to GPX_MIME_TYPE,
                        "X-Upload-Content-Length" to file.length().toString(),
                    ),
            )
        return URL(
            initiation.location ?: throw IOException("Drive returned no resumable upload URL.")
        )
    }

    fun queryResumableSession(sessionUrl: URL, totalLength: Long): ResumableSessionStatus =
        try {
            val response =
                request(
                    method = "PUT",
                    url = sessionUrl,
                    contentType = GPX_MIME_TYPE,
                    body = ByteArray(0),
                    extraHeaders = mapOf("Content-Range" to "bytes */$totalLength"),
                    acceptedStatusCodes = setOf(200, 201, 308),
                )
            when (response.statusCode) {
                200,
                201,
                    -> ResumableSessionStatus.Complete(parseFile(JSONObject(response.body)))

                else -> {
                    val nextOffset = (response.rangeEndInclusive ?: -1L) + 1L
                    if (nextOffset !in 0L..totalLength) {
                        throw IOException("Drive returned an invalid resumable session range.")
                    }
                    ResumableSessionStatus.InProgress(nextOffset)
                }
            }
        } catch (error: DriveHttpException) {
            if (error.statusCode == 404 || error.statusCode == 410) {
                ResumableSessionStatus.Expired
            } else {
                throw error
            }
        }

    fun uploadResumable(
        sessionUrl: URL,
        file: File,
        startOffset: Long,
        shouldContinue: () -> Boolean,
    ): DriveFile {
        val totalLength = file.length()
        var offset = startOffset
        if (offset !in 0L until totalLength) {
            throw IOException("Drive returned an invalid resumable upload offset.")
        }
        RandomAccessFile(file, "r").use { input ->
            while (offset < totalLength) {
                if (!shouldContinue()) {
                    throw java.util.concurrent.CancellationException("Drive upload was cancelled.")
                }
                val bytesToRead =
                    minOf(RESUMABLE_CHUNK_BYTES.toLong(), totalLength - offset).toInt()
                val chunk = ByteArray(bytesToRead)
                input.seek(offset)
                input.readFully(chunk)
                val response =
                    request(
                        method = "PUT",
                        url = sessionUrl,
                        contentType = GPX_MIME_TYPE,
                        body = chunk,
                        extraHeaders =
                            mapOf(
                                "Content-Range" to
                                        "bytes $offset-${offset + bytesToRead - 1}/$totalLength",
                            ),
                        acceptedStatusCodes = setOf(200, 201, 308),
                    )

                if (response.statusCode == 200 || response.statusCode == 201) {
                    return parseFile(JSONObject(response.body))
                }

                val acknowledgedEnd =
                    response.rangeEndInclusive
                        ?: throw IOException("Drive acknowledged no bytes for a resumable chunk.")
                val nextOffset = acknowledgedEnd + 1L
                if (nextOffset <= offset || nextOffset > totalLength) {
                    throw IOException("Drive returned an invalid resumable upload range.")
                }
                offset = nextOffset
            }
        }
        throw IOException("Drive resumable upload ended without a file response.")
    }

    private fun listFiles(query: String): List<DriveFile> {
        val response =
            request(
                method = "GET",
                url =
                    apiUrl(
                        "/drive/v3/files",
                        mapOf(
                            "q" to query,
                            "spaces" to "drive",
                            "pageSize" to "10",
                            "fields" to "files($FILE_FIELDS)",
                        ),
                    ),
            )
        val files = JSONObject(response.body).optJSONArray("files") ?: return emptyList()
        return buildList {
            for (index in 0 until files.length()) {
                files.optJSONObject(index)?.let { add(parseFile(it)) }
            }
        }
    }

    private fun request(
        method: String,
        url: URL,
        contentType: String? = null,
        body: ByteArray? = null,
        extraHeaders: Map<String, String> = emptyMap(),
        acceptedStatusCodes: Set<Int> = emptySet(),
    ): HttpResponse {
        if (!url.isTrustedGoogleApisUrl()) {
            throw IOException("Drive returned an untrusted upload URL.")
        }
        val connection = connectionFactory(url)
        activeConnection = connection
        return try {
            connection.requestMethod = method
            connection.instanceFollowRedirects = false
            connection.connectTimeout = CONNECT_TIMEOUT_MILLIS
            connection.readTimeout = READ_TIMEOUT_MILLIS
            connection.setRequestProperty("Authorization", "Bearer $accessToken")
            connection.setRequestProperty("Accept", "application/json")
            contentType?.let { connection.setRequestProperty("Content-Type", it) }
            extraHeaders.forEach(connection::setRequestProperty)

            if (body != null) {
                connection.doOutput = true
                connection.setFixedLengthStreamingMode(body.size)
                connection.outputStream.use { it.write(body) }
            }

            val statusCode = connection.responseCode
            val responseBody =
                (if (statusCode in 200..399) connection.inputStream else connection.errorStream)
                    ?.bufferedReader(StandardCharsets.UTF_8)
                    ?.use { it.readText() }
                    .orEmpty()

            if (statusCode !in 200..299 && statusCode !in acceptedStatusCodes) {
                throw DriveHttpException(statusCode, parseDriveErrorReason(responseBody))
            }

            HttpResponse(
                statusCode = statusCode,
                body = responseBody,
                location = connection.getHeaderField("Location"),
                rangeEndInclusive = parseRangeEnd(connection.getHeaderField("Range")),
            )
        } finally {
            if (activeConnection === connection) activeConnection = null
            connection.disconnect()
        }
    }

    private fun apiUrl(path: String, parameters: Map<String, String>): URL {
        val query =
            parameters.entries.joinToString("&") { (key, value) ->
                "${encodeQuery(key)}=${encodeQuery(value)}"
            }
        return URL("$API_BASE_URL$path${if (query.isEmpty()) "" else "?$query"}")
    }

    private data class HttpResponse(
        val statusCode: Int,
        val body: String,
        val location: String?,
        val rangeEndInclusive: Long?,
    )

    companion object {
        private const val API_BASE_URL = "https://www.googleapis.com"
        private const val JSON_CONTENT_TYPE = "application/json; charset=UTF-8"
        private const val GPX_MIME_TYPE = "application/gpx+xml"
        private const val FOLDER_MIME_TYPE = "application/vnd.google-apps.folder"
        private const val FOLDER_MARKER_KEY = "gpxAnimatorFolder"
        private const val SCHEMA_VERSION_KEY = "schemaVersion"
        private const val SCHEMA_VERSION = "1"
        private const val TRIP_ID_KEY = "tripId"
        private const val SHA_256_KEY = "sha256"
        private const val MULTIPART_LIMIT_BYTES = 5L * 1024L * 1024L
        private const val RESUMABLE_CHUNK_BYTES = 8 * 1024 * 1024
        private const val CONNECT_TIMEOUT_MILLIS = 20_000
        private const val READ_TIMEOUT_MILLIS = 60_000
        private const val FILE_FIELDS = "id,name,mimeType,trashed,parents,appProperties"

        fun folderMarkerRoot(): String = "root-v1"

        fun folderMarkerRoutes(): String = "routes-v1"

        fun folderMarkerYear(year: Int): String = "year-$year-v1"
    }
}

private fun DriveUploadMetadata.toJson(): JSONObject =
    JSONObject()
        .put("id", id)
        .put("name", name)
        .put("mimeType", "application/gpx+xml")
        .put("parents", JSONArray().put(parentId))
        .put(
            "appProperties",
            JSONObject()
                .put("tripId", tripId)
                .put("sha256", sha256)
                .put("schemaVersion", "1")
                .put("source", "GPX Animator Ride"),
        )

private fun parseFile(json: JSONObject): DriveFile {
    val appProperties = mutableMapOf<String, String>()
    json.optJSONObject("appProperties")?.let { properties ->
        val keys = properties.keys()
        while (keys.hasNext()) {
            val key = keys.next()
            properties.optString(key).takeIf(String::isNotBlank)?.let { appProperties[key] = it }
        }
    }
    val parents = buildList {
        val values = json.optJSONArray("parents") ?: return@buildList
        for (index in 0 until values.length()) {
            values.optString(index).takeIf(String::isNotBlank)?.let(::add)
        }
    }
    return DriveFile(
        id = json.getString("id"),
        name = json.optString("name").takeIf(String::isNotBlank),
        mimeType = json.optString("mimeType").takeIf(String::isNotBlank),
        trashed = json.optBoolean("trashed", false),
        parents = parents,
        appProperties = appProperties,
    )
}

private fun parseDriveErrorReason(body: String): String? {
    return try {
        val error = JSONObject(body).optJSONObject("error") ?: return null
        val legacyReason =
            error.optJSONArray("errors")
                ?.optJSONObject(0)
                ?.optString("reason")
                ?.takeIf(String::isNotBlank)
        legacyReason
            ?: error.optString("status").takeIf(String::isNotBlank)
            ?: error.optString("message").takeIf(String::isNotBlank)?.take(120)
    } catch (_: Exception) {
        null
    }
}

private fun parseRangeEnd(value: String?): Long? {
    val normalized = value?.substringAfter("bytes=", missingDelimiterValue = "") ?: return null
    return normalized.substringAfter('-', missingDelimiterValue = "").toLongOrNull()
}

private fun encodeQuery(value: String): String =
    URLEncoder.encode(value, StandardCharsets.UTF_8.name()).replace("+", "%20")

private fun encodePath(value: String): String = encodeQuery(value)

internal fun URL.isTrustedGoogleApisUrl(): Boolean =
    protocol.equals("https", ignoreCase = true) &&
            (host.equals("googleapis.com", ignoreCase = true) ||
                    host.endsWith(".googleapis.com", ignoreCase = true))

private fun escapeQueryLiteral(value: String): String =
    value.replace("\\", "\\\\").replace("'", "\\'")

private fun String.utf8(): ByteArray = toByteArray(StandardCharsets.UTF_8)
