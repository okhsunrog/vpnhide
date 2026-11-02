package dev.okhsunrog.vpnhide.hook

import android.os.Process
import android.os.SystemClock
import android.system.Os
import android.system.OsConstants
import dev.okhsunrog.vpnhide.BuildConfig
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put
import java.io.File
import java.io.FileInputStream
import java.io.FileOutputStream
import java.util.UUID

internal const val FRAMEWORK_AUTHORITY_FILE = "vpnhide-diagnostic-framework.json"
private const val FRAMEWORK_ACK_FILE = "vpnhide-diagnostic-framework-ack.json"
private const val FRAMEWORK_DIRECTORY = "/data/system"

internal fun fixtureStartTicks(pid: Int): Long? =
    runCatching {
        val text = File("/proc/$pid/stat").readText()
        text.substring(text.lastIndexOf(')') + 2).split(' ')[19].toLong()
    }.getOrNull()

internal class FrameworkFixtureChannel {
    private val boot = runCatching { File("/proc/sys/kernel/random/boot_id").readText().trim() }.getOrNull()

    fun read(): FixtureAuthority? =
        runCatching {
            val path = "$FRAMEWORK_DIRECTORY/$FRAMEWORK_AUTHORITY_FILE"
            val fd = Os.open(path, OsConstants.O_RDONLY or OsConstants.O_CLOEXEC or OsConstants.O_NOFOLLOW or OsConstants.O_NONBLOCK, 0)
            val bytes =
                FileInputStream(fd).use { stream ->
                    val before = Os.fstat(fd)
                    require(OsConstants.S_ISREG(before.st_mode) && before.st_uid == 0 && before.st_nlink == 1L)
                    require(before.st_mode and 18 == 0 && before.st_size in 1..4096)
                    val bytes = stream.readBytesBounded()
                    val after = Os.fstat(fd)
                    val linked = Os.lstat(path)
                    require(before.st_dev == after.st_dev && before.st_ino == after.st_ino && before.st_size == after.st_size)
                    require(before.st_mtime == after.st_mtime && before.st_ctime == after.st_ctime)
                    require(before.st_dev == linked.st_dev && before.st_ino == linked.st_ino && before.st_uid == linked.st_uid)
                    bytes
                }
            requireNotNull(parseFixtureAuthority(bytes.decodeToString(throwOnInvalidSequence = true))).also {
                require(it.boot == boot && it.hookHash == BuildConfig.HOOK_SOURCE_SHA256)
                val now = SystemClock.elapsedRealtime()
                require(now < it.expiry && it.expiry - now <= 25_000)
            }
        }.getOrNull()

    private fun FileInputStream.readBytesBounded(): ByteArray {
        val buffer = ByteArray(4097)
        var size = 0
        while (size < buffer.size) {
            val count = read(buffer, size, buffer.size - size)
            if (count < 0) break
            size += count
        }
        require(size in 1..4096)
        return buffer.copyOf(size)
    }

    fun acknowledge(
        authority: FixtureAuthority,
        objects: FrameworkFixtureObjects,
        evidence: FixtureEvidenceSnapshot,
    ) {
        check(read() == authority) { "fixture authority unavailable" }
        val json = encode(authority, objects, evidence)
        require(json.size <= 4096)
        val temporary = "$FRAMEWORK_DIRECTORY/.vpnhide-framework-${UUID.randomUUID()}"
        val fd =
            Os.open(
                temporary,
                OsConstants.O_WRONLY or OsConstants.O_CLOEXEC or OsConstants.O_NOFOLLOW or OsConstants.O_CREAT or OsConstants.O_EXCL,
                416,
            )
        try {
            FileOutputStream(fd).use {
                it.write(json)
                it.flush()
                it.fd.sync()
            }
            check(read() == authority) { "fixture authority changed" }
            Os.rename(temporary, "$FRAMEWORK_DIRECTORY/$FRAMEWORK_ACK_FILE")
            val directory =
                Os.open(
                    FRAMEWORK_DIRECTORY,
                    OsConstants.O_RDONLY or OsConstants.O_CLOEXEC or OsConstants.O_NOFOLLOW,
                    0,
                )
            try {
                require(OsConstants.S_ISDIR(Os.fstat(directory).st_mode))
                Os.fsync(directory)
            } finally {
                Os.close(directory)
            }
        } finally {
            runCatching { Os.remove(temporary) }
        }
    }

    private fun encodeLegacy(evidence: FixtureEvidenceSnapshot) =
        buildJsonObject {
            val flags = evidence.flags
            put("type_calls", evidence.legacyTypeCalls)
            put("enumeration_calls", evidence.legacyEnumerationCalls)
            put("type_started_boot_ns", evidence.legacyTypeStartedNs)
            put("enumeration_started_boot_ns", evidence.legacyEnumerationStartedNs)
            put("type_source", "legacy_type_source" in flags)
            put("enumeration_source", "legacy_enumeration_source" in flags)
            put("type_normalized", "legacy_type_normalized" in flags)
            put("enumeration_normalized", "legacy_enumeration_normalized" in flags)
            put("enumeration_count", evidence.legacyBaseline?.count)
            put(
                "ordinary",
                evidence.legacyBaseline?.ordinary?.let { ordinary ->
                    buildJsonObject {
                        put("count", ordinary.count)
                        put("sha256", ordinary.sha256)
                        put("fixture_present", ordinary.fixturePresent)
                    }
                } ?: JsonNull,
            )
        }

    private fun encode(
        authority: FixtureAuthority,
        objects: FrameworkFixtureObjects,
        evidence: FixtureEvidenceSnapshot,
    ): ByteArray =
        buildJsonObject {
            val flags = evidence.flags
            put("version", 2)
            put("session", authority.session)
            put("core_epoch", authority.epoch)
            put("nonce", authority.nonce)
            put("generation", authority.generation)
            put("phase", authority.phase.name.lowercase())
            put("worker_uid", authority.worker.uid)
            put("worker_pid", authority.worker.pid)
            put("worker_start_ticks", authority.worker.startTicks)
            put("system_server_pid", Process.myPid())
            put("system_server_start_ticks", requireNotNull(fixtureStartTicks(Process.myPid())))
            put("hook_sha256", BuildConfig.HOOK_SOURCE_SHA256)
            put(
                "sources",
                buildJsonObject {
                    put("vpn_handle", objects.vpn.networkHandle)
                    put("ordinary_handle", objects.ordinary.networkHandle)
                    put("profile_sha256", objects.profileHash)
                },
            )
            put(
                "fixture",
                buildJsonObject {
                    val spec = authority.objects
                    put("vpn_net_id", spec.vpnNetId)
                    put("ordinary_net_id", spec.ordinaryNetId)
                    put("interface_name", spec.interfaceName)
                    put("mtu", spec.mtu)
                    put("ordinary_interface_name", spec.ordinaryInterfaceName)
                    put("ordinary_mtu", spec.ordinaryMtu)
                },
            )
            put("legacy", encodeLegacy(evidence))
            for (name in listOf(
                "networks_source",
                "capabilities_source",
                "properties_source",
                "networks_filtered",
                "capabilities_filtered",
                "properties_filtered",
            )) {
                put(
                    name,
                    name in flags,
                )
            }
        }.toString().toByteArray()
}
