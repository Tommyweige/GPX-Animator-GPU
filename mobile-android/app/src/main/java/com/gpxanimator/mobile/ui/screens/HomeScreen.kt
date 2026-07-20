package com.gpxanimator.mobile.ui.screens

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import com.gpxanimator.mobile.R
import com.gpxanimator.mobile.ui.BackgroundProtectionState
import com.gpxanimator.mobile.ui.RidePermissionState
import com.gpxanimator.mobile.ui.RideUiState
import com.gpxanimator.mobile.ui.components.EmptyStateCard
import com.gpxanimator.mobile.ui.components.MetricTile
import com.gpxanimator.mobile.ui.components.PageIntro
import com.gpxanimator.mobile.ui.components.PillTone
import com.gpxanimator.mobile.ui.components.PrimaryRideButton
import com.gpxanimator.mobile.ui.components.RideListRow
import com.gpxanimator.mobile.ui.components.SectionHeading
import com.gpxanimator.mobile.ui.components.StatusPill
import com.gpxanimator.mobile.ui.components.formatDuration
import com.gpxanimator.mobile.ui.theme.RideCoral
import com.gpxanimator.mobile.ui.theme.RideTeal

@Composable
fun HomeScreen(
    state: RideUiState,
    permissions: RidePermissionState,
    backgroundProtection: BackgroundProtectionState,
    onStartRecording: () -> Unit,
    onOpenActiveRide: () -> Unit,
    onOpenHistory: () -> Unit,
    onOpenTrip: (String) -> Unit,
    onRequestPermissions: () -> Unit,
    onOpenBatterySettings: () -> Unit,
    onOpenLocationSettings: () -> Unit,
    modifier: Modifier = Modifier,
) {
    LazyColumn(
        modifier = modifier.fillMaxSize(),
        contentPadding = androidx.compose.foundation.layout.PaddingValues(
            20.dp,
            24.dp,
            20.dp,
            32.dp
        ),
        verticalArrangement = Arrangement.spacedBy(18.dp),
    ) {
        item {
            PageIntro(
                eyebrow = stringResource(R.string.app_name),
                title =
                    stringResource(
                        if (state.activeRide == null) R.string.home_ready_title else R.string.home_recording_title,
                    ),
                subtitle = stringResource(R.string.home_subtitle),
            )
        }

        item {
            when {
                state.isLoading -> LoadingRideCard()
                state.activeRide != null ->
                    ActiveRideHero(
                        ride = state.activeRide,
                        onOpenRide = onOpenActiveRide,
                    )

                else ->
                    ReadyRideHero(
                        permissions = permissions,
                        onStartRecording = onStartRecording,
                    )
            }
        }

        if (!permissions.canRecord || !backgroundProtection.ignoresBatteryOptimizations) {
            item {
                ProtectionCard(
                    permissions = permissions,
                    backgroundProtection = backgroundProtection,
                    onRequestPermissions = onRequestPermissions,
                    onOpenBatterySettings = onOpenBatterySettings,
                    onOpenLocationSettings = onOpenLocationSettings,
                )
            }
        }

        item {
            SectionHeading(
                title = stringResource(R.string.recent_rides),
                actionLabel = if (state.trips.isEmpty()) null else stringResource(R.string.see_all),
                onAction = onOpenHistory,
            )
        }

        if (state.hasDataError) {
            item {
                EmptyStateCard(
                    title = stringResource(R.string.data_unavailable_title),
                    body = stringResource(R.string.data_unavailable_body),
                )
            }
        } else if (state.trips.isEmpty() && !state.isLoading) {
            item {
                EmptyStateCard(
                    title = stringResource(R.string.no_rides_title),
                    body = stringResource(R.string.no_rides_body),
                )
            }
        } else {
            items(count = minOf(3, state.trips.size), key = { state.trips[it].id }) { index ->
                val trip = state.trips[index]
                RideListRow(trip = trip, onClick = { onOpenTrip(trip.id) })
            }
        }
    }
}

@Composable
private fun LoadingRideCard() {
    Card(
        modifier = Modifier.fillMaxWidth().height(210.dp),
        colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.surface),
    ) {
        Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
            CircularProgressIndicator(color = RideTeal)
        }
    }
}

@Composable
private fun ReadyRideHero(
    permissions: RidePermissionState,
    onStartRecording: () -> Unit,
) {
    Card(
        modifier = Modifier.fillMaxWidth(),
        colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.surface),
        shape = MaterialTheme.shapes.large,
    ) {
        Box(Modifier.fillMaxWidth()) {
            RouteDecoration(Modifier.align(Alignment.TopEnd).size(180.dp))
            Column(Modifier.padding(24.dp)) {
                StatusPill(
                    text =
                        stringResource(
                            if (permissions.canRecord) R.string.system_ready else R.string.permissions_needed,
                        ),
                    tone = if (permissions.canRecord) PillTone.Active else PillTone.Warning,
                )
                Spacer(Modifier.height(26.dp))
                Text(
                    text = stringResource(R.string.ready_card_title),
                    style = MaterialTheme.typography.headlineLarge,
                    modifier = Modifier.fillMaxWidth(.78f),
                )
                Spacer(Modifier.height(8.dp))
                Text(
                    text = stringResource(R.string.ready_card_body),
                    style = MaterialTheme.typography.bodyLarge,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    modifier = Modifier.fillMaxWidth(.78f),
                )
                Spacer(Modifier.height(26.dp))
                PrimaryRideButton(
                    text =
                        stringResource(
                            if (permissions.canRecord) R.string.start_recording else R.string.allow_and_start,
                        ),
                    onClick = onStartRecording,
                )
            }
        }
    }
}

