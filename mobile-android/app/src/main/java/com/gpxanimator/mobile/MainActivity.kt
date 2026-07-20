package com.gpxanimator.mobile

import android.Manifest
import android.app.Activity
import android.app.PendingIntent
import android.content.ActivityNotFoundException
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.location.LocationManager
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.os.PowerManager
import android.provider.Settings
import androidx.activity.ComponentActivity
import androidx.activity.enableEdgeToEdge
import androidx.activity.compose.setContent
import androidx.activity.result.IntentSenderRequest
import androidx.activity.result.contract.ActivityResultContracts
import androidx.activity.viewModels
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.core.content.ContextCompat
import androidx.core.content.edit
import androidx.lifecycle.lifecycleScope
import androidx.work.await
import com.gpxanimator.mobile.data.SyncState
import com.gpxanimator.mobile.gpx.RideFileActions
import com.gpxanimator.mobile.gpx.RideFinalizer
import com.gpxanimator.mobile.locale.AppLanguage
import com.gpxanimator.mobile.locale.AppLanguagePreferences
import com.gpxanimator.mobile.recording.RecordingController
import com.gpxanimator.mobile.sync.DriveAuthorizationManager
import com.gpxanimator.mobile.sync.DriveAuthorizationOutcome
import com.gpxanimator.mobile.sync.DriveRevocationOutcome
import com.gpxanimator.mobile.sync.DriveSyncCoordinator
import com.gpxanimator.mobile.sync.DriveSyncEpoch
import com.gpxanimator.mobile.ui.BackgroundProtectionState
import com.gpxanimator.mobile.ui.DriveConnectionUiState
import com.gpxanimator.mobile.ui.DriveUiState
import com.gpxanimator.mobile.ui.GpxAnimatorRideApp
import com.gpxanimator.mobile.ui.RideAppCallbacks
import com.gpxanimator.mobile.ui.RidePermissionState
import com.gpxanimator.mobile.ui.RideViewModel
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch

class MainActivity : ComponentActivity() {
    private val rideViewModel by viewModels<RideViewModel>()
    private val driveAuthorizationManager by lazy { DriveAuthorizationManager(this) }
    private val permissionPreferences by lazy {
        getSharedPreferences(PERMISSION_PREFERENCES, MODE_PRIVATE)
    }
    private var permissionState by mutableStateOf(RidePermissionState())
    private var backgroundProtection by mutableStateOf(BackgroundProtectionState())
    private var appLanguage by mutableStateOf(AppLanguage.SystemDefault)
    private var driveState by
        mutableStateOf(DriveUiState(canStartAuthorization = true))
    private var pendingDriveAuthorization: PendingIntent? = null
    private var startAfterPermissionGrant = false

    private val permissionLauncher =
        registerForActivityResult(ActivityResultContracts.RequestMultiplePermissions()) {
            permissionPreferences.edit { putBoolean(KEY_PERMISSIONS_REQUESTED, true) }
            refreshDeviceState()
            if (startAfterPermissionGrant && permissionState.canRecord) {
                RecordingController.start(this)
            }
            startAfterPermissionGrant = false
        }

    private val driveAuthorizationLauncher =
        registerForActivityResult(ActivityResultContracts.StartIntentSenderForResult()) { result ->
            val outcome =
                if (result.resultCode == Activity.RESULT_OK) {
                    driveAuthorizationManager.completeAuthorization(result.data)
                } else {
                    DriveAuthorizationOutcome.Failed("Google authorization was cancelled.")
                }
            applyAuthorizationOutcome(outcome, launchResolution = false)
        }

    override fun attachBaseContext(newBase: Context) {
        super.attachBaseContext(AppLanguagePreferences.localizedContext(newBase))
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        appLanguage = AppLanguagePreferences.current(this)
        refreshDeviceState()
        setContent {
            GpxAnimatorRideApp(
                viewModel = rideViewModel,
                permissions = permissionState,
                backgroundProtection = backgroundProtection,
                language = appLanguage,
                driveState = driveState,
                callbacks =
                    RideAppCallbacks(
                        onStartRecording = ::startRecordingWithPermissions,
                        onFinishRecording = { RecordingController.finish(this) },
                        onRequestPermissions = ::requestRecordingPermissions,
                        onOpenBatterySettings = ::openBatterySettings,
                        onOpenAppSettings = ::openAppSettings,
                        onOpenLocationSettings = ::openLocationSettings,
                        onConnectDrive = ::connectDrive,
                        onDisconnectDrive = ::disconnectDrive,
                        onLanguageChange = ::changeLanguage,
                        onShareRide = ::shareRide,
                        onRetryDriveSync = ::retryDriveSync,
                        onFinalizeInterruptedRide = ::finalizeInterruptedRide,
                    ),
            )
        }
        lifecycleScope.launch { RecordingController.reconcileInterruptedRide(this@MainActivity) }
        refreshDriveAuthorization()
    }

