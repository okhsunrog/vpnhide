package dev.okhsunrog.vpnhide

import android.content.pm.PackageManager
import android.database.sqlite.SQLiteDatabase
import android.os.Build
import android.os.SystemClock
import dev.okhsunrog.vpnhide.diagnostics.DiagnosticGate
import dev.okhsunrog.vpnhide.diagnostics.DiagnosticReport
import dev.okhsunrog.vpnhide.diagnostics.DiagnosticsCache
import dev.okhsunrog.vpnhide.diagnostics.LayerStatus
import dev.okhsunrog.vpnhide.diagnostics.Verdict
import dev.okhsunrog.vpnhide.diagnostics.buildDiagnosticReport
import dev.okhsunrog.vpnhide.diagnostics.classifyKpmProblem
import dev.okhsunrog.vpnhide.diagnostics.renderKpmProblem
import dev.okhsunrog.vpnhide.diagnostics.standaloneKpmLoaded
import dev.okhsunrog.vpnhide.diagnostics.token
import dev.okhsunrog.vpnhide.diagnostics.verdict
import dev.okhsunrog.vpnhide.generated.HookIds
import dev.okhsunrog.vpnhide.hook.HookEntry
import dev.okhsunrog.vpnhide.picker.TargetsSnapshot
import dev.okhsunrog.vpnhide.picker.parseTargetsSnapshot
import dev.okhsunrog.vpnhide.settings.SettingsRepository
import dev.okhsunrog.vpnhide.settings.installedNativeOptionalHooks
import dev.okhsunrog.vpnhide.settings.resolveFilesystemHidingState
import dev.okhsunrog.vpnhide.startup.StartupTrace
import kotlinx.coroutines.flow.first
import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import java.io.File

// ── Domain types — invalid states are unrepresentable ────────────────────

@Serializable
sealed interface ModuleState {
    @Serializable
    @SerialName("not_installed")
    data object NotInstalled : ModuleState

    @Serializable
    @SerialName("installed")
    data class Installed(
        val version: String?,
        val active: Boolean,
        // Only populated for kmod builds that carry the stamped `gkiVariant=` field
        // in module.prop (CI-built zips from v0.6.3+). Older builds report null.
        val gkiVariant: String? = null,
        // Non-null when the module is installed but the installation itself
        // is permanently broken (distinct from "active=false" which usually
        // just means a reboot is pending). UI colors the card red.
        val brokenReason: ModuleBrokenReason? = null,
        // True when a complete install is staged in modules_update/ and the
        // active dir has no activator yet — the module just needs a reboot to
        // take effect. UI colors the card orange, not red, and the dashboard
        // surfaces a reboot warning rather than a corruption error.
        val pendingReboot: Boolean = false,
        // False when the liveness probe for this module reads a runtime resource
        // the snapshot shell could not access (e.g. the 0600-root /proc/vpnhide_ctl
        // or an iptables query when the shell lacked root). Then `active=false` is
        // NOT trustworthy — the module may well be running — so the UI renders
        // "not verified", never a false "inactive". True when the probe was
        // authoritative. See [snapshotRuntimeCheckable].
        val runtimeCheckable: Boolean = true,
    ) : ModuleState
}

// Shared across every flashable module kind (kmod, KPM, ...) that stamps
// ModuleState.Installed.brokenReason — not just kmod, despite the name
// prefixes on the individual kmod-only cases below.
@Serializable
enum class ModuleBrokenReason {
    WrongVariant,
    UnsupportedKernel,
    MissingKprobes,
    UnknownVariantInactive,
    AmbiguousLoadFailed,
    SignatureEnforced,
    ActivatorMissing,
    ActivatorNotExecutable,
}

/**
 * The single module problem to surface. [reason] drives the red module-card
 * color; [text] drives the dashboard error banner. Computed once (see
 * `loadDashboardState`) so the card and the banner can never disagree —
 * previously the priority order was hand-mirrored in two separate `when`
 * blocks. [reason] is null for a generic load failure where we only have raw
 * stderr/exit-status text to show, not a named diagnosis. Shared by the kmod
 * (`classifyKmodProblem`) and KPM (`classifyKpmProblem`) diagnosis paths.
 */
internal data class ModuleProblem(
    val reason: ModuleBrokenReason?,
    val text: String,
    // The zip to offer for one-tap download when the fix is re-flashing a specific
    // artifact (wrong/unknown/unsupported variant, ambiguous load). Null for
    // problems no download fixes (kprobes missing, signature enforcement).
    val downloadArtifact: String? = null,
)

@Serializable
sealed interface LsposedState {
    @Serializable
    @SerialName("not_installed")
    data object NotInstalled : LsposedState

    @Serializable
    @SerialName("installed_inactive")
    data class InstalledInactive(
        val version: String?,
    ) : LsposedState

    @Serializable
    @SerialName("needs_reboot")
    data class NeedsReboot(
        val version: String?,
    ) : LsposedState

    @Serializable
    @SerialName("active")
    data class Active(
        val version: String?,
        val targetCount: Int,
    ) : LsposedState
}

@Serializable
internal sealed interface ProtectionCheck {
    // The gate stopped the run before anything could be measured — VPN off, this app
    // split-tunnelled out (hiding is moot for us → "add to tunnel"), or a pending
    // self-restart. Carries the shared [DiagnosticGate] so the hero/agent explain
    // which without a second enum. Never [DiagnosticGate.ROUTED] — that is [Checked].
    @Serializable
    @SerialName("blocked")
    data class Blocked(
        val gate: DiagnosticGate,
    ) : ProtectionCheck {
        init {
            require(gate != DiagnosticGate.ROUTED) { "Blocked gate must not be ROUTED" }
        }
    }

    // The run threw (root dropped, shell exec failure) — the VPN may well be up, we
    // just couldn't measure. Distinct from a VPN-off gate so the hero doesn't tell an
    // active-VPN user to turn their VPN on.
    @Serializable
    @SerialName("failed")
    data object Failed : ProtectionCheck

    @Serializable
    @SerialName("checked")
    data class Checked(
        val native: LayerStatus,
        val java: LayerStatus,
    ) : ProtectionCheck
}

internal enum class FlashableModuleKind { Kmod, Builtin, Kpm, Zygisk, Ports }

internal enum class MultiNativeSeverity { None, Warning, Error }

/**
 * Severity of having more than one native backend active at runtime
 * (docs/storage.md §4.3). The .ko + KPM pair is an ERROR because they wrap the same kernel
 * functions and co-residence can hard-freeze the kernel; any other active
 * overlap is a WARNING (merely redundant — the backstops pick one).
 */
internal fun classifyMultiNative(
    kmodActive: Boolean,
    kpmActive: Boolean,
    zygiskActive: Boolean,
): MultiNativeSeverity {
    val count = listOf(kmodActive, kpmActive, zygiskActive).count { it }
    return when {
        count <= 1 -> MultiNativeSeverity.None
        kmodActive && kpmActive -> MultiNativeSeverity.Error
        else -> MultiNativeSeverity.Warning
    }
}

/**
 * True when the KPM boot script recorded that it deliberately stood down this
 * boot because the .ko backend is present (`load_status` runtime=conflict for
 * the current boot_id — see docs/storage.md §4.3 and kmod/kpm/module/post-fs-data.sh).
 *
 * This is the only reliable way to surface the .ko+KPM co-installation: the
 * [classifyMultiNative] Error needs *both* backends active, but two active
 * kernel hookers would already have frozen the device, so that state is never
 * actually observed. The deferred KPM (loaded=0) is the real, safe-but-
 * redundant state worth warning about so the user removes one.
 */
internal fun kpmDeferredForConflict(
    status: KpmLoadStatus,
    currentBootId: String,
): Boolean = status.runtime == KpmRuntime.Conflict && status.isFreshFor(currentBootId)

/**
 * True when the KPM boot script stood down this boot because it runs under
 * APatch/FolkPatch and has no usable saved SuperKey or trusted `su` token
 * (`runtime=apatch, loaded=0, detail=awaiting_superkey` for the current
 * boot_id — see kmod/kpm/module/service.sh and docs/storage.md §4.3).
 *
 * Distinct from [kpmDeferredForConflict] (that writes `runtime=conflict`): here the
 * module is installed and healthy but dormant, waiting for an APatch SuperKey
 * in Settings so the service activator can load it. Without this, the dashboard
 * would just show KPM as inactive with no explanation.
 */
