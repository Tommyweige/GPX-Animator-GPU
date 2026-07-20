package com.gpxanimator.mobile.gpx

import android.content.Context
import android.content.ClipData
import android.content.Intent
import androidx.core.content.FileProvider
import com.gpxanimator.mobile.BuildConfig
import com.gpxanimator.mobile.data.TripEntity
import java.io.File

object RideFileActions {
    fun share(context: Context, trip: TripEntity): Intent? {
        val path = trip.localGpxPath ?: return null
        val file = File(path)
        if (!file.isFile) return null
        val uri =
            try {
                FileProvider.getUriForFile(
                    context,
                    "${BuildConfig.APPLICATION_ID}.files",
                    file,
                )
            } catch (_: IllegalArgumentException) {
                return null
            }
        return Intent(Intent.ACTION_SEND).apply {
            type = "application/gpx+xml"
            putExtra(Intent.EXTRA_STREAM, uri)
            putExtra(Intent.EXTRA_TITLE, file.name)
            clipData = ClipData.newRawUri(file.name, uri)
            addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
        }
    }
}
