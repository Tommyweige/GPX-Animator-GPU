package com.gpxanimator.mobile.ui.screens

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import com.gpxanimator.mobile.R
import com.gpxanimator.mobile.ui.BackgroundProtectionState
import com.gpxanimator.mobile.ui.DriveConnectionUiState
import com.gpxanimator.mobile.ui.DriveUiState
import com.gpxanimator.mobile.ui.RidePermissionState
import com.gpxanimator.mobile.ui.components.PageIntro
import com.gpxanimator.mobile.ui.components.PillTone
import com.gpxanimator.mobile.ui.components.StatusPill
import com.gpxanimator.mobile.ui.theme.RideTeal

@Composable
fun SettingsScreen(
    permissions: RidePermissionState,
    backgroundProtection: BackgroundProtectionState,
    driveState: DriveUiState,
    onRequestPermissions: () -> Unit,
    onOpenBatterySettings: () -> Unit,
    onOpenAppSettings: () -> Unit,
    onOpenLocationSettings: () -> Unit,
    onConnectDrive: () -> Unit,
    onDisconnectDrive: () -> Unit,
    modifier: Modifier = Modifier,
) {
    var showDriveGuide by rememberSaveable { mutableStateOf(false) }
    LazyColumn(
        modifier = modifier.fillMaxSize(),
        contentPadding = androidx.compose.foundation.layout.PaddingValues(
            20.dp,
            24.dp,
            20.dp,
            36.dp
        ),
        verticalArrangement = Arrangement.spacedBy(16.dp),
    ) {
        item {
            PageIntro(
                eyebrow = stringResource(R.string.nav_settings),
                title = stringResource(R.string.settings_title),
                subtitle = stringResource(R.string.settings_subtitle),
            )
        }
        item {
            SettingsCard(
                title = stringResource(R.string.permissions_title),
                body = stringResource(R.string.permissions_settings_body),
            ) {
                SettingsStatusRow(
                    label = stringResource(R.string.precise_location),
                    ready = permissions.hasFineLocation,
                )
                Spacer(Modifier.height(12.dp))
                SettingsStatusRow(
                    label = stringResource(R.string.notifications),
                    ready = permissions.hasNotifications,
                )
                Spacer(Modifier.height(12.dp))
                SettingsStatusRow(
                    label = stringResource(R.string.location_services),
                    ready = permissions.locationServicesEnabled,
                )
                Spacer(Modifier.height(16.dp))
                Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
                    OutlinedButton(
                        onClick = onRequestPermissions,
                        modifier = Modifier.fillMaxWidth(),
                    ) {
                        Text(stringResource(R.string.review_permissions))
                    }
                    OutlinedButton(
                        onClick = onOpenAppSettings,
                        modifier = Modifier.fillMaxWidth(),
                    ) {
                        Text(stringResource(R.string.app_settings))
                    }
                }
                if (!permissions.locationServicesEnabled) {
                    Spacer(Modifier.height(10.dp))
                    OutlinedButton(
                        onClick = onOpenLocationSettings,
                        modifier = Modifier.fillMaxWidth(),
                    ) {
                        Text(stringResource(R.string.location_settings))
                    }
                }
            }
        }
        item {
            SettingsCard(
                title = stringResource(R.string.background_protection_title),
                body = stringResource(R.string.background_protection_body),
            ) {
                SettingsStatusRow(
                    label = stringResource(R.string.battery_optimization),
                    ready = backgroundProtection.ignoresBatteryOptimizations,
                )
                if (!backgroundProtection.ignoresBatteryOptimizations) {
                    Spacer(Modifier.height(16.dp))
                    OutlinedButton(onClick = onOpenBatterySettings) {
                        Text(stringResource(R.string.battery_settings))
                    }
                }
            }
        }
        item {
            SettingsCard(
                title = stringResource(R.string.google_drive_title),
                body = stringResource(R.string.google_drive_body),
            ) {
                val driveLabel =
                    when (driveState.connection) {
                        DriveConnectionUiState.NotConnected -> stringResource(R.string.drive_not_connected)
                        DriveConnectionUiState.Connected ->
                            driveState.accountLabel ?: stringResource(R.string.drive_connected)

                        DriveConnectionUiState.AuthorizationRequired -> stringResource(R.string.drive_reconnect_needed)
                    }
                val driveTone =
                    when (driveState.connection) {
                        DriveConnectionUiState.NotConnected -> PillTone.Neutral
                        DriveConnectionUiState.Connected -> PillTone.Active
                        DriveConnectionUiState.AuthorizationRequired -> PillTone.Warning
                    }
                StatusPill(text = driveLabel, tone = driveTone)
                Spacer(Modifier.height(16.dp))
                OutlinedButton(
                    onClick = {
                        when (driveState.connection) {
                            DriveConnectionUiState.Connected -> onDisconnectDrive()
                            else -> {
                                if (driveState.canStartAuthorization) {
                                    onConnectDrive()
                                } else {
                                    showDriveGuide = true
                                }
                            }
                        }
                    },
                ) {
                    Text(
                        stringResource(
                            when (driveState.connection) {
                                DriveConnectionUiState.Connected -> R.string.drive_disconnect
                                DriveConnectionUiState.AuthorizationRequired -> R.string.drive_reconnect
                                DriveConnectionUiState.NotConnected -> R.string.drive_setup
                            },
                        ),
                    )
                }
            }
        }
        item {
            SettingsCard(
                title = stringResource(R.string.privacy_title),
                body = stringResource(R.string.privacy_body),
            ) {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Canvas(Modifier.size(10.dp)) { drawCircle(RideTeal) }
                    Spacer(Modifier.size(10.dp))
                    Text(
                        stringResource(R.string.no_tracking_sdk),
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
            }
        }
    }

    if (showDriveGuide) {
        AlertDialog(
            onDismissRequest = { showDriveGuide = false },
            title = { Text(stringResource(R.string.drive_setup_dialog_title)) },
            text = { Text(stringResource(R.string.drive_setup_dialog_body)) },
            confirmButton = {
                TextButton(onClick = { showDriveGuide = false }) {
                    Text(stringResource(R.string.got_it))
                }
            },
        )
    }
}

@Composable
private fun SettingsCard(
    title: String,
    body: String,
    content: @Composable () -> Unit,
) {
    Card(
        modifier = Modifier.fillMaxWidth(),
        colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.surface),
    ) {
        Column(Modifier.padding(20.dp)) {
            Text(title, style = MaterialTheme.typography.titleLarge)
            Spacer(Modifier.height(6.dp))
            Text(
                body,
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            Spacer(Modifier.height(18.dp))
            content()
        }
    }
}

@Composable
private fun SettingsStatusRow(label: String, ready: Boolean) {
    Row(
        modifier = Modifier.fillMaxWidth(),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.SpaceBetween,
    ) {
        Text(label, style = MaterialTheme.typography.bodyLarge)
        StatusPill(
            text = stringResource(if (ready) R.string.status_ready else R.string.status_action_needed),
            tone = if (ready) PillTone.Active else PillTone.Warning,
        )
    }
}