internal fun kpmAwaitingSuperkey(
    status: KpmLoadStatus,
    currentBootId: String,
): Boolean =
    status.runtime == KpmRuntime.Apatch &&
        status.loaded == false &&
        status.reason == KpmFailureReason.AwaitingSuperkey &&
        status.isFreshFor(currentBootId)

/**
 * True when a *live* KernelPatch runtime is present — the kernel is actually
 * patched and can load KPMs. Either APatch (its /data/adb/ap dir) or a
 * KPatch-Next-Module whose `kpatch hello` succeeded ([hello_exit]=0). A
 * KPatch-Next-Module that is installed but whose kernel has not been patched
 * from its UI reports hello_exit≠0 and is correctly treated as unavailable.
 * See the kpatch_runtime probe in RootSnapshotCache.
 */
internal fun kpatchRuntimeAvailable(kpatchRuntimeSection: String): Boolean {
    val props = parseKeyValueLines(kpatchRuntimeSection)
    if (props["apatch_dir"]?.trim() == "1") return true
    return props["hello_exit"]?.trim() == "0"
}

internal data class PortsApplyProblem(
    val failureDetail: String?,
)

internal fun detectPortsApplyProblem(
    ports: ModuleState,
    targetCount: Int,
    loadStatusSection: String,
    currentBootId: String,
    portsDisabled: Boolean,
): PortsApplyProblem? {
    val installed = ports as? ModuleState.Installed ?: return null
    if (installed.active || installed.brokenReason != null || targetCount == 0) return null
    // A module the user turned off via their manager (a `disable` marker) is
    // inactive by design — the activator skips it for the same reason. Don't
    // nag "iptables rules are not active" for a deliberately disabled module.
    if (portsDisabled) return null

    val load = parseKeyValueLines(loadStatusSection)
    val bootId = load["boot_id"]?.trim()
    val sameBoot = !bootId.isNullOrEmpty() && bootId == currentBootId.trim()
    val failedThisBoot = sameBoot && load["loaded"]?.trim() == "0"
    val detail =
        if (failedThisBoot) {
            load["detail"]?.trim()?.takeIf { it.isNotBlank() }
        } else {
            null
        }
    return PortsApplyProblem(failureDetail = detail)
}

internal data class ModuleMismatch(
    val kind: FlashableModuleKind,
    val moduleVersion: String,
    val appVersion: String,
)

// Pure: given a list of (state, kind) pairs and the app version, returns
// the subset whose base version disagrees with the app. Extracted so the
// three kmod / zygisk / ports callsites in loadDashboardState share one
// code path instead of three near-identical if-blocks.
internal fun detectModuleMismatches(
    modules: List<Pair<ModuleState, FlashableModuleKind>>,
    appVersion: String,
): List<ModuleMismatch> =
    modules.mapNotNull { (state, kind) ->
        val installed = state as? ModuleState.Installed ?: return@mapNotNull null
        val moduleVersion = installed.version ?: return@mapNotNull null
        if (versionsMismatch(moduleVersion, appVersion)) {
            ModuleMismatch(kind, moduleVersion, appVersion)
        } else {
            null
        }
    }

internal sealed interface LsposedFramework {
    data object NotInstalled : LsposedFramework

    data class Installed(
        val disabled: Boolean,
    ) : LsposedFramework
}

internal sealed interface LsposedConfig {
    data object ModuleNotConfigured : LsposedConfig

    data object Disabled : LsposedConfig

    data class Enabled(
        val entries: List<String>,
        val hasSystemFramework: Boolean,
        val extraEntries: List<String>,
    ) : LsposedConfig
}

@Serializable
internal enum class DashboardMessageSeverity { ERROR, WARNING, INFO }

// Optional call-to-action rendered on a message banner. ContactAuthor opens the
// shared community/feedback modal (GitHub / Telegram / 4PDA); OpenDiagnostics
// opens the detailed diagnostics screen.
@Serializable
internal enum class DashboardMessageAction { ContactAuthor, OpenDiagnostics }

@Serializable
internal data class DashboardMessage(
    val severity: DashboardMessageSeverity,
    val text: String,
    val action: DashboardMessageAction? = null,
    // When set, the banner renders a "download this zip" button that grabs this
    // named artifact from the latest release (wrong variant, outdated module, …).
    val downloadArtifact: String? = null,
)

@Serializable
internal data class DashboardState(
    val kmod: ModuleState,
    val kpm: ModuleState,
    val zygisk: ModuleState,
    val lsposed: LsposedState,
    val ports: ModuleState,
    val nativeTargetCount: Int,
    val portsTargetCount: Int,
    // The one native backend surfaced on the dashboard (kmod > KPM > Zygisk).
    val nativeBackend: DisplayNativeBackend,
    val nativeInstallRecommendation: NativeInstallRecommendation?,
    val kmodLoadStatus: KmodLoadStatus?,
    val protection: ProtectionCheck,
    val messages: List<DashboardMessage>,
    // Optional native hooks this boot actually installed. Retained so the Detailed
    // diagnostics screen can rebuild the canonical DiagnosticReport (which vectors
    // the active backend owns) rather than deriving ownership a second way.
    val installedOptionalHooks: Set<HookIds.Hook> = emptySet(),
    // A pre-1.0 config still on disk next to a config that already has roles —
    // the startup importer leaves that case to the user (LegacyConfigImport).
    val legacyImport: LegacyImportPrompt? = null,
)

internal enum class HeroStatus { Protected, Attention, Unprotected, VpnOff }

internal fun protectionFullyPassed(protection: ProtectionCheck): Boolean =
    protection is ProtectionCheck.Checked &&
        (protection.native as? LayerStatus.Active)?.verdict == Verdict.Ok &&
        (protection.java as? LayerStatus.Active)?.verdict == Verdict.Ok

/** Worst-signal rank a layer contributes to the hero: leaking-and-dead = 2,
 * partial / inactive / absent = 1, ok = 0. */
private fun LayerStatus.heroRank(): Int =
    when (this) {
        LayerStatus.Absent -> {
            1
        }

        LayerStatus.Inactive -> {
            1
        }

        LayerStatus.Unverified -> {
            1
        }

        is LayerStatus.Active -> {
            when (verdict) {
                Verdict.Ok -> 0
                Verdict.Partial -> 1
                Verdict.Broken -> 2
            }
        }
    }

/** Overall health, ranked worst-signal-wins from protection state + errors/warnings. */
internal fun computeHeroStatus(
    state: DashboardState,
    errorCount: Int,
    warningCount: Int,
): HeroStatus {
    val p = state.protection
    if (p is ProtectionCheck.Blocked && p.gate == DiagnosticGate.VPN_OFF) return HeroStatus.VpnOff
    // 0 = protected, 1 = attention, 2 = unprotected — keep the worst signal.
    var rank = 0
    when (p) {
        // A non-VPN-off block (self-not-routed / needs-restart) is attention, not off.
        is ProtectionCheck.Blocked -> {
            rank = maxOf(rank, 1)
        }

        // Couldn't measure — attention, not "off" (we don't know protection is broken).
        ProtectionCheck.Failed -> {
            rank = maxOf(rank, 1)
        }

        is ProtectionCheck.Checked -> {
            rank = maxOf(rank, p.native.heroRank(), p.java.heroRank())
        }
    }
    when {
        errorCount > 0 -> rank = maxOf(rank, 2)
        warningCount > 0 -> rank = maxOf(rank, 1)
    }
    return when (rank) {
        0 -> HeroStatus.Protected
        1 -> HeroStatus.Attention
        else -> HeroStatus.Unprotected
    }
}

internal fun moduleSummaryText(state: DashboardState): String = "${activeModuleCount(state)}/3"

// Three logical layers now: Java (LSPosed), Native (the one active backend),
// and Ports. The native layer counts once regardless of which backend serves it.
internal fun activeModuleCount(state: DashboardState): Int =
    listOf(
        state.lsposed is LsposedState.Active,
        moduleActive(state.nativeBackend.state),
        moduleActive(state.ports),
    ).count { it }

internal fun moduleActive(state: ModuleState): Boolean = (state as? ModuleState.Installed)?.active == true

/** uid the root-snapshot shell actually ran as, parsed from the `snapshot_shell_uid`
 * probe. Null when the probe is absent (older snapshot) — treated as trusted (0). */