@Composable
private fun ActiveRideHero(
    ride: com.gpxanimator.mobile.ui.ActiveRideUi,
    onOpenRide: () -> Unit,
) {
    Card(
        modifier = Modifier.fillMaxWidth(),
        colors = CardDefaults.cardColors(
            containerColor = MaterialTheme.colorScheme.primaryContainer.copy(
                alpha = .58f
            )
        ),
        shape = MaterialTheme.shapes.large,
    ) {
        Column(Modifier.padding(24.dp)) {
            Row(
                modifier = Modifier.fillMaxWidth(),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.SpaceBetween,
            ) {
                StatusPill(
                    text = stringResource(R.string.recording_active),
                    tone = PillTone.Active,
                )
                Text(
                    text =
                        pluralStringResource(
                            R.plurals.points_count,
                            ride.pointCount,
                            ride.pointCount,
                        ),
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
            Spacer(Modifier.height(18.dp))
            Text(
                text = formatDuration(ride.elapsedMillis),
                style = MaterialTheme.typography.displaySmall,
            )
            Text(
                text = stringResource(R.string.elapsed_time),
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            Spacer(Modifier.height(18.dp))
            Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                MetricTile(
                    label = stringResource(R.string.distance),
                    value = stringResource(R.string.value_km, ride.distanceMeters / 1_000.0),
                    modifier = Modifier.weight(1f),
                    accent = RideTeal,
                )
                MetricTile(
                    label = stringResource(R.string.current_speed),
                    value =
                        ride.currentSpeedMetersPerSecond?.let { speed ->
                            stringResource(R.string.value_kmh, speed * 3.6f)
                        } ?: stringResource(R.string.value_unavailable),
                    modifier = Modifier.weight(1f),
                )
            }
            Spacer(Modifier.height(18.dp))
            PrimaryRideButton(text = stringResource(R.string.return_to_ride), onClick = onOpenRide)
        }
    }
}

@Composable
private fun ProtectionCard(
    permissions: RidePermissionState,
    backgroundProtection: BackgroundProtectionState,
    onRequestPermissions: () -> Unit,
    onOpenBatterySettings: () -> Unit,
    onOpenLocationSettings: () -> Unit,
) {
    Card(
        modifier = Modifier.fillMaxWidth(),
        colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.surface),
    ) {
        Column(Modifier.padding(20.dp)) {
            Text(
                stringResource(R.string.protection_check_title),
                style = MaterialTheme.typography.titleMedium
            )
            Spacer(Modifier.height(6.dp))
            Text(
                text = stringResource(R.string.protection_check_body),
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                style = MaterialTheme.typography.bodyMedium,
            )
            Spacer(Modifier.height(16.dp))
            ProtectionRow(
                label = stringResource(R.string.precise_location),
                ready = permissions.hasFineLocation,
            )
            Spacer(Modifier.height(10.dp))
            ProtectionRow(
                label = stringResource(R.string.notifications),
                ready = permissions.hasNotifications,
            )
            Spacer(Modifier.height(10.dp))
            ProtectionRow(
                label = stringResource(R.string.location_services),
                ready = permissions.locationServicesEnabled,
            )
            Spacer(Modifier.height(10.dp))
            ProtectionRow(
                label = stringResource(R.string.battery_protection),
                ready = backgroundProtection.ignoresBatteryOptimizations,
            )
            Spacer(Modifier.height(16.dp))
            Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
                if (!permissions.hasFineLocation || !permissions.hasNotifications) {
                    OutlinedButton(
                        onClick = onRequestPermissions,
                        modifier = Modifier.fillMaxWidth(),
                    ) {
                        Text(stringResource(R.string.review_permissions))
                    }
                }
                if (!permissions.locationServicesEnabled) {
                    OutlinedButton(
                        onClick = onOpenLocationSettings,
                        modifier = Modifier.fillMaxWidth(),
                    ) {
                        Text(stringResource(R.string.location_settings))
                    }
                }
                if (!backgroundProtection.ignoresBatteryOptimizations) {
                    OutlinedButton(
                        onClick = onOpenBatterySettings,
                        modifier = Modifier.fillMaxWidth(),
                    ) {
                        Text(stringResource(R.string.battery_settings))
                    }
                }
            }
        }
    }
}

@Composable
private fun ProtectionRow(label: String, ready: Boolean) {
    Row(
        modifier = Modifier.fillMaxWidth(),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.SpaceBetween,
    ) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Canvas(Modifier.size(10.dp)) {
                drawCircle(if (ready) RideTeal else RideCoral)
            }
            Spacer(Modifier.width(10.dp))
            Text(label, style = MaterialTheme.typography.bodyLarge)
        }
        Text(
            text = stringResource(if (ready) R.string.status_ready else R.string.status_action_needed),
            color = if (ready) RideTeal else RideCoral,
            style = MaterialTheme.typography.labelLarge,
        )
    }
}

@Composable
private fun RouteDecoration(modifier: Modifier = Modifier) {
    Canvas(modifier) {
        val path = Path()
        path.moveTo(size.width * .88f, size.height * .12f)
        path.cubicTo(
            size.width * .28f,
            size.height * .12f,
            size.width * .76f,
            size.height * .58f,
            size.width * .16f,
            size.height * .82f,
        )
        drawPath(
            path = path,
            color = RideTeal.copy(alpha = .22f),
            style = Stroke(width = 5.dp.toPx(), cap = StrokeCap.Round),
        )
        drawCircle(
            RideCoral.copy(alpha = .8f),
            6.dp.toPx(),
            Offset(size.width * .16f, size.height * .82f)
        )
    }
}
