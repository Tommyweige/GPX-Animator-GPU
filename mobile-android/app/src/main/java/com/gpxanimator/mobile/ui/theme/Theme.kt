package com.gpxanimator.mobile.ui.theme

import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.darkColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.ui.graphics.Color

private val RideDarkColors =
    darkColorScheme(
        primary = Color(0xFF2DD4BF),
        onPrimary = Color(0xFF05201D),
        secondary = Color(0xFFFF7A66),
        background = Color(0xFF101417),
        onBackground = Color(0xFFF7F3EE),
        surface = Color(0xFF192126),
        onSurface = Color(0xFFF7F3EE),
        surfaceVariant = Color(0xFF243036),
        onSurfaceVariant = Color(0xFFB7C6CC),
        error = Color(0xFFFFB4AB),
    )

@Composable
fun GpxAnimatorRideTheme(content: @Composable () -> Unit) {
    MaterialTheme(
        colorScheme = RideDarkColors,
        content = content,
    )
}
