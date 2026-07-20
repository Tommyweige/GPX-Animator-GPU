package com.gpxanimator.mobile.sync

import android.content.Context
import androidx.core.content.edit

internal object DriveSyncEpoch {
    private const val PREFERENCES_NAME = "drive-sync-state"
    private const val KEY_EPOCH = "sync-epoch"

    fun current(context: Context): Long =
        context.applicationContext
            .getSharedPreferences(PREFERENCES_NAME, Context.MODE_PRIVATE)
            .getLong(KEY_EPOCH, 0L)

    @Synchronized
    fun advance(context: Context): Long {
        val preferences =
            context.applicationContext.getSharedPreferences(PREFERENCES_NAME, Context.MODE_PRIVATE)
        val next = preferences.getLong(KEY_EPOCH, 0L) + 1L
        preferences.edit(commit = true) { putLong(KEY_EPOCH, next) }
        return next
    }
}
