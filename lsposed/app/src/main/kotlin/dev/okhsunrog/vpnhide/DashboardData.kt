package dev.okhsunrog.vpnhide

import android.content.pm.PackageManager
import android.database.sqlite.SQLiteDatabase
import android.os.Build
import android.os.SystemClock
import dev.okhsunrog.vpnhide.generated.HookIds
import dev.okhsunrog.vpnhide.settings.SettingsRepository
import kotlinx.coroutines.flow.first
import java.io.File

// ── Domain types — invalid states are unrepresentable ────────────────────

sealed interface ModuleState {
    data object NotInstalled : ModuleState

    data class Installed(
        val version: String?,
        val active: Boolean,
        val targetCount: Int,
        // Only populated for kmod builds that carry the stamped `gkiVariant=` field
        // in module.prop (CI-built zips from v0.6.3+). Older builds report null.
        val gkiVariant: String? = null,
        // Non-null when the module is installed but the installation itself
        // is permanently broken (distinct from "active=false" which usually
        // just means a reboot is pending). UI colors the card red.
        val brokenReason: ModuleBrokenReason? = null,
    ) : ModuleState
}

// Shared across every flashable module kind (kmod, KPM, ...) that stamps
// ModuleState.Installed.brokenReason — not just kmod, despite the name
// prefixes on the individual kmod-only cases below.
enum class ModuleBrokenReason {
    WrongVariant,
    UnsupportedKernel,
    MissingKprobes,
    UnknownVariantInactive,
    AmbiguousLoadFailed,
    SignatureEnforced,
    KpmActivatorMissing,
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

sealed interface LsposedState {
    data object NotInstalled : LsposedState

    data class InstalledInactive(
        val version: String?,
    ) : LsposedState

    data class NeedsReboot(
        val version: String?,
    ) : LsposedState

    data class Active(
        val version: String?,
        val targetCount: Int,
    ) : LsposedState
}

sealed interface ProtectionCheck {
    data object NoVpn : ProtectionCheck

    data object NeedsRestart : ProtectionCheck

    // A VPN is up, but this app is not routed through it (split-tunnelled out) —
    // hiding is moot for us, so we ask the user to add VPN Hide to the tunnel.
    data object SelfNotRouted : ProtectionCheck

