package com.gpxanimator.mobile.ui.screens

import androidx.compose.animation.core.LinearEasing
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.core.tween
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.focusable
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.gestures.waitForUpOrCancellation
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
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.input.key.Key
import androidx.compose.ui.input.key.KeyEventType
import androidx.compose.ui.input.key.key
import androidx.compose.ui.input.key.onPreviewKeyEvent
import androidx.compose.ui.input.key.type
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.onClick
import androidx.compose.ui.semantics.onLongClick
import androidx.compose.ui.semantics.role
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.unit.dp
import com.gpxanimator.mobile.R
import com.gpxanimator.mobile.ui.ActiveRideUi
import com.gpxanimator.mobile.ui.components.BackHeader
import com.gpxanimator.mobile.ui.components.MetricTile
import com.gpxanimator.mobile.ui.components.PillTone
import com.gpxanimator.mobile.ui.components.StatusPill
import com.gpxanimator.mobile.ui.components.formatDuration
import com.gpxanimator.mobile.ui.theme.RideCoral
import com.gpxanimator.mobile.ui.theme.RideTeal
import kotlinx.coroutines.delay

@Composable
fun RecordingScreen(
    activeRide: ActiveRideUi?,
    isLoading: Boolean,
    onBack: () -> Unit,
    onFinishRecording: () -> Unit,
    onRideFinished: () -> Unit,
    modifier: Modifier = Modifier,
) {
    var observedRide by rememberSaveable { mutableStateOf(false) }
    LaunchedEffect(activeRide?.id, isLoading) {
        if (activeRide != null) {
            observedRide = true
        } else if (observedRide && !isLoading) {
            onRideFinished()
        }
    }

    if (activeRide == null) {
        AwaitingRecordingScreen(onBack = onBack, modifier = modifier)
        return
    }

    var showFinishConfirmation by rememberSaveable { mutableStateOf(false) }
    val largeMetricStyle =
        if (LocalDensity.current.fontScale > 1.3f) {
            MaterialTheme.typography.headlineLarge
        } else {
            MaterialTheme.typography.displayLarge
        }
    LazyColumn(
        modifier = modifier.fillMaxSize(),
        contentPadding = androidx.compose.foundation.layout.PaddingValues(
            20.dp,
            16.dp,
            20.dp,
            32.dp
        ),
        verticalArrangement = Arrangement.spacedBy(18.dp),
    ) {
        item {
            BackHeader(title = activeRide.name, onBack = onBack)
        }
        item {
            Column(
                modifier = Modifier.fillMaxWidth(),
                horizontalAlignment = Alignment.CenterHorizontally,
            ) {
                StatusPill(
                    text = stringResource(R.string.recording_active),
                    tone = PillTone.Active,
                )
                Spacer(Modifier.height(18.dp))
                Text(
                    text = formatDuration(activeRide.elapsedMillis),
                    style = largeMetricStyle,
                )
                Text(
                    text = stringResource(R.string.elapsed_time),
                    style = MaterialTheme.typography.bodyLarge,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        }
        item {
            Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                MetricTile(
                    label = stringResource(R.string.distance),
                    value = stringResource(R.string.value_km, activeRide.distanceMeters / 1_000.0),
                    modifier = Modifier.weight(1f),
                    accent = RideTeal,
                )
                MetricTile(
                    label = stringResource(R.string.current_speed),
                    value =
                        if (activeRide.currentSpeedMetersPerSecond == null) {
                            stringResource(R.string.value_unavailable)
                        } else {
                            stringResource(
                                R.string.value_kmh,
                                activeRide.currentSpeedMetersPerSecond * 3.6f,
                            )
                        },
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
                            activeRide.averageSpeedMetersPerSecond * 3.6,
                        ),
                    modifier = Modifier.weight(1f),
                )
                MetricTile(
                    label = stringResource(R.string.saved_points),
                    value = activeRide.pointCount.toString(),
                    modifier = Modifier.weight(1f),
                )
            }
        }
        item {
            GpsStatusCard(activeRide)
        }
        item {
            ScreenOffSafetyCard()
        }
        item {
            HoldToFinishButton(onHeld = { showFinishConfirmation = true })
        }
    }

    if (showFinishConfirmation) {
        AlertDialog(
            onDismissRequest = { showFinishConfirmation = false },
            title = { Text(stringResource(R.string.finish_confirm_title)) },
            text = { Text(stringResource(R.string.finish_confirm_body)) },
            dismissButton = {
                TextButton(onClick = { showFinishConfirmation = false }) {
                    Text(stringResource(R.string.cancel))
                }
            },
            confirmButton = {
                TextButton(
                    onClick = {
                        showFinishConfirmation = false
                        onFinishRecording()
                    },
                ) {
                    Text(stringResource(R.string.finish_ride), color = RideCoral)
                }
            },
        )
    }
}

