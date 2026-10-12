package dev.oldtown.android.ui.components

import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import dev.oldtown.android.ui.theme.Mono
import dev.oldtown.android.ui.theme.Ot
import dev.oldtown.android.ui.theme.OtShapes

/** Barre du haut : retour éventuel, titre, actions à droite. */
@Composable
fun TopBar(
    title: String,
    onBack: (() -> Unit)? = null,
    actions: @Composable RowScope.() -> Unit = {},
) {
    Row(
        Modifier.fillMaxWidth().height(56.dp).padding(horizontal = 8.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        if (onBack != null) {
            GhostIcon(OtIcons.Back, "Retour", onBack)
        } else {
            Spacer(Modifier.width(8.dp))
        }
        Text(
            title,
            style = MaterialTheme.typography.titleMedium,
            color = Ot.colors.foreground,
            modifier = Modifier.weight(1f).padding(horizontal = 8.dp),
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
        )
        actions()
    }
}

/** Bouton icône sans fond, 40 dp. */
@Composable
fun GhostIcon(
    icon: ImageVector,
    description: String,
    onClick: () -> Unit,
    tint: Color = Ot.colors.foreground,
    enabled: Boolean = true,
) {
    Box(
        Modifier
            .size(40.dp)
            .clip(OtShapes.control)
            .clickable(enabled = enabled, role = Role.Button, onClick = onClick)
            .alpha(if (enabled) 1f else 0.4f),
        contentAlignment = Alignment.Center,
    ) {
        Icon(icon, contentDescription = description, tint = tint, modifier = Modifier.size(20.dp))
    }
}

/** Carte : fond à peine plus clair que l'écran, bordure fine. */
@Composable
fun OtCard(
    modifier: Modifier = Modifier,
    padding: PaddingValues = PaddingValues(16.dp),
    content: @Composable ColumnScope.() -> Unit,
) {
    Surface(
        modifier = modifier.fillMaxWidth(),
        shape = OtShapes.card,
        color = Ot.colors.card,
        border = BorderStroke(1.dp, Ot.colors.border),
    ) {
        Column(Modifier.padding(padding), content = content)
    }
}

/** Petit titre de section, discret. */
@Composable
fun SectionLabel(text: String, modifier: Modifier = Modifier) {
    Text(
        text,
        style = MaterialTheme.typography.labelMedium,
        color = Ot.colors.muted,
        modifier = modifier.padding(start = 4.dp, bottom = 8.dp, top = 4.dp),
    )
}

enum class ButtonKind { Primary, Secondary, Danger }

@Composable
fun OtButton(
    text: String,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    kind: ButtonKind = ButtonKind.Primary,
    icon: ImageVector? = null,
    enabled: Boolean = true,
    loading: Boolean = false,
) {
    val c = Ot.colors
    val (bg, fg, border) = when (kind) {
        ButtonKind.Primary -> Triple(c.primary, c.onPrimary, c.primary)
        ButtonKind.Secondary -> Triple(c.card, c.foreground, c.inputBorder)
        ButtonKind.Danger -> Triple(c.danger, c.dangerForeground, c.dangerBorder)
    }
    val active = enabled && !loading
    // Bouton désactivé : couleurs atténuées, sans modificateur `alpha`.
    val dim = if (enabled) 1f else 0.4f
    val bgColor = bg.copy(alpha = bg.alpha * dim)
    val fgColor = fg.copy(alpha = fg.alpha * dim)
    Row(
        modifier
            .height(44.dp)
            .clip(OtShapes.control)
            .background(bgColor, OtShapes.control)
            .border(1.dp, border.copy(alpha = border.alpha * dim), OtShapes.control)
            .clickable(enabled = active, role = Role.Button, onClick = onClick)
            .padding(horizontal = 16.dp),
        horizontalArrangement = Arrangement.Center,
        verticalAlignment = Alignment.CenterVertically,
    ) {
        if (loading) {
            CircularProgressIndicator(Modifier.size(16.dp), color = fgColor, strokeWidth = 2.dp)
            Spacer(Modifier.width(10.dp))
        } else if (icon != null) {
            Icon(icon, contentDescription = null, tint = fgColor, modifier = Modifier.size(18.dp))
            Spacer(Modifier.width(8.dp))
        }
        Text(text, style = MaterialTheme.typography.labelLarge, color = fgColor, maxLines = 1)
    }
}

/** Ligne de liste : icône, titre, sous-titre, élément à droite. */
@Composable
fun ListRow(
    title: String,
    modifier: Modifier = Modifier,
    subtitle: String? = null,
    leading: ImageVector? = null,
    subtitleMono: Boolean = false,
    onClick: (() -> Unit)? = null,
    trailing: @Composable (() -> Unit)? = null,
) {
    Row(
        modifier
            .fillMaxWidth()
            .then(if (onClick != null) Modifier.clickable(onClick = onClick) else Modifier)
            .padding(horizontal = 16.dp, vertical = 12.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        if (leading != null) {
            Icon(leading, contentDescription = null, tint = Ot.colors.muted, modifier = Modifier.size(20.dp))
            Spacer(Modifier.width(14.dp))
        }
        Column(Modifier.weight(1f)) {
            Text(title, style = MaterialTheme.typography.bodyMedium, color = Ot.colors.foreground)
            if (subtitle != null) {
                Text(
                    subtitle,
                    style = MaterialTheme.typography.bodySmall.let {
                        if (subtitleMono) it.copy(fontFamily = Mono) else it
                    },
                    color = Ot.colors.muted,
                    maxLines = 2,
                    overflow = TextOverflow.Ellipsis,
                )
            }
        }
        if (trailing != null) {
            Spacer(Modifier.width(12.dp))
            trailing()
        }
    }
}

@Composable
fun Divider() = HorizontalDivider(color = Ot.colors.border, thickness = 1.dp)

/** Pastille de couleur (état de connexion, présence). */
@Composable
fun Dot(color: Color, size: Int = 8) {
    Box(Modifier.size(size.dp).clip(CircleShape).background(color))
}

/** Pastille d'état : point coloré et libellé. */
@Composable
fun StatusPill(label: String, color: Color, pulsing: Boolean = false) {
    val alpha by animateFloatAsState(if (pulsing) 0.5f else 1f, label = "status")
    Row(
        Modifier
            .clip(OtShapes.pill)
            .background(Ot.colors.groupedCard)
            .padding(horizontal = 10.dp, vertical = 5.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Box(Modifier.alpha(alpha)) { Dot(color, 7) }
        Spacer(Modifier.width(7.dp))
        Text(label, style = MaterialTheme.typography.labelMedium, color = Ot.colors.foreground)
    }
}

enum class BannerKind { Info, Warning, Danger }

/** Bandeau d'information ou d'alerte, avec une action facultative. */
@Composable
fun Banner(
    text: String,
    kind: BannerKind = BannerKind.Info,
    action: String? = null,
    onAction: (() -> Unit)? = null,
) {
    val c = Ot.colors
    val (bg, border, fg) = when (kind) {
        BannerKind.Info -> Triple(c.groupedCard, c.border, c.foreground)
        BannerKind.Warning -> Triple(c.warning, c.warningBorder, c.warningForeground)
        BannerKind.Danger -> Triple(c.danger, c.dangerBorder, c.dangerForeground)
    }
    Row(
        Modifier
            .fillMaxWidth()
            .clip(OtShapes.control)
            .background(bg)
            .border(1.dp, border, OtShapes.control)
            .padding(horizontal = 14.dp, vertical = 12.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        if (kind != BannerKind.Info) {
            Icon(OtIcons.Alert, contentDescription = null, tint = fg, modifier = Modifier.size(18.dp))
            Spacer(Modifier.width(10.dp))
        }
        Text(text, style = MaterialTheme.typography.bodySmall, color = fg, modifier = Modifier.weight(1f))
        if (action != null && onAction != null) {
            Spacer(Modifier.width(10.dp))
            Text(
                action,
                style = MaterialTheme.typography.labelMedium.copy(fontWeight = FontWeight.SemiBold),
                color = if (kind == BannerKind.Info) c.primaryText else fg,
                modifier = Modifier.clip(OtShapes.control).clickable(onClick = onAction).padding(6.dp),
            )
        }
    }
}

/** Champ de saisie à la manière de T3 Code : fond d'entrée, bordure fine. */
@Composable
fun OtTextField(
    value: String,
    onValueChange: (String) -> Unit,
    placeholder: String,
    modifier: Modifier = Modifier,
    singleLine: Boolean = true,
    mono: Boolean = false,
    minLines: Int = 1,
    keyboardOptions: KeyboardOptions = KeyboardOptions.Default,
    keyboardActions: KeyboardActions = KeyboardActions.Default,
) {
    val c = Ot.colors
    val style: TextStyle = MaterialTheme.typography.bodyMedium.copy(
        color = c.foreground,
        fontFamily = if (mono) Mono else MaterialTheme.typography.bodyMedium.fontFamily,
    )
    BasicTextField(
        value = value,
        onValueChange = onValueChange,
        modifier = modifier.fillMaxWidth(),
        textStyle = style,
        singleLine = singleLine,
        minLines = minLines,
        cursorBrush = SolidColor(c.primary),
        keyboardOptions = keyboardOptions,
        keyboardActions = keyboardActions,
        decorationBox = { inner ->
            Box(
                Modifier
                    .fillMaxWidth()
                    .clip(OtShapes.control)
                    .background(c.screen)
                    .border(1.dp, c.inputBorder, OtShapes.control)
                    .padding(horizontal = 12.dp, vertical = 11.dp),
            ) {
                if (value.isEmpty()) Text(placeholder, style = style.copy(color = c.muted))
                inner()
            }
        },
    )
}