internal fun snapshotShellUid(sections: Map<String, String>): Int? =
    sections["snapshot_shell_uid"]
        .orEmpty()
        .lineSequence()
        .firstOrNull { it.startsWith("uid=") }
        ?.substringAfter("uid=")
        ?.trim()
        ?.toIntOrNull()

/** Whether the snapshot shell had the privilege to trust its runtime-liveness
 * probes (0600 /proc/vpnhide_ctl, iptables). A shell that isn't uid 0 reads a
 * false negative from those, so a negative liveness result is "not verified",
 * not "inactive". A missing probe (older snapshot) defaults to trusted. */
internal fun snapshotRuntimeCheckable(sections: Map<String, String>): Boolean {
    val uid = snapshotShellUid(sections) ?: return true
    return uid == 0
}

@Serializable
internal data class NativeInstallRecommendation(
    val androidVersion: String,
    val kernelVersion: String,
    val kernelBranch: String?,
    val recommended: NativeBackendId,
    val recommendedArtifact: String,
    val recommendedGkiVariant: String?,
    // For a KPM recommendation: whether a KPatch runtime (APatch or the
    // KPatch-Next-Module on any manager) was detected. When false the UI also
    // tells the user to install that module first, and offers zygisk as the
    // no-extra-setup fallback.
    val kpatchRuntimeAvailable: Boolean = false,
    // Set when the kernel's GKI KMI couldn't be parsed from uname -r but the
    // kernel series ships with multiple KMI variants (5.10: android12 / 13;
    // 5.15: android13 / 14). Both candidates are valid picks — the UI shows
    // the primary plus "if it doesn't load, try the alternative". Series with
    // a single shipping variant (6.1 / 6.6 / 6.12) stay unambiguous even
    // without a KMI tag.
    val variantAmbiguous: Boolean = false,
    val alternativeArtifact: String? = null,
    val alternativeGkiVariant: String? = null,
) {
    // True when the recommended backend is the .ko. The kmod-problem classifier
    // reuses this to decide whether a wrong-variant / unsupported-kernel
    // diagnosis applies to an installed-but-inactive kmod.
    val preferKmod: Boolean get() = recommended == NativeBackendId.Kmod
}

// Boot-time diagnostics written by kmod/module/post-fs-data.sh into
// /data/adb/vpnhide_kmod/load_status. Stays valid across reboots,
// so bootId is compared against the current boot to know if the
// record is fresh.
@Serializable
internal data class KmodLoadStatus(
    val timestamp: Long?,
    val bootId: String?,
    val unameR: String?,
    val gkiVariant: String?,
    val kmodVersion: String?,
    val rootManager: String?,
    val kprobes: String?,
    val kretprobes: String?,
    val insmodExit: Int?,
    val loaded: Boolean,
    val insmodStderr: String?,
    val dmesgTail: String?,
    val freshForCurrentBoot: Boolean,
    val filesystemHiding: Boolean? = null,
    val filesystemConfigExit: Int? = null,
    val filesystemConfigError: String? = null,
)

private const val TAG = LogTags.DASHBOARD

// Non-GKI kernel series KernelPatch (the KPM runtime) supports but the .ko does
// not: no GKI KMI, no DDK build. Mirrors the sub-5.10 rows of the KPM kver
// offset table in kmod/kpm/kver_offsets.h. GKI series (5.10+) prefer the .ko;
// other series aren't in the table and fall back to zygisk.
internal val KPM_NON_GKI_SERIES = setOf("4.9", "4.14", "4.19", "5.4")

internal fun parseKernelSeries(raw: String): String? = Regex("""\b(\d+\.\d+)""").find(raw)?.groupValues?.get(1)

internal fun parseKernelAndroidBranch(raw: String): String? =
    Regex("""android(\d+)""")
        .find(raw)
        ?.groupValues
        ?.get(1)
        ?.let { "Android $it" }

/**
 * Pick the right native-module artifact for the device based on its
 * kernel version (from `uname -r`) and Android OS label (from
 * `Build.VERSION.RELEASE`). Pulled out as a pure top-level function
 * so it can be unit-tested without a real device — the `uname -r`
 * read and `Build.VERSION` probe happen in the caller.
 *
 * Strategy, in order:
 *  1. Exact `(GKI KMI × kernel series)` match from the supported
 *     shipping matrix → specific kmod zip, recommended=Kmod.
 *  2. KMI tag missing from `uname -r` (custom kernel stripped it)
 *     but the kernel series is GKI-shipping:
 *       - 6.1 / 6.6 / 6.12 have a single shipping variant each →
 *         deterministic kmod recommendation, recommended=Kmod.
 *       - 5.10 / 5.15 have two shipping variants each → return the
 *         primary plus an alternative via `variantAmbiguous=true`;
 *         the UI shows "try primary, if it doesn't load try alt".
 *  3. Non-GKI kernels that KernelPatch covers (4.9 / 4.14 / 4.19 / 5.4 —
 *     no GKI KMI, no DDK kmod build) → KPM, the single
 *     universal binary. [kpatchRuntimeAvailable] decides whether the
 *     UI also asks the user to install the KPatch-Next-Module first.
 *  4. Any other series or an unparseable kernel version → fall back to
 *     zygisk (recommended=Zygisk); the KPM offset table has no validated
 *     layout for it, and no kmod build loads against such kernels.
 *
 * Returns `null` only if [kernelRaw] is blank (no uname output).
 * `deviceAndroidLabel` is only reflected back in the returned
 * `androidVersion` for display — it's never used for KMI matching
 * (those spaces are independent: an Android 15 ROM routinely runs
 * an android12 KMI kernel).
 *
 * detekt LongMethod is suppressed: this is a GKI KMI × kernel-series lookup
 * table, not tangled control flow.
 */
@Suppress("LongMethod")
internal fun buildNativeInstallRecommendation(
    kernelRaw: String,
    deviceAndroidLabel: String,
    kpatchRuntimeAvailable: Boolean = false,
): NativeInstallRecommendation? {
    val kernelVersion = kernelRaw.trim().ifBlank { return null }
    val kernelSeries = parseKernelSeries(kernelVersion)
    val kernelBranch = parseKernelAndroidBranch(kernelVersion) // GKI KMI

    data class KmiMatch(
        val kmi: String,
        val zip: String,
    )

    val exact: KmiMatch? =
        when (kernelBranch to kernelSeries) {
            "Android 12" to "5.10" -> KmiMatch("android12-5.10", "vpnhide-kmod-android12-5.10.zip")
            "Android 13" to "5.10" -> KmiMatch("android13-5.10", "vpnhide-kmod-android13-5.10.zip")
            "Android 13" to "5.15" -> KmiMatch("android13-5.15", "vpnhide-kmod-android13-5.15.zip")
            "Android 14" to "5.15" -> KmiMatch("android14-5.15", "vpnhide-kmod-android14-5.15.zip")
            "Android 14" to "6.1" -> KmiMatch("android14-6.1", "vpnhide-kmod-android14-6.1.zip")
            "Android 15" to "6.6" -> KmiMatch("android15-6.6", "vpnhide-kmod-android15-6.6.zip")
            "Android 16" to "6.12" -> KmiMatch("android16-6.12", "vpnhide-kmod-android16-6.12.zip")
            else -> null
        }
    if (exact != null) {
        return NativeInstallRecommendation(
            androidVersion = deviceAndroidLabel,
            kernelVersion = kernelVersion,
            kernelBranch = kernelBranch,
            recommended = NativeBackendId.Kmod,
            recommendedArtifact = exact.zip,
            recommendedGkiVariant = exact.kmi,
        )
    }

    val fallback: Pair<KmiMatch, KmiMatch?>? =
        when (kernelSeries) {
            "5.10" -> {
                KmiMatch("android12-5.10", "vpnhide-kmod-android12-5.10.zip") to
                    KmiMatch("android13-5.10", "vpnhide-kmod-android13-5.10.zip")
            }

            "5.15" -> {
                KmiMatch("android13-5.15", "vpnhide-kmod-android13-5.15.zip") to
                    KmiMatch("android14-5.15", "vpnhide-kmod-android14-5.15.zip")
            }

            "6.1" -> {
                KmiMatch("android14-6.1", "vpnhide-kmod-android14-6.1.zip") to null
            }

            "6.6" -> {
                KmiMatch("android15-6.6", "vpnhide-kmod-android15-6.6.zip") to null
            }

            "6.12" -> {
                KmiMatch("android16-6.12", "vpnhide-kmod-android16-6.12.zip") to null
            }

            else -> {
                null
            }
        }
    if (fallback != null) {
        val (primary, alternative) = fallback
        return NativeInstallRecommendation(
            androidVersion = deviceAndroidLabel,
            kernelVersion = kernelVersion,
            kernelBranch = kernelBranch,
            recommended = NativeBackendId.Kmod,
            recommendedArtifact = primary.zip,
            recommendedGkiVariant = primary.kmi,
            variantAmbiguous = alternative != null,
            alternativeArtifact = alternative?.zip,
            alternativeGkiVariant = alternative?.kmi,
        )
    }

    // Non-GKI kernels KernelPatch supports (4.9 / 4.14 / 4.19 / 5.4) — no GKI KMI and
    // no DDK kmod build, but they're in the KPM kver offset table
    // (kmod/kpm/kver_offsets.h). Recommend the universal KPM.
    if (kernelSeries in KPM_NON_GKI_SERIES) {
        return NativeInstallRecommendation(
            androidVersion = deviceAndroidLabel,
            kernelVersion = kernelVersion,
            kernelBranch = kernelBranch,
            recommended = NativeBackendId.Kpm,
            recommendedArtifact = "vpnhide-kpm.zip",
            recommendedGkiVariant = null,
            kpatchRuntimeAvailable = kpatchRuntimeAvailable,
        )
    }

    return NativeInstallRecommendation(
        androidVersion = deviceAndroidLabel,
        kernelVersion = kernelVersion,
        kernelBranch = kernelBranch,
        recommended = NativeBackendId.Zygisk,
        recommendedArtifact = "vpnhide-zygisk.zip",
        recommendedGkiVariant = null,
    )
}