    data class Checked(
        val native: LayerStatus,
        val java: LayerStatus,
    ) : ProtectionCheck
}

internal enum class FlashableModuleKind { Kmod, Kpm, Zygisk, Ports }

internal enum class MultiNativeSeverity { None, Warning, Error }

/**
 * Severity of having more than one native backend active at runtime (protocol
 * §1.5). The .ko + KPM pair is an ERROR because they wrap the same kernel
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
 * the current boot_id — see protocol §1.5 and kmod/kpm/module/post-fs-data.sh).
 *
 * This is the only reliable way to surface the .ko+KPM co-installation: the
 * [classifyMultiNative] Error needs *both* backends active, but two active
 * kernel hookers would already have frozen the device, so that state is never
 * actually observed. The deferred KPM (loaded=0) is the real, safe-but-
 * redundant state worth warning about so the user removes one.
 */
internal fun kpmDeferredForConflict(
    loadStatusSection: String,
    currentBootId: String,
): Boolean {
    val load = parseKeyValueLines(loadStatusSection)
    val bootId = load["boot_id"]?.trim()
    return load["runtime"]?.trim() == "conflict" &&
        !bootId.isNullOrEmpty() &&
        bootId == currentBootId.trim()
}

/**
 * True when the KPM boot script stood down this boot because it runs under APatch
 * and no superkey is saved yet (`runtime=apatch, loaded=0, detail=awaiting_superkey`
 * for the current boot_id — see kmod/kpm/module/post-fs-data.sh and protocol §1.5).
 *
 * Distinct from [kpmDeferredForConflict] (that writes `runtime=conflict`): here the
 * module is installed and healthy but dormant, waiting for the user to save the
 * APatch superkey in Settings so the service activator can load it. Without this,
 * the dashboard would just show KPM as inactive with no explanation.
 */
internal fun kpmAwaitingSuperkey(
    loadStatusSection: String,
    currentBootId: String,
): Boolean {
    val load = parseKeyValueLines(loadStatusSection)
    val bootId = load["boot_id"]?.trim()
    return load["runtime"]?.trim() == "apatch" &&
        load["loaded"]?.trim() == "0" &&
        load["detail"]?.trim() == "awaiting_superkey" &&
        !bootId.isNullOrEmpty() &&
        bootId == currentBootId.trim()
}

internal data class PortsApplyProblem(
    val failureDetail: String?,
)

internal fun detectPortsApplyProblem(
    ports: ModuleState,
    loadStatusSection: String,
    currentBootId: String,
    portsDisabled: Boolean,
): PortsApplyProblem? {
    val installed = ports as? ModuleState.Installed ?: return null
    if (installed.active || installed.targetCount == 0) return null
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

internal enum class DashboardMessageSeverity { ERROR, WARNING, INFO }

// Optional call-to-action rendered on a message banner. ContactAuthor opens the
// shared community/feedback modal (GitHub / Telegram / 4PDA); OpenDiagnostics
// opens the detailed diagnostics screen.
internal enum class DashboardMessageAction { ContactAuthor, OpenDiagnostics }

internal data class DashboardMessage(
    val severity: DashboardMessageSeverity,
    val text: String,
    val action: DashboardMessageAction? = null,
    // When set, the banner renders a "download this zip" button that grabs this
    // named artifact from the latest release (wrong variant, outdated module, …).
    val downloadArtifact: String? = null,
)

internal data class DashboardState(
    val kmod: ModuleState,
    val kpm: ModuleState,
    val zygisk: ModuleState,
    val lsposed: LsposedState,
    val ports: ModuleState,
    // The one native backend surfaced on the dashboard (kmod > KPM > Zygisk).
    val nativeBackend: DisplayNativeBackend,
    val nativeInstallRecommendation: NativeInstallRecommendation?,
    val kmodLoadStatus: KmodLoadStatus?,
    val protection: ProtectionCheck,
    val messages: List<DashboardMessage>,
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
    if (p is ProtectionCheck.NoVpn) return HeroStatus.VpnOff
    // 0 = protected, 1 = attention, 2 = unprotected — keep the worst signal.
    var rank = 0
    when (p) {
        ProtectionCheck.NeedsRestart, ProtectionCheck.SelfNotRouted -> {
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

// Which native backend to recommend installing for this device (protocol §1.5).
// kmod is the battle-tested default on GKI kernels; KPM (beta) is a single
// universal binary that covers non-GKI / old kernels (4.14–5.4) where no kmod
// build exists; zygisk is the detectable last resort.
internal enum class RecommendedBackend { Kmod, Kpm, Zygisk }

internal data class NativeInstallRecommendation(
    val androidVersion: String,
    val kernelVersion: String,
    val kernelBranch: String?,
    val recommended: RecommendedBackend,
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
    val preferKmod: Boolean get() = recommended == RecommendedBackend.Kmod
}

// Boot-time diagnostics written by kmod/module/post-fs-data.sh into
// /data/adb/vpnhide_kmod/load_status. Stays valid across reboots,
// so bootId is compared against the current boot to know if the
// record is fresh.
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
)

private const val TAG = LogTags.DASHBOARD

// Non-GKI kernel series KernelPatch (the KPM runtime) supports but the .ko does
// not: no GKI KMI, no DDK build. Mirrors the sub-5.10 rows of the KPM kver
// offset table in kmod/kpm/kver_offsets.h. GKI series (5.10+) prefer the .ko;
// anything below 4.14 isn't in the table and falls back to zygisk.
internal val KPM_NON_GKI_SERIES = setOf("4.14", "4.19", "5.4")

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
 *  3. Non-GKI kernels that KernelPatch covers (4.14 / 4.19 / 5.4 —
 *     no GKI KMI, no DDK kmod build) → KPM (beta), the single
 *     universal binary. [kpatchRuntimeAvailable] decides whether the
 *     UI also asks the user to install the KPatch-Next-Module first.
 *  4. Pre-4.14 series or unparseable kernel version → fall back to
 *     zygisk (recommended=Zygisk); KernelPatch's kver offset table
 *     starts at 4.14, and no kmod build loads against such kernels.
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
            recommended = RecommendedBackend.Kmod,
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
            recommended = RecommendedBackend.Kmod,
            recommendedArtifact = primary.zip,
            recommendedGkiVariant = primary.kmi,
            variantAmbiguous = alternative != null,
            alternativeArtifact = alternative?.zip,
            alternativeGkiVariant = alternative?.kmi,
        )
    }

    // Non-GKI kernels KernelPatch supports (4.14 / 4.19 / 5.4) — no GKI KMI and
    // no DDK kmod build, but they're in the KPM kver offset table
    // (kmod/kpm/kver_offsets.h). Recommend the universal KPM (beta).
    if (kernelSeries in KPM_NON_GKI_SERIES) {
        return NativeInstallRecommendation(
            androidVersion = deviceAndroidLabel,
            kernelVersion = kernelVersion,
            kernelBranch = kernelBranch,
            recommended = RecommendedBackend.Kpm,
            recommendedArtifact = "vpnhide-kpm.zip",
            recommendedGkiVariant = null,
            kpatchRuntimeAvailable = kpatchRuntimeAvailable,
        )
    }

    return NativeInstallRecommendation(
        androidVersion = deviceAndroidLabel,
        kernelVersion = kernelVersion,
        kernelBranch = kernelBranch,
        recommended = RecommendedBackend.Zygisk,
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

/** Count configured target packages, excluding the app's own package (it is
 * always present invisibly and must not inflate the user-facing count). */
internal fun countTargets(
    raw: String,
    selfPkg: String,
): Int = parseConfigLines(raw).count { it != selfPkg }

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

internal sealed interface KpmProblemKind {
    val reason: ModuleBrokenReason?

    // The activator binary itself is missing from the module directory — a
    // corrupted or partial KPM install (see kmod/kpm/module/service.sh).
    // [path] is the path the boot script tried to exec.
    data class ActivatorMissing(
        val path: String,
    ) : KpmProblemKind {
        override val reason get() = ModuleBrokenReason.KpmActivatorMissing
    }

    // The activator ran but exited non-zero for a reason we don't have a
    // named diagnosis for. [detail] is the raw `rc=<code> <captured output>`
    // the boot script wrote — the KPM analogue of kmod's raw insmod stderr.
    // Null reason (mirrors KmodProblemKind.LoadFailed): an untriaged exit
    // code isn't a confident enough diagnosis to paint the module card red.
    data class LoadFailed(
        val detail: String,
    ) : KpmProblemKind {
        override val reason: ModuleBrokenReason? get() = null
    }
}

/**
 * Diagnose a KPM install that's present but didn't load, for the
 * `runtime=activator` outcomes the boot script (kmod/kpm/module/service.sh)
 * writes. The other two runtimes it can write — `conflict` (a co-installed
 * .ko took the single-active slot) and `apatch` (dormant, awaiting a saved
 * superkey) — are expected, recoverable states already handled separately as
 * warnings by [kpmDeferredForConflict] / [kpmAwaitingSuperkey]; this function
 * only fires for `runtime=activator`, so it can never double up with those.
 * `runtime=activator` itself covers both the success case (`loaded=1`, which
 * the `!kpm.active` guard below already excludes) and the failure cases this
 * diagnoses — the KPM analogue of classifyKmodProblem's LoadFailed fallback,
 * sourced from the activator's own captured detail instead of insmod stderr.
 */
internal fun classifyKpmProblem(
    kpm: ModuleState,
    loadStatusSection: String,
    currentBootId: String,
): KpmProblemKind? {
    if (kpm !is ModuleState.Installed || kpm.active) return null
    val load = parseKeyValueLines(loadStatusSection)
    val bootId = load["boot_id"]?.trim()
    if (load["runtime"]?.trim() != "activator" ||
        load["loaded"]?.trim() != "0" ||
        bootId.isNullOrEmpty() ||
        bootId != currentBootId.trim()
    ) {
        return null
    }
    val detail = load["detail"]?.trim().orEmpty()
    val missingPrefix = "activator missing at "
    return if (detail.startsWith(missingPrefix)) {
        KpmProblemKind.ActivatorMissing(detail.removePrefix(missingPrefix))
    } else {
        KpmProblemKind.LoadFailed(detail)
    }
}

private fun renderKpmProblem(
    kind: KpmProblemKind,
    res: android.content.res.Resources,
): ModuleProblem =
    ModuleProblem(
        reason = kind.reason,
        text =
            when (kind) {
                is KpmProblemKind.ActivatorMissing -> {
                    res.getString(R.string.dashboard_issue_kpm_activator_missing, kind.path)
                }

                is KpmProblemKind.LoadFailed -> {
                    res.getString(R.string.dashboard_issue_kpm_load_failed, kind.detail)
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

internal fun detectKmodModule(
    sections: Map<String, String>,
    selfPkg: String,
): ModuleState {
    val prop = parseModuleProp(sections["kmod_prop"].orEmpty())
    if (!prop.installed) return ModuleState.NotInstalled
    val active = sections["proc_exists"].orEmpty().trim() == "1"
    // brokenReason is applied by the caller once the kernel recommendation +
    // load status are known (see classifyKmodProblem).
    return ModuleState.Installed(
        version = prop.version,
        active = active,
        targetCount = countTargets(sections["kmod_targets"].orEmpty(), selfPkg),
        gkiVariant = prop.gkiVariant,
    )
}

internal fun detectZygiskModule(
    sections: Map<String, String>,
    zygiskStatusRaw: String,
    selfPkg: String,
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
        targetCount = countTargets(sections["zygisk_targets"].orEmpty(), selfPkg),
    )
}

internal fun detectKpmModule(
    sections: Map<String, String>,
    selfPkg: String,
    currentBootId: String,
): ModuleState {
    val prop = parseModuleProp(sections["kpm_prop"].orEmpty())
    if (!prop.installed) return ModuleState.NotInstalled
    // The KPM has no /proc marker (its runtime channel is the kpatch ctl0
    // supercall). The boot script writes load_status with loaded=1 and the
    // boot_id it loaded under, so "active" = loaded for the current boot —
    // the same freshness check the zygisk heartbeat uses.
    val load = parseKeyValueLines(sections["kpm_load_status"].orEmpty())
    val bootId = load["boot_id"]?.trim()
    val active = load["loaded"]?.trim() == "1" && bootId != null && bootId == currentBootId.trim()
    return ModuleState.Installed(
        version = prop.version,
        active = active,
        targetCount = countTargets(sections["kpm_targets"].orEmpty(), selfPkg),
    )
}

internal fun detectPortsModule(
    sections: Map<String, String>,
    selfPkg: String,
): ModuleState {
    val prop = parseModuleProp(sections["ports_prop"].orEmpty())
    if (!prop.installed) return ModuleState.NotInstalled
    val active = sections["ports_chain"].orEmpty().trim() == "1"
    return ModuleState.Installed(
        version = prop.version,
        active = active,
        targetCount = countTargets(sections["ports_observers"].orEmpty(), selfPkg),
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

private fun buildModuleVersionIssue(
    res: android.content.res.Resources,
    kind: FlashableModuleKind,
    moduleVersion: String,
    appVersion: String,
    // Only meaningful for FlashableModuleKind.Kmod: the GKI-specific zip the
    // kernel recommends, so an "update the module" nudge can name the exact
    // file instead of sending the user back to KernelSU/Magisk to guess
    // which variant they originally flashed (issue #225).
    recommendedArtifact: String? = null,
): String =
    when (compareSemver(normalizeVersion(moduleVersion), normalizeVersion(appVersion))) {
        null, 0 -> {
            res.getString(
                when (kind) {
                    FlashableModuleKind.Kmod -> R.string.dashboard_issue_kmod_version_mismatch
                    FlashableModuleKind.Kpm -> R.string.dashboard_issue_kpm_version_mismatch
                    FlashableModuleKind.Zygisk -> R.string.dashboard_issue_zygisk_version_mismatch
                    FlashableModuleKind.Ports -> R.string.dashboard_issue_ports_version_mismatch
                },
                moduleVersion,
                appVersion,
            )
        }

        in Int.MIN_VALUE..-1 -> {
            if (kind == FlashableModuleKind.Kmod && recommendedArtifact != null) {
                res.getString(R.string.dashboard_issue_update_kmod_named, moduleVersion, appVersion, recommendedArtifact)
            } else {
                res.getString(
                    when (kind) {
                        FlashableModuleKind.Kmod -> R.string.dashboard_issue_update_kmod
                        FlashableModuleKind.Kpm -> R.string.dashboard_issue_update_kpm
                        FlashableModuleKind.Zygisk -> R.string.dashboard_issue_update_zygisk
                        FlashableModuleKind.Ports -> R.string.dashboard_issue_update_ports
                    },
                    moduleVersion,
                    appVersion,
                )
            }
        }

        else -> {
            res.getString(
                when (kind) {
                    FlashableModuleKind.Kmod -> R.string.dashboard_issue_update_app_for_kmod
                    FlashableModuleKind.Kpm -> R.string.dashboard_issue_update_app_for_kpm
                    FlashableModuleKind.Zygisk -> R.string.dashboard_issue_update_app_for_zygisk
                    FlashableModuleKind.Ports -> R.string.dashboard_issue_update_app_for_ports
                },
                moduleVersion,
                appVersion,
            )
        }
    }

private fun resolveScopeEntryLabel(
    context: android.content.Context,
    entry: String,
): String {
    if (entry == "system" || entry == "system/0") return "System Framework"

    val packageName = entry.substringBefore('/')
    val userId = entry.substringAfter('/', "")
    return try {
        val appInfo = context.packageManager.getApplicationInfo(packageName, 0)
        val appLabel =
            context.packageManager
                .getApplicationLabel(appInfo)
                .toString()
                .trim()
        when {
            appLabel.isEmpty() -> packageName
            userId.isNotEmpty() && userId != "0" -> "$appLabel ($userId)"
            else -> appLabel
        }
    } catch (_: PackageManager.NameNotFoundException) {
        packageName
    }
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
@Suppress("LongMethod", "CyclomaticComplexMethod")
internal suspend fun loadDashboardState(
    context: android.content.Context,
    selfNeedsRestart: Boolean,
    rootSnapshot: RootSnapshot,
): DashboardState {
    val messages = mutableListOf<DashboardMessage>()
    val res = context.resources
    val selfPkg = context.packageName

    fun err(
        text: String,
        downloadArtifact: String? = null,
    ) {
        messages += DashboardMessage(DashboardMessageSeverity.ERROR, text, downloadArtifact = downloadArtifact)
    }

    fun warn(
        text: String,
        downloadArtifact: String? = null,
    ) {
        messages += DashboardMessage(DashboardMessageSeverity.WARNING, text, downloadArtifact = downloadArtifact)
    }

    fun info(
        text: String,
        action: DashboardMessageAction? = null,
    ) {
        messages += DashboardMessage(DashboardMessageSeverity.INFO, text, action)
    }

    VpnHideLog.i(TAG, "=== Loading dashboard state ===")
    StartupTrace.mark("dashboard_derive_start")
    val shellSnapshot = rootSnapshot.sections
    val targetsSnapshot = parseTargetsSnapshot(rootSnapshot)

    fun countPackages(pkgs: Set<String>): Int = pkgs.count { it != selfPkg }

    fun ModuleState.withTargetCount(count: Int): ModuleState = if (this is ModuleState.Installed) copy(targetCount = count) else this

    // ── Module detection ──
    // Each module's state comes from a pure detector (unit-tested). kmod's
    // brokenReason is layered on below, once the kernel recommendation and
    // load status are known (classifyKmodProblem).
    val currentBootId = shellSnapshot["current_boot_id"].orEmpty()
    val nativeTargetCount = countPackages(targetsSnapshot.nativeTargets)
    val rawNativeBackends = detectNativeBackendStates(shellSnapshot, selfPkg, currentBootId)
    val kmodRaw = rawNativeBackends.kmod.withTargetCount(nativeTargetCount)
    val zygiskStatusRaw = shellSnapshot["zygisk_status"].orEmpty()
    val zygisk = rawNativeBackends.zygisk.withTargetCount(nativeTargetCount)
    val kpmRaw = rawNativeBackends.kpm.withTargetCount(nativeTargetCount)
    val ports = detectPortsModule(shellSnapshot, selfPkg).withTargetCount(countPackages(targetsSnapshot.portsObservers))
    val kmodTargetCount = (kmodRaw as? ModuleState.Installed)?.targetCount ?: 0
    val kpmTargetCount = (kpmRaw as? ModuleState.Installed)?.targetCount ?: 0
    val zygiskTargetCount = (zygisk as? ModuleState.Installed)?.targetCount ?: 0
    VpnHideLog.i(TAG, "modules: kmodRaw=$kmodRaw kpmRaw=$kpmRaw zygisk=$zygisk ports=$ports")
    StartupTrace.mark("dashboard_modules_done")

    // Recommendation based purely on the kernel — used by the install card,
    // the "kmod-capable kernel, only zygisk installed" warning (W1), and the
    // wrong-variant detection below.
    val kernelRaw = shellSnapshot["kernel_release"].orEmpty()
    val kpatchRuntimeAvailable = shellSnapshot["kpatch_runtime"].orEmpty().trim() == "1"
    val kernelRecommendation =
        buildNativeInstallRecommendation(kernelRaw, androidMajorVersionLabel(), kpatchRuntimeAvailable)
    val kmodLoadStatus =
        readKmodLoadStatus(
            currentBootId.trim(),
            shellSnapshot["kmod_load_status"].orEmpty(),
            shellSnapshot["kmod_load_dmesg"].orEmpty(),
        )
    VpnHideLog.i(TAG, "kmodLoadStatus=$kmodLoadStatus")

    // Single source of truth for "what's wrong with the installed kmod" —
    // classifyKmodProblem (pure, unit-tested) decides the priority-ordered
    // diagnosis; renderKmodProblem maps it to the localized banner text.
    // [reason] colors the card, [text] is the banner — both derive from the
    // one classification so they can't disagree.
    val kmodProblem: ModuleProblem? =
        classifyKmodProblem(kmodRaw, kernelRecommendation, kmodLoadStatus)
            ?.let { renderKmodProblem(it, res) }
    val kmod: ModuleState =
        if (kmodRaw is ModuleState.Installed && kmodProblem?.reason != null) {
            kmodRaw.copy(brokenReason = kmodProblem.reason)
        } else {
            kmodRaw
        }
    VpnHideLog.i(TAG, "kmod (with brokenReason): $kmod")

    // Same single-source-of-truth pattern as kmod above, for the KPM
    // runtime=activator failure cases classifyKpmProblem diagnoses (the
    // conflict / awaiting-superkey cases stay separate warnings below).
    val kpmProblem: ModuleProblem? =
        classifyKpmProblem(kpmRaw, shellSnapshot["kpm_load_status"].orEmpty(), currentBootId)
            ?.let { renderKpmProblem(it, res) }
    val kpm: ModuleState =
        if (kpmRaw is ModuleState.Installed && kpmProblem?.reason != null) {
            kpmRaw.copy(brokenReason = kpmProblem.reason)
        } else {
            kpmRaw
        }
    VpnHideLog.i(TAG, "kpm (with brokenReason): $kpm")

    // The one place all three backends are grouped together — every
    // "is anything installed / active" gate below reads from this instead of
    // re-deriving its own kmod/kpm/zygisk boolean combination.
    val backends = NativeBackendStates(kmod = kmod, kpm = kpm, zygisk = zygisk)
    // The single native backend the dashboard shows (kmod > KPM > Zygisk).
    val nativeBackend = displayNativeBackend(backends)
    VpnHideLog.i(TAG, "nativeBackend=$nativeBackend")
    // Only surface the blue "what to install" card when nothing is
    // installed yet. Wrong-variant / broken / unsupported-kernel cases
    // already emit a red error below with the same CTA — showing both
    // duplicates the instruction.
    val nativeInstallRecommendation = kernelRecommendation?.takeIf { backends.noneInstalled }
    VpnHideLog.i(
        TAG,
        "nativeInstallRecommendation=$nativeInstallRecommendation " +
            "(raw=$kernelRecommendation kmodProblem=$kmodProblem kpmProblem=$kpmProblem)",
    )
    StartupTrace.mark("dashboard_kernel_done")

    // lsposed runtime state
    val lsposedStateRaw = shellSnapshot["lsposed_state"].orEmpty()
    val lsposedStatus = Protocol.parseStatus(lsposedStateRaw)
    val hookProps = parseLsposedStateMetadata(lsposedStateRaw)
    val hookVersion = hookProps["version"]
    val hookBootId = hookProps["boot_id"]
    val hooksActiveThisBoot =
        lsposedStatus?.backend ==
            HookIds.Backend.LSPOSED.id
                .toLong() &&
            hookBootId != null &&
            hookBootId == currentBootId.trim()
    val lsposedTargetCount = countPackages(targetsSnapshot.lsposedTargets)
    val lsposedFramework = detectLsposedFramework(shellSnapshot)
    val lsposedConfig =
        if (hooksActiveThisBoot) {
            // A current-boot hook heartbeat is stronger evidence than the
            // on-disk LSPosed DB: the module is active, and config warnings
            // are intentionally suppressed for active hooks below.
            null
        } else {
            when (lsposedFramework) {
                LsposedFramework.NotInstalled -> {
                    LsposedConfig.ModuleNotConfigured
                }

                is LsposedFramework.Installed -> {
                    if (lsposedFramework.disabled) {
                        LsposedConfig.Disabled
                    } else {
                        readLsposedConfig(context, selfPkg)
                    }
                }
            }
        }
    StartupTrace.mark("dashboard_lsposed_config_done")
    val lsposed: LsposedState =
        resolveLsposedState(
            hooksActiveThisBoot = hooksActiveThisBoot,
            hookVersion = hookVersion,
            lsposedTargetCount = lsposedTargetCount,
            framework = lsposedFramework,
            config = lsposedConfig,
        )
    VpnHideLog.i(
        TAG,
        "lsposed: $lsposed (hookBootId=$hookBootId currentBootId=${currentBootId.trim()} " +
            "status=$lsposedStatus framework=$lsposedFramework hooksActive=$hooksActiveThisBoot config=$lsposedConfig)",
    )
    StartupTrace.mark("dashboard_lsposed_done")

    // ── Messages ──
    val hasNative = backends.anyInstalled
    if (!hasNative) {
        err(res.getString(R.string.dashboard_issue_no_native))
    }
    if (lsposedFramework is LsposedFramework.NotInstalled && lsposed !is LsposedState.Active) {
        err(res.getString(R.string.dashboard_issue_lsposed_not_installed))
    }
    if (lsposed is LsposedState.NeedsReboot) {
        err(res.getString(R.string.dashboard_issue_reboot))
    }
    // Only report LSPosed config issues when hooks are not already active at runtime —
    // if hooks are active, the config is clearly working regardless of what we detect on disk
    if (lsposed !is LsposedState.Active) {
        when (lsposedConfig) {
            null -> {
                err(res.getString(R.string.dashboard_issue_lsposed_config_unreadable))
            }

            LsposedConfig.ModuleNotConfigured -> {
                if (lsposedFramework is LsposedFramework.Installed) {
                    err(res.getString(R.string.dashboard_issue_lsposed_not_enabled))
                }
            }

            LsposedConfig.Disabled -> {
                err(res.getString(R.string.dashboard_issue_lsposed_not_enabled))
            }

            is LsposedConfig.Enabled -> {
                if (!lsposedConfig.hasSystemFramework) {
                    err(res.getString(R.string.dashboard_issue_lsposed_no_system_scope))
                }
                if (lsposedConfig.extraEntries.isNotEmpty()) {
                    // Extra entries work, they're just cosmetic noise — warn.
                    warn(
                        res.getString(
                            R.string.dashboard_issue_lsposed_extra_scope,
                            lsposedConfig.extraEntries.joinToString(", ") { resolveScopeEntryLabel(context, it) },
                        ),
                    )
                }
            }
        }
    }

    // AOSP-drift detector: HookEntry's install-time smoke-check on the
    // private NetworkCapabilities/NetworkInfo/LinkProperties fields it
    // touches by reflection. Non-empty means the running AOSP renamed
    // or retyped a field — the corresponding writeToParcel hook was
    // skipped at install time, Java-layer protection is degraded for
    // that class. Independent of lsposed Active/Inactive state: hooks
    // can still be "active" in heartbeat sense but with partial coverage.
    val brokenFields = hookProps["broken_fields"]?.takeIf { it.isNotBlank() }
    if (brokenFields != null) {
        val sdkLabel = hookProps["aosp_sdk"]?.takeIf { it.isNotBlank() } ?: "?"
        err(res.getString(R.string.dashboard_issue_lsposed_field_rename, brokenFields, sdkLabel))
    }
    val installFailures = hookProps["install_failures"]?.takeIf { it.isNotBlank() }
    if (installFailures != null && brokenFields == null) {
        err(res.getString(R.string.dashboard_issue_lsposed_install_failures, installFailures))
    }

    val appVersion = BuildConfig.VERSION_NAME
    // Version mismatches are warnings — modules keep working, user just needs to
    // update the lagging side. Full coverage is not affected by a patch-level gap.
    val moduleMismatches =
        detectModuleMismatches(
            listOf(
                kmod to FlashableModuleKind.Kmod,
                kpm to FlashableModuleKind.Kpm,
                zygisk to FlashableModuleKind.Zygisk,
                ports to FlashableModuleKind.Ports,
            ),
            appVersion,
        )
    moduleMismatches.forEach { mismatch ->
        val recommendedArtifact =
            if (mismatch.kind == FlashableModuleKind.Kmod && kernelRecommendation?.preferKmod == true) {
                kernelRecommendation.recommendedArtifact
            } else {
                null
            }
        // Offer the newer module for one-tap download only when the installed
        // module is OLDER than the app (module newer means the app is behind — the
        // fix there is updating the app, not re-flashing the module).
        val moduleOlder =
            (compareSemver(baseVersion(mismatch.moduleVersion), baseVersion(mismatch.appVersion)) ?: 0) < 0
        val downloadArtifact =
            if (moduleOlder) {
                when (mismatch.kind) {
                    FlashableModuleKind.Kmod -> recommendedArtifact
                    FlashableModuleKind.Kpm -> "vpnhide-kpm.zip"
                    FlashableModuleKind.Zygisk -> "vpnhide-zygisk.zip"
                    FlashableModuleKind.Ports -> "vpnhide-ports.zip"
                }
            } else {
                null
            }
        warn(
            buildModuleVersionIssue(res, mismatch.kind, mismatch.moduleVersion, mismatch.appVersion, recommendedArtifact),
            downloadArtifact = downloadArtifact,
        )
    }
    val totalTargets = lsposedTargetCount + kmodTargetCount + kpmTargetCount + zygiskTargetCount
    if (totalTargets == 0) {
        // A fresh, not-yet-configured install isn't broken — guide the user to add
        // apps rather than flag a red error.
        info(res.getString(R.string.dashboard_issue_no_targets))
    }
    if (ports is ModuleState.Installed && ports.targetCount == 0) {
        info(res.getString(R.string.dashboard_issue_ports_no_observers))
    }
    detectPortsApplyProblem(
        ports,
        shellSnapshot["ports_load_status"].orEmpty(),
        currentBootId,
        portsDisabled = shellSnapshot["ports_disabled"].orEmpty().trim() == "1",
    )?.let { problem ->
        val detail = problem.failureDetail
        warn(
            if (detail == null) {
                res.getString(R.string.dashboard_issue_ports_rules_inactive)
            } else {
                res.getString(R.string.dashboard_issue_ports_apply_failed, detail)
            },
        )
    }
    // The running-LSPosed-vs-installed-APK check compares the FULL version by
    // default: the hook code lives in system_server and only swaps on reboot, so
    // a dev who reinstalls the APK on the same base keeps running the old hook
    // until reboot. Developers who reinstall constantly can flip
    // suppressVersionWarnings to fall back to base-compare (release users see no
    // difference — release versions carry no dev suffix).
    val appSettings = SettingsRepository(context.applicationContext).settings.first()
    val suppressVersionWarnings = appSettings.suppressVersionWarnings
    var lsposedVersionMismatch: String? = null
    if (lsposed is LsposedState.Active) {
        val runningVersion = lsposed.version
        val mismatch =
            if (suppressVersionWarnings) {
                versionsMismatch(runningVersion, appVersion)
            } else {
                versionsMismatchFull(runningVersion, appVersion)
            }
        if (mismatch) {
            VpnHideLog.w(TAG, "version mismatch: running=$runningVersion app=$appVersion")
            lsposedVersionMismatch = res.getString(R.string.dashboard_issue_version_mismatch, runningVersion, appVersion)
        }
    }

    // ── Low-priority info: suboptimal-but-working setups ──

    // A stealthier kernel backend fits this kernel, but the user only installed
    // Zygisk. Zygisk is detectable by banking / payment apps when the Native role
    // is enabled for them, whereas kmod/KPM are invisible to anti-tamper.
    // Only nudge when the better backend is actually installable now:
    // kmod always is; KPM only when
    // a KPatch runtime is already present (else replacing a working zygisk would
    // mean installing two more things — too pushy for a low-priority hint).
    if (zygisk is ModuleState.Installed &&
        kmod is ModuleState.NotInstalled &&
        kpm is ModuleState.NotInstalled
    ) {
        when (kernelRecommendation?.recommended) {
            RecommendedBackend.Kmod -> {
                info(
                    res.getString(
                        R.string.dashboard_issue_kmod_capable_but_zygisk,
                        kernelRecommendation.recommendedArtifact,
                    ),
                )
            }

            RecommendedBackend.Kpm -> {
                if (kernelRecommendation.kpatchRuntimeAvailable) {
                    info(
                        res.getString(
                            R.string.dashboard_issue_kpm_capable_but_zygisk,
                            kernelRecommendation.recommendedArtifact,
                        ),
                    )
                }
            }

            else -> {}
        }
    }

    // The KPM backend is experimental (beta). When it's the active native
    // backend, surface a neutral note with a contact-author action so users can
    // reach the author if something misbehaves, and point them at the more
    // battle-tested alternative for their kernel (kmod on GKI, else Zygisk).
    if (moduleActive(kpm)) {
        val experimentalText =
            if (kernelRecommendation?.recommended == RecommendedBackend.Kmod) {
                res.getString(R.string.dashboard_issue_kpm_experimental_kmod)
            } else {
                res.getString(R.string.dashboard_issue_kpm_experimental_zygisk)
            }
        info(experimentalText, action = DashboardMessageAction.ContactAuthor)
    }

    // More than one native backend active. Disabled / inactive modules may
    // still have directories under /data/adb/modules; they are not a runtime
    // freeze risk and must not trigger the .ko+KPM conflict banner.
    when (
        classifyMultiNative(
            kmodActive = moduleActive(kmod),
            kpmActive = moduleActive(kpm),
            zygiskActive = moduleActive(zygisk),
        )
    ) {
        MultiNativeSeverity.Error -> {
            err(res.getString(R.string.dashboard_issue_native_conflict_kernel))
        }

        MultiNativeSeverity.Warning -> {
            warn(res.getString(R.string.dashboard_issue_multiple_native))
        }

        MultiNativeSeverity.None -> {
            // The active-pair Error above is effectively unobservable (two live
            // kernel hookers freeze the device). The KPM standing down for a
            // co-installed .ko is the real state to surface — warn so the user
            // removes one of the two kernel backends.
            if (kpmDeferredForConflict(shellSnapshot["kpm_load_status"].orEmpty(), currentBootId)) {
                warn(res.getString(R.string.dashboard_issue_native_conflict_deferred))
            }
        }
    }

    // KPM is installed under APatch but dormant because no superkey is saved yet.
    // Without this the module just reads as inactive with no reason; tell the user
    // to save the superkey so the service activator can load it.
    if (kpm is ModuleState.Installed &&
        kpmAwaitingSuperkey(shellSnapshot["kpm_load_status"].orEmpty(), currentBootId)
    ) {
        warn(res.getString(R.string.dashboard_issue_kpm_awaiting_superkey))
    }

    // User has debug logging turned on. Only adb/root can read those
    // verbose lines, so this is a neutral dashboard note rather than an issue.
    if (targetsSnapshot.canonicalConfig?.debug == true) {
        info(res.getString(R.string.dashboard_issue_debug_logging_on))
    }

    // The agent control bridge is on: a loopback HTTP server is listening, which
    // is an on-device fingerprint. Neutral note (same weight as debug logging) so
    // it isn't left running unnoticed; turn it off in Settings when done.
    if (appSettings.agentControlEnabled) {
        info(res.getString(R.string.dashboard_issue_agent_bridge_on))
    }

    // SELinux Permissive exposes six detection vectors we rely on SELinux
    // to block (RTM_GETROUTE, /proc/net/{tcp,tcp6,udp,udp6,dev,fib_trie},
    // /sys/class/net). See the coverage table in the top-level README.
    val getenforce = shellSnapshot["getenforce"].orEmpty()
    if (getenforce.trim().equals("Permissive", ignoreCase = true)) {
        warn(res.getString(R.string.dashboard_issue_selinux_permissive))
    }

    // VPN Hide installed in more than one user profile (work profile,
    // MIUI Second Space, etc.). Each instance can write to the shared
    // canonical config, but each one's app picker only sees apps from its own
    // profile (PackageManager.getInstalledApplications is per-user). A Save
    // from a profile that doesn't see all the targets would silently drop them.
    // Recommend uninstalling everywhere except the main profile.
    val selfUidCount =
        parsePackageUidMap(shellSnapshot["pm_packages"].orEmpty())[selfPkg]
            ?.distinct()
            ?.size
            ?: 0
    if (selfUidCount > 1) {
        warn(res.getString(R.string.dashboard_issue_self_multi_profile, selfUidCount))
    }

    // ── Errors: kmod / KPM variant / load problems ──
    // Each diagnosis (reason + banner text) was computed once above as
    // `kmodProblem` / `kpmProblem`; emit their text here. Only one banner per
    // backend fires, and its priority can't drift from the card color
    // because both come from the same value. classifyKpmProblem only matches
    // runtime=activator, so this can never double up with the
    // conflict/awaiting-superkey warnings below.
    kmodProblem?.let { err(it.text, it.downloadArtifact) }
    kpmProblem?.let { err(it.text) }

    // ── Protection checks ──
    StartupTrace.mark("dashboard_protection_start")
    val vpnActive = isVpnActiveFromSnapshot(shellSnapshot["vpn_ifaces"].orEmpty())
    VpnHideLog.i(TAG, "vpnActive=$vpnActive selfNeedsRestart=$selfNeedsRestart")

    // Native leaks the tile doesn't score: vectors the active backend does not own
    // (SELinux/zygisk territory), plus the Java-implemented native-level probes
    // (NetworkInterface enum, /proc/net/route via ART) which carry no root
    // differential and so aren't in nativeOutcomes. Set during the protection
    // computation, surfaced via the hero warning so a leak there is never invisible.
    var unownedNativeLeakCount = 0
    val protection: ProtectionCheck =
        when {
            !vpnActive -> {
                ProtectionCheck.NoVpn
            }

            selfNeedsRestart -> {
                ProtectionCheck.NeedsRestart
            }

            else -> {
                // Single source of truth: reuse the cached check run instead of
                // probing again here. Dashboard waits for the full Diagnostics
                // result so its "OK" state means every protection probe passed.
                val checks = DiagnosticsCache.awaitFullResults(context)
                if (checks == null) {
                    // No results to summarize. Distinguish the self-not-routed gate
                    // (VPN up but this app split-tunnelled out) from a genuine
                    // no-VPN / failed run, so the hero can guide "add to tunnel"
                    // instead of the wrong "turn on VPN".
                    if (DiagnosticsCache.state.value is DiagnosticsCache.State.SelfNotRouted) {
                        ProtectionCheck.SelfNotRouted
                    } else {
                        ProtectionCheck.NoVpn
                    }
                } else {
                    // Tiles judge each backend on the vectors it owns; unowned leaks
                    // (out of scope for the tile) are surfaced via the hero warning below.
                    val native = summarizeNativeLayer(nativeBackend, checks.nativeOutcomes)
                    val java = summarizeJavaLayer(lsposed is LsposedState.Active, checks.java)
                    // Rust-probe leaks the tile doesn't own, plus any Java-native
                    // probe (nativeExtra) that saw the VPN — the latter has no
                    // outcome, so fold its raw fails in here so they still warn.
                    unownedNativeLeakCount =
                        unownedNativeLeaks(nativeBackend, checks.nativeOutcomes) +
                        checks.nativeExtra.count { it.passed == false }
                    VpnHideLog.i(TAG, "nativeLayer=$native javaLayer=$java unownedLeaks=$unownedNativeLeakCount")
                    ProtectionCheck.Checked(native, java)
                }
            }
        }

    lsposedVersionMismatch?.let { text ->
        if (protectionFullyPassed(protection)) {
            info(text)
        } else {
            warn(text)
        }
    }

    // A hiding layer is active but some of its runtime probes still leak (native
    // partial/full, or a Java probe fails). Without this the state shows only as
    // an amber hero/tile with no explanation — surface a warning that links to
    // the full diagnostics for the per-check breakdown.
    // A leak is a leak: an active layer's owned vector leaks, or a native surface
    // the backend doesn't cover leaks (only SELinux would). Either way the VPN is
    // detectable — link to the per-check breakdown.
    val checked = protection as? ProtectionCheck.Checked
    val nativeLeaks = (checked?.native as? LayerStatus.Active)?.leaks ?: 0
    val javaLeaks = (checked?.java as? LayerStatus.Active)?.leaks ?: 0
    if (nativeLeaks > 0 || javaLeaks > 0 || unownedNativeLeakCount > 0) {
        messages +=
            DashboardMessage(
                DashboardMessageSeverity.WARNING,
                res.getString(R.string.dashboard_issue_checks_failed),
                DashboardMessageAction.OpenDiagnostics,
            )
    }

    StartupTrace.mark("dashboard_messages_done")
    VpnHideLog.i(TAG, "protection=$protection messages=$messages")
    StartupTrace.mark("dashboard_protection_done")
    VpnHideLog.i(TAG, "=== Dashboard state loaded ===")

    return DashboardState(
        kmod = kmod,
        kpm = kpm,
        zygisk = zygisk,
        lsposed = lsposed,
        ports = ports,
        nativeBackend = nativeBackend,
        nativeInstallRecommendation = nativeInstallRecommendation,
        kmodLoadStatus = kmodLoadStatus,
        protection = protection,
        messages = messages,
    )
}
