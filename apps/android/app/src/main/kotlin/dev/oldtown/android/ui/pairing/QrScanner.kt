package dev.oldtown.android.ui.pairing

import android.Manifest
import android.content.pm.PackageManager
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.camera.core.CameraSelector
import androidx.camera.core.ImageAnalysis
import androidx.camera.core.Preview
import androidx.camera.lifecycle.ProcessCameraProvider
import androidx.camera.view.PreviewView
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.core.content.ContextCompat
import androidx.lifecycle.compose.LocalLifecycleOwner
import dev.oldtown.android.ui.components.OtButton
import dev.oldtown.android.ui.components.ButtonKind
import dev.oldtown.android.ui.theme.Ot
import dev.oldtown.android.ui.theme.OtShapes
import java.util.concurrent.Executors
import java.util.concurrent.atomic.AtomicBoolean
import zxingcpp.BarcodeReader

/**
 * Lecture d'un QR par la caméra : CameraX et zxing-cpp (Apache 2.0, sans
 * Google Play Services, note 0016). Appelle [onResult] une seule fois.
 */
@Composable
fun QrScanner(onResult: (String) -> Unit, modifier: Modifier = Modifier) {
    val context = LocalContext.current
    var granted by remember {
        mutableStateOf(
            ContextCompat.checkSelfPermission(context, Manifest.permission.CAMERA) ==
                PackageManager.PERMISSION_GRANTED,
        )
    }
    var asked by remember { mutableStateOf(false) }
    val launcher = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) {
        granted = it
        asked = true
    }
    LaunchedEffect(Unit) { if (!granted) launcher.launch(Manifest.permission.CAMERA) }

    Box(modifier.clip(OtShapes.card), contentAlignment = Alignment.Center) {
        if (granted) {
            CameraPreview(onResult)
            Frame()
        } else {
            Box(Modifier.fillMaxSize().padding(24.dp), contentAlignment = Alignment.Center) {
                if (asked) {
                    Text(
                        "Accès à la caméra refusé : saisir le code à la main, ou l'autoriser dans les réglages du téléphone.",
                        style = MaterialTheme.typography.bodySmall,
                        color = Ot.colors.muted,
                        textAlign = TextAlign.Center,
                    )
                } else {
                    OtButton(
                        "Autoriser la caméra",
                        onClick = { launcher.launch(Manifest.permission.CAMERA) },
                        kind = ButtonKind.Secondary,
                    )
                }
            }
        }
    }
}

@Composable
private fun CameraPreview(onResult: (String) -> Unit) {
    val context = LocalContext.current
    val owner = LocalLifecycleOwner.current
    val callback by rememberUpdatedState(onResult)
    val view = remember { PreviewView(context).apply { scaleType = PreviewView.ScaleType.FILL_CENTER } }

    DisposableEffect(owner) {
        val executor = Executors.newSingleThreadExecutor()
        val done = AtomicBoolean(false)
        val reader = BarcodeReader(
            BarcodeReader.Options(
                formats = setOf(BarcodeReader.Format.QR_CODE),
                tryHarder = true,
                // `ot pair … --qr` dessine en clair sur fond sombre.
                tryInvert = true,
            ),
        )
        val analysis = ImageAnalysis.Builder()
            .setBackpressureStrategy(ImageAnalysis.STRATEGY_KEEP_ONLY_LATEST)
            .build()
        analysis.setAnalyzer(executor) { image ->
            image.use {
                val text = reader.read(it).firstNotNullOfOrNull { r -> r.text?.takeIf(String::isNotBlank) }
                if (text != null && done.compareAndSet(false, true)) {
                    ContextCompat.getMainExecutor(context).execute { callback(text) }
                }
            }
        }
        val preview = Preview.Builder().build().also { it.surfaceProvider = view.surfaceProvider }
        val future = ProcessCameraProvider.getInstance(context)
        future.addListener({
            val provider = future.get()
            provider.unbindAll()
            provider.bindToLifecycle(owner, CameraSelector.DEFAULT_BACK_CAMERA, preview, analysis)
        }, ContextCompat.getMainExecutor(context))
        onDispose {
            if (future.isDone) future.get().unbindAll()
            analysis.clearAnalyzer()
            executor.shutdown()
        }
    }
    AndroidView({ view }, Modifier.fillMaxSize())
}

/** Cadre de visée : quatre coins arrondis. */
@Composable
private fun Frame() {
    val accent = Color.White
    Canvas(Modifier.fillMaxSize()) {
        val side = size.minDimension * 0.62f
        val topLeft = Offset((size.width - side) / 2, (size.height - side) / 2)
        drawRoundRect(
            color = accent.copy(alpha = 0.85f),
            topLeft = topLeft,
            size = Size(side, side),
            cornerRadius = CornerRadius(28f, 28f),
            style = Stroke(width = 4f),
        )
    }
}
