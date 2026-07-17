package com.gpxanimator.mobile.ui.components

import androidx.annotation.StringRes
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.NavigationBar
import androidx.compose.material3.NavigationBarItem
import androidx.compose.material3.NavigationBarItemDefaults
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import com.gpxanimator.mobile.R
import com.gpxanimator.mobile.data.SyncState
import com.gpxanimator.mobile.data.TripState
import com.gpxanimator.mobile.ui.TripListItemUi
import com.gpxanimator.mobile.ui.theme.RideCoral
import com.gpxanimator.mobile.ui.theme.RideMuted
import com.gpxanimator.mobile.ui.theme.RideTeal
import java.text.DateFormat
import java.util.Date
import java.util.Locale

enum class PillTone {
    Active,
    Neutral,
    Warning,
    Error,
}

@Composable
fun StatusPill(
    text: String,
    modifier: Modifier = Modifier,
    tone: PillTone = PillTone.Neutral,
) {
    val foreground =
        when (tone) {
            PillTone.Active -> RideTeal
            PillTone.Warning -> RideCoral
            PillTone.Error -> MaterialTheme.colorScheme.error
            PillTone.Neutral -> MaterialTheme.colorScheme.onSurfaceVariant
        }
    val background = foreground.copy(alpha = 0.12f)
    Surface(
        modifier = modifier,
        color = background,
        contentColor = foreground,
        shape = MaterialTheme.shapes.small,
    ) {
        Text(
            text = text,
            modifier = Modifier.padding(horizontal = 10.dp, vertical = 6.dp),
            style = MaterialTheme.typography.labelLarge,
            maxLines = 2,
        )
    }
}

@Composable
fun SyncStatusPill(syncState: SyncState, modifier: Modifier = Modifier) {
    val label =
        when (syncState) {
            SyncState.LOCAL_ONLY -> R.string.sync_local_only
            SyncState.PENDING -> R.string.sync_pending
            SyncState.UPLOADING -> R.string.sync_uploading
            SyncState.SYNCED -> R.string.sync_synced
            SyncState.FAILED -> R.string.sync_failed
            SyncState.AUTH_REQUIRED -> R.string.sync_auth_required
        }
    val tone =
        when (syncState) {
            SyncState.SYNCED -> PillTone.Active
            SyncState.UPLOADING, SyncState.PENDING -> PillTone.Warning
            SyncState.FAILED, SyncState.AUTH_REQUIRED -> PillTone.Error
            SyncState.LOCAL_ONLY -> PillTone.Neutral
        }
    StatusPill(
        text = stringResource(label),
        modifier = modifier,
        tone = tone,
    )
}

@Composable
fun LocalStatusPill(hasLocalGpx: Boolean, modifier: Modifier = Modifier) {
    StatusPill(
        text =
            stringResource(
                if (hasLocalGpx) R.string.local_gpx_ready else R.string.local_recording,
            ),
        tone = if (hasLocalGpx) PillTone.Active else PillTone.Neutral,
        modifier = modifier,
    )
}

@Composable
fun SectionHeading(
    title: String,
    modifier: Modifier = Modifier,
    actionLabel: String? = null,
    onAction: (() -> Unit)? = null,
) {
    Row(
        modifier = modifier.fillMaxWidth(),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.SpaceBetween,
    ) {
        Text(text = title, style = MaterialTheme.typography.titleLarge)
        if (actionLabel != null && onAction != null) {
            TextButton(onClick = onAction) { Text(actionLabel) }
        }
    }
}

