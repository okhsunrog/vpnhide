package dev.okhsunrog.vpnhide

import dev.okhsunrog.vpnhide.generated.HookIds
import org.junit.Assert.assertEquals
import org.junit.Test

class LsposedStateTest {
    @Test
    fun `state file carries protocol status metadata and stats`() {
        val text =
            formatLsposedState(
                status =
                    Protocol.Status(
                        backend =
                            HookIds.Backend.LSPOSED.id
                                .toLong(),
                        kver = 0,
                        hooks = HookIds.LSPOSED_HOOK_MASK.toLong(),
                        error =
                            HookIds.StatusError.OK.code
                                .toLong(),
                    ),
                metadata =
                    linkedMapOf(
                        LsposedStateMetadata.VERSION to "1.2.3",
                        LsposedStateMetadata.BOOT_ID to "boot-123",
                        LsposedStateMetadata.INSTALL_FAILURES to "NC.writeToParcel: NoSuchMethodError",
                    ),
                stats =
                    listOf(
                        Protocol.StatEntry(
                            uid = 10123,
                            hookId =
                                HookIds.Hook.LSPOSED_NETWORK_CAPABILITIES.id
                                    .toLong(),
                            count = 7,
                        ),
                    ),
            )

        assertEquals(
            Protocol.Status(
                backend =
                    HookIds.Backend.LSPOSED.id
                        .toLong(),
                kver = 0,
                hooks = HookIds.LSPOSED_HOOK_MASK.toLong(),
                error =
                    HookIds.StatusError.OK.code
                        .toLong(),
            ),
            Protocol.parseStatus(text),
        )
        assertEquals(
            mapOf(
                LsposedStateMetadata.VERSION to "1.2.3",
                LsposedStateMetadata.BOOT_ID to "boot-123",
                LsposedStateMetadata.INSTALL_FAILURES to "NC.writeToParcel: NoSuchMethodError",
            ),
            parseLsposedStateMetadata(text),
        )
        assertEquals(
            "vpnhide 1 stats\n0x278b 0xb:0x7\n",
            text.substringAfter("vpnhide 1 stats\n").let { "vpnhide 1 stats\n$it" },
        )
    }
}
