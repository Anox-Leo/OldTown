package dev.oldtown.android.ui.theme

import androidx.compose.runtime.Immutable
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.graphics.Color

/**
 * Palette reprise du thème par défaut de T3 Code (appli mobile), sombre et
 * claire : écran presque noir, cartes à peine plus claires, bordures fines,
 * un seul accent bleu.
 */
@Immutable
data class OtColors(
    val screen: Color,
    val card: Color,
    val groupedCard: Color,
    val rowHover: Color,
    val foreground: Color,
    val muted: Color,
    val tertiary: Color,
    val border: Color,
    val inputBorder: Color,
    val primary: Color,
    val onPrimary: Color,
    val primaryText: Color,
    val success: Color,
    val warning: Color,
    val warningBorder: Color,
    val warningForeground: Color,
    val danger: Color,
    val dangerBorder: Color,
    val dangerForeground: Color,
    val backdrop: Color,
    val isDark: Boolean,
)

val DarkColors = OtColors(
    screen = Color(0xFF0A0A0A),
    card = Color(0xFF111111),
    groupedCard = Color(0xFF1A1B1B),
    rowHover = Color(0xFF141414),
    foreground = Color(0xFFF5F5F5),
    muted = Color(0xFF838383),
    tertiary = Color(0xFF818181),
    border = Color(0xFF191919),
    inputBorder = Color(0xFF1E1E1E),
    primary = Color(0xFF346BF1),
    onPrimary = Color(0xFFFFFFFF),
    primaryText = Color(0xFF4B7CF3),
    success = Color(0xFF34D399),
    warning = Color(0xFF312108),
    warningBorder = Color(0x52FE9A00),
    warningForeground = Color(0xFFFFB900),
    danger = Color(0xFF301214),
    dangerBorder = Color(0x52FB414A),
    dangerForeground = Color(0xFFFF6467),
    backdrop = Color(0x7A000000),
    isDark = true,
)

val LightColors = OtColors(
    screen = Color(0xFFFCFCFC),
    card = Color(0xFFFFFFFF),
    groupedCard = Color(0xFFF4F4F5),
    rowHover = Color(0xFFF4F4F5),
    foreground = Color(0xFF27272A),
    muted = Color(0xFF6F6F79),
    tertiary = Color(0xFF71717B),
    border = Color(0xFFE4E4E7),
    inputBorder = Color(0xFFD4D4D8),
    primary = Color(0xFF1B4ED8),
    onPrimary = Color(0xFFFFFFFF),
    primaryText = Color(0xFF1B4ED8),
    success = Color(0xFF059669),
    warning = Color(0xFFFCF4E8),
    warningBorder = Color(0x52FE9A00),
    warningForeground = Color(0xFFBB4D00),
    danger = Color(0xFFFCEBEC),
    dangerBorder = Color(0x52FB2C36),
    dangerForeground = Color(0xFFC10007),
    backdrop = Color(0x38000000),
    isDark = false,
)

val LocalOtColors = staticCompositionLocalOf { DarkColors }
