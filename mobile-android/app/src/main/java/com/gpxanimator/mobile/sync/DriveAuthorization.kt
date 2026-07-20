package com.gpxanimator.mobile.sync

import android.app.Activity
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import com.google.android.gms.auth.api.identity.AuthorizationRequest
import com.google.android.gms.auth.api.identity.AuthorizationResult
import com.google.android.gms.auth.api.identity.ClearTokenRequest
import com.google.android.gms.auth.api.identity.Identity
import com.google.android.gms.auth.api.identity.RevokeAccessRequest
import com.google.android.gms.common.api.ApiException
import com.google.android.gms.common.api.Scope
import com.google.android.gms.tasks.Task
import java.util.concurrent.CancellationException
import kotlin.coroutines.resume
import kotlin.coroutines.resumeWithException
import kotlinx.coroutines.suspendCancellableCoroutine

/** OAuth scopes used by the Drive REST integration without pulling in the Drive Java client. */
object DriveScopes {
    const val DRIVE_FILE = "https://www.googleapis.com/auth/drive.file"
}

sealed interface DriveAuthorizationOutcome {
    data object Authorized : DriveAuthorizationOutcome

    data class UserActionRequired(val pendingIntent: PendingIntent) : DriveAuthorizationOutcome

    data class Failed(val message: String) : DriveAuthorizationOutcome
}

sealed interface DriveRevocationOutcome {
    data object Revoked : DriveRevocationOutcome

    data class Failed(val message: String) : DriveRevocationOutcome
}

/**
 * Activity-facing Google authorization entry point.
 *
 * Access tokens never leave the sync package. UI code only launches the returned PendingIntent and
 * reports its result back through [completeAuthorization].
 */
class DriveAuthorizationManager(context: Context) {
    private val appContext = context.applicationContext

    suspend fun beginAuthorization(activity: Activity): DriveAuthorizationOutcome =
        try {
            Identity.getAuthorizationClient(activity)
                .authorize(authorizationRequest())
                .awaitResult()
                .toPublicOutcome()
        } catch (error: ApiException) {
            DriveAuthorizationOutcome.Failed(
                "Google authorization failed (status ${error.statusCode}).",
            )
        } catch (error: CancellationException) {
            throw error
        } catch (_: Exception) {
            DriveAuthorizationOutcome.Failed("Google authorization is currently unavailable.")
        }

    fun completeAuthorization(resultIntent: Intent?): DriveAuthorizationOutcome {
        if (resultIntent == null) {
            return DriveAuthorizationOutcome.Failed("Google authorization was cancelled.")
        }

        return try {
            Identity.getAuthorizationClient(appContext)
                .getAuthorizationResultFromIntent(resultIntent)
                .toPublicOutcome()
        } catch (error: ApiException) {
            DriveAuthorizationOutcome.Failed(
                "Google authorization failed (status ${error.statusCode}).",
            )
        }
    }

    suspend fun revokeAuthorization(): DriveRevocationOutcome =
        try {
            Identity.getAuthorizationClient(appContext)
                .revokeAccess(
                    RevokeAccessRequest.builder()
                        .setScopes(requestedScopes())
                        .build(),
                )
                .awaitResult()
            DriveRevocationOutcome.Revoked
        } catch (error: ApiException) {
            DriveRevocationOutcome.Failed(
                "Google authorization could not be revoked (status ${error.statusCode}).",
            )
        } catch (error: CancellationException) {
            throw error
        } catch (_: Exception) {
            DriveRevocationOutcome.Failed("Google authorization could not be revoked.")
        }
}

internal sealed interface SilentAuthorizationResult {
    data class Authorized(val accessToken: String) : SilentAuthorizationResult

    data object UserActionRequired : SilentAuthorizationResult

    data class RetryableFailure(val message: String) : SilentAuthorizationResult

    data class PermanentFailure(val message: String) : SilentAuthorizationResult
}

internal class DriveAuthorizationGateway(private val context: Context) {
    suspend fun acquireTokenSilently(): SilentAuthorizationResult =
        try {
            val result =
                Identity.getAuthorizationClient(context)
                    .authorize(authorizationRequest())
                    .awaitResult()

            when {
                result.hasResolution() -> SilentAuthorizationResult.UserActionRequired
                DriveScopes.DRIVE_FILE !in result.grantedScopes ->
                    SilentAuthorizationResult.UserActionRequired

                result.accessToken.isNullOrBlank() -> SilentAuthorizationResult.UserActionRequired
                else -> SilentAuthorizationResult.Authorized(result.accessToken!!)
            }
        } catch (error: ApiException) {
            when (DriveErrorClassifier.classifyAuthorizationStatus(error.statusCode)) {
                DriveFailureKind.AUTH_REQUIRED -> SilentAuthorizationResult.UserActionRequired
                DriveFailureKind.RETRYABLE ->
                    SilentAuthorizationResult.RetryableFailure(
                        "Google authorization is temporarily unavailable (status ${error.statusCode}).",
                    )

                DriveFailureKind.PERMANENT ->
                    SilentAuthorizationResult.PermanentFailure(
                        "Google authorization failed (status ${error.statusCode}).",
                    )
            }
        } catch (error: CancellationException) {
            throw error
        } catch (_: Exception) {
            SilentAuthorizationResult.RetryableFailure(
                "Google authorization is temporarily unavailable.",
            )
        }

    suspend fun clearToken(accessToken: String): Boolean =
        try {
            Identity.getAuthorizationClient(context)
                .clearToken(ClearTokenRequest.builder().setToken(accessToken).build())
                .awaitResult()
            true
        } catch (error: CancellationException) {
            throw error
        } catch (_: Exception) {
            false
        }
}

private fun authorizationRequest(): AuthorizationRequest =
    AuthorizationRequest.builder()
        .setRequestedScopes(requestedScopes())
        .setOptOutIncludingGrantedScopes(true)
        .build()

private fun requestedScopes(): List<Scope> = listOf(Scope(DriveScopes.DRIVE_FILE))

private fun AuthorizationResult.toPublicOutcome(): DriveAuthorizationOutcome =
    when {
        hasResolution() && pendingIntent != null ->
            DriveAuthorizationOutcome.UserActionRequired(pendingIntent!!)

        hasResolution() ->
            DriveAuthorizationOutcome.Failed("Google authorization cannot be opened.")

        DriveScopes.DRIVE_FILE !in grantedScopes ->
            DriveAuthorizationOutcome.Failed("Google Drive access was not granted.")

        accessToken.isNullOrBlank() ->
            DriveAuthorizationOutcome.Failed("Google authorization returned no access grant.")

        else -> DriveAuthorizationOutcome.Authorized
    }

private suspend fun <T> Task<T>.awaitResult(): T =
    suspendCancellableCoroutine { continuation ->
        addOnSuccessListener { value ->
            if (continuation.isActive) continuation.resume(value)
        }
        addOnFailureListener { error ->
            if (continuation.isActive) continuation.resumeWithException(error)
        }
        addOnCanceledListener {
            continuation.cancel(CancellationException("Google Play services task was cancelled."))
        }
    }
