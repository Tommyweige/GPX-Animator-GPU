package com.gpxanimator.mobile.ui.screens

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Button
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import com.gpxanimator.mobile.R
import com.gpxanimator.mobile.data.SyncState
import com.gpxanimator.mobile.data.TripState
import com.gpxanimator.mobile.ui.RideDetailUi
import com.gpxanimator.mobile.ui.TrackCoordinateUi
import com.gpxanimator.mobile.ui.components.BackHeader
import com.gpxanimator.mobile.ui.components.LocalStatusPill
import com.gpxanimator.mobile.ui.components.MetricTile
import com.gpxanimator.mobile.ui.components.SectionHeading
import com.gpxanimator.mobile.ui.components.SyncStatusPill
import com.gpxanimator.mobile.ui.components.formatDateTime
import com.gpxanimator.mobile.ui.components.formatDuration
import com.gpxanimator.mobile.ui.theme.RideCoral
import com.gpxanimator.mobile.ui.theme.RideTeal
import kotlin.math.max

@Composable
fun RideDetailScreen(
    detail: RideDetailUi?,
    onBack: () -> Unit,
    onShareRide: (String) -> Unit,
    onRetryDriveSync: (String) -> Unit,
    onFinalizeInterruptedRide: (String) -> Unit,
    modifier: Modifier = Modifier,
) {
    if (detail == null) {
        Column(modifier.fillMaxSize().padding(20.dp)) {
            BackHeader(title = stringResource(R.string.ride_details), onBack = onBack)
            Spacer(Modifier.weight(1f))
            CircularProgressIndicator(
                Modifier.align(Alignment.CenterHorizontally),
                color = RideTeal
            )
            Spacer(Modifier.weight(1f))
        }
        return
    }

    LazyColumn(
        modifier = modifier.fillMaxSize(),
        contentPadding = androidx.compose.foundation.layout.PaddingValues(
            20.dp,
            16.dp,
            20.dp,
            36.dp
        ),
        verticalArrangement = Arrangement.spacedBy(18.dp),
    ) {
        item { BackHeader(title = detail.trip.name, onBack = onBack) }
        item {
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                LocalStatusPill(detail.trip.hasLocalGpx)
                SyncStatusPill(detail.trip.syncState)
            }
        }
        if (
            detail.trip.pointCount >= 2 &&
            detail.trip.state in setOf(TripState.INTERRUPTED, TripState.EXPORT_FAILED)
        ) {
            item {
                Button(
                    onClick = { onFinalizeInterruptedRide(detail.trip.id) },
                    modifier = Modifier.fillMaxWidth(),
                ) {
                    Text(
                        stringResource(
                            if (detail.trip.state == TripState.INTERRUPTED) {
                                R.string.export_saved_points
                            } else {
                                R.string.retry_gpx_export
                            },
                        ),
                    )
                }
            }
        }
        if (detail.trip.hasLocalGpx) {
            item {
                Row(
                    modifier = Modifier.fillMaxWidth(),
                    horizontalArrangement = Arrangement.spacedBy(12.dp),
                ) {
                    Button(
                        onClick = { onShareRide(detail.trip.id) },
                        modifier = Modifier.weight(1f),
                    ) {
                        Text(stringResource(R.string.share_gpx))
                    }
                    if (
                        detail.trip.syncState in
                        setOf(SyncState.LOCAL_ONLY, SyncState.FAILED, SyncState.AUTH_REQUIRED)
                    ) {
                        OutlinedButton(
                            onClick = { onRetryDriveSync(detail.trip.id) },
                            modifier = Modifier.weight(1f),
                        ) {
                            Text(stringResource(R.string.retry_drive_sync))
                        }
                    }
                }
            }
        }
        item {
            TrackPreview(
                points = detail.track,
                modifier = Modifier.fillMaxWidth().aspectRatio(1.45f),
            )
        }
        item { SectionHeading(stringResource(R.string.ride_summary)) }
        item {
            Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                MetricTile(
                    label = stringResource(R.string.distance),
                    value = stringResource(R.string.value_km, detail.trip.distanceMeters / 1_000.0),
                    modifier = Modifier.weight(1f),
                    accent = RideTeal,
                )
                MetricTile(
                    label = stringResource(R.string.elapsed_time),
                    value = formatDuration(detail.trip.durationMillis),
                    modifier = Modifier.weight(1f),
                )
            }
        }
        item {
            Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                MetricTile(
                    label = stringResource(R.string.average_speed),
                    value =
                        stringResource(
                            R.string.value_kmh,
                            averageSpeedKmh(detail.trip.distanceMeters, detail.trip.durationMillis),
                        ),
                    modifier = Modifier.weight(1f),
                )
                MetricTile(
                    label = stringResource(R.string.saved_points),
                    value = detail.trip.pointCount.toString(),
                    modifier = Modifier.weight(1f),
                )
            }
        }
        item {
            InfoCard(
                title = stringResource(R.string.ride_started),
                value = formatDateTime(detail.trip.startedAtEpochMillis),
            )
        }
        if (detail.localGpxPath != null) {
            item {
                InfoCard(
                    title = stringResource(R.string.local_gpx_file),
                    value = detail.localGpxPath,
                )
            }
        }
        if (!detail.lastError.isNullOrBlank()) {
            item {
                InfoCard(
                    title = stringResource(R.string.last_sync_issue),
                    value = localizedIssue(detail),
                    isError = true,
                )
            }
        }
    }
}

