package com.gpxanimator.mobile.sync

import java.io.ByteArrayInputStream
import java.io.ByteArrayOutputStream
import java.io.InputStream
import java.net.HttpURLConnection
import java.net.URL
import java.nio.charset.StandardCharsets
import org.junit.Assert.assertEquals
import org.junit.Assert.assertThrows
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder

class DriveRestClientProtocolTest {
    @get:Rule
    val temporaryFolder = TemporaryFolder()

    @Test
    fun `multipart upload includes metadata bytes and bearer only in header`() {
        val connection =
            FakeConnection(
                url = URL("https://www.googleapis.com/upload/drive/v3/files"),
                statusCode = 200,
                responseBody = """{"id":"remote-1","name":"ride.gpx","trashed":false}""",
            )
        val client = DriveRestClient("secret-token") { connection }
        val file = temporaryFolder.newFile("ride.gpx").apply { writeText("<gpx />") }

        val uploaded =
            client.uploadMultipart(
                file,
                DriveUploadMetadata(
                    id = "generated-1",
                    name = "ride.gpx",
                    parentId = "folder-1",
                    tripId = "trip-1",
                    sha256 = "abc123",
                ),
            )

        val body = connection.requestBody.toString(StandardCharsets.UTF_8.name())
        assertEquals("remote-1", uploaded.id)
        assertEquals("Bearer secret-token", connection.getRequestProperty("Authorization"))
        assertTrue(body.contains("\"id\":\"generated-1\""))
        assertTrue(body.contains("\"tripId\":\"trip-1\""))
        assertTrue(body.contains("<gpx />"))
        assertTrue(!body.contains("secret-token"))
    }

    @Test
    fun `status probe resumes after the acknowledged byte`() {
        val connection =
            FakeConnection(
                url = URL("https://www.googleapis.com/upload/drive/v3/files?upload_id=one"),
                statusCode = 308,
                headers = mapOf("Range" to "bytes=0-524287"),
            )
        val client = DriveRestClient("token") { connection }

        val status = client.queryResumableSession(connection.url, 1_048_576L)

        assertEquals(ResumableSessionStatus.InProgress(524_288L), status)
        assertEquals("bytes */1048576", connection.getRequestProperty("Content-Range"))
    }

    @Test
    fun `expired session is detected and chunk response without range is rejected`() {
        val expiredConnection =
            FakeConnection(
                url = URL("https://www.googleapis.com/upload/drive/v3/files?upload_id=expired"),
                statusCode = 404,
                responseBody = "{}",
            )
        val expiredClient = DriveRestClient("token") { expiredConnection }
        assertEquals(
            ResumableSessionStatus.Expired,
            expiredClient.queryResumableSession(expiredConnection.url, 1_024L),
        )

        val noRangeConnection =
            FakeConnection(
                url = URL("https://www.googleapis.com/upload/drive/v3/files?upload_id=no-range"),
                statusCode = 308,
            )
        val noRangeClient = DriveRestClient("token") { noRangeConnection }
        val file = temporaryFolder.newFile("chunk.gpx").apply { writeBytes(ByteArray(1_024) { 7 }) }
        assertThrows(java.io.IOException::class.java) {
            noRangeClient.uploadResumable(
                sessionUrl = noRangeConnection.url,
                file = file,
                startOffset = 0L,
                shouldContinue = { true },
            )
        }
    }

    @Test
    fun `resumable upload cooperates with worker cancellation before sending`() {
        val connection =
            FakeConnection(
                url = URL("https://www.googleapis.com/upload/drive/v3/files?upload_id=cancel"),
                statusCode = 200,
            )
        val client = DriveRestClient("token") { connection }
        val file = temporaryFolder.newFile("cancel.gpx").apply { writeBytes(ByteArray(128)) }

        assertThrows(java.util.concurrent.CancellationException::class.java) {
            client.uploadResumable(
                sessionUrl = connection.url,
                file = file,
                startOffset = 0L,
                shouldContinue = { false },
            )
        }
        assertEquals(0, connection.requestBody.size())
    }

    @Test
    fun `expired chunk session is restarted at most once`() {
        val sessionUrl = URL("https://www.googleapis.com/upload/drive/v3/files?upload_id=expired")

        assertTrue(
            shouldRestartExpiredResumableSession(
                DriveHttpException(404, null),
                sessionUrl,
                alreadyRetried = false,
            ),
        )
        assertTrue(
            shouldRestartExpiredResumableSession(
                DriveHttpException(410, null),
                sessionUrl,
                alreadyRetried = false,
            ),
        )
        assertTrue(
            !shouldRestartExpiredResumableSession(
                DriveHttpException(404, null),
                sessionUrl,
                alreadyRetried = true,
            ),
        )
        assertTrue(
            !shouldRestartExpiredResumableSession(
                DriveHttpException(500, null),
                sessionUrl,
                alreadyRetried = false,
            ),
        )
    }
}

private class FakeConnection(
    url: URL,
    private val statusCode: Int,
    private val responseBody: String = "",
    private val headers: Map<String, String> = emptyMap(),
) : HttpURLConnection(url) {
    val requestBody = ByteArrayOutputStream()

    override fun connect() = Unit

    override fun disconnect() = Unit

    override fun usingProxy(): Boolean = false

    override fun getResponseCode(): Int = statusCode

    override fun getInputStream(): InputStream =
        ByteArrayInputStream(responseBody.toByteArray(StandardCharsets.UTF_8))

    override fun getErrorStream(): InputStream =
        ByteArrayInputStream(responseBody.toByteArray(StandardCharsets.UTF_8))

    override fun getOutputStream(): ByteArrayOutputStream = requestBody

    override fun getHeaderField(name: String?): String? = headers[name]
}