@Composable
fun PageIntro(
    eyebrow: String,
    title: String,
    subtitle: String,
    modifier: Modifier = Modifier,
) {
    Column(modifier = modifier) {
        Text(
            text = eyebrow,
            style = MaterialTheme.typography.labelLarge,
            color = RideTeal,
        )
        Spacer(Modifier.height(8.dp))
        Text(text = title, style = MaterialTheme.typography.headlineLarge)
        Spacer(Modifier.height(8.dp))
        Text(
            text = subtitle,
            style = MaterialTheme.typography.bodyLarge,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
    }
}

@Composable
fun BackHeader(
    title: String,
    onBack: () -> Unit,
    modifier: Modifier = Modifier,
) {
    Row(
        modifier = modifier.fillMaxWidth(),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        TextButton(onClick = onBack) {
            Text(text = "‹", style = MaterialTheme.typography.headlineMedium)
            Spacer(Modifier.width(4.dp))
            Text(stringResource(R.string.back))
        }
        Spacer(Modifier.width(8.dp))
        Text(
            text = title,
            style = MaterialTheme.typography.titleLarge,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
        )
    }
}

@Composable
fun MetricTile(
    label: String,
    value: String,
    modifier: Modifier = Modifier,
    accent: Color = MaterialTheme.colorScheme.onSurface,
) {
    Card(
        modifier = modifier,
        colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.surfaceVariant),
        shape = MaterialTheme.shapes.medium,
    ) {
        Column(Modifier.padding(18.dp)) {
            Text(
                text = label,
                style = MaterialTheme.typography.labelLarge,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            Spacer(Modifier.height(8.dp))
            Text(
                text = value,
                style = MaterialTheme.typography.headlineMedium,
                color = accent,
            )
        }
    }
}

@Composable
fun RideListRow(
    trip: TripListItemUi,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
) {
    Card(
        onClick = onClick,
        modifier = modifier.fillMaxWidth(),
        colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.surface),
        shape = MaterialTheme.shapes.medium,
    ) {
        Column(Modifier.padding(18.dp)) {
            Row(
                modifier = Modifier.fillMaxWidth(),
                verticalAlignment = Alignment.Top,
                horizontalArrangement = Arrangement.SpaceBetween,
            ) {
                Column(Modifier.weight(1f)) {
                    Text(
                        text = trip.name,
                        style = MaterialTheme.typography.titleMedium,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                    )
                    Spacer(Modifier.height(4.dp))
                    Text(
                        text = formatDateTime(trip.startedAtEpochMillis),
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
                Spacer(Modifier.width(12.dp))
                SyncStatusPill(trip.syncState)
            }
            Spacer(Modifier.height(16.dp))
            Row(horizontalArrangement = Arrangement.spacedBy(22.dp)) {
                CompactMetric(
                    label = stringResource(R.string.distance),
                    value = stringResource(R.string.value_km, trip.distanceMeters / 1_000.0),
                )
                CompactMetric(
                    label = stringResource(R.string.elapsed_time),
                    value = formatDuration(trip.durationMillis),
                )
                if (trip.state == TripState.INTERRUPTED || trip.state == TripState.EXPORT_FAILED) {
                    StatusPill(
                        text =
                            stringResource(
                                if (trip.state == TripState.INTERRUPTED) {
                                    R.string.trip_interrupted
                                } else {
                                    R.string.trip_export_failed
                                },
                            ),
                        tone = PillTone.Error,
                    )
                }
            }
        }
    }
}

@Composable
private fun CompactMetric(label: String, value: String) {
    Column {
        Text(
            text = label,
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        Text(
            text = value,
            style = MaterialTheme.typography.titleMedium,
            fontWeight = FontWeight.SemiBold,
        )
    }
}

@Composable
fun EmptyStateCard(
    title: String,
    body: String,
    modifier: Modifier = Modifier,
) {
    Card(
        modifier = modifier.fillMaxWidth(),
        colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.surface),
    ) {
        Column(Modifier.padding(24.dp), horizontalAlignment = Alignment.CenterHorizontally) {
            Canvas(Modifier.size(44.dp)) {
                drawCircle(color = RideTeal.copy(alpha = 0.15f))
                drawCircle(color = RideTeal, radius = size.minDimension * 0.18f)
            }
            Spacer(Modifier.height(12.dp))
            Text(text = title, style = MaterialTheme.typography.titleMedium)
            Spacer(Modifier.height(6.dp))
            Text(
                text = body,
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
    }
}

enum class TopLevelDestination(
    val route: String,
    @param:StringRes val label: Int,
) {
    Home("home", R.string.nav_home),
    History("history", R.string.nav_history),
    Settings("settings", R.string.nav_settings),
}

@Composable
fun RideBottomBar(
    selectedRoute: String?,
    onSelect: (TopLevelDestination) -> Unit,
) {
    NavigationBar(
        containerColor = MaterialTheme.colorScheme.surface,
        tonalElevation = 0.dp,
    ) {
        TopLevelDestination.entries.forEach { destination ->
            val selected = destination.route == selectedRoute
            NavigationBarItem(
                selected = selected,
                onClick = { onSelect(destination) },
                icon = { NavigationGlyph(destination, selected) },
                label = { Text(stringResource(destination.label)) },
                colors =
                    NavigationBarItemDefaults.colors(
                        selectedIconColor = RideTeal,
                        selectedTextColor = RideTeal,
                        indicatorColor = RideTeal.copy(alpha = 0.12f),
                        unselectedIconColor = RideMuted,
                        unselectedTextColor = RideMuted,
                    ),
            )
        }
    }
}

@Composable
private fun NavigationGlyph(destination: TopLevelDestination, selected: Boolean) {
    val color = if (selected) RideTeal else RideMuted
    Canvas(Modifier.size(22.dp)) {
        when (destination) {
            TopLevelDestination.Home -> {
                drawLine(
                    color,
                    Offset(size.width * .18f, size.height * .5f),
                    Offset(size.width * .5f, size.height * .2f),
                    strokeWidth = 2.3.dp.toPx(),
                    cap = StrokeCap.Round
                )
                drawLine(
                    color,
                    Offset(size.width * .5f, size.height * .2f),
                    Offset(size.width * .82f, size.height * .5f),
                    strokeWidth = 2.3.dp.toPx(),
                    cap = StrokeCap.Round
                )
                drawRoundRect(
                    color,
                    Offset(size.width * .25f, size.height * .45f),
                    Size(size.width * .5f, size.height * .4f),
                    style = Stroke(2.3.dp.toPx())
                )
            }

            TopLevelDestination.History -> {
                drawCircle(color, size.minDimension * .36f, style = Stroke(2.3.dp.toPx()))
                drawLine(
                    color,
                    center,
                    Offset(center.x, center.y - size.height * .2f),
                    strokeWidth = 2.3.dp.toPx(),
                    cap = StrokeCap.Round
                )
                drawLine(
                    color,
                    center,
                    Offset(center.x + size.width * .18f, center.y),
                    strokeWidth = 2.3.dp.toPx(),
                    cap = StrokeCap.Round
                )
            }

            TopLevelDestination.Settings -> {
                drawCircle(color, size.minDimension * .33f, style = Stroke(2.3.dp.toPx()))
                drawCircle(color, size.minDimension * .1f)
            }
        }
    }
}

@Composable
fun PrimaryRideButton(
    text: String,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
) {
    Button(
        onClick = onClick,
        modifier = modifier.fillMaxWidth().height(58.dp),
        enabled = enabled,
        colors =
            ButtonDefaults.buttonColors(
                containerColor = RideTeal,
                contentColor = MaterialTheme.colorScheme.onPrimary,
            ),
        shape = MaterialTheme.shapes.medium,
    ) {
        Text(text = text, style = MaterialTheme.typography.titleMedium)
    }
}

fun formatDuration(durationMillis: Long): String {
    val totalSeconds = (durationMillis.coerceAtLeast(0) / 1_000)
    val hours = totalSeconds / 3_600
    val minutes = (totalSeconds % 3_600) / 60
    val seconds = totalSeconds % 60
    return String.format(Locale.getDefault(), "%02d:%02d:%02d", hours, minutes, seconds)
}

fun formatDateTime(epochMillis: Long): String =
    DateFormat.getDateTimeInstance(DateFormat.MEDIUM, DateFormat.SHORT).format(Date(epochMillis))
