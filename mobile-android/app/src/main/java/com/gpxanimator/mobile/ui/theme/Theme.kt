package com.gpxanimator.mobile.ui.theme

import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Shapes
import androidx.compose.material3.Typography
import androidx.compose.material3.darkColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.foundation.shape.RoundedCornerShape

val RideGraphite = Color(0xFF0B1114)
val RideSurface = Color(0xFF151D21)
val RideSurfaceRaised = Color(0xFF1C282D)
val RideTeal = Color(0xFF31D6B4)
val RideCoral = Color(0xFFFF806B)
val RideCream = Color(0xFFF6F1E8)
val RideMuted = Color(0xFFA9BBC1)

private val RideDarkColors =
    darkColorScheme(
        primary = RideTeal,
        onPrimary = Color(0xFF05201D),
        primaryContainer = Color(0xFF123D35),
        onPrimaryContainer = Color(0xFF9DF4DF),
        secondary = RideCoral,
        onSecondary = Color(0xFF32100A),
        secondaryContainer = Color(0xFF51251D),
        onSecondaryContainer = Color(0xFFFFDAD3),
        tertiary = RideCream,
        background = RideGraphite,
        onBackground = RideCream,
        surface = RideSurface,
        onSurface = RideCream,
        surfaceVariant = RideSurfaceRaised,
        onSurfaceVariant = RideMuted,
        outline = Color(0xFF45565C),
        outlineVariant = Color(0xFF28363B),
        error = Color(0xFFFFB4AB),
    )

private val RideTypography =
    Typography(
        displayLarge =
            TextStyle(
                fontFamily = FontFamily.SansSerif,
                fontWeight = FontWeight.SemiBold,
                fontSize = 58.sp,
                lineHeight = 62.sp,
                letterSpacing = (-1.5).sp,
            ),
        displaySmall =
            TextStyle(
                fontFamily = FontFamily.SansSerif,
                fontWeight = FontWeight.SemiBold,
                fontSize = 38.sp,
                lineHeight = 42.sp,
                letterSpacing = (-0.8).sp,
            ),
        headlineLarge =
            TextStyle(
                fontFamily = FontFamily.SansSerif,
                fontWeight = FontWeight.SemiBold,
                fontSize = 30.sp,
                lineHeight = 36.sp,
            ),
        headlineMedium =
            TextStyle(
                fontFamily = FontFamily.SansSerif,
                fontWeight = FontWeight.SemiBold,
                fontSize = 24.sp,
                lineHeight = 30.sp,
            ),
        titleLarge =
            TextStyle(
                fontFamily = FontFamily.SansSerif,
                fontWeight = FontWeight.SemiBold,
                fontSize = 20.sp,
                lineHeight = 26.sp,
            ),
        titleMedium =
            TextStyle(
                fontFamily = FontFamily.SansSerif,
                fontWeight = FontWeight.SemiBold,
                fontSize = 16.sp,
                lineHeight = 22.sp,
            ),
        bodyLarge =
            TextStyle(
                fontFamily = FontFamily.SansSerif,
                fontWeight = FontWeight.Normal,
                fontSize = 16.sp,
                lineHeight = 24.sp,
            ),
        bodyMedium =
            TextStyle(
                fontFamily = FontFamily.SansSerif,
                fontWeight = FontWeight.Normal,
                fontSize = 14.sp,
                lineHeight = 20.sp,
            ),
        labelLarge =
            TextStyle(
                fontFamily = FontFamily.SansSerif,
                fontWeight = FontWeight.SemiBold,
                fontSize = 14.sp,
                lineHeight = 18.sp,
                letterSpacing = 0.2.sp,
            ),
    )

private val RideShapes =
    Shapes(
        small = RoundedCornerShape(12.dp),
        medium = RoundedCornerShape(20.dp),
        large = RoundedCornerShape(28.dp),
    )

@Composable
fun GpxAnimatorRideTheme(content: @Composable () -> Unit) {
    MaterialTheme(
        colorScheme = RideDarkColors,
        typography = RideTypography,
        shapes = RideShapes,
        content = content,
    )
}