// ── Module-prop / status-file parsing (pure, unit-tested) ────────────────

// Strip the `v` prefix from module.prop versions at parse time so everything
// downstream sees a plain semver string (APK versionName has no `v`).
internal data class ModulePropInfo(
    val installed: Boolean,
    val version: String?,
    val gkiVariant: String?,
)

// Older CI-built zips didn't stamp `gkiVariant=` but their injected updateJson
// URL already encodes the KMI: `.../update-kmod-<kmi>.json`. Recover the variant
// from there so wrong-variant detection works for existing installs.
private val UPDATE_JSON_KMI_REGEX = Regex("""update-kmod-([^/]+)\.json""")

internal fun parseModuleProp(raw: String): ModulePropInfo {
    if (raw.isBlank()) return ModulePropInfo(false, null, null)
    var version: String? = null
    var gkiVariant: String? = null
    var updateJsonKmi: String? = null
    for (line in raw.lines()) {
        when {
            line.startsWith("version=") -> {
                version = normalizeVersion(line.removePrefix("version="))
            }

            line.startsWith("gkiVariant=") -> {
                gkiVariant = line.removePrefix("gkiVariant=").trim().ifBlank { null }
            }

            line.startsWith("updateJson=") -> {
                updateJsonKmi = UPDATE_JSON_KMI_REGEX.find(line.removePrefix("updateJson="))?.groupValues?.get(1)
            }
        }
    }
    return ModulePropInfo(true, version, gkiVariant ?: updateJsonKmi)
}

internal fun readKmodLoadStatus(
    currentBootId: String,
    raw: String,
    dmesgRaw: String,
): KmodLoadStatus? {
    if (raw.isBlank()) return null
    val props = parseKeyValueLines(raw)
    val bootId = props["boot_id"]?.trim()
    return KmodLoadStatus(
        timestamp = props["timestamp"]?.trim()?.toLongOrNull(),
        bootId = bootId,
        unameR = props["uname_r"]?.trim(),
        gkiVariant = props["gki_variant"]?.trim()?.ifBlank { null },
        kmodVersion = props["kmod_version"]?.trim()?.ifBlank { null },
        rootManager = props["root_manager"]?.trim()?.ifBlank { null },
        kprobes = props["kprobes"]?.trim()?.ifBlank { null },
        kretprobes = props["kretprobes"]?.trim()?.ifBlank { null },
        insmodExit = props["insmod_exit"]?.trim()?.toIntOrNull(),
        loaded = props["loaded"]?.trim() == "1",
        insmodStderr = props["insmod_stderr"]?.trim()?.ifBlank { null },
        dmesgTail = dmesgRaw.trim().ifBlank { null },
        freshForCurrentBoot = bootId != null && bootId == currentBootId,
        filesystemHiding =
            props["filesystem_hiding"]
                ?.trim()
                ?.let { value -> value == "1" },
        filesystemConfigExit = props["filesystem_config_exit"]?.trim()?.toIntOrNull(),
        filesystemConfigError = props["filesystem_config_error"]?.trim()?.ifBlank { null },
    )
}

// ── kmod problem classification (pure, unit-tested) ──────────────────────

/**
 * The single diagnosed problem for an installed-but-not-working kmod, as a
 * data-only value with the exact arguments its banner text needs. [reason]
 * drives the red module-card color; the [renderKmodProblem] mapping turns the
 * kind into the localized banner string. Computing the kind in one pure place
 * keeps the card color and the banner text from ever disagreeing — they used
 * to be hand-mirrored across two `when` blocks.
 */
internal sealed interface KmodProblemKind {
    val reason: ModuleBrokenReason?

    data object KprobesMissing : KmodProblemKind {
        override val reason get() = ModuleBrokenReason.MissingKprobes
    }

    // The kernel enforces module signature verification and refused our
    // unsigned .ko (insmod → EKEYREJECTED). No GKI variant will ever load on
    // such a kernel, so the only fix is removing the enforcement — KernelSU
    // Next (GKI mode) does that. See issue #132.
    data object SignatureEnforced : KmodProblemKind {
        override val reason get() = ModuleBrokenReason.SignatureEnforced
    }

    data class UnsupportedKernel(
        val unameR: String,
        val recommendedArtifact: String,
    ) : KmodProblemKind {
        override val reason get() = ModuleBrokenReason.UnsupportedKernel
    }

    data class WrongVariant(
        val installedVariant: String,
        val recommendedKmi: String,
        val recommendedArtifact: String,
    ) : KmodProblemKind {
        override val reason get() = ModuleBrokenReason.WrongVariant
    }

    data class UnknownVariant(
        val recommendedArtifact: String,
    ) : KmodProblemKind {
        override val reason get() = ModuleBrokenReason.UnknownVariantInactive
    }

    data class AmbiguousLoadFailed(
        val installedVariant: String,
        val tryArtifact: String,
    ) : KmodProblemKind {
        override val reason get() = ModuleBrokenReason.AmbiguousLoadFailed
    }

    // Generic insmod failure where we only have raw stderr, no named diagnosis.
    data class LoadFailed(
        val insmodStderr: String,
    ) : KmodProblemKind {
        override val reason: ModuleBrokenReason? get() = null
    }
}

/**
 * Diagnose what's wrong with an installed kmod, or null if it's fine / not a
 * diagnosable failure. Priority order: kprobes-missing first (no variant will
 * ever work), then unsupported-kernel (wrong tool), wrong-variant (concrete
 * mismatch), unknown-variant (old build that didn't stamp gkiVariant),
 * ambiguous-load-failed (one of two valid candidates failed this boot), and
 * finally a generic insmod failure when we have stderr to show.
 *
 * An active kmod (`/proc/vpnhide_ctl` present) is empirical proof the
 * install works, so every check except the activity-independent kprobes probe
 * is gated on `!active`.
 *
 * detekt CyclomaticComplexMethod is suppressed: this is priority-ordered
 * dispatch where each branch is a distinct, tested diagnosis — the branch count
 * is the point (see ClassifyKmodProblemTest).
 */
