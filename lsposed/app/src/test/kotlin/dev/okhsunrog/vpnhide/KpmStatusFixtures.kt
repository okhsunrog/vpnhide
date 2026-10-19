package dev.okhsunrog.vpnhide

/**
 * Test-only shorthand for a [KpmLoadStatus]: `key=value` lines in the vocabulary the
 * classifier tests were written in. Production builds the status from core's typed
 * [NativeBootStatus] ([NativeBootStatus.kpmLoadStatus]); nothing on the device writes this.
 */
internal fun kpmStatusOf(raw: String): KpmLoadStatus {
    val values = parseKeyValueLines(raw)

    fun bool(key: String): Boolean? =
        when (values[key]?.trim()) {
            "1" -> true
            "0" -> false
            else -> null
        }
    return KpmLoadStatus(
        timestamp = values["timestamp"]?.trim()?.toLongOrNull(),
        bootId = values["boot_id"]?.trim()?.ifEmpty { null },
        unameR = values["uname_r"]?.trim()?.ifEmpty { null },
        runtime =
            when (values["runtime"]?.trim()) {
                "activator" -> KpmRuntime.Activator
                "kpatch-next" -> KpmRuntime.KpatchNext
                "apatch" -> KpmRuntime.Apatch
                "conflict" -> KpmRuntime.Conflict
                else -> KpmRuntime.Unknown
            },
        loaded = bool("loaded"),
        filesystemHiding = bool("filesystem_hiding"),
        reason =
            when (values["reason"]?.trim()) {
                "ok" -> KpmFailureReason.Ok
                "conflicting_backend" -> KpmFailureReason.ConflictingBackend
                "missing_kpm" -> KpmFailureReason.MissingKpm
                "awaiting_superkey" -> KpmFailureReason.AwaitingSuperkey
                "unsupported_kernel" -> KpmFailureReason.UnsupportedKernel
                "activation_failed" -> KpmFailureReason.ActivationFailed
                "load_failed" -> KpmFailureReason.LoadFailed
                else -> KpmFailureReason.Unknown
            },
        detail = values["detail"]?.trim()?.ifEmpty { null },
    )
}

/** A root-snapshot entry carrying core's decision that [backend] was selected on [bootId]. */
internal fun selectedBootSection(
    backend: NativeBackendId,
    bootId: String,
): Pair<String, String> =
    NATIVE_BOOT_SECTION to
        org.json
            .JSONObject()
            .put("boot_id", bootId)
            .put("decided_epoch", 1)
            .put("decided_at_ms", 1)
            .put("updated_at_ms", 1)
            .put(
                "outcome",
                org.json.JSONObject().put(
                    "Selected",
                    org.json
                        .JSONObject()
                        .put("backend", backend.name)
                        .put("mode", "Loaded")
                        .put("filesystem_hiding", org.json.JSONObject.NULL),
                ),
            ).put("installed", org.json.JSONArray(listOf(backend.name)))
            .put("live", org.json.JSONArray())
            .put("staged", org.json.JSONArray())
            .put("zygisk_targets", "NotInstalled")
            .toString()
