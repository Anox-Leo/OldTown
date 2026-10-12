package dev.oldtown.android.ui.theme

import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Shapes
import androidx.compose.material3.Typography
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.ReadOnlyComposable
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.Font
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontVariation
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import dev.oldtown.android.R

/** DM Sans (police de T3 Code), fonte variable : une graisse par entrée. */
private fun dmSans(weight: FontWeight) = Font(
    R.font.dm_sans,
    weight = weight,
    variationSettings = FontVariation.Settings(FontVariation.weight(weight.weight)),
)

val DmSans = FontFamily(
    dmSans(FontWeight.Normal),
    dmSans(FontWeight.Medium),
    dmSans(FontWeight.SemiBold),
    dmSans(FontWeight.Bold),
)

private fun style(size: Int, weight: FontWeight = FontWeight.Normal, line: Int = size + 6) =
    TextStyle(fontFamily = DmSans, fontWeight = weight, fontSize = size.sp, lineHeight = line.sp)

private val OtTypography = Typography(
    headlineSmall = style(24, FontWeight.SemiBold, 30),
    titleLarge = style(20, FontWeight.SemiBold, 26),
    titleMedium = style(17, FontWeight.Medium, 22),
    titleSmall = style(15, FontWeight.Medium, 20),
    bodyLarge = style(16, line = 23),
    bodyMedium = style(15, line = 21),
    bodySmall = style(13, line = 18),
    labelLarge = style(15, FontWeight.Medium, 20),
    labelMedium = style(13, FontWeight.Medium, 16),
    labelSmall = style(11, FontWeight.Medium, 14),
)

/** Arrondis : cartes 14 dp, champs et boutons 10 dp, pastilles 999. */
object OtShapes {
    val card = RoundedCornerShape(14.dp)
    val control = RoundedCornerShape(10.dp)
    val pill = RoundedCornerShape(999.dp)
    val sheet = RoundedCornerShape(topStart = 20.dp, topEnd = 20.dp)
}

/** Police à chasse fixe des empreintes et des codes. */
val Mono = FontFamily.Monospace

@Composable
fun OldTownTheme(dark: Boolean = isSystemInDarkTheme(), content: @Composable () -> Unit) {
    val c = if (dark) DarkColors else LightColors
    val scheme = if (dark) {
        darkColorScheme(
            primary = c.primary, onPrimary = c.onPrimary,
            background = c.screen, onBackground = c.foreground,
            surface = c.card, onSurface = c.foreground,
            surfaceVariant = c.groupedCard, onSurfaceVariant = c.muted,
            surfaceContainerLowest = c.screen, surfaceContainerLow = c.card,
            surfaceContainer = c.card, surfaceContainerHigh = c.groupedCard,
            surfaceContainerHighest = c.groupedCard,
            outline = c.inputBorder, outlineVariant = c.border,
            error = c.dangerForeground, onError = c.onPrimary,
            scrim = c.backdrop,
        )
    } else {
        lightColorScheme(
            primary = c.primary, onPrimary = c.onPrimary,
            background = c.screen, onBackground = c.foreground,
            surface = c.card, onSurface = c.foreground,
            surfaceVariant = c.groupedCard, onSurfaceVariant = c.muted,
            surfaceContainerLowest = c.card, surfaceContainerLow = c.screen,
            surfaceContainer = c.card, surfaceContainerHigh = c.groupedCard,
            surfaceContainerHighest = c.groupedCard,
            outline = c.inputBorder, outlineVariant = c.border,
            error = c.dangerForeground, onError = c.onPrimary,
            scrim = c.backdrop,
        )
    }
    CompositionLocalProvider(LocalOtColors provides c) {
        MaterialTheme(
            colorScheme = scheme,
            typography = OtTypography,
            shapes = Shapes(small = OtShapes.control, medium = OtShapes.card, large = OtShapes.card),
            content = content,
        )
    }
}

/** Accès court à la palette : `Ot.colors.muted`. */
object Ot {
    val colors: OtColors
        @Composable @ReadOnlyComposable get() = LocalOtColors.current
}
