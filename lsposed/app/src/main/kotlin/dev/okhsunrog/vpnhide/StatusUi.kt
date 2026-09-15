package dev.okhsunrog.vpnhide

import android.content.Context
import android.content.Intent
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Save
import androidx.compose.material.icons.filled.Share
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.unit.dp
import androidx.core.content.FileProvider
import dev.okhsunrog.vpnhide.ui.components.EnhancedButton
import dev.okhsunrog.vpnhide.ui.components.EnhancedOutlinedButton
import dev.okhsunrog.vpnhide.ui.components.container
import dev.okhsunrog.vpnhide.ui.theme.LocalDarkTheme
import java.io.File

/**
 * Centralised status palette.
 *
 * Material You remixes `colorScheme.errorContainer` / `tertiaryContainer`
 * toward the wallpaper, which on real devices landed on "lavender" / "pink"
 * that read as a note, not a problem. So every status card and banner pins
 * these hand-picked container + accent colors instead of the theme's.
 * Defined once here rather than copy-pasted into each card.
 */
internal object StatusColors {
    @Composable
    private fun container(
        darkArgb: Long,
        darkAlpha: Float,
        lightArgb: Long,
    ): Color = if (LocalDarkTheme.current) Color(darkArgb).copy(alpha = darkAlpha) else Color(lightArgb)

    @Composable fun successContainer() = container(0xFF0A4A43, 0.34f, 0xFFE4F7F1)

    @Composable fun warningContainer() = container(0xFF7A4B00, 0.3f, 0xFFFFF3D8)

    @Composable fun errorContainer() = container(0xFF8C1D35, 0.34f, 0xFFFFE8ED)

    @Composable fun infoContainer() = container(0xFF124A73, 0.34f, 0xFFE6F3FF)

    @Composable fun neutralContainer() = container(0xFF334155, 0.28f, 0xFFF1F5F9)

    // Distinct from warning only in dark mode — the "install zygisk instead"
    // recommendation card uses a brown tint where warnings use orange.
    @Composable fun zygiskRecommendContainer() = container(0xFF4A3A2A, 0.34f, 0xFFFFF0DC)

    @Composable fun errorHeader() = if (LocalDarkTheme.current) Color(0xFFFFB3C0) else Color(0xFFC9184A)

    @Composable fun warningHeader() = if (LocalDarkTheme.current) Color(0xFFFFC56D) else Color(0xFFC96A00)

    @Composable fun neutralHeader() = if (LocalDarkTheme.current) Color(0xFFCBD5E1) else Color(0xFF475569)

    // Accent colors (status dots / status text / pass-fail badges). These are
    // fixed regardless of theme — they sit on the tinted containers above.
    val successDot = Color(0xFF0BAE7A)
    val successBadge = Color(0xFF087C61)
    val warningAccent = Color(0xFFE58A00)
    val errorDot = Color(0xFFD92D4B)
    val errorAccent = Color(0xFFC9184A)
    val infoAccent = Color(0xFF2E7CF6)
    val neutralAccent = Color(0xFF64748B)
}

/**
 * Flat colored banner card. Shared by the Dashboard protection / issues
 * banners and the Diagnostics status banners (it was a private duplicate in
 * both screens).
 */
@Composable
internal fun StatusBanner(
    text: String,
    containerColor: Color,
    contentColor: Color,
    modifier: Modifier = Modifier,
    action: (@Composable () -> Unit)? = null,
) {
    Box(
        modifier =
            modifier
                .fillMaxWidth()
                .container(
                    shape = MaterialTheme.shapes.medium,
                    color = containerColor,
                    drawBorder = false,
                    shadowElevation = 0.dp,
                ),
    ) {
        Column(modifier = Modifier.padding(12.dp)) {
            Text(
                text = text,
                style = MaterialTheme.typography.bodyMedium,
                color = contentColor,
            )
            if (action != null) {
                Spacer(Modifier.height(8.dp))
                action()
            }
        }
    }
}

/** Share a cache file via FileProvider + ACTION_SEND chooser. Identical
 * logic was inlined for both the debug-zip and the logcat export. */
internal fun shareFileViaProvider(
    context: Context,
    file: File,
    mimeType: String,
) {
    val uri =
        FileProvider.getUriForFile(
            context,
            "${context.packageName}.fileprovider",
            file,
        )
    val intent =
        Intent(Intent.ACTION_SEND).apply {
            type = mimeType
            putExtra(Intent.EXTRA_STREAM, uri)
            addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
        }
    context.startActivity(Intent.createChooser(intent, null))
}

/**
 * Save + Share button row used by both Diagnostics export flows. [sharePrimary]
 * paints the Share button filled (debug-zip) vs outlined (logcat).
 */
@Composable
internal fun FileSaveShareRow(
    saveLabel: String,
    shareLabel: String,
    sharePrimary: Boolean,
    onSave: () -> Unit,
    onShare: () -> Unit,
) {
    Row(
        modifier = Modifier.fillMaxWidth(),
        horizontalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        EnhancedOutlinedButton(onClick = onSave, modifier = Modifier.weight(1f)) {
            Icon(Icons.Default.Save, contentDescription = null, modifier = Modifier.size(18.dp))
            Spacer(Modifier.width(8.dp))
            Text(saveLabel)
        }
        val shareContent: @Composable RowScope.() -> Unit = {
            Icon(Icons.Default.Share, contentDescription = null, modifier = Modifier.size(18.dp))
            Spacer(Modifier.width(8.dp))
            Text(shareLabel)
        }
        if (sharePrimary) {
            EnhancedButton(onClick = onShare, modifier = Modifier.weight(1f)) { shareContent() }
        } else {
            EnhancedOutlinedButton(onClick = onShare, modifier = Modifier.weight(1f)) { shareContent() }
        }
    }
}
