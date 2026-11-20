package dev.oldtown.android.ui.components

import androidx.compose.foundation.Image
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.PathFillType
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.addPathNodes
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import dev.oldtown.android.ui.theme.Ot

// Signe « archivolte » et logotype d'OldTown, repris tels quels de `branding/`
// (`archivolte-signe.svg`, `archivolte-petit.svg`, `logotype-mono.svg`), comme
// `brand.ts` de l'extension ; `BrandTest` le vérifie. Le signe est toujours en
// pierre (`mark`), le nom dans la couleur du texte.

/** Boîte serrée du signe et du logotype, sur la grille de 128. */
private const val BOX_X = 18f
private const val BOX_Y = 21.66f
private const val MARK_W = 92f
private const val LOGO_W = 360.82f
private const val BOX_H = 96.34f

/** Sous cette hauteur, la taille optique (charte, § 3). */
private val OPTICAL_BELOW = 24.dp

/** Signe complet (24 dp et plus). */
const val MARK_PATH =
    "M18 118V80A60 60 0 0 1 64 21.66A60 60 0 0 1 110 80V118zM26 118V80A52 52 0 0 1 64 29.92" +
        "A52 52 0 0 1 102 80V118zM32 118V80A46 46 0 0 1 64 36.18A46 46 0 0 1 96 80V118zM40 110V86" +
        "A21 21 0 0 1 50.5 67.81A21 21 0 0 1 61 86V110zM67 110V86A21 21 0 0 1 77.5 67.81" +
        "A21 21 0 0 1 88 86V110z"

/** Taille optique (sous 24 dp) : archivolte épaissie, noyau plein. */
const val MARK_SMALL_PATH =
    "M18 118V80A60 60 0 0 1 64 21.66A60 60 0 0 1 110 80V118zM32 118V80A46 46 0 0 1 64 36.18" +
        "A46 46 0 0 1 96 80V118zM40 118V80A38 38 0 0 1 64 44.67A38 38 0 0 1 88 80V118z"