@Suppress("CyclomaticComplexMethod")
internal fun classifyKmodProblem(
    kmod: ModuleState,
    recommendation: NativeInstallRecommendation?,
    loadStatus: KmodLoadStatus?,
): KmodProblemKind? {
    if (kmod !is ModuleState.Installed) return null
    val freshLoad = loadStatus != null && loadStatus.freshForCurrentBoot

    if (freshLoad && loadStatus.kretprobes == "n") {
        return KmodProblemKind.KprobesMissing
    }
    // Past this point every diagnosis means "installed but not loaded".
    if (kmod.active) return null

    // Module-signature enforcement is the root cause regardless of GKI
    // variant — an unsigned module never loads on such a kernel, so this
    // takes priority over the variant/recommendation diagnoses below, which
    // would otherwise mislead the user into reinstalling a different zip.
    if (freshLoad && loadStatus.isModuleSignatureRejected()) {
        return KmodProblemKind.SignatureEnforced
    }

    val rec = recommendation
    // rec.recommendedArtifact is a non-null field, so a non-null rec always
    // yields a non-null artifact — the `!!` uses below are safe under the
    // `rec != null` / `rec?.preferKmod == true` guards that precede them.
    val recommendedArtifact = rec?.recommendedArtifact
    val installedVariant = kmod.gkiVariant

    if (rec != null && !rec.preferKmod) {
        return KmodProblemKind.UnsupportedKernel(loadStatus?.unameR ?: "?", recommendedArtifact!!)
    }

    val recommendedKmi = rec?.recommendedGkiVariant
    if (rec?.preferKmod == true &&
        recommendedKmi != null &&
        installedVariant != null &&
        installedVariant != recommendedKmi &&
        installedVariant != rec.alternativeGkiVariant
    ) {
        return KmodProblemKind.WrongVariant(installedVariant, recommendedKmi, recommendedArtifact!!)
    }

    if (installedVariant == null && rec?.preferKmod == true) {
        return KmodProblemKind.UnknownVariant(recommendedArtifact!!)
    }

    if (freshLoad &&
        rec?.variantAmbiguous == true &&
        installedVariant != null &&
        (installedVariant == rec.recommendedGkiVariant || installedVariant == rec.alternativeGkiVariant)
    ) {
        val tryArtifact =
            if (installedVariant == rec.recommendedGkiVariant) rec.alternativeArtifact else rec.recommendedArtifact
        return KmodProblemKind.AmbiguousLoadFailed(installedVariant, tryArtifact ?: "?")
    }

    if (freshLoad && loadStatus.insmodStderr != null) {
        return KmodProblemKind.LoadFailed(loadStatus.insmodStderr)
    }

    return null
}

// EKEYREJECTED — the errno the kernel's module_sig_check() returns for an
// unsigned module when signature enforcement is on ("Loading of unsigned
// module is rejected"). insmod surfaces it both as its exit code and as the
// strerror text, so match either to stay robust across insmod implementations.
private const val EKEYREJECTED = 129

internal fun KmodLoadStatus.isModuleSignatureRejected(): Boolean {
    if (insmodExit == EKEYREJECTED) return true
    val stderr = insmodStderr?.lowercase() ?: return false
    return "key was rejected" in stderr || "required key not available" in stderr
}

private fun renderKmodProblem(
    kind: KmodProblemKind,
    res: android.content.res.Resources,
): ModuleProblem =
    ModuleProblem(
        reason = kind.reason,
        downloadArtifact =
            when (kind) {
                is KmodProblemKind.UnsupportedKernel -> kind.recommendedArtifact
                is KmodProblemKind.WrongVariant -> kind.recommendedArtifact
                is KmodProblemKind.UnknownVariant -> kind.recommendedArtifact
                is KmodProblemKind.AmbiguousLoadFailed -> kind.tryArtifact.takeIf { it.endsWith(".zip") }
                else -> null
            },
        text =
            when (kind) {
                KmodProblemKind.KprobesMissing -> {
                    res.getString(R.string.dashboard_issue_kprobes_missing)
                }

                KmodProblemKind.SignatureEnforced -> {
                    res.getString(R.string.dashboard_issue_kmod_signature_enforced)
                }

                is KmodProblemKind.UnsupportedKernel -> {
                    res.getString(R.string.dashboard_issue_kmod_not_supported_kernel, kind.unameR, kind.recommendedArtifact)
                }

                is KmodProblemKind.WrongVariant -> {
                    res.getString(
                        R.string.dashboard_issue_kmod_wrong_variant,
                        kind.installedVariant,
                        kind.recommendedKmi,
                        kind.recommendedArtifact,
                    )
                }

                is KmodProblemKind.UnknownVariant -> {
                    res.getString(R.string.dashboard_issue_kmod_unknown_variant, kind.recommendedArtifact)
                }

                is KmodProblemKind.AmbiguousLoadFailed -> {
                    res.getString(
                        R.string.dashboard_issue_kmod_ambiguous_try_alternative,
                        kind.installedVariant,
                        kind.tryArtifact,
                    )
                }

                is KmodProblemKind.LoadFailed -> {
                    res.getString(R.string.dashboard_issue_kmod_load_failed, kind.insmodStderr)
                }
            },
    )

// ── LSPosed state resolution (pure, unit-tested) ─────────────────────────

/**
 * Resolve the user-facing [LsposedState] from the framework presence, the
 * on-disk module config, and whether the hook heartbeat is fresh for this
 * boot. A current-boot heartbeat is the strongest signal (module is active);
 * otherwise the config/framework decide inactive/needs-reboot/not-configured.
 */
internal fun resolveLsposedState(
    hooksActiveThisBoot: Boolean,
    hookVersion: String?,
    lsposedTargetCount: Int,
    framework: LsposedFramework,
    config: LsposedConfig?,
): LsposedState {
    if (hooksActiveThisBoot) {
        return LsposedState.Active(hookVersion, lsposedTargetCount)
    }
    return when (config) {
        null -> {
            LsposedState.InstalledInactive(null)
        }

        LsposedConfig.ModuleNotConfigured -> {
            when (framework) {
                LsposedFramework.NotInstalled -> LsposedState.NotInstalled
                is LsposedFramework.Installed -> LsposedState.InstalledInactive(null)
            }
        }

        LsposedConfig.Disabled -> {
            LsposedState.InstalledInactive(null)
        }

        is LsposedConfig.Enabled -> {
            if (config.hasSystemFramework) {
                LsposedState.NeedsReboot(hookVersion)
            } else {
                LsposedState.InstalledInactive(null)
            }
        }
    }
}

// ── Module detection (pure, from the root snapshot) ──────────────────────

// The backend that currently owns /proc/vpnhide_ctl, read from the `backend 0x<n>`
// line of the control status reply captured in the `kmod_state` section (a full
// `cat /proc/vpnhide_ctl`). The .ko reports 0x0, the in-tree driver 0x4; the two
// share the node and are mutually exclusive, so this is the one signal that tells
// a kmod device from a builtin one.
//
// Null when the id can't be read — the node was absent, or the snapshot shell
// wasn't root and got EACCES on the 0600 node (see [snapshotRuntimeCheckable]).
// A null id must never be treated as "the other backend"; callers keep their
// existing honest behaviour when it is null.
internal fun parseCtlBackendId(sections: Map<String, String>): Int? =
    sections["kmod_state"]
        .orEmpty()
        .lineSequence()
        .firstNotNullOfOrNull { line ->
            Regex("""\bbackend\s+0x([0-9a-fA-F]+)\b""")
                .find(line)
                ?.groupValues
                ?.get(1)
                ?.toIntOrNull(16)
        }

private val BUILTIN_BACKEND_ID = HookIds.Backend.BUILTIN.id

internal fun detectKmodModule(sections: Map<String, String>): ModuleState {
    val prop = parseModuleProp(sections["kmod_prop"].orEmpty())
    if (!prop.installed) return ModuleState.NotInstalled
    // The .ko and the in-tree driver share /proc/vpnhide_ctl. If the node
    // explicitly reports the built-in backend (0x4) then this proc is NOT the
    // .ko's — the kmod is installed but not the thing that's live. Only a
    // definitely-not-kmod id demotes it; a null/unread id (non-root snapshot,
    // 0600 EACCES) or a 0x0 id keeps the historical proc-present == active rule.
    val procPresent = sections["proc_exists"].orEmpty().trim() == "1"
    val ctlBackendId = parseCtlBackendId(sections)
    val active = procPresent && ctlBackendId != BUILTIN_BACKEND_ID
    // brokenReason is applied by the caller once the kernel recommendation +
    // load status are known (see classifyKmodProblem).
    return ModuleState.Installed(
        version = prop.version,
        active = active,
        gkiVariant = prop.gkiVariant,
        // A negative liveness read from a non-root snapshot shell is untrustworthy
        // (0600 /proc/vpnhide_ctl → EACCES → false "0"). Mark it unverified so the
        // UI never shows a false "inactive". A positive read is self-verifying.
        runtimeCheckable = active || snapshotRuntimeCheckable(sections),
    )
}

