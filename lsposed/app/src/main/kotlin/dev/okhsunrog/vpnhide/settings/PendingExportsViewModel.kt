package dev.okhsunrog.vpnhide.settings

import androidx.lifecycle.ViewModel

/** Prepared documents survive Activity recreation while Android's document picker is open.
 * Kept in memory, not the saved-state Bundle: a configuration export can be large.
 */
internal class PendingExportsViewModel : ViewModel() {
    var config: String? = null
    var packages: String? = null
}
