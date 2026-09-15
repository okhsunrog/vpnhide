package dev.okhsunrog.vpnhide.help

import android.content.Context
import android.graphics.BitmapFactory
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.gestures.detectTransformGestures
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import dev.okhsunrog.vpnhide.LogTags
import dev.okhsunrog.vpnhide.VpnHideLog
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import java.io.IOException

/**
 * Where an article's images live, so relative Markdown image paths resolve to APK
 * assets. [baseDir] is the article's asset directory (e.g. `help/en`); a
 * destination like `../images/foo.png` resolves against it. Provided by
 * [MarkdownText] so the block renderer can load images without threading the
 * context/path through every node function.
 */
internal data class HelpImageEnv(
    val context: Context,
    val baseDir: String,
)

internal val LocalHelpImageEnv = staticCompositionLocalOf<HelpImageEnv?> { null }

/**
 * Resolve a Markdown image destination to an APK asset path, relative to
 * [baseDir]. Handles `./` and `../` segments. Returns null for remote/absolute
 * URLs (the CSP-free offline guide ships its images as assets) so the caller can
 * fall back to showing the alt text.
 */
internal fun resolveHelpAsset(
    baseDir: String,
    destination: String,
): String? {
    val dest = destination.trim().substringBefore('#').substringBefore('?')
    if (dest.isEmpty()) return null
    if (dest.startsWith("http://") || dest.startsWith("https://") || dest.startsWith("//")) return null
    val start = if (dest.startsWith("/")) mutableListOf() else baseDir.split('/').toMutableList()
    val segments = start.filter { it.isNotEmpty() }.toMutableList()
    for (part in dest.split('/')) {
        when (part) {
            "", "." -> Unit
            ".." -> if (segments.isNotEmpty()) segments.removeAt(segments.size - 1)
            else -> segments.add(part)
        }
    }
    return segments.joinToString("/").ifEmpty { null }
}

/**
 * A block-level article image: the picture scaled to the content width with its
 * alt text as a caption underneath, tappable to open a full-screen zoomable view.
 * Falls back to the caption text alone when the asset can't be decoded.
 */
@Composable
internal fun HelpBlockImage(
    destination: String,
    caption: String,
) {
    val env = LocalHelpImageEnv.current
    val assetPath = env?.let { resolveHelpAsset(it.baseDir, destination) }
    val bitmap =
        if (env != null && assetPath != null) rememberAssetImage(env.context, assetPath) else null
    var zoomed by remember { mutableStateOf(false) }

    if (bitmap == null) {
        if (caption.isNotBlank()) {
            Text(
                text = caption,
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                textAlign = TextAlign.Center,
                modifier = Modifier.fillMaxWidth(),
            )
        }
        return
    }

    Column(
        modifier = Modifier.fillMaxWidth(),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(4.dp),
    ) {
        Image(
            bitmap = bitmap,
            contentDescription = caption.ifBlank { null },
            contentScale = ContentScale.FillWidth,
            modifier =
                Modifier
                    .fillMaxWidth()
                    .clip(RoundedCornerShape(10.dp))
                    .clickable(
                        interactionSource = remember { MutableInteractionSource() },
                        indication = null,
                    ) { zoomed = true },
        )
        if (caption.isNotBlank()) {
            Text(
                text = caption,
                style = MaterialTheme.typography.labelMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                textAlign = TextAlign.Center,
            )
        }
    }

    if (zoomed) {
        ZoomableImageDialog(bitmap, caption) { zoomed = false }
    }
}

/**
 * Full-screen, pinch-to-zoom / drag-to-pan view of one image. Tap the backdrop or
 * press back to close; double-tap toggles between fit and 2.5x.
 */
@Composable
private fun ZoomableImageDialog(
    bitmap: ImageBitmap,
    caption: String,
    onDismiss: () -> Unit,
) {
    Dialog(onDismissRequest = onDismiss, properties = DialogProperties(usePlatformDefaultWidth = false)) {
        var scale by remember { mutableFloatStateOf(1f) }
        var offsetX by remember { mutableFloatStateOf(0f) }
        var offsetY by remember { mutableFloatStateOf(0f) }
        val animatedScale by animateFloatAsState(scale, label = "help-zoom")

        Box(
            modifier =
                Modifier
                    .fillMaxSize()
                    .background(Color.Black.copy(alpha = 0.92f))
                    .pointerInput(Unit) { detectTapGestures(onTap = { onDismiss() }) },
            contentAlignment = Alignment.Center,
        ) {
            Image(
                bitmap = bitmap,
                contentDescription = caption.ifBlank { null },
                contentScale = ContentScale.Fit,
                modifier =
                    Modifier
                        .fillMaxSize()
                        .graphicsLayer(
                            scaleX = animatedScale,
                            scaleY = animatedScale,
                            translationX = offsetX,
                            translationY = offsetY,
                        ).pointerInput(Unit) {
                            detectTapGestures(
                                onTap = { onDismiss() },
                                onDoubleTap = {
                                    if (scale > 1f) {
                                        scale = 1f
                                        offsetX = 0f
                                        offsetY = 0f
                                    } else {
                                        scale = 2.5f
                                    }
                                },
                            )
                        }.pointerInput(Unit) {
                            detectTransformGestures { _, pan, zoom, _ ->
                                scale = (scale * zoom).coerceIn(1f, 5f)
                                if (scale > 1f) {
                                    offsetX += pan.x
                                    offsetY += pan.y
                                } else {
                                    offsetX = 0f
                                    offsetY = 0f
                                }
                            }
                        },
            )
        }
    }
}

/** Decode an asset image off the main thread; null while loading or on failure. */
@Composable
private fun rememberAssetImage(
    context: Context,
    assetPath: String,
): ImageBitmap? {
    val value by produceState<ImageBitmap?>(initialValue = null, assetPath) {
        value =
            withContext(Dispatchers.IO) {
                try {
                    context.assets
                        .open(assetPath)
                        .use { BitmapFactory.decodeStream(it) }
                        ?.asImageBitmap()
                } catch (e: IOException) {
                    VpnHideLog.w(LogTags.STARTUP, "help image asset unavailable: $assetPath (${e.message})")
                    null
                }
            }
    }
    return value
}
