package dev.okhsunrog.vpnhide

import android.content.pm.PackageManager
import android.database.sqlite.SQLiteDatabase
import android.os.Build
import android.os.SystemClock
import dev.okhsunrog.vpnhide.core.CoreProbe
import dev.okhsunrog.vpnhide.core.CoreRoot
import dev.okhsunrog.vpnhide.diagnostics.DiagnosticGate
import dev.okhsunrog.vpnhide.diagnostics.DiagnosticPresentation
import dev.okhsunrog.vpnhide.diagnostics.DiagnosticReport
import dev.okhsunrog.vpnhide.diagnostics.LayerStatus
import dev.okhsunrog.vpnhide.diagnostics.MeasurementCoverage
import dev.okhsunrog.vpnhide.diagnostics.Verdict
import dev.okhsunrog.vpnhide.diagnostics.classifyKpmProblem
import dev.okhsunrog.vpnhide.diagnostics.token
import dev.okhsunrog.vpnhide.diagnostics.verdict
import dev.okhsunrog.vpnhide.generated.HookIds
import dev.okhsunrog.vpnhide.hook.HookEntry
import dev.okhsunrog.vpnhide.picker.TargetsSnapshot
import dev.okhsunrog.vpnhide.picker.parseTargetsSnapshot
import dev.okhsunrog.vpnhide.settings.AppLanguageContext
import dev.okhsunrog.vpnhide.settings.SettingsRepository
import dev.okhsunrog.vpnhide.settings.installedNativeOptionalHooks
import dev.okhsunrog.vpnhide.settings.nativeBackendStatusError
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
        // True when an install is staged in modules_update/ awaiting the next
        // boot — the module just needs a reboot to take effect. UI colors the card orange, not red, and the dashboard
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
    ArtifactMissing,
}

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

internal enum class FlashableModuleKind { Kmod, Builtin, Kpm, Zygisk, Core }

internal enum class MultiNativeSeverity { None, Warning, Error }

/**
 * Severity of having more than one native backend active at runtime
 * (docs/storage.md §4.3). The .ko/built-in + KPM pair is an ERROR because they wrap the same kernel
 * functions and co-residence can hard-freeze the kernel; any other active
 * overlap is a WARNING (merely redundant — the backstops pick one).
 */
internal fun classifyMultiNative(
    kmodActive: Boolean,
    kpmActive: Boolean,
    zygiskActive: Boolean,
    builtinActive: Boolean = false,
): MultiNativeSeverity {
    val count = listOf(kmodActive, builtinActive, kpmActive, zygiskActive).count { it }
    return when {
        count <= 1 -> MultiNativeSeverity.None
        (kmodActive || builtinActive) && kpmActive -> MultiNativeSeverity.Error
        else -> MultiNativeSeverity.Warning
    }
}

