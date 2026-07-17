package com.gpxanimator.mobile.ui

import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawing
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Surface
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.navigation.NavGraph.Companion.findStartDestination
import androidx.navigation.compose.NavHost
import androidx.navigation.compose.composable
import androidx.navigation.compose.currentBackStackEntryAsState
import androidx.navigation.compose.rememberNavController
import com.gpxanimator.mobile.ui.components.RideBottomBar
import com.gpxanimator.mobile.ui.components.TopLevelDestination
import com.gpxanimator.mobile.ui.screens.HistoryScreen
import com.gpxanimator.mobile.ui.screens.HomeScreen
import com.gpxanimator.mobile.ui.screens.RecordingScreen
import com.gpxanimator.mobile.ui.screens.RideDetailScreen
import com.gpxanimator.mobile.ui.screens.SettingsScreen
import com.gpxanimator.mobile.ui.theme.GpxAnimatorRideTheme

private const val RECORDING_ROUTE = "recording"
private const val DETAIL_ROUTE = "detail/{tripId}"

@Composable
fun GpxAnimatorRideApp(
    viewModel: RideViewModel,
    permissions: RidePermissionState,
    backgroundProtection: BackgroundProtectionState,
    driveState: DriveUiState = DriveUiState(),
    callbacks: RideAppCallbacks,
) {
    val state by viewModel.uiState.collectAsStateWithLifecycle()
    val navController = rememberNavController()
    val backStackEntry by navController.currentBackStackEntryAsState()
    val currentRoute = backStackEntry?.destination?.route
    val showBottomBar = TopLevelDestination.entries.any { it.route == currentRoute }
    var waitingForRideStart by rememberSaveable { mutableStateOf(false) }

    LaunchedEffect(state.activeRide?.id, waitingForRideStart, currentRoute) {
        if (waitingForRideStart && state.activeRide != null && currentRoute == TopLevelDestination.Home.route) {
            waitingForRideStart = false
            navController.navigate(RECORDING_ROUTE) { launchSingleTop = true }
        }
    }

    GpxAnimatorRideTheme {
        Surface(modifier = Modifier.fillMaxSize(), color = MaterialTheme.colorScheme.background) {
            Scaffold(
                containerColor = MaterialTheme.colorScheme.background,
                contentWindowInsets = WindowInsets.safeDrawing,
                bottomBar = {
                    if (showBottomBar) {
                        RideBottomBar(
                            selectedRoute = currentRoute,
                            onSelect = { destination ->
                                navController.navigate(destination.route) {
                                    popUpTo(navController.graph.findStartDestination().id) {
                                        saveState = true
                                    }
                                    launchSingleTop = true
                                    restoreState = true
                                }
                            },
                        )
                    }
                },
            ) { contentPadding ->
                NavHost(
                    navController = navController,
                    startDestination = TopLevelDestination.Home.route,
                    modifier = Modifier.fillMaxSize().padding(contentPadding),
                ) {
                    composable(TopLevelDestination.Home.route) {
                        HomeScreen(
                            state = state,
                            permissions = permissions,
                            backgroundProtection = backgroundProtection,
                            onStartRecording = {
                                waitingForRideStart = true
                                callbacks.onStartRecording()
                            },
                            onOpenActiveRide = {
                                navController.navigate(RECORDING_ROUTE) { launchSingleTop = true }
                            },
                            onOpenHistory = {
                                navController.navigate(TopLevelDestination.History.route) {
                                    launchSingleTop = true
                                }
                            },
                            onOpenTrip = { tripId ->
                                if (tripId == state.activeRide?.id) {
                                    navController.navigate(RECORDING_ROUTE) {
                                        launchSingleTop = true
                                    }
                                } else {
                                    navController.navigate("detail/$tripId")
                                }
                            },
                            onRequestPermissions = callbacks.onRequestPermissions,
                            onOpenBatterySettings = callbacks.onOpenBatterySettings,
                            onOpenLocationSettings = callbacks.onOpenLocationSettings,
                        )
                    }
                    composable(TopLevelDestination.History.route) {
                        HistoryScreen(
                            state = state,
                            onOpenTrip = { tripId ->
                                if (tripId == state.activeRide?.id) {
                                    navController.navigate(RECORDING_ROUTE) {
                                        launchSingleTop = true
                                    }
                                } else {
                                    navController.navigate("detail/$tripId")
                                }
                            },
                        )
                    }
                    composable(TopLevelDestination.Settings.route) {
                        SettingsScreen(
                            permissions = permissions,
                            backgroundProtection = backgroundProtection,
                            driveState = driveState,
                            onRequestPermissions = callbacks.onRequestPermissions,
                            onOpenBatterySettings = callbacks.onOpenBatterySettings,
                            onOpenAppSettings = callbacks.onOpenAppSettings,
                            onOpenLocationSettings = callbacks.onOpenLocationSettings,
                            onConnectDrive = callbacks.onConnectDrive,
                            onDisconnectDrive = callbacks.onDisconnectDrive,
                        )
                    }
                    composable(RECORDING_ROUTE) {
                        RecordingScreen(
                            activeRide = state.activeRide,
                            isLoading = state.isLoading,
                            onBack = { navController.popBackStack() },
                            onFinishRecording = callbacks.onFinishRecording,
                            onRideFinished = {
                                navController.navigate(TopLevelDestination.Home.route) {
                                    popUpTo(RECORDING_ROUTE) { inclusive = true }
                                    launchSingleTop = true
                                }
                            },
                        )
                    }
                    composable(DETAIL_ROUTE) { entry ->
                        val tripId = entry.arguments?.getString("tripId")
                        LaunchedEffect(tripId) { viewModel.selectTrip(tripId) }
                        DisposableEffect(tripId) {
                            onDispose { viewModel.selectTrip(null) }
                        }
                        RideDetailScreen(
                            detail = state.selectedRide?.takeIf { it.trip.id == tripId },
                            onBack = { navController.popBackStack() },
                            onShareRide = callbacks.onShareRide,
                            onRetryDriveSync = callbacks.onRetryDriveSync,
                            onFinalizeInterruptedRide = callbacks.onFinalizeInterruptedRide,
                        )
                    }
                }
            }
        }
    }
}
