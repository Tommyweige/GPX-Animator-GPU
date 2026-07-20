package com.gpxanimator.mobile.sync

import java.net.URL
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class DriveRestClientSecurityTest {
    @Test
    fun `bearer credentials are limited to https googleapis hosts`() {
        assertTrue(URL("https://www.googleapis.com/upload/drive/v3/files").isTrustedGoogleApisUrl())
        assertTrue(URL("https://content.googleapis.com/upload/drive/v3/files").isTrustedGoogleApisUrl())
        assertFalse(URL("http://www.googleapis.com/upload/drive/v3/files").isTrustedGoogleApisUrl())
        assertFalse(URL("https://googleapis.com.example.test/upload").isTrustedGoogleApisUrl())
        assertFalse(URL("https://example.test/upload").isTrustedGoogleApisUrl())
    }
}