/**
 * True when core's boot scan found the KPM to be the native candidate but KernelPatch
 * refused authentication (`NativeBootOutcome.AwaitingAuthentication` for the current boot):
 * the module is installed and healthy but dormant until a SuperKey is saved in Settings,
 * after which core rescans and loads it. Without this, the dashboard would just show KPM
 * as inactive with no explanation.
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
    lastApply: PortsBootStatus?,
    portsDisabled: Boolean,
): PortsApplyProblem? {
    val installed = ports as? ModuleState.Installed ?: return null
    if (installed.active || installed.brokenReason != null || targetCount == 0) return null
    // An inactive reading the snapshot could not verify (probe failed, shell not
    // root) is no evidence that the rules are missing: nothing to warn about.
    if (!installed.runtimeCheckable) return null
    // A module the user turned off via their manager (a `disable` marker) is
    // inactive by design — core does not start for the same reason. Don't
    // nag "iptables rules are not active" for a deliberately disabled module.
    if (portsDisabled) return null

    // Core publishes its last Ports apply this boot: a family that did not confirm
    // carries the reason the rules could be missing.
    val detail =
        lastApply?.takeIf { !it.applied }?.let { apply ->
            apply.detail.trim().ifBlank { "IPv4 ${apply.ipv4}, IPv6 ${apply.ipv6}" }
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
    // When set (and there is no download/other action), the banner shows a
    // "Learn more" button that opens this help article — the contextual entry
    // from an issue to the guide section that explains and fixes it.
    val helpArticle: String? = null,
    val reason: DashboardIssueReason? = null,
    val presentationLanguage: String? = null,
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
    // The native backend loaded but reports PARTIAL_HOOKS (a hook it needed on
    // this kernel is missing). Structural, straight from the module's status —
    // the same signal the Statistics card colours by — so the backend card's dot
    // and Statistics agree instead of one being green and the other yellow. Not
    // the alarming leak banner (that needs a measurement); this is the plain
    // "not fully hooked" fact.
    val nativePartialHooks: Boolean = false,
    val messages: List<DashboardMessage>,
    // Optional native hooks this boot actually installed. Retained so the Detailed
    // diagnostics screen can rebuild the canonical DiagnosticReport (which vectors
    // the active backend owns) rather than deriving ownership a second way.
    val installedOptionalHooks: Set<HookIds.Hook> = emptySet(),
    // A pre-1.0 config still on disk next to a config that already has roles —
    // the startup importer leaves that case to the user (LegacyConfigImport).
    val legacyImport: LegacyImportPrompt? = null,
)

internal fun protectionFullyPassed(protection: ProtectionCheck): Boolean =
    protection is ProtectionCheck.Checked &&
        (protection.native as? LayerStatus.Active)?.verdict == Verdict.Ok &&
        (protection.java as? LayerStatus.Active)?.verdict == Verdict.Ok

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

// The kmod view of core's once-per-boot native decision
// ([NativeBootStatus.kmodLoadStatus]); core loads the .ko itself and
// publishes the outcome over RPC. bootId ties it to the current boot.
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
    /** Core saw the kernel refuse the module signature (EKEYREJECTED). */
    val signatureRejected: Boolean? = null,
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
    if (signatureRejected == true) return true
    if (insmodExit == EKEYREJECTED) return true
    val stderr = insmodStderr?.lowercase() ?: return false
    return "key was rejected" in stderr || "required key not available" in stderr
}

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
    Protocol.parseStatusFields(sections["kmod_state"].orEmpty())?.backend?.toInt()

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

/** The in-tree driver owns the shared control node (backend id 0x4). */
internal fun builtinKernelPresent(sections: Map<String, String>): Boolean =
    sections["proc_exists"]?.trim() == "1" && parseCtlBackendId(sections) == BUILTIN_BACKEND_ID

// Built-in kernel backend: the driver is compiled into the kernel and has no
// module of its own; core configures it over /proc/vpnhide_ctl. It is
// "installed" exactly when the live node reports the built-in backend id (0x4),
// and then it is also active. Its version is the kernel's, so there is no
// module/app version pair to compare.
internal fun detectBuiltinModule(sections: Map<String, String>): ModuleState =
    if (builtinKernelPresent(sections)) {
        ModuleState.Installed(version = null, active = true)
    } else {
        ModuleState.NotInstalled
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
    // The KPM has no /proc marker: its runtime channel is the KernelPatch ctl0
    // supercall, which core reads into `kpm_state`. A complete status reply there
    // is the live runtime. Core selecting (or finding) the KPM this boot also counts.
    val active =
        Protocol.parseStatusFields(sections["kpm_state"].orEmpty()) != null ||
            (loadStatus.loaded == true && loadStatus.isFreshFor(currentBootId))
    // A vpnhide.kpm loaded by hand or autoloaded by APatch, without the
    // vpnhide-kpm.zip module, is a normal installation: core adopts it once its
    // version checks out. Like the built-in kernel it has no module to report.
    if (!prop.installed) return if (active) ModuleState.Installed(version = null, active = true) else ModuleState.NotInstalled
    return ModuleState.Installed(
        version = prop.version,
        active = active,
    )
}