// Built-in kernel backend: the driver is compiled into the kernel, so the
// companion module ships only the userspace activator. "Installed" = that
// companion module is present; "active" = the shared control node is live AND
// it reports the built-in backend id (0x4). Because a non-root snapshot can't
// read the 0600 node, a proc-present-but-unread id leaves active=false but
// unverified — never a false "inactive", mirroring detectKmodModule.
internal fun detectBuiltinModule(sections: Map<String, String>): ModuleState {
    val prop = parseModuleProp(sections["builtin_prop"].orEmpty())
    if (!prop.installed) return ModuleState.NotInstalled
    val procPresent = sections["proc_exists"].orEmpty().trim() == "1"
    val ctlBackendId = parseCtlBackendId(sections)
    val active = procPresent && ctlBackendId == BUILTIN_BACKEND_ID
    return ModuleState.Installed(
        version = prop.version,
        active = active,
        // A positive read (backend==0x4) is self-verifying. Otherwise the id may
        // simply be unread (non-root snapshot on the 0600 node), so a false
        // "inactive" is not trustworthy — render "not verified" instead.
        runtimeCheckable = active || snapshotRuntimeCheckable(sections),
    )
}

internal fun detectZygiskModule(
    sections: Map<String, String>,
    zygiskStatusRaw: String,
    currentBootId: String,
): ModuleState {
    val prop = parseModuleProp(sections["zygisk_prop"].orEmpty())
    if (!prop.installed) return ModuleState.NotInstalled
    // Active = the in-process heartbeat the module writes on fork matches the
    // current boot. A stale heartbeat (previous boot) means not loaded yet.
    val heartbeatBootId = parseKeyValueLines(zygiskStatusRaw)["boot_id"]
    val active = heartbeatBootId != null && heartbeatBootId == currentBootId.trim()
    return ModuleState.Installed(
        version = prop.version,
        active = active,
    )
}

internal fun detectKpmModule(
    sections: Map<String, String>,
    loadStatus: KpmLoadStatus,
    currentBootId: String,
): ModuleState {
    val prop = parseModuleProp(sections["kpm_prop"].orEmpty())
    if (!prop.installed) return ModuleState.NotInstalled
    // The KPM has no /proc marker (its runtime channel is the kpatch ctl0
    // supercall). The boot script writes load_status with loaded=1 and the
    // boot_id it loaded under, so "active" = loaded for the current boot —
    // the same freshness check the zygisk heartbeat uses.
    val active = loadStatus.loaded == true && loadStatus.isFreshFor(currentBootId)
    return ModuleState.Installed(
        version = prop.version,
        active = active,
    )
}

internal fun detectPortsModule(sections: Map<String, String>): ModuleState {
    val prop = parseModuleProp(sections["ports_prop"].orEmpty())
    if (!prop.installed) return ModuleState.NotInstalled
    val active = sections["ports_chain"].orEmpty().trim() == "1"
    return ModuleState.Installed(
        version = prop.version,
        active = active,
        // `ports_chain` runs iptables, which needs CAP_NET_ADMIN; a non-root
        // snapshot shell reads a false "0". Mark unverified rather than inactive.
        runtimeCheckable = active || snapshotRuntimeCheckable(sections),
    )
}

// ── lsposed framework / config probes (need Android: PM, SQLite, root) ────

private fun androidMajorVersionLabel(): String {
    @Suppress("DEPRECATION")
    val release =
        if (Build.VERSION.SDK_INT >= 30) {
            Build.VERSION.RELEASE_OR_CODENAME
        } else {
            Build.VERSION.RELEASE
        }.substringBefore('.')
    return "Android $release"
}

private fun detectLsposedFramework(sections: Map<String, String>): LsposedFramework {
    val out = sections["lsposed_framework"].orEmpty()
    val props = parseKeyValueLines(out)
    val installedValue = props["installed"]
    val malformed =
        props["probe_ok"] != "1" ||
            installedValue == null ||
            (installedValue == "1" && props["disabled"] == null)
    if (malformed) {
        VpnHideLog.w(TAG, "lsposed framework probe returned malformed output: $out")
        return LsposedFramework.NotInstalled
    }
    val framework =
        if (installedValue == "1") {
            LsposedFramework.Installed(disabled = props["disabled"] == "1")
        } else {
            LsposedFramework.NotInstalled
        }
    VpnHideLog.i(TAG, "lsposed framework: $framework (raw=$out)")
    return framework
}

// Copy the LSPosed config DB (+ WAL/SHM) out of root-only storage into our
// cache dir so SQLiteDatabase can open it read-only. Returns the copied main
// db file, or null if the root copy failed.
private fun copyLsposedConfigDb(context: android.content.Context): File? {
    val dbCopy = File(context.cacheDir, "vpnhide_lspd_modules_config.db")
    val walCopy = File(context.cacheDir, "vpnhide_lspd_modules_config.db-wal")
    val shmCopy = File(context.cacheDir, "vpnhide_lspd_modules_config.db-shm")
    dbCopy.delete()
    walCopy.delete()
    shmCopy.delete()

    val src = "/data/adb/lspd/config/modules_config.db"
    val copyStart = SystemClock.elapsedRealtime()
    val (copyExit, copyOut) =
        suExec(
            "cat $src > ${dbCopy.absolutePath} && chmod 644 ${dbCopy.absolutePath} && " +
                "(cat $src-wal > ${walCopy.absolutePath} 2>/dev/null && chmod 644 ${walCopy.absolutePath} || true) && " +
                "(cat $src-shm > ${shmCopy.absolutePath} 2>/dev/null && chmod 644 ${shmCopy.absolutePath} || true) && " +
                "ls -l ${dbCopy.absolutePath} ${walCopy.absolutePath} ${shmCopy.absolutePath} 2>/dev/null || true",
        )
    StartupTrace.metric("dashboard_lsposed_db_copy", SystemClock.elapsedRealtime() - copyStart)
    if (copyExit != 0 || !dbCopy.isFile) {
        VpnHideLog.w(TAG, "failed to copy LSPosed config db for inspection: exit=$copyExit out=$copyOut")
        return null
    }
    VpnHideLog.i(TAG, "lsposed db copy: ${copyOut.trim()}")
    return dbCopy
}

// Nesting depth comes from the chained SQLite `.use {}` resource scopes
// (db → modules cursor → scope cursor), not from branching logic.
@Suppress("NestedBlockDepth")
internal fun readLsposedConfig(
    context: android.content.Context,
    selfPkg: String,
): LsposedConfig? {
    val dbCopy = copyLsposedConfigDb(context) ?: return null
    val queryStart = SystemClock.elapsedRealtime()
    return try {
        SQLiteDatabase.openDatabase(dbCopy.absolutePath, null, SQLiteDatabase.OPEN_READONLY).use { db ->
            db
                .rawQuery("SELECT mid, enabled FROM modules WHERE module_pkg_name = ?", arrayOf(selfPkg))
                .use { moduleCursor ->
                    if (!moduleCursor.moveToFirst()) return LsposedConfig.ModuleNotConfigured
                    val mid = moduleCursor.getLong(0)
                    if (moduleCursor.getInt(1) == 0) return LsposedConfig.Disabled

                    val scopeEntries = mutableListOf<Pair<String, Int>>()
                    db
                        .rawQuery(
                            "SELECT app_pkg_name, user_id FROM scope WHERE mid = ? ORDER BY user_id, app_pkg_name",
                            arrayOf(mid.toString()),
                        ).use { scopeCursor ->
                            while (scopeCursor.moveToNext()) {
                                scopeEntries += scopeCursor.getString(0) to scopeCursor.getInt(1)
                            }
                        }

                    fun isSystemFramework(
                        pkg: String,
                        userId: Int,
                    ) = pkg == "system" && userId == 0
                    LsposedConfig.Enabled(
                        entries = scopeEntries.map { (pkg, userId) -> if (isSystemFramework(pkg, userId)) "system" else "$pkg/$userId" },
                        hasSystemFramework = scopeEntries.any { (pkg, userId) -> isSystemFramework(pkg, userId) },
                        extraEntries =
                            scopeEntries
                                .filterNot { (pkg, userId) -> isSystemFramework(pkg, userId) || pkg == selfPkg }
                                .map { (pkg, userId) -> "$pkg/$userId" },
                    )
                }
        }
    } catch (e: Exception) {
        VpnHideLog.w(TAG, "failed to inspect LSPosed config db: ${e.message}")
        null
    } finally {
        StartupTrace.metric("dashboard_lsposed_db_query", SystemClock.elapsedRealtime() - queryStart)
        dbCopy.delete()
        File(dbCopy.absolutePath + "-wal").delete()
        File(dbCopy.absolutePath + "-shm").delete()
    }
}

