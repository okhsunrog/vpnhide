package dev.okhsunrog.vpnhide.core

/**
 * Production IPC paths (core-2.0 §9 / step 7). Kept in sync with `core-rpc::PRODUCTION_SOCKET_PATH`
 * and the Magisk common module persist dir.
 */
object CorePaths {
    const val PERSIST_DIR = "/data/adb/vpnhide/core"
    const val SOCKET = "$PERSIST_DIR/vpnhide-core.sock"
    const val ALLOWED_UIDS = "$PERSIST_DIR/allowed_uids"

    /** Dev sandbox socket used by CoreSpikeActivity when extras omit `socket`. */
    const val DEV_SOCKET = "/data/local/tmp/vpnhide-dev/core.sock"
}
