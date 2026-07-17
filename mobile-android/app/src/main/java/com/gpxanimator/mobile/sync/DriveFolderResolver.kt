package com.gpxanimator.mobile.sync

import android.content.Context
import androidx.core.content.edit
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock

internal class DriveFolderResolver(
    context: Context,
    private val client: DriveRestClient,
) {
    private val preferences =
        context.applicationContext.getSharedPreferences(PREFERENCES_NAME, Context.MODE_PRIVATE)

    suspend fun resolveYearFolder(year: Int, tripFolderId: String?): String =
        resolutionMutex.withLock {
            val expectedMarker = DriveRestClient.folderMarkerYear(year)
            validateFolder(tripFolderId, expectedMarker)?.let { return@withLock it.id }
            validateFolder(preferences.getString(yearPreferenceKey(year), null), expectedMarker)
                ?.let { return@withLock it.id }

            val rootFolder =
                resolveFolder(
                    preferenceKey = ROOT_FOLDER_KEY,
                    name = "GPX Animator",
                    parentId = "root",
                    marker = DriveRestClient.folderMarkerRoot(),
                )
            val routesFolder =
                resolveFolder(
                    preferenceKey = ROUTES_FOLDER_KEY,
                    name = "Routes",
                    parentId = rootFolder.id,
                    marker = DriveRestClient.folderMarkerRoutes(),
                )
            val yearFolder =
                client.findFolder(routesFolder.id, expectedMarker)
                    ?: client.createFolder(year.toString(), routesFolder.id, expectedMarker)
            preferences.edit { putString(yearPreferenceKey(year), yearFolder.id) }
            yearFolder.id
        }

    private fun resolveFolder(
        preferenceKey: String,
        name: String,
        parentId: String,
        marker: String,
    ): DriveFile {
        validateFolder(
            preferences.getString(preferenceKey, null),
            marker,
            parentId
        )?.let { return it }
        val folder =
            client.findFolder(parentId, marker) ?: client.createFolder(name, parentId, marker)
        preferences.edit { putString(preferenceKey, folder.id) }
        return folder
    }

    private fun validateFolder(
        folderId: String?,
        marker: String,
        expectedParentId: String? = null,
    ): DriveFile? {
        val folder = folderId?.let(client::getFile) ?: return null
        if (folder.trashed || folder.mimeType != FOLDER_MIME_TYPE) return null
        if (folder.appProperties[FOLDER_MARKER_KEY] != marker) return null
        if (expectedParentId != null && expectedParentId != "root" && expectedParentId !in folder.parents) {
            return null
        }
        return folder
    }

    companion object {
        private const val PREFERENCES_NAME = "drive-folder-cache"
        private const val ROOT_FOLDER_KEY = "folder.root"
        private const val ROUTES_FOLDER_KEY = "folder.routes"
        private const val FOLDER_MARKER_KEY = "gpxAnimatorFolder"
        private const val FOLDER_MIME_TYPE = "application/vnd.google-apps.folder"
        private val resolutionMutex = Mutex()

        private fun yearPreferenceKey(year: Int): String = "folder.year.$year"
    }
}