// Ports is a core feature: the card carries the core module's version, and is
// active while core's iptables chains are in place.
internal fun detectPortsModule(sections: Map<String, String>): ModuleState {
    val prop = parseModuleProp(sections["core_prop"].orEmpty())
    if (!prop.installed) return ModuleState.NotInstalled
    // "1" = chains present and jumped to, "0" = a chain or jump is absent,
    // "error=<rc>" (or nothing) = the probe itself could not run: the xtables lock
    // was held, the shell lacked CAP_NET_ADMIN, the phase never ran. Only a "0"
    // from a root shell is evidence of an inactive module; the rest is unverified.
    val probe = sections["ports_chain"].orEmpty().trim()
    val active = probe == "1"
    return ModuleState.Installed(
        version = prop.version,
        active = active,
        runtimeCheckable = active || (probe == "0" && snapshotRuntimeCheckable(sections)),
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

// Core copies the LSPosed config DB (+ WAL/SHM) out of root-only storage into the
// app's export dir, owned by the app, so SQLiteDatabase can open it read-only.
// Returns the copied main db file, or null if core could not copy it.
private fun copyLsposedConfigDb(context: android.content.Context): File? {
    CoreRoot.attach(context)
    val copyStart = SystemClock.elapsedRealtime()
    val result = CoreRoot.collectOneBlocking(CoreProbe.LsposedModulesDb)
    StartupTrace.metric("dashboard_lsposed_db_copy", SystemClock.elapsedRealtime() - copyStart)
    val dbCopy = result.artifacts.firstOrNull { it.name == "modules_config.db" }
    if (!result.ok || dbCopy == null || !dbCopy.isFile) {
        VpnHideLog.w(TAG, "failed to copy LSPosed config db for inspection: exit=${result.exit} out=${result.text}")
        return null
    }
    VpnHideLog.i(TAG, "lsposed db copy: ${result.text.trim()}")
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
 * differed only in kind and the backend-specific classifier — which is exactly
 * the shape that lets one of them quietly drift.
 *
 * Order matters: a staged install is not a corrupt one, so [modulePendingReboot]
 * suppresses the diagnosis entirely; integrity beats the backend classifier so a
 * missing payload is not reported as a load failure.
 */
private fun deriveModuleFact(
    kind: FlashableModuleKind,
    raw: ModuleState,
    sections: Map<String, String>,
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
            )?.let { ModuleProblem.Integrity(it) }
                ?: classifyBackendProblem()
        }
    return ModuleFact(
        state = raw.withBrokenReason(problem?.reason).withPendingReboot(pendingReboot),
        problem = problem,
        pendingReboot = pendingReboot,
    )
}

/** Every module's state, as the cards show it and the banners read it. */
private fun deriveKmodFact(
    raw: NativeBackendStates,
    sections: Map<String, String>,
    recommendation: NativeInstallRecommendation?,
    loadStatus: KmodLoadStatus?,
): ModuleFact =
    deriveModuleFact(FlashableModuleKind.Kmod, raw.kmod, sections) {
        if (parseCtlBackendId(sections) == BUILTIN_BACKEND_ID) {
            null // The redundant-companion banner explains why this .ko must stay unloaded.
        } else {
            classifyKmodProblem(raw.kmod, recommendation, loadStatus)?.let { ModuleProblem.Kmod(it) }
        }
    }

private fun deriveModuleFacts(
    sections: Map<String, String>,
    kernelRecommendation: NativeInstallRecommendation?,
    hasKpatchRuntime: Boolean,
    appVersion: String,
): ModuleFacts {
    val currentBootId = sections["current_boot_id"].orEmpty()
    val nativeBoot = nativeBootStatus(sections)
    val kpmLoadStatus = nativeBoot.kpmLoadStatus()
    val raw = detectNativeBackendStates(sections, currentBootId = currentBootId, kpmLoadStatus = kpmLoadStatus)
    val portsRaw = detectPortsModule(sections)
    val kmodLoadStatus = nativeBoot.kmodLoadStatus(currentBootId)
    VpnHideLog.i(TAG, "kmodLoadStatus=$kmodLoadStatus")

    val kmod = deriveKmodFact(raw, sections, kernelRecommendation, kmodLoadStatus)
    // The in-tree backend has no module: nothing to diagnose or stage.
    val builtin = deriveModuleFact(FlashableModuleKind.Builtin, raw.builtin, sections)
    val kpm =
        deriveModuleFact(FlashableModuleKind.Kpm, raw.kpm, sections) {
            classifyKpmProblem(
                kpm = raw.kpm,
                status = kpmLoadStatus,
                currentBootId = currentBootId,
                hasKpatchRuntime = hasKpatchRuntime,
                apatchSuperkeySaved = sections["superkey_saved"]?.trim() == "1",
            )?.let { ModuleProblem.Kpm(it) }
        }
    val zygisk = deriveModuleFact(FlashableModuleKind.Zygisk, raw.zygisk, sections)
    // Ports is applied by core, so its card is the core module's fact: version
    // skew and a staged update are the core's.
    val ports = deriveModuleFact(FlashableModuleKind.Core, portsRaw, sections)

    // The one place the three native backends are grouped: every "is anything
    // installed / active" gate reads from this instead of re-deriving its own
    // kmod/kpm/zygisk boolean combination.
    val backends =
        NativeBackendStates(kmod = kmod.state, builtin = builtin.state, kpm = kpm.state, zygisk = zygisk.state)
    VpnHideLog.i(TAG, "modules: $backends ports=${ports.state}")
    return ModuleFacts(
        kmod = kmod,
        builtin = builtin,
        builtinKernelPresent = builtinKernelPresent(sections),
        kpm = kpm,
        zygisk = zygisk,
        ports = ports,
        backends = backends,
        // The single native backend the dashboard shows (kmod > builtin > KPM > Zygisk).
        nativeBackend = displayNativeBackend(backends),
        kpmLoadStatus = kpmLoadStatus,
        kmodLoadStatus = kmodLoadStatus,
        currentBootId = currentBootId,
        nativeBoot = nativeBoot,
        mismatches =
            detectModuleMismatches(
                listOf(
                    kmod.state to FlashableModuleKind.Kmod,
                    builtin.state to FlashableModuleKind.Builtin,
                    kpm.state to FlashableModuleKind.Kpm,
                    zygisk.state to FlashableModuleKind.Zygisk,
                    ports.state to FlashableModuleKind.Core,
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

/**
 * The device state not owned by any one module.
 *
 * App-local settings are deliberately absent: they are not read from root and
 * these facts are cached until an observation invalidates them, so a preference
 * in here would only take effect after the next refresh. They reach the
 * Dashboard through [DeveloperFlagsCache] instead.
 */
private suspend fun deriveEnvironmentFacts(
    context: android.content.Context,
    sections: Map<String, String>,
    targetsSnapshot: TargetsSnapshot,
    ports: ModuleState,
    portsTargetCount: Int,
): EnvironmentFacts =
    EnvironmentFacts(
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
                nativeBootStatus(sections)?.takeIf { it.isFreshFor(sections["current_boot_id"].orEmpty()) }?.ports,
                portsDisabled = sections["core_disabled"].orEmpty().trim() == "1",
            ),
        obsoleteModules = parseObsoleteModules(sections),
    )

/** Root snapshot `obsolete_modules`: one installed 1.x module id per line. */
internal fun parseObsoleteModules(sections: Map<String, String>): List<String> =
    sections[OBSOLETE_MODULES_SECTION]
        .orEmpty()
        .lines()
        .map { it.trim() }
        .filter { it.isNotEmpty() }

internal const val OBSOLETE_MODULES_SECTION = "obsolete_modules"

/**
 * Everything the Dashboard derives from one root snapshot alone: the module,
 * LSPosed, target and environment facts, the kernel recommendation, the optional
 * hooks this boot installed, and the legacy-import prompt. It costs root shells
 * and parsing, so it is cached ([DashboardCache]); the protection tiles and the
 * banners that depend on the diagnostic presentation are assembled on top of it
 * by [assembleDashboardState], a pure step that follows every presentation.
 */
internal data class DashboardRootFacts(
    val selfNeedsRestart: Boolean,
    val modules: ModuleFacts,
    val lsposed: LsposedFacts,
    val targets: TargetCounts,
    val environment: EnvironmentFacts,
    val kernelRecommendation: NativeInstallRecommendation?,
    val appVersion: String,
    val installedOptionalHooks: Set<HookIds.Hook>,
    val partialHookGap: PartialHookGap?,
    val legacyImport: LegacyImportPrompt?,
)

/**
 * Fold the diagnostic presentation into the protection facts.
 *
 * The tiles come from the one canonical report (the same object the debug bundle
 * renders), so the on-screen verdict and the exported one can never diverge;
 * [protectionVerdict] builds it from the latest complete measurement against the
 * layers it was measured with, and carries the reason for "no results" (blocked
 * gate vs failed run) through instead of re-deriving it from a second VPN sensor.
 */
private fun resolveProtectionFacts(
    facts: DashboardRootFacts,
    diagnostics: DiagnosticPresentation,
): ProtectionFacts {
    val liveLayers =
        MeasurementCoverage(facts.modules.nativeBackend, facts.installedOptionalHooks, facts.lsposed.state is LsposedState.Active)
    val verdict = protectionVerdict(diagnostics, liveLayers)
    return ProtectionFacts(
        check = verdict.check,
        report = verdict.report,
        partialHookGap = facts.partialHookGap,
        installedOptionalHooks = facts.installedOptionalHooks,
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
            kernelRecommendation?.takeIf { modules.backends.noneInstalled && !modules.builtinKernelPresent },
        kmodLoadStatus = modules.kmodLoadStatus,
        protection = protection.check,
        nativePartialHooks = protection.partialHookGap != null,
        messages = messages,
        installedOptionalHooks = protection.installedOptionalHooks,
        legacyImport = legacyImport,
    )

/**
 * Everything the Dashboard shows, from one root snapshot and one diagnostic
 * presentation: the root facts, then the assembly on top of them. The bridge
 * uses this one-shot form; the screen's cache keeps the root facts and assembles
 * on every presentation instead.
 */
internal suspend fun loadDashboardState(
    context: android.content.Context,
    selfNeedsRestart: Boolean,
    rootSnapshot: RootSnapshot,
    diagnostics: DiagnosticPresentation,
): DashboardState {
    val facts = deriveDashboardRootFacts(context, selfNeedsRestart, rootSnapshot)
    val developer = SettingsRepository(context.applicationContext).settings.first().toDeveloperFlags()
    return assembleDashboardState(context, facts, diagnostics, developer)
}

/**
 * The root-derived half of the Dashboard. The banner logic itself is
 * deliberately not here — [dashboardIssues] decides and [toMessage] words it, so
 * the ~25 guards are reachable from a unit test that needs no `Context`.
 */
internal suspend fun deriveDashboardRootFacts(
    context: android.content.Context,
    selfNeedsRestart: Boolean,
    rootSnapshot: RootSnapshot,
): DashboardRootFacts {
    VpnHideLog.i(TAG, "=== Loading dashboard state ===")
    StartupTrace.mark("dashboard_derive_start")
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
    val modules = deriveModuleFacts(sections, kernelRecommendation, hasKpatchRuntime, appVersion)
    StartupTrace.mark("dashboard_modules_done")
    StartupTrace.mark("dashboard_kernel_done")

    val lsposed = deriveLsposedFacts(context, sections, modules.currentBootId, targets.lsposed)
    StartupTrace.mark("dashboard_lsposed_done")

    val environment =
        deriveEnvironmentFacts(
            context = context,
            sections = sections,
            targetsSnapshot = targetsSnapshot,
            ports = modules.ports.state,
            portsTargetCount = targets.ports,
        )
    val installedOptionalHooks = installedNativeOptionalHooks(modules.nativeBackend.id, sections, modules.currentBootId)
    val nativeStatusError = nativeBackendStatusError(modules.nativeBackend.id, sections)
    VpnHideLog.i(TAG, "selfNeedsRestart=$selfNeedsRestart")
    VpnHideLog.i(TAG, "=== Dashboard root facts loaded ===")
    return DashboardRootFacts(
        selfNeedsRestart = selfNeedsRestart,
        modules = modules,
        lsposed = lsposed,
        targets = targets,
        environment = environment,
        kernelRecommendation = kernelRecommendation,
        appVersion = appVersion,
        installedOptionalHooks = installedOptionalHooks,
        partialHookGap = partialHookGap(modules.nativeBackend, installedOptionalHooks, nativeStatusError),
        legacyImport = parseLegacyConfigCandidate(sections, targetsSnapshot.uidToPkg)?.toPrompt(),
    )
}

/**
 * The presentation-dependent half: the protection tiles, the banners and the
 * screen state, assembled from cached root facts and the presentation of the
 * moment. Pure apart from wording the messages, so it can follow every
 * presentation change and the tiles can never lag behind the hero.
 */
internal fun assembleDashboardState(
    context: android.content.Context,
    facts: DashboardRootFacts,
    diagnostics: DiagnosticPresentation,
    developer: DeveloperFlags,
): DashboardState {
    val localized = AppLanguageContext.localized(context)
    val protection = resolveProtectionFacts(facts, diagnostics)
    val dashboardFacts =
        DashboardFacts(
            facts.modules,
            facts.lsposed,
            facts.targets,
            facts.environment,
            protection,
            facts.kernelRecommendation,
            facts.appVersion,
            developer,
        )
    val messages =
        dashboardIssues(dashboardFacts).map { issue ->
            issue.toMessage(localized, localized.resources).copy(
                reason = issue.toReason(),
                presentationLanguage =
                    localized.resources.configuration.locales[0]
                        .toLanguageTag(),
            )
        }
    VpnHideLog.i(TAG, "protection=${protection.check} messages=$messages")
    return dashboardFacts.toDashboardState(messages = messages, legacyImport = facts.legacyImport)
}