    override fun onResume() {
        super.onResume()
        appLanguage = AppLanguagePreferences.current(this)
        refreshDeviceState()
        if (driveState.connection == DriveConnectionUiState.Connected) {
            refreshDriveAuthorization()
        }
    }

    private fun startRecordingWithPermissions() {
        refreshDeviceState()
        when {
            !permissionState.locationServicesEnabled -> openLocationSettings()
            permissionState.canRecord -> RecordingController.start(this)
            else -> launchRecordingPermissionRequest(startWhenGranted = true)
        }
    }

    private fun requestRecordingPermissions() {
        launchRecordingPermissionRequest(startWhenGranted = false)
    }

    private fun changeLanguage(language: AppLanguage) {
        if (language == appLanguage) return
        AppLanguagePreferences.set(this, language)
        appLanguage = language
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU) {
            recreate()
        }
    }

    private fun launchRecordingPermissionRequest(startWhenGranted: Boolean) {
        val deniedPermissions = recordingPermissions().filterNot(::hasPermission)
        if (deniedPermissions.isEmpty()) {
            refreshDeviceState()
            if (startWhenGranted && permissionState.canRecord) RecordingController.start(this)
            return
        }
        val requestedBefore = permissionPreferences.getBoolean(KEY_PERMISSIONS_REQUESTED, false)
        val permanentlyDenied =
            requestedBefore &&
                    deniedPermissions.any { permission ->
                        !shouldShowRequestPermissionRationale(permission)
                    }
        if (permanentlyDenied) {
            startAfterPermissionGrant = false
            openAppSettings()
            return
        }
        startAfterPermissionGrant = startWhenGranted
        permissionLauncher.launch(recordingPermissions())
    }

    private fun recordingPermissions(): Array<String> =
        buildList {
            add(Manifest.permission.ACCESS_FINE_LOCATION)
            add(Manifest.permission.ACCESS_COARSE_LOCATION)
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
                add(Manifest.permission.POST_NOTIFICATIONS)
            }
        }.toTypedArray()

    private fun refreshDeviceState() {
        permissionState =
            RidePermissionState(
                hasFineLocation = hasPermission(Manifest.permission.ACCESS_FINE_LOCATION),
                hasCoarseLocation = hasPermission(Manifest.permission.ACCESS_COARSE_LOCATION),
                hasNotifications =
                    Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU ||
                            hasPermission(Manifest.permission.POST_NOTIFICATIONS),
            )
        val powerManager = getSystemService(POWER_SERVICE) as PowerManager
        val locationManager = getSystemService(LOCATION_SERVICE) as LocationManager
        permissionState =
            permissionState.copy(locationServicesEnabled = locationManager.isLocationEnabled)
        backgroundProtection =
            BackgroundProtectionState(
                ignoresBatteryOptimizations =
                    powerManager.isIgnoringBatteryOptimizations(packageName),
            )
    }

    private fun hasPermission(permission: String): Boolean =
        ContextCompat.checkSelfPermission(this, permission) == PackageManager.PERMISSION_GRANTED

    private fun openBatterySettings() {
        val intent = Intent(Settings.ACTION_IGNORE_BATTERY_OPTIMIZATION_SETTINGS)
        try {
            startActivity(intent)
        } catch (_: ActivityNotFoundException) {
            openAppSettings()
        }
    }

    private fun openAppSettings() {
        startActivity(
            Intent(
                Settings.ACTION_APPLICATION_DETAILS_SETTINGS,
                Uri.fromParts("package", packageName, null),
            ),
        )
    }

    private fun openLocationSettings() {
        try {
            startActivity(Intent(Settings.ACTION_LOCATION_SOURCE_SETTINGS))
        } catch (_: ActivityNotFoundException) {
            openAppSettings()
        }
    }

    private fun refreshDriveAuthorization() {
        lifecycleScope.launch {
            applyAuthorizationOutcome(
                driveAuthorizationManager.beginAuthorization(this@MainActivity),
                launchResolution = false,
            )
        }
    }

    private fun connectDrive() {
        pendingDriveAuthorization?.let {
            launchDriveAuthorization(it)
            return
        }
        lifecycleScope.launch {
            applyAuthorizationOutcome(
                driveAuthorizationManager.beginAuthorization(this@MainActivity),
                launchResolution = true,
            )
        }
    }

    private fun applyAuthorizationOutcome(
        outcome: DriveAuthorizationOutcome,
        launchResolution: Boolean,
    ) {
        when (outcome) {
            DriveAuthorizationOutcome.Authorized -> {
                pendingDriveAuthorization = null
                driveState =
                    DriveUiState(
                        connection = DriveConnectionUiState.Connected,
                        canStartAuthorization = true,
                    )
                lifecycleScope.launch { enqueueAwaitingDriveRides() }
            }

            is DriveAuthorizationOutcome.UserActionRequired -> {
                pendingDriveAuthorization = outcome.pendingIntent
                driveState =
                    DriveUiState(
                        connection = DriveConnectionUiState.AuthorizationRequired,
                        canStartAuthorization = true,
                    )
                if (launchResolution) launchDriveAuthorization(outcome.pendingIntent)
            }

            is DriveAuthorizationOutcome.Failed -> {
                pendingDriveAuthorization = null
                driveState =
                    DriveUiState(
                        connection = DriveConnectionUiState.NotConnected,
                        canStartAuthorization = true,
                    )
            }
        }
    }

    private fun launchDriveAuthorization(pendingIntent: PendingIntent) {
        try {
            driveAuthorizationLauncher.launch(IntentSenderRequest.Builder(pendingIntent).build())
        } catch (_: RuntimeException) {
            pendingDriveAuthorization = null
            driveState =
                DriveUiState(
                    connection = DriveConnectionUiState.AuthorizationRequired,
                    canStartAuthorization = true,
                )
        }
    }

    private fun disconnectDrive() {
        lifecycleScope.launch {
            DriveSyncEpoch.advance(this@MainActivity)
            DriveSyncCoordinator.cancelAll(this@MainActivity).await()
            when (driveAuthorizationManager.revokeAuthorization()) {
                DriveRevocationOutcome.Revoked -> {
                    pendingDriveAuthorization = null
                    driveState = DriveUiState(canStartAuthorization = true)
                    markQueuedRidesAuthorizationRequired()
                }

                is DriveRevocationOutcome.Failed -> enqueueDurableDriveRides()
            }
        }
    }

    private suspend fun enqueueAwaitingDriveRides() {
        rideRepository().trips.first()
            .filter { trip ->
                trip.localGpxPath != null &&
                        trip.syncState in setOf(SyncState.PENDING, SyncState.AUTH_REQUIRED)
            }.forEach { DriveSyncCoordinator.enqueue(applicationContext, it.id) }
    }

    private suspend fun markQueuedRidesAuthorizationRequired() {
        val repository = rideRepository()
        repository.trips.first()
            .filter {
                it.localGpxPath != null &&
                        it.syncState in
                        setOf(
                            SyncState.PENDING,
                            SyncState.UPLOADING,
                            SyncState.FAILED,
                            SyncState.AUTH_REQUIRED,
                        )
            }.forEach { trip ->
                repository.updateTrip(
                    trip.copy(
                        syncState = SyncState.AUTH_REQUIRED,
                        driveUploadSessionUrl = null,
                        driveUploadSessionSha256 = null,
                        driveUploadSessionLength = null,
                        lastError = "Reconnect Google Drive to continue syncing.",
                        updatedAtEpochMillis = System.currentTimeMillis(),
                    ),
                )
            }
    }

    private suspend fun enqueueDurableDriveRides() {
        rideRepository().trips.first()
            .filter {
                it.localGpxPath != null &&
                        it.syncState in setOf(SyncState.PENDING, SyncState.UPLOADING)
            }.forEach { DriveSyncCoordinator.enqueue(applicationContext, it.id) }
    }

    private fun shareRide(tripId: String) {
        lifecycleScope.launch {
            val trip = rideRepository().getTrip(tripId) ?: return@launch
            val shareIntent = RideFileActions.share(this@MainActivity, trip) ?: return@launch
            try {
                startActivity(
                    Intent.createChooser(
                        shareIntent,
                        getString(R.string.share_gpx_chooser)
                    )
                )
            } catch (_: ActivityNotFoundException) {
                Unit
            }
        }
    }

    private fun retryDriveSync(tripId: String) {
        lifecycleScope.launch {
            val trip = rideRepository().getTrip(tripId) ?: return@launch
            if (trip.syncState == SyncState.AUTH_REQUIRED) {
                connectDrive()
            } else {
                DriveSyncCoordinator.enqueue(this@MainActivity, tripId)
            }
        }
    }

    private fun finalizeInterruptedRide(tripId: String) {
        RideFinalizer.enqueue(this, tripId)
    }

    private fun rideRepository() =
        (application as GpxAnimatorRideApplication).container.rideRepository

    private companion object {
        const val PERMISSION_PREFERENCES = "recording-permission-state"
        const val KEY_PERMISSIONS_REQUESTED = "permissions-requested"
    }
}