@Composable
private fun AwaitingRecordingScreen(onBack: () -> Unit, modifier: Modifier = Modifier) {
    Column(
        modifier = modifier.fillMaxSize().padding(20.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        BackHeader(title = stringResource(R.string.preparing_ride), onBack = onBack)
        Spacer(Modifier.weight(1f))
        Canvas(Modifier.size(72.dp)) {
            drawCircle(RideTeal.copy(alpha = .15f))
            drawCircle(RideTeal, size.minDimension * .22f, style = Stroke(5.dp.toPx()))
        }
        Spacer(Modifier.height(20.dp))
        Text(
            stringResource(R.string.preparing_ride),
            style = MaterialTheme.typography.headlineMedium
        )
        Spacer(Modifier.height(8.dp))
        Text(
            stringResource(R.string.preparing_ride_body),
            style = MaterialTheme.typography.bodyLarge,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        Spacer(Modifier.weight(1f))
    }
}

@Composable
private fun GpsStatusCard(ride: ActiveRideUi) {
    val accuracy = ride.gpsAccuracyMeters
    val hasStaleFix = (ride.gpsFixAgeMillis ?: 0L) > FRESH_FIX_MAX_AGE_MILLIS
    val tone =
        when {
            hasStaleFix -> PillTone.Error
            accuracy == null -> PillTone.Neutral
            accuracy <= 20f -> PillTone.Active
            accuracy <= 50f -> PillTone.Warning
            else -> PillTone.Error
        }
    val label =
        if (hasStaleFix) {
            stringResource(R.string.gps_stale)
        } else if (accuracy == null) {
            stringResource(R.string.gps_waiting)
        } else {
            stringResource(R.string.gps_accuracy, accuracy)
        }
    Card(
        modifier = Modifier.fillMaxWidth(),
        colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.surface),
    ) {
        Row(
            modifier = Modifier.fillMaxWidth().padding(18.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.SpaceBetween,
        ) {
            Column {
                Text(
                    stringResource(R.string.gps_signal),
                    style = MaterialTheme.typography.titleMedium
                )
                Text(
                    stringResource(R.string.gps_signal_body),
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
            StatusPill(text = label, tone = tone)
        }
    }
}

@Composable
private fun ScreenOffSafetyCard() {
    Card(
        modifier = Modifier.fillMaxWidth(),
        colors = CardDefaults.cardColors(
            containerColor = MaterialTheme.colorScheme.primaryContainer.copy(
                alpha = .42f
            )
        ),
    ) {
        Row(
            modifier = Modifier.fillMaxWidth().padding(18.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Canvas(Modifier.size(34.dp)) {
                drawCircle(RideTeal.copy(alpha = .2f))
                drawLine(
                    RideTeal,
                    start = androidx.compose.ui.geometry.Offset(size.width * .32f, center.y),
                    end = androidx.compose.ui.geometry.Offset(size.width * .68f, center.y),
                    strokeWidth = 3.dp.toPx(),
                    cap = StrokeCap.Round,
                )
            }
            Spacer(Modifier.size(14.dp))
            Column {
                Text(
                    stringResource(R.string.screen_off_safe),
                    style = MaterialTheme.typography.titleMedium
                )
                Text(
                    stringResource(R.string.screen_off_safe_body),
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        }
    }
}

@Composable
private fun HoldToFinishButton(onHeld: () -> Unit) {
    var pressed by remember { mutableStateOf(false) }
    val haptics = LocalHapticFeedback.current
    val progress by
    animateFloatAsState(
        targetValue = if (pressed) 1f else 0f,
        animationSpec =
            if (pressed) tween(1_500, easing = LinearEasing) else tween(180),
        label = "finishHoldProgress",
    )
    val holdDescription = stringResource(R.string.finish_hold_accessibility)
    val stateText = stringResource(if (pressed) R.string.keep_holding else R.string.finish_hold)

    LaunchedEffect(pressed) {
        if (pressed) {
            delay(1_500)
            if (pressed) {
                pressed = false
                haptics.performHapticFeedback(HapticFeedbackType.LongPress)
                onHeld()
            }
        }
    }

    Surface(
        modifier =
            Modifier.fillMaxWidth()
                .height(62.dp)
                .clip(RoundedCornerShape(20.dp))
                .semantics {
                    role = Role.Button
                    contentDescription = holdDescription
                    stateDescription = stateText
                    onClick(label = holdDescription) {
                        onHeld()
                        true
                    }
                    onLongClick(label = holdDescription) {
                        onHeld()
                        true
                    }
                }.onPreviewKeyEvent { event ->
                    if (
                        event.type == KeyEventType.KeyUp &&
                        event.key in setOf(Key.Enter, Key.NumPadEnter, Key.Spacebar)
                    ) {
                        onHeld()
                        true
                    } else {
                        false
                    }
                }.focusable()
                .pointerInput(Unit) {
                    awaitEachGesture {
                        awaitFirstDown(requireUnconsumed = false)
                        pressed = true
                        waitForUpOrCancellation()
                        pressed = false
                    }
                },
        color = RideCoral.copy(alpha = .14f),
        contentColor = RideCoral,
        shape = RoundedCornerShape(20.dp),
    ) {
        Box(Modifier.fillMaxSize()) {
            Box(
                Modifier.fillMaxWidth(progress)
                    .height(62.dp)
                    .align(Alignment.CenterStart)
                    .clip(RoundedCornerShape(20.dp)),
            ) {
                Surface(
                    modifier = Modifier.fillMaxSize(),
                    color = RideCoral.copy(alpha = .22f),
                    content = {},
                )
            }
            Text(
                text = stateText,
                modifier = Modifier.align(Alignment.Center),
                style = MaterialTheme.typography.titleMedium,
            )
        }
    }
}

private const val FRESH_FIX_MAX_AGE_MILLIS = 15_000L
