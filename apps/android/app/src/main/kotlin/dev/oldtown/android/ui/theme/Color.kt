package dev.oldtown.android.ui.theme

import androidx.compose.runtime.Immutable
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.graphics.Color

/**
 * Palette des écrans : les rôles du système de design (`Tokens.kt`, généré
 * depuis `design/tokens.json`), sombre « nuit » et clair « parchemin ».
 * Aucune valeur ici : les noms des champs sont ceux des écrans, les jetons
 * gardent ceux de l'extension.
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
    /** Bulle envoyée, sous le texte normal ([foreground]). */
    val bubbleOut: Color,
    val success: Color,
    val warning: Color,
    val warningBorder: Color,
    val warningForeground: Color,
    val danger: Color,
    val dangerBorder: Color,
    val dangerForeground: Color,
    val backdrop: Color,
    /** Tuile de l'icône, sous le signe (« Tous mes appareils »). */
    val tile: Color,
    /** Le signe, toujours en pierre. */
    val mark: Color,
    /** Pictogramme posé sur la couleur d'un avatar. */
    val onAvatar: Color,
    val isDark: Boolean,
)

val DarkColors = with(OtTokens.Dark) {
    OtColors(
        screen = screen,
        card = card,
        groupedCard = grouped,
        rowHover = hover,
        foreground = fg,
        muted = muted,
        tertiary = tertiary,
        border = border,
        inputBorder = inputBorder,
        primary = primary,
        onPrimary = onPrimary,
        primaryText = primaryText,
        bubbleOut = bubbleOut,
        success = success,
        warning = warning,
        warningBorder = warningBorder,
        warningForeground = warningFg,
        danger = danger,
        dangerBorder = dangerBorder,
        dangerForeground = dangerFg,
        backdrop = backdrop,
        tile = tile,
        mark = mark,
        onAvatar = onAvatar,
        isDark = true,
    )
}

val LightColors = with(OtTokens.Light) {
    OtColors(
        screen = screen,
        card = card,
        groupedCard = grouped,
        rowHover = hover,
        foreground = fg,
        muted = muted,
        tertiary = tertiary,
        border = border,
        inputBorder = inputBorder,
        primary = primary,
        onPrimary = onPrimary,
        primaryText = primaryText,
        bubbleOut = bubbleOut,
        success = success,
        warning = warning,
        warningBorder = warningBorder,
        warningForeground = warningFg,
        danger = danger,
        dangerBorder = dangerBorder,
        dangerForeground = dangerFg,
        backdrop = backdrop,
        tile = tile,
        mark = mark,
        onAvatar = onAvatar,
        isDark = false,
    )
}

val LocalOtColors = staticCompositionLocalOf { DarkColors }
