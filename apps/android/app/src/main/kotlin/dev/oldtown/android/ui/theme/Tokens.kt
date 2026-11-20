// Fichier généré par tools/design-tokens/gen.mjs depuis design/tokens.json : ne pas modifier.
package dev.oldtown.android.ui.theme

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp

/**
 * Jetons du système de design (`design/tokens.json`), mêmes valeurs que
 * l'extension. `Color.kt` et `Theme.kt` les liront à la place de leurs
 * valeurs (lot G, PR D2).
 */
object OtTokens {
    /** Rôles du thème sombre « nuit ». */
    object Dark {
        val screen = Color(0xFF0F1120)
        val card = Color(0xFF161A2E)
        val grouped = Color(0xFF1D2238)
        val hover = Color(0xFF1D2238)
        val fg = Color(0xFFEEECF3)
        val muted = Color(0xFF8A90AB)
        val tertiary = Color(0xFF8A90AB)
        val border = Color(0xFF232843)
        val inputBorder = Color(0xFF2A3050)
        val primary = Color(0xFFD27443)
        val onPrimary = Color(0xFF0F1120)
        val primaryText = Color(0xFFE39C75)
        val bubbleOut = Color(0xFF2A2F4F)
        val success = Color(0xFF34D399)
        val warning = Color(0xFF312108)
        val warningBorder = Color(0x52FE9A00)
        val warningFg = Color(0xFFFFB900)
        val danger = Color(0xFF301214)
        val dangerBorder = Color(0x52FB414A)
        val dangerFg = Color(0xFFFF6467)
        val backdrop = Color(0x7A0F1120)
        val tile = Color(0xFF232841)
        val mark = Color(0xFFD27443)
        val onAvatar = Color(0xFFFFFFFF)
    }

    /** Rôles du thème clair « parchemin ». */
    object Light {
        val screen = Color(0xFFFAF7F2)
        val card = Color(0xFFFFFFFF)
        val grouped = Color(0xFFF3EEE6)
        val hover = Color(0xFFF3EEE6)
        val fg = Color(0xFF232841)
        val muted = Color(0xFF6B6577)
        val tertiary = Color(0xFF6B6577)
        val border = Color(0xFFE8E1D6)
        val inputBorder = Color(0xFFDCD3C6)
        val primary = Color(0xFFB4552A)
        val onPrimary = Color(0xFFFFFFFF)
        val primaryText = Color(0xFFA2492A)
        val bubbleOut = Color(0xFFF6E3D6)
        val success = Color(0xFF059669)
        val warning = Color(0xFFFCF4E8)
        val warningBorder = Color(0x52FE9A00)
        val warningFg = Color(0xFFBB4D00)
        val danger = Color(0xFFFCEBEC)
        val dangerBorder = Color(0x52FB2C36)
        val dangerFg = Color(0xFFC10007)
        val backdrop = Color(0x380F1120)
        val tile = Color(0xFF232841)
        val mark = Color(0xFFD27443)
        val onAvatar = Color(0xFFFFFFFF)
    }

    /** Couleurs des avatars (ARGB), dans l'ordre : les mêmes que l'extension. */
    val avatarColors = longArrayOf(
        0xFFC4534F,
        0xFFC77A1C,
        0xFFA68A22,
        0xFF3E8F62,
        0xFF2A8780,
        0xFF3D74AD,
        0xFF6464C8,
        0xFF9058B0,
        0xFFBE4F75,
        0xFF5A7488,
    )

    /** Arrondis : contrôles, cartes, feuilles, pastilles. */
    object Radius {
        val control = 10.dp
        val card = 14.dp
        val sheet = 20.dp
        val pill = 999.dp
    }

    /** Espacements, au pas de 4. */
    object Space {
        val xs = 4.dp
        val sm = 8.dp
        val md = 12.dp
        val lg = 16.dp
        val xl = 24.dp
        val xxl = 32.dp
    }

    /** Échelle typographique. */
    object TextSize {
        val label = 11.sp
        val small = 13.sp
        val popup = 14.sp
        val body = 15.sp
        val title = 17.sp
        val large = 20.sp
        val headline = 24.sp
    }
}
