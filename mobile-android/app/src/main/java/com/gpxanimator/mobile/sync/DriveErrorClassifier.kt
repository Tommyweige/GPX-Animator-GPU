package com.gpxanimator.mobile.sync

enum class DriveFailureKind {
    AUTH_REQUIRED,
    RETRYABLE,
    PERMANENT,
}

/** Pure policy used by the worker and local unit tests. */
object DriveErrorClassifier {
    fun classifyHttp(statusCode: Int, driveReason: String? = null): DriveFailureKind =
        when {
            statusCode == 401 -> DriveFailureKind.AUTH_REQUIRED
            statusCode == 403 && driveReason != null && driveReason in RETRYABLE_DRIVE_REASONS ->
                DriveFailureKind.RETRYABLE

            statusCode == 408 || statusCode == 429 -> DriveFailureKind.RETRYABLE
            statusCode in 500..599 -> DriveFailureKind.RETRYABLE
            else -> DriveFailureKind.PERMANENT
        }

    fun classifyAuthorizationStatus(statusCode: Int): DriveFailureKind =
        when (statusCode) {
            // CommonStatusCodes.SIGN_IN_REQUIRED and RESOLUTION_REQUIRED.
            4,
            6,
                -> DriveFailureKind.AUTH_REQUIRED

            // NETWORK_ERROR, INTERNAL_ERROR, API_NOT_CONNECTED, CONNECTION_SUSPENDED,
            // RECONNECTION_TIMED_OUT, and RECONNECTION_TIMED_OUT_DURING_UPDATE.
            7,
            8,
            17,
            20,
            21,
            22,
                -> DriveFailureKind.RETRYABLE

            else -> DriveFailureKind.PERMANENT
        }

    private val RETRYABLE_DRIVE_REASONS =
        setOf(
            "rateLimitExceeded",
            "userRateLimitExceeded",
            "sharingRateLimitExceeded",
        )
}