@Composable
private fun localizedIssue(detail: RideDetailUi): String =
    stringResource(
        when {
            detail.trip.state == TripState.INTERRUPTED -> R.string.issue_recording_interrupted
            detail.trip.state == TripState.EXPORT_FAILED -> R.string.issue_export_failed
            detail.trip.syncState == SyncState.AUTH_REQUIRED -> R.string.issue_drive_authorization
            else -> R.string.issue_drive_sync
        },
    )

@Composable
private fun TrackPreview(
    points: List<TrackCoordinateUi>,
    modifier: Modifier = Modifier,
) {
    val gridColor = MaterialTheme.colorScheme.outlineVariant.copy(alpha = .45f)
    val previewSurfaceColor = MaterialTheme.colorScheme.surface
    Card(
        modifier = modifier,
        colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.surface),
        shape = MaterialTheme.shapes.large,
    ) {
        Box(Modifier.fillMaxSize()) {
            Canvas(Modifier.fillMaxSize().padding(22.dp)) {
                repeat(5) { index ->
                    val fraction = index / 4f
                    drawLine(
                        gridColor,
                        Offset(size.width * fraction, 0f),
                        Offset(size.width * fraction, size.height)
                    )
                    drawLine(
                        gridColor,
                        Offset(0f, size.height * fraction),
                        Offset(size.width, size.height * fraction)
                    )
                }
                if (points.size >= 2) {
                    val minLon = points.minOf { it.longitude }
                    val maxLon = points.maxOf { it.longitude }
                    val minLat = points.minOf { it.latitude }
                    val maxLat = points.maxOf { it.latitude }
                    val lonRange = max(maxLon - minLon, 0.000001)
                    val latRange = max(maxLat - minLat, 0.000001)
                    val inset = 10.dp.toPx()
                    fun toOffset(point: TrackCoordinateUi): Offset {
                        val x =
                            inset + (((point.longitude - minLon) / lonRange).toFloat() * (size.width - inset * 2))
                        val y =
                            inset + ((1f - ((point.latitude - minLat) / latRange).toFloat()) * (size.height - inset * 2))
                        return Offset(x, y)
                    }

                    val path = Path()
                    val start = toOffset(points.first())
                    path.moveTo(start.x, start.y)
                    points.drop(1).forEach { point ->
                        val offset = toOffset(point)
                        path.lineTo(offset.x, offset.y)
                    }
                    drawPath(
                        path = path,
                        color = RideTeal,
                        style = Stroke(width = 4.dp.toPx(), cap = StrokeCap.Round),
                    )
                    drawCircle(RideTeal, radius = 7.dp.toPx(), center = start)
                    drawCircle(RideCoral, radius = 7.dp.toPx(), center = toOffset(points.last()))
                    drawCircle(previewSurfaceColor, radius = 3.dp.toPx(), center = start)
                    drawCircle(
                        previewSurfaceColor,
                        radius = 3.dp.toPx(),
                        center = toOffset(points.last())
                    )
                }
            }
            Column(
                modifier = Modifier.align(Alignment.TopStart).padding(18.dp),
            ) {
                Text(
                    stringResource(R.string.route_preview),
                    style = MaterialTheme.typography.titleMedium
                )
                Text(
                    stringResource(R.string.route_preview_local),
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
            if (points.size < 2) {
                Text(
                    text = stringResource(R.string.route_preview_empty),
                    modifier = Modifier.align(Alignment.Center).padding(36.dp),
                    style = MaterialTheme.typography.bodyLarge,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        }
    }
}

@Composable
private fun InfoCard(title: String, value: String, isError: Boolean = false) {
    Card(
        modifier = Modifier.fillMaxWidth(),
        colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.surface),
    ) {
        Column(Modifier.padding(18.dp)) {
            Text(
                text = title,
                style = MaterialTheme.typography.labelLarge,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            Spacer(Modifier.height(6.dp))
            Text(
                text = value,
                style = MaterialTheme.typography.bodyLarge,
                color = if (isError) MaterialTheme.colorScheme.error else MaterialTheme.colorScheme.onSurface,
                maxLines = 3,
                overflow = TextOverflow.Ellipsis,
            )
        }
    }
}

private fun averageSpeedKmh(distanceMeters: Double, durationMillis: Long): Double =
    if (durationMillis > 0) distanceMeters / (durationMillis / 1_000.0) * 3.6 else 0.0