/** « OldTown » en DM Sans demi-gras, vectorisé. */
const val WORDMARK_PATH =
    "M158.46 98.53Q151.81 98.53 146.84 95.65Q141.87 92.77 139.09 87.61" +
        "Q136.3 82.45 136.3 75.61Q136.3 68.84 139.09 63.68Q141.87 58.52 146.84 55.6" +
        "Q151.81 52.69 158.46 52.69Q165.05 52.69 170.02 55.6Q174.99 58.52 177.77 63.68" +
        "Q180.56 68.84 180.56 75.61Q180.56 82.45 177.77 87.61Q174.99 92.77 170.02 95.65" +
        "Q165.05 98.53 158.46 98.53ZM158.46 91.69Q162.77 91.69 166 89.76" +
        "Q169.22 87.83 171.03 84.22Q172.83 80.61 172.83 75.61Q172.83 70.61 171.03 67" +
        "Q169.22 63.39 166 61.46Q162.77 59.53 158.46 59.53Q154.09 59.53 150.83 61.46" +
        "Q147.57 63.39 145.8 67Q144.03 70.61 144.03 75.61Q144.03 80.61 145.8 84.22" +
        "Q147.57 87.83 150.83 89.76Q154.09 91.69 158.46 91.69ZM185 97.77L185 53.45L192.59 53.45" +
        "L192.59 97.77ZM211.08 98.53Q206.65 98.53 203.32 96.38Q200 94.22 198.2 90.52" +
        "Q196.39 86.82 196.39 82.07Q196.39 77.38 198.23 73.62Q200.06 69.85 203.45 67.73" +
        "Q206.84 65.61 211.52 65.61Q215.38 65.61 218.14 67.13Q220.89 68.65 222.41 70.74" +
        "L222.41 53.45L230.01 53.45L230.01 97.77L223.62 97.77L223.17 92.07L222.92 92.07" +
        "Q221.72 94.16 219.94 95.59Q218.17 97.01 215.95 97.77Q213.74 98.53 211.08 98.53Z" +
        "M213.3 92.26Q216.08 92.26 218.17 90.93Q220.26 89.6 221.37 87.32" +
        "Q222.48 85.04 222.48 82.07Q222.48 79.03 221.37 76.75Q220.26 74.47 218.14 73.17" +
        "Q216.02 71.88 213.3 71.88Q210.51 71.88 208.42 73.17Q206.33 74.47 205.22 76.75" +
        "Q204.12 79.03 204.12 82.01Q204.12 85.11 205.22 87.36Q206.33 89.6 208.39 90.93" +
        "Q210.45 92.26 213.3 92.26ZM245.6 97.77L245.6 59.59L232.56 59.59L232.56 53.45" +
        "L266.18 53.45L266.18 59.59L253.14 59.59L253.14 97.77ZM279.95 98.53" +
        "Q275.46 98.53 271.82 96.47Q268.18 94.41 266.09 90.68Q264 86.94 264 82.07" +
        "Q264 77.19 266.12 73.46Q268.24 69.72 271.88 67.67Q275.52 65.61 280.02 65.61" +
        "Q284.51 65.61 288.15 67.67Q291.79 69.72 293.91 73.46Q296.03 77.19 296.03 82.07" +
        "Q296.03 86.94 293.88 90.68Q291.73 94.41 288.09 96.47Q284.45 98.53 279.95 98.53Z" +
        "M279.95 92.26Q282.23 92.26 284.1 91.12Q285.97 89.98 287.11 87.74" +
        "Q288.25 85.49 288.25 82.07Q288.25 78.65 287.14 76.37Q286.03 74.09 284.13 72.98" +
        "Q282.23 71.88 280.08 71.88Q277.86 71.88 275.96 72.98Q274.06 74.09 272.89 76.37" +
        "Q271.72 78.65 271.72 82.07Q271.72 85.49 272.89 87.74Q274.06 89.98 275.93 91.12" +
        "Q277.8 92.26 279.95 92.26ZM304.4 97.77L296.17 66.37L303.7 66.37L309.34 91.95" +
        "L309.59 91.95L315.67 66.37L324.53 66.37L330.8 91.95L331.06 91.95L336.56 66.37" +
        "L344.1 66.37L335.74 97.77L326.56 97.77L320.29 72.19L320.04 72.19L313.52 97.77Z" +
        "M346.33 97.77L346.33 66.37L352.85 66.37L353.55 71.43L353.8 71.43" +
        "Q355.32 68.71 357.95 67.16Q360.58 65.61 364.44 65.61Q368.24 65.61 370.96 67.22" +
        "Q373.68 68.84 375.14 72Q376.6 75.17 376.6 79.92L376.6 97.77L369.06 97.77L369.06 80.68" +
        "Q369.06 76.5 367.32 74.25Q365.58 72 361.97 72Q359.69 72 357.86 73.14" +
        "Q356.02 74.28 354.98 76.37Q353.93 78.46 353.93 81.5L353.93 97.77Z"

/**
 * Tracés à la grille de 128, ramenés à l'origine de leur boîte : le signe en
 * pair-impair (`fill-rule="evenodd"` des SVG), le nom en non nul.
 */
private fun vector(name: String, width: Float, height: Dp, mark: String, markColor: Color, word: Color?): ImageVector =
    ImageVector.Builder(name, height * (width / BOX_H), height, width, BOX_H).apply {
        addGroup(translationX = -BOX_X, translationY = -BOX_Y)
        addPath(pathData = addPathNodes(mark), pathFillType = PathFillType.EvenOdd, fill = SolidColor(markColor))
        if (word != null) addPath(pathData = addPathNodes(WORDMARK_PATH), fill = SolidColor(word))
        clearGroup()
    }.build()

/** Le signe seul, décoratif ; taille optique sous 24 dp de haut. */
@Composable
fun Mark(height: Dp, modifier: Modifier = Modifier, color: Color = Ot.colors.mark) {
    val path = if (height < OPTICAL_BELOW) MARK_SMALL_PATH else MARK_PATH
    val image = remember(path, height, color) { vector("mark", MARK_W, height, path, color, null) }
    Image(image, contentDescription = null, modifier = modifier.size(height * (MARK_W / BOX_H), height))
}

/** Signe et nom : en-tête de l'accueil, premier écran. */
@Composable
fun Logo(height: Dp, modifier: Modifier = Modifier) {
    val c = Ot.colors
    val mark = if (height < OPTICAL_BELOW) MARK_SMALL_PATH else MARK_PATH
    val image = remember(mark, height, c.mark, c.foreground) {
        vector("logo", LOGO_W, height, mark, c.mark, c.foreground)
    }
    Image(image, contentDescription = "OldTown", modifier = modifier.size(height * (LOGO_W / BOX_H), height))
}