// Linear orchestrator: pure detectors above produce the module/lsposed state,
// then a flat list of independent guards builds the dashboard message banners.
// Kept as one top-to-bottom narrative — splitting the flat guard list behind a
// parameter bundle would add indirection without improving clarity.

/**
 * One flashable module's derived state: the integrity/load diagnosis, whether
 * it is merely staged for the next reboot, and the [ModuleState] with both
 * folded in.
 *
 * The four backends used to do this inline, in four ~18-line blocks that
 * differed only in kind, activator path and the backend-specific classifier —
 * which is exactly the shape that lets one of them quietly drift.
 *
 * Order matters: a staged install is not a corrupt one, so [modulePendingReboot]
 * suppresses the diagnosis entirely; integrity beats the backend classifier so a
 * missing activator is not reported as a load failure.
 */
private fun deriveModuleFact(
    kind: FlashableModuleKind,
    raw: ModuleState,
    sections: Map<String, String>,
    activatorPath: String,
    res: android.content.res.Resources,
    classifyBackendProblem: () -> ModuleProblem? = { null },
): ModuleFact {
    val pendingReboot = modulePendingReboot(kind, raw, sections)
    val problem =
        if (pendingReboot) {
            null
        } else {
            moduleIntegrityProblem(
                kind = kind,
                module = raw,
                sections = sections,
                activatorPath = activatorPath,
            )?.let { renderModuleIntegrityProblem(it, res) }
                ?: classifyBackendProblem()
        }
    return ModuleFact(
        state = raw.withBrokenReason(problem?.reason).withPendingReboot(pendingReboot),
        problem = problem,
        pendingReboot = pendingReboot,
    )
}

/** Every module's state, as the cards show it and the banners read it. */
private fun deriveModuleFacts(
    sections: Map<String, String>,
    res: android.content.res.Resources,
    kernelRecommendation: NativeInstallRecommendation?,
    hasKpatchRuntime: Boolean,
    appVersion: String,
): ModuleFacts {
    val currentBootId = sections["current_boot_id"].orEmpty()
    val kpmLoadStatus = parseKpmLoadStatus(sections["kpm_load_status"].orEmpty())
    val raw = detectNativeBackendStates(sections, currentBootId = currentBootId, kpmLoadStatus = kpmLoadStatus)
    val portsRaw = detectPortsModule(sections)
    val kmodLoadStatus =
        readKmodLoadStatus(
            currentBootId.trim(),
            sections["kmod_load_status"].orEmpty(),
            sections["kmod_load_dmesg"].orEmpty(),
        )
    VpnHideLog.i(TAG, "kmodLoadStatus=$kmodLoadStatus")

    val kmod =
        deriveModuleFact(FlashableModuleKind.Kmod, raw.kmod, sections, KMOD_ACTIVATOR, res) {
            classifyKmodProblem(raw.kmod, kernelRecommendation, kmodLoadStatus)?.let { renderKmodProblem(it, res) }
        }
    // The in-tree backend has no .ko to diagnose, so it needs no classifier —
    // only the shared integrity/pending-reboot checks on its companion activator.
    val builtin = deriveModuleFact(FlashableModuleKind.Builtin, raw.builtin, sections, BUILTIN_ACTIVATOR, res)
    val kpm =
        deriveModuleFact(FlashableModuleKind.Kpm, raw.kpm, sections, KPM_ACTIVATOR, res) {
            classifyKpmProblem(
                kpm = raw.kpm,
                status = kpmLoadStatus,
                currentBootId = currentBootId,
                hasKpatchRuntime = hasKpatchRuntime,
                apatchSuperkeySaved = sections["superkey_saved"]?.trim() == "1",
            )?.let { renderKpmProblem(it, res) }
        }
    val zygisk = deriveModuleFact(FlashableModuleKind.Zygisk, raw.zygisk, sections, ZYGISK_ACTIVATOR, res)
    val ports = deriveModuleFact(FlashableModuleKind.Ports, portsRaw, sections, PORTS_ACTIVATOR, res)

    // The one place the three native backends are grouped: every "is anything
    // installed / active" gate reads from this instead of re-deriving its own
    // kmod/kpm/zygisk boolean combination.
    val backends =
        NativeBackendStates(kmod = kmod.state, builtin = builtin.state, kpm = kpm.state, zygisk = zygisk.state)
    VpnHideLog.i(TAG, "modules: $backends ports=${ports.state}")
    return ModuleFacts(
        kmod = kmod,
        builtin = builtin,
        kpm = kpm,
        zygisk = zygisk,
        ports = ports,
        backends = backends,
        // The single native backend the dashboard shows (kmod > builtin > KPM > Zygisk).
        nativeBackend = displayNativeBackend(backends),
        standaloneKpm = standaloneKpmLoaded(raw.kpm, sections["kpm_runtime_modules"].orEmpty()),
        kpmLoadStatus = kpmLoadStatus,
        kmodLoadStatus = kmodLoadStatus,
        currentBootId = currentBootId,
        mismatches =
            detectModuleMismatches(
                listOf(
                    kmod.state to FlashableModuleKind.Kmod,
                    builtin.state to FlashableModuleKind.Builtin,
                    kpm.state to FlashableModuleKind.Kpm,
                    zygisk.state to FlashableModuleKind.Zygisk,
                    ports.state to FlashableModuleKind.Ports,
                ),
                appVersion,
            ),
    )
}

/** LSPosed's runtime and on-disk state, and the hooks' own install health. */
private fun deriveLsposedFacts(
    context: android.content.Context,
    sections: Map<String, String>,
    currentBootId: String,
    lsposedTargetCount: Int,
): LsposedFacts {
    val lsposedStateRaw = sections["lsposed_state"].orEmpty()
    val hookProps = parseLsposedStateMetadata(lsposedStateRaw)
    val hooksActiveThisBoot = lsposedHooksActiveThisBoot(lsposedStateRaw, currentBootId)
    val framework = detectLsposedFramework(sections)
    val config =
        if (hooksActiveThisBoot) {
            // A current-boot hook heartbeat is stronger evidence than the on-disk
            // LSPosed DB: the module is active, and config warnings are suppressed
            // for active hooks anyway.
            null
        } else {
            when (framework) {
                LsposedFramework.NotInstalled -> {
                    LsposedConfig.ModuleNotConfigured
                }

                is LsposedFramework.Installed -> {
                    if (framework.disabled) {
                        LsposedConfig.Disabled
                    } else {
                        readLsposedConfig(context, context.packageName)
                    }
                }
            }
        }
    StartupTrace.mark("dashboard_lsposed_config_done")
    val state =
        resolveLsposedState(
            hooksActiveThisBoot = hooksActiveThisBoot,
            hookVersion = hookProps["version"],
            lsposedTargetCount = lsposedTargetCount,
            framework = framework,
            config = config,
        )
    VpnHideLog.i(
        TAG,
        "lsposed: $state (hookBootId=${hookProps["boot_id"]} currentBootId=${currentBootId.trim()} " +
            "status=${Protocol.parseStatus(lsposedStateRaw)} framework=$framework " +
            "hooksActive=$hooksActiveThisBoot config=$config)",
    )
    return LsposedFacts(
        state = state,
        framework = framework,
        config = config,
        // The install-time smoke check on the private NetworkCapabilities /
        // NetworkInfo / LinkProperties fields the hooks reflect on. Non-empty means
        // the running AOSP renamed or retyped one and the matching writeToParcel
        // hook was skipped — independent of Active/Inactive, since hooks can be live
        // with partial coverage.
        brokenFields = hookProps["broken_fields"]?.takeIf { it.isNotBlank() },
        installFailures = hookProps["install_failures"]?.takeIf { it.isNotBlank() },
        aospSdkLabel = hookProps["aosp_sdk"]?.takeIf { it.isNotBlank() } ?: "?",
    )
}

