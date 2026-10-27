package dev.okhsunrog.vpnhide

import uniffi.vpnhide_core_ffi.CoreFamilyApplyOutcome
import uniffi.vpnhide_core_ffi.CoreNativeBackend
import uniffi.vpnhide_core_ffi.CoreNativeBootOutcome
import uniffi.vpnhide_core_ffi.CoreNativeBootStatus
import uniffi.vpnhide_core_ffi.CoreNativeCapacityReport
import uniffi.vpnhide_core_ffi.CorePortsSource
import uniffi.vpnhide_core_ffi.CoreSelectionMode
import uniffi.vpnhide_core_ffi.CoreZygiskTargets

internal fun CoreNativeBackend.toBackend(): NativeBackendId =
    when (this) {
        CoreNativeBackend.KMOD -> NativeBackendId.Kmod
        CoreNativeBackend.BUILTIN -> NativeBackendId.Builtin
        CoreNativeBackend.KPM -> NativeBackendId.Kpm
        CoreNativeBackend.ZYGISK -> NativeBackendId.Zygisk
    }

internal fun CoreNativeCapacityReport.toCapacity(): NativeCapacityRefusal =
    NativeCapacityRefusal(total.toInt(), capacity.toInt(), packages.map { NativeCapacityPackage(it.`package`, it.uids.map(UInt::toInt)) })

internal fun CoreNativeBootStatus.toBootStatus(): NativeBootStatus =
    NativeBootStatus(
        bootId,
        decidedAtMs.toLong(),
        outcome.toBootOutcome(),
        installed.map { it.toBackend() },
        live.map { it.toBackend() },
        staged.map { it.toBackend() },
        kmod?.let { KmodBootReport(it.kprobes, it.kretprobes, it.gkiVariant, it.version, it.uname) },
        zygiskTargets.toTargets(),
        ports?.let {
            PortsBootStatus(
                it.source.toSource(),
                it.ipv4.toFamily(),
                it.ipv6.toFamily(),
                it.targetCount.toInt(),
                it.netdReady,
                it.detail,
                it.atMs.toLong(),
            )
        },
        obsoleteModules,
    )

@Suppress("CyclomaticComplexMethod") // The generated enum is intentionally exhaustive.
private fun CoreNativeBootOutcome.toBootOutcome(): NativeBootOutcome =
    when (this) {
        is CoreNativeBootOutcome.Selected -> {
            NativeBootOutcome.Selected(backend.toBackend(), mode.toMode(), filesystemHiding)
        }

        is CoreNativeBootOutcome.Conflict -> {
            NativeBootOutcome.Conflict(
                installed.map {
                    it.toBackend()
                },
                live.map { it.toBackend() },
                staged.map { it.toBackend() },
            )
        }

        is CoreNativeBootOutcome.AwaitingAuthentication -> {
            NativeBootOutcome.AwaitingAuthentication(detail)
        }

        is CoreNativeBootOutcome.KpmUnverifiable -> {
            NativeBootOutcome.KpmUnverifiable(detail)
        }

        is CoreNativeBootOutcome.KpmIncompatible -> {
            NativeBootOutcome.KpmIncompatible(found, minimum, detail)
        }

        is CoreNativeBootOutcome.CapacityExceeded -> {
            NativeBootOutcome.CapacityExceeded(backend.toBackend(), report.toCapacity())
        }

        is CoreNativeBootOutcome.UnsupportedKernel -> {
            NativeBootOutcome.UnsupportedKernel(backend.toBackend(), uname, detail)
        }

        is CoreNativeBootOutcome.MissingArtifact -> {
            NativeBootOutcome.MissingArtifact(backend.toBackend(), path)
        }

        is CoreNativeBootOutcome.LoadFailed -> {
            NativeBootOutcome.LoadFailed(
                backend.toBackend(),
                exit,
                stderr,
                dmesgTail,
                kprobes,
                kretprobes,
                signatureRejected,
            )
        }

        is CoreNativeBootOutcome.DeliveryFailed -> {
            NativeBootOutcome.DeliveryFailed(backend.toBackend(), detail)
        }

        is CoreNativeBootOutcome.ForeignOwner -> {
            NativeBootOutcome.ForeignOwner(id.toLong())
        }

        CoreNativeBootOutcome.NoBackend -> {
            NativeBootOutcome.NoBackend
        }

        is CoreNativeBootOutcome.Unknown -> {
            NativeBootOutcome.Unknown(detail)
        }
    }

private fun CoreZygiskTargets.toTargets(): ZygiskTargetsState =
    when (this) {
        CoreZygiskTargets.NotInstalled -> ZygiskTargetsState.NotInstalled
        CoreZygiskTargets.Pending -> ZygiskTargetsState.Pending
        CoreZygiskTargets.Published -> ZygiskTargetsState.Published
        CoreZygiskTargets.Retired -> ZygiskTargetsState.Retired
        is CoreZygiskTargets.RetireFailed -> ZygiskTargetsState.RetireFailed(detail)
    }

private fun CoreSelectionMode.toMode(): NativeSelectionMode =
    when (this) {
        CoreSelectionMode.ADOPTED -> NativeSelectionMode.Adopted
        CoreSelectionMode.LOADED -> NativeSelectionMode.Loaded
    }

private fun CorePortsSource.toSource(): PortsBootSource =
    when (this) {
        CorePortsSource.BOOT -> PortsBootSource.Boot
        CorePortsSource.BOOT_RETRY -> PortsBootSource.BootRetry
        CorePortsSource.EDIT -> PortsBootSource.Edit
    }

private fun CoreFamilyApplyOutcome.toFamily(): FamilyOutcome =
    when (this) {
        CoreFamilyApplyOutcome.CONFIRMED -> FamilyOutcome.Confirmed
        CoreFamilyApplyOutcome.FAILED_KNOWN -> FamilyOutcome.FailedKnown
        CoreFamilyApplyOutcome.UNKNOWN -> FamilyOutcome.Unknown
        CoreFamilyApplyOutcome.NOT_ATTEMPTED -> FamilyOutcome.NotAttempted
    }
