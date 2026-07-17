package com.gpxanimator.mobile.recording

import android.content.Context
import android.os.SystemClock
import android.provider.Settings
import androidx.core.content.edit

internal enum class RecordingRecoveryAction {
    KEEP_RUNNING,
    RECOVER_SERVICE,
    MARK_INTERRUPTED,
}

internal object RecordingRecoveryPolicy {
    fun decide(
        serviceRunning: Boolean,
        sameBoot: Boolean,
        leaseFresh: Boolean,
    ): RecordingRecoveryAction =
        when {
            serviceRunning -> RecordingRecoveryAction.KEEP_RUNNING
            sameBoot && leaseFresh -> RecordingRecoveryAction.RECOVER_SERVICE
            else -> RecordingRecoveryAction.MARK_INTERRUPTED
        }
}

internal class RecordingLease(context: Context) {
    private val appContext = context.applicationContext
    private val preferences =
        appContext.getSharedPreferences(PREFERENCES_NAME, Context.MODE_PRIVATE)

    fun refresh(tripId: String) {
        preferences.edit {
            putString(KEY_TRIP_ID, tripId)
            putInt(KEY_BOOT_COUNT, currentBootCount(appContext))
            putLong(KEY_HEARTBEAT_ELAPSED_REALTIME, SystemClock.elapsedRealtime())
        }
    }

    fun isFresh(tripId: String, maxAgeMillis: Long = MAX_LEASE_AGE_MILLIS): Boolean {
        if (preferences.getString(KEY_TRIP_ID, null) != tripId) return false
        if (preferences.getInt(KEY_BOOT_COUNT, UNKNOWN_BOOT) != currentBootCount(appContext)) {
            return false
        }
        val heartbeat = preferences.getLong(KEY_HEARTBEAT_ELAPSED_REALTIME, -1L)
        if (heartbeat < 0L) return false
        return SystemClock.elapsedRealtime() - heartbeat in 0L..maxAgeMillis
    }

    fun clear(tripId: String? = null) {
        if (tripId != null && preferences.getString(KEY_TRIP_ID, null) != tripId) return
        preferences.edit { clear() }
    }

    companion object {
        private const val PREFERENCES_NAME = "recording-lease"
        private const val KEY_TRIP_ID = "trip-id"
        private const val KEY_BOOT_COUNT = "boot-count"
        private const val KEY_HEARTBEAT_ELAPSED_REALTIME = "heartbeat-elapsed-realtime"
        private const val MAX_LEASE_AGE_MILLIS = 15_000L
        internal const val UNKNOWN_BOOT = -1

        fun currentBootCount(context: Context): Int =
            runCatching {
                Settings.Global.getInt(
                    context.contentResolver,
                    Settings.Global.BOOT_COUNT,
                    UNKNOWN_BOOT,
                )
            }.getOrDefault(UNKNOWN_BOOT)
    }
}