/** The device and the app's own settings — everything not owned by one module. */
private suspend fun deriveEnvironmentFacts(
    context: android.content.Context,
    sections: Map<String, String>,
    targetsSnapshot: TargetsSnapshot,
    ports: ModuleState,
    portsTargetCount: Int,
    currentBootId: String,
): EnvironmentFacts {
    val appSettings = SettingsRepository(context.applicationContext).settings.first()
    return EnvironmentFacts(
        selinuxPermissive = sections["getenforce"].orEmpty().trim().equals("Permissive", ignoreCase = true),
        // Each profile's picker only lists its own apps (getInstalledApplications is
        // per-user), so a Save from a profile that cannot see every target would
        // silently drop the rest.
        selfProfileCount =
            parsePackageUidMap(sections["pm_packages"].orEmpty())[context.packageName]
                ?.distinct()
                ?.size
                ?: 0,
        debugLoggingOn = targetsSnapshot.canonicalConfig?.debug == true,
        agentBridgeOn = appSettings.agentControlEnabled,
        suppressVersionWarnings = appSettings.suppressVersionWarnings,
        filesystemHiding =
            resolveFilesystemHidingState(
                desiredEnabled =
                    OPTIONAL_FEATURE_FILESYSTEM_IFACE_PATHS in
                        targetsSnapshot.canonicalConfig
                            ?.settings
                            ?.optionalFeatures
                            .orEmpty(),
                sections = sections,
            ),
        portsApply =
            detectPortsApplyProblem(
                ports,
                portsTargetCount,
                sections["ports_load_status"].orEmpty(),
                currentBootId,
                portsDisabled = sections["ports_disabled"].orEmpty().trim() == "1",
            ),
    )
}

/**
 * Await the check run and fold it into the protection verdict.
 *
 * The cache does all the gating (VPN off / needs-restart / self-not-routed) in
 * one fold, and `awaitTerminal` returns the terminal state itself — so the
 * reason for "no results" (blocked gate vs failed run) is carried through
 * instead of re-derived from a second VPN sensor or a raced `state.value` read.
 */
private suspend fun resolveProtectionFacts(
    context: android.content.Context,
    selfNeedsRestart: Boolean,
    modules: ModuleFacts,
    lsposedActive: Boolean,
    sections: Map<String, String>,
): ProtectionFacts {
    VpnHideLog.i(
        TAG,
        "vpnActive=${isVpnActiveFromSnapshot(sections["vpn_ifaces"].orEmpty())} " +
            "selfNeedsRestart=$selfNeedsRestart",
    )
    val nativeBackend = modules.nativeBackend
    val installedOptionalHooks =
        installedNativeOptionalHooks(nativeBackend.id, sections, modules.currentBootId)
    var report: DiagnosticReport? = null
    val check: ProtectionCheck =
        when (val terminal = DiagnosticsCache.awaitTerminal(context, selfNeedsRestart)) {
            is DiagnosticsCache.State.Blocked -> {
                ProtectionCheck.Blocked(terminal.gate)
            }

            is DiagnosticsCache.State.Ready -> {
                // Derive tiles from the one canonical report (the same object the
                // debug bundle renders), so the on-screen verdict and the exported
                // one can never diverge. Tiles judge each backend on the vectors it
                // owns; unowned leaks are left to the issue list.
                val built =
                    buildDiagnosticReport(
                        gate = DiagnosticGate.ROUTED,
                        results = terminal.results,
                        backend = nativeBackend,
                        lsposedActive = lsposedActive,
                        complete = true,
                        installedOptionalHooks = installedOptionalHooks,
                    )
                report = built
                ProtectionCheck.Checked(built.native.status, built.java.status)
            }

            // State.Failed, and defensively the never-terminal NotRun/Running: the
            // run couldn't measure — distinct from a VPN-off gate.
            else -> {
                ProtectionCheck.Failed
            }
        }
    return ProtectionFacts(
        check = check,
        report = report,
        partialHookGap = partialHookGap(nativeBackend, installedOptionalHooks),
        installedOptionalHooks = installedOptionalHooks,
    )
}

/**
 * The screen's state object, assembled from the facts it was derived from.
 *
 * Separate from [loadDashboardState] because it is pure plumbing: no decision is
 * made here, every field is either copied out of [DashboardFacts] or is the one
 * value the facts deliberately do not carry ([messages], already worded).
 */
private fun DashboardFacts.toDashboardState(
    messages: List<DashboardMessage>,
    legacyImport: LegacyImportPrompt?,
): DashboardState =
    DashboardState(
        kmod = modules.kmod.state,
        kpm = modules.kpm.state,
        zygisk = modules.zygisk.state,
        lsposed = lsposed.state,
        ports = modules.ports.state,
        nativeTargetCount = targets.native,
        portsTargetCount = targets.ports,
        nativeBackend = modules.nativeBackend,
        // Only surface the blue "what to install" card when nothing is installed
        // yet. Wrong-variant / broken / unsupported-kernel cases already emit a red
        // error with the same call to action — showing both duplicates it.
        nativeInstallRecommendation =
            kernelRecommendation?.takeIf { modules.backends.noneInstalled && !modules.standaloneKpm },
        kmodLoadStatus = modules.kmodLoadStatus,
        protection = protection.check,
        messages = messages,
        installedOptionalHooks = protection.installedOptionalHooks,
        legacyImport = legacyImport,
    )

/**
 * Everything the Dashboard shows, derived from one root snapshot.
 *
 * Reads as four steps: derive the facts, await the check run, turn the facts
 * into banners, assemble. The banner logic itself is deliberately not here —
 * [dashboardIssues] decides and [toMessage] words it, so the ~25 guards are
 * reachable from a unit test that needs no `Context`.
 */
internal suspend fun loadDashboardState(
    context: android.content.Context,
    selfNeedsRestart: Boolean,
    rootSnapshot: RootSnapshot,
): DashboardState {
    VpnHideLog.i(TAG, "=== Loading dashboard state ===")
    StartupTrace.mark("dashboard_derive_start")
    val res = context.resources
    val selfPkg = context.packageName
    val sections = rootSnapshot.sections
    val targetsSnapshot = parseTargetsSnapshot(rootSnapshot)

    fun countPackages(pkgs: Set<String>): Int = pkgs.count { it != selfPkg }
    val targets =
        TargetCounts(
            lsposed = countPackages(targetsSnapshot.lsposedTargets),
            native = countPackages(targetsSnapshot.nativeTargets),
            ports = countPackages(targetsSnapshot.portsObservers),
        )

    // Recommendation based purely on the kernel — used by the install card, the
    // "kmod-capable kernel, only zygisk installed" nudge, and wrong-variant
    // detection inside the kmod diagnosis.
    val hasKpatchRuntime = kpatchRuntimeAvailable(sections["kpatch_runtime"].orEmpty())
    val kernelRecommendation =
        buildNativeInstallRecommendation(
            sections["kernel_release"].orEmpty(),
            androidMajorVersionLabel(),
            hasKpatchRuntime,
        )
    val appVersion = BuildConfig.VERSION_NAME
    val modules = deriveModuleFacts(sections, res, kernelRecommendation, hasKpatchRuntime, appVersion)
    StartupTrace.mark("dashboard_modules_done")
    StartupTrace.mark("dashboard_kernel_done")

    val lsposed = deriveLsposedFacts(context, sections, modules.currentBootId, targets.lsposed)
    StartupTrace.mark("dashboard_lsposed_done")

    StartupTrace.mark("dashboard_protection_start")
    val protection =
        resolveProtectionFacts(
            context = context,
            selfNeedsRestart = selfNeedsRestart,
            modules = modules,
            lsposedActive = lsposed.state is LsposedState.Active,
            sections = sections,
        )
    VpnHideLog.i(TAG, "protection=${protection.check}")
    StartupTrace.mark("dashboard_protection_done")

    val environment =
        deriveEnvironmentFacts(
            context = context,
            sections = sections,
            targetsSnapshot = targetsSnapshot,
            ports = modules.ports.state,
            portsTargetCount = targets.ports,
            currentBootId = modules.currentBootId,
        )
    val facts =
        DashboardFacts(modules, lsposed, targets, environment, protection, kernelRecommendation, appVersion)
    val messages = dashboardIssues(facts).map { it.toMessage(context, res) }
    StartupTrace.mark("dashboard_issues_done")
    VpnHideLog.i(TAG, "messages=$messages")
    VpnHideLog.i(TAG, "=== Dashboard state loaded ===")

    return facts.toDashboardState(
        messages = messages,
        legacyImport = parseLegacyConfigCandidate(sections, targetsSnapshot.uidToPkg)?.toPrompt(),
    )
}
