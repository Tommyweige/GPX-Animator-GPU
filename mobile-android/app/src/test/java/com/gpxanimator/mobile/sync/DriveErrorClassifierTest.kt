package com.gpxanimator.mobile.sync

import org.junit.Assert.assertEquals
import org.junit.Test

class DriveErrorClassifierTest {
    @Test
    fun `401 requires interactive authorization`() {
        assertEquals(DriveFailureKind.AUTH_REQUIRED, DriveErrorClassifier.classifyHttp(401))
    }

    @Test
    fun `request timeout rate limit and server errors retry`() {
        assertEquals(DriveFailureKind.RETRYABLE, DriveErrorClassifier.classifyHttp(408))
        assertEquals(DriveFailureKind.RETRYABLE, DriveErrorClassifier.classifyHttp(429))
        assertEquals(DriveFailureKind.RETRYABLE, DriveErrorClassifier.classifyHttp(500))
        assertEquals(DriveFailureKind.RETRYABLE, DriveErrorClassifier.classifyHttp(503))
        assertEquals(DriveFailureKind.RETRYABLE, DriveErrorClassifier.classifyHttp(599))
    }

    @Test
    fun `403 quota and permission failures are permanent`() {
        assertEquals(DriveFailureKind.PERMANENT, DriveErrorClassifier.classifyHttp(403))
        assertEquals(
            DriveFailureKind.PERMANENT,
            DriveErrorClassifier.classifyHttp(403, "storageQuotaExceeded"),
        )
    }

    @Test
    fun `403 rate limit reasons retry`() {
        assertEquals(
            DriveFailureKind.RETRYABLE,
            DriveErrorClassifier.classifyHttp(403, "rateLimitExceeded"),
        )
        assertEquals(
            DriveFailureKind.RETRYABLE,
            DriveErrorClassifier.classifyHttp(403, "userRateLimitExceeded"),
        )
    }

    @Test
    fun `other client failures are permanent`() {
        assertEquals(DriveFailureKind.PERMANENT, DriveErrorClassifier.classifyHttp(400))
        assertEquals(DriveFailureKind.PERMANENT, DriveErrorClassifier.classifyHttp(404))
        assertEquals(DriveFailureKind.PERMANENT, DriveErrorClassifier.classifyHttp(409))
    }

    @Test
    fun `authorization statuses distinguish resolution retry and configuration failures`() {
        assertEquals(
            DriveFailureKind.AUTH_REQUIRED,
            DriveErrorClassifier.classifyAuthorizationStatus(4),
        )
        assertEquals(
            DriveFailureKind.AUTH_REQUIRED,
            DriveErrorClassifier.classifyAuthorizationStatus(6),
        )
        assertEquals(
            DriveFailureKind.RETRYABLE,
            DriveErrorClassifier.classifyAuthorizationStatus(7),
        )
        assertEquals(
            DriveFailureKind.PERMANENT,
            DriveErrorClassifier.classifyAuthorizationStatus(10),
        )
    }
}
