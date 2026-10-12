package dev.oldtown.android.ui.pairing

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.unit.dp
import dev.oldtown.android.ui.theme.OtShapes
import dev.oldtown.core.OtException
import dev.oldtown.core.qrCode

/** QR code de [text], modules calculés par le cœur Rust, noir sur blanc. */
@Composable
fun QrImage(text: String, modifier: Modifier = Modifier) {
    val qr = remember(text) {
        try {
            qrCode(text)
        } catch (_: OtException) {
            null
        }
    } ?: return
    Box(
        modifier
            .fillMaxWidth()
            .aspectRatio(1f)
            .clip(OtShapes.card)
            .background(Color.White)
            .padding(16.dp),
    ) {
        Canvas(Modifier.fillMaxWidth().aspectRatio(1f)) {
            val n = qr.size.toInt()
            val cell = size.minDimension / n
            for (y in 0 until n) {
                for (x in 0 until n) {
                    if (qr.modules[y * n + x]) {
                        // Un léger recouvrement évite les lignes claires entre modules.
                        drawRect(Color.Black, Offset(x * cell, y * cell), Size(cell + 0.5f, cell + 0.5f))
                    }
                }
            }
        }
    }
}
