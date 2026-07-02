package dev.okhsunrog.vpnhide

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.File

class StorageConfigTest {
    @Test
    fun `native hook entries are raw backend hooks`() {
        assertEquals(
            listOf(
                "fib_route_seq_show",
                "ipv6_route_seq_show",
                "rtnl_fill_ifinfo",
                "inet_fill_ifaddr",
                "inet6_fill_ifaddr",
                "dev_ioctl",
                "sock_ioctl",
                "fib_dump_info",
                "rt6_fill_node",
                "fib_nl_fill_rule",
            ),
            NativeKernelHookEntries.map { it.hookName },
        )
        assertEquals(
            listOf(
                "zygisk_ioctl",
                "zygisk_getifaddrs",
                "zygisk_openat",
                "zygisk_recvmsg",
                "zygisk_recv",
                "zygisk_recvfrom",
                "zygisk_recvfrom_chk",
            ),
            ZygiskNativeHookEntries.map { it.hookName },
        )
    }

    @Test
    fun `canonical config parses roles settings and hook lists`() {
        val cfg =
            requireNotNull(
                parseCanonicalConfig(
                    """
                    {
                      "version": 1,
                      "debug": true,
                      "apps": {
                        "com.bank": {
                          "java": ["lsposed_network_capabilities"],
                          "native": ["sock_ioctl"],
                          "appHiding": false,
                          "ports": true
                        },
                        "dev.okhsunrog.vpnhide": { "hidden": true }
                      },
                      "settings": {
                        "rememberSuperkey": true,
                        "autoHideVpnServices": false,
                        "autoHideVpnName": true,
                        "autoHiddenPackages": ["com.vpn.client"]
                      }
                    }
                    """.trimIndent(),
                ),
            )

        assertTrue(cfg.debug)
        assertEquals(
            CanonicalSettings(
                rememberSuperkey = true,
                autoHideVpnServices = false,
                autoHideVpnName = true,
                autoHiddenPackages = setOf("com.vpn.client"),
            ),
            cfg.settings,
        )
        assertTrue(cfg.apps.getValue("com.bank").java)
        assertEquals(listOf("lsposed_network_capabilities"), cfg.apps.getValue("com.bank").javaHooks)
        assertEquals(
            NativeRole(
                enabled = true,
                overrides = NativeHookOverrides(kernel = listOf("sock_ioctl")),
            ),
            cfg.apps.getValue("com.bank").native,
        )
        assertTrue(cfg.apps.getValue("dev.okhsunrog.vpnhide").hidden)
    }

    @Test
    fun `canonical config migrates debugSwitch from legacy debug`() {
        val cfg =
            requireNotNull(
                parseCanonicalConfig(
                    """
                    {
                      "version": 1,
                      "debug": true,
                      "apps": {}
                    }
                    """.trimIndent(),
                ),
            )

        assertEquals(true, cfg.debug)
        assertEquals(true, cfg.debugSwitch)
    }

    @Test
    fun `canonical config reads explicit debugSwitch independently of debug`() {
        val cfg =
            requireNotNull(
                parseCanonicalConfig(
                    """
                    {
                      "version": 1,
                      "debug": true,
                      "debugSwitch": false,
                      "apps": {}
                    }
                    """.trimIndent(),
                ),
            )

        assertEquals(true, cfg.debug)
        assertEquals(false, cfg.debugSwitch)
    }

    @Test
    fun `canonical config parses backend specific native hook overrides`() {
        val cfg =
            requireNotNull(
                parseCanonicalConfig(
                    """
                    {
                      "version": 1,
                      "apps": {
                        "com.bank": {
                          "native": {
                            "enabled": true,
                            "kernel": ["sock_ioctl"],
                            "zygisk": ["zygisk_ioctl", "zygisk_recvfrom_chk"]
                          }
                        }
                      }
                    }
                    """.trimIndent(),
                ),
            )

        assertEquals(
            NativeRole(
                enabled = true,
                overrides =
                    NativeHookOverrides(
                        kernel = listOf("sock_ioctl"),
                        zygisk = listOf("zygisk_ioctl", "zygisk_recvfrom_chk"),
                    ),
            ),
            cfg.apps.getValue("com.bank").native,
        )
        assertEquals(cfg, requireNotNull(parseCanonicalConfig(canonicalConfigJson(cfg))))
    }

    @Test
    fun `canonical config parses custom ports policy`() {
        val cfg =
            requireNotNull(
                parseCanonicalConfig(
                    """
                    {
                      "version": 1,
                      "apps": {
                        "com.bank": {
                          "ports": true,
                          "portPolicy": {
                            "mode": "custom",
                            "rules": [
                              { "protocol": "tcp", "start": 7890, "end": 7892 },
                              { "start": 1080 }
                            ]
                          }
                        }
                      }
                    }
                    """.trimIndent(),
                ),
            )

        assertEquals(
            PortPolicy(
                mode = PortPolicyMode.Custom,
                rules =
                    listOf(
                        PortRule(start = 1080),
                        PortRule(protocol = PortProtocol.Tcp, start = 7890, end = 7892),
                    ),
            ),
            cfg.apps.getValue("com.bank").portPolicy,
        )
    }

    @Test
    fun `canonical config defaults new auto hide settings for old json`() {
        val cfg =
            requireNotNull(
                parseCanonicalConfig(
                    """
                    {
                      "version": 1,
                      "apps": {},
                      "settings": { "rememberSuperkey": true }
                    }
                    """.trimIndent(),
                ),
            )

        assertEquals(
            CanonicalSettings(
                rememberSuperkey = true,
                autoHideVpnServices = true,
                autoHideVpnName = false,
                autoHiddenPackages = emptySet(),
            ),
            cfg.settings,
        )
    }

    @Test
    fun `parses shared storage fixture`() {
        val cfg = requireNotNull(parseCanonicalConfig(sharedStorageFixture()))

        assertTrue(cfg.debug)
        assertEquals(CanonicalSettings(rememberSuperkey = true), cfg.settings)
        assertEquals(NativeRole.All, cfg.apps.getValue("com.example.bank").native)
        val proxy = cfg.apps.getValue("org.example.proxy")
        assertEquals(
            NativeRole(
                enabled = true,
                overrides = NativeHookOverrides(kernel = listOf("fib_route_seq_show", "sock_ioctl")),
            ),
            proxy.native,
        )
        // Per-hook Java selection: the same array shape the native activator
        // must tolerate (it parses but ignores Java; LSPosed self-read acts on it).
        assertTrue(proxy.java)
        assertEquals(listOf("lsposed_network_capabilities", "lsposed_network_info"), proxy.javaHooks)
        assertTrue(cfg.apps.getValue("dev.okhsunrog.vpnhide").hidden)
    }

    @Test
    fun `builder preserves an existing native hook list when role remains enabled`() {
        val existing =
            CanonicalConfig(
                apps =
                    mapOf(
                        "com.bank" to
                            CanonicalApp(
                                native =
                                    NativeRole(
                                        enabled = true,
                                        overrides = NativeHookOverrides(kernel = listOf("sock_ioctl")),
                                    ),
                            ),
                    ),
            )

        val cfg =
            buildCanonicalConfig(
                debug = false,
                javaPkgs = emptySet(),
                nativePkgs = setOf("com.bank", "com.new"),
                hiddenPkgs = emptySet(),
                observerPkgs = emptySet(),
                portsPkgs = emptySet(),
                existing = existing,
            )

        assertEquals(
            NativeRole(
                enabled = true,
                overrides = NativeHookOverrides(kernel = listOf("sock_ioctl")),
            ),
            cfg.apps.getValue("com.bank").native,
        )
        assertEquals(NativeRole.All, cfg.apps.getValue("com.new").native)
    }

    @Test
    fun `builder preserves an existing java hook list when role remains enabled`() {
        val existing =
            CanonicalConfig(
                apps =
                    mapOf(
                        "com.bank" to
                            CanonicalApp(
                                java = true,
                                javaHooks = listOf("lsposed_network_capabilities"),
                            ),
                    ),
            )

        val cfg =
            buildCanonicalConfig(
                debug = false,
                javaPkgs = setOf("com.bank", "com.new"),
                nativePkgs = emptySet(),
                hiddenPkgs = emptySet(),
                observerPkgs = emptySet(),
                portsPkgs = emptySet(),
                existing = existing,
            )

        assertEquals(listOf("lsposed_network_capabilities"), cfg.apps.getValue("com.bank").javaHooks)
        assertEquals(null, cfg.apps.getValue("com.new").javaHooks)
    }

    @Test
    fun `builder preserves an existing ports policy when role remains enabled`() {
        val policy = requireNotNull(portPolicyForPreset(PORT_PRESET_COMMON_PROXY))
        val existing =
            CanonicalConfig(
                apps =
                    mapOf(
                        "com.proxy" to CanonicalApp(ports = true, portPolicy = policy),
                    ),
            )

        val cfg =
            buildCanonicalConfig(
                debug = false,
                javaPkgs = emptySet(),
                nativePkgs = emptySet(),
                hiddenPkgs = emptySet(),
                observerPkgs = emptySet(),
                portsPkgs = setOf("com.proxy", "com.new"),
                existing = existing,
            )

        assertEquals(policy, cfg.apps.getValue("com.proxy").portPolicy)
        assertEquals(null, cfg.apps.getValue("com.new").portPolicy)
    }

    @Test
    fun `canonical JSON is deterministic and round trips through parser`() {
        val cfg =
            CanonicalConfig(
                debug = true,
                debugSwitch = true,
                apps =
                    buildCanonicalConfig(
                        debug = true,
                        debugSwitch = true,
                        javaPkgs = setOf("com.java"),
                        nativePkgs = setOf("com.native"),
                        hiddenPkgs = setOf("dev.okhsunrog.vpnhide"),
                        observerPkgs = setOf("com.observer"),
                        portsPkgs = setOf("com.ports"),
                    ).apps +
                        (
                            "com.ports.preset" to
                                CanonicalApp(
                                    ports = true,
                                    portPolicy = requireNotNull(portPolicyForPreset(PORT_PRESET_COMMON_PROXY)),
                                )
                        ),
            )

        val reparsed = requireNotNull(parseCanonicalConfig(canonicalConfigJson(cfg)))

        assertEquals(cfg, reparsed)
    }

    @Test
    fun `canonical json serializes both debug flags`() {
        val cfg = CanonicalConfig(debug = true, debugSwitch = false)
        val json = canonicalConfigJson(cfg)

        assertTrue(json.contains("\"debug\": true"))
        assertTrue(json.contains("\"debugSwitch\": false"))
        assertEquals(cfg, requireNotNull(parseCanonicalConfig(json)))
    }

    @Test
    fun `self target merge adds java native and hidden without dropping settings`() {
        val cfg =
            CanonicalConfig(
                settings = CanonicalSettings(rememberSuperkey = true),
                apps = mapOf("com.bank" to CanonicalApp(java = true)),
            )

        val updated = canonicalConfigWithSelfTarget(cfg, "dev.okhsunrog.vpnhide")

        assertEquals(CanonicalSettings(rememberSuperkey = true), updated.settings)
        assertEquals(
            CanonicalApp(java = true, native = NativeRole.All, hidden = true),
            updated.apps.getValue("dev.okhsunrog.vpnhide"),
        )
    }

    @Test
    fun `self target merge forces full hook roles`() {
        val cfg =
            CanonicalConfig(
                apps =
                    mapOf(
                        "dev.okhsunrog.vpnhide" to
                            CanonicalApp(
                                java = true,
                                javaHooks = listOf("lsposed_network_capabilities"),
                                native =
                                    NativeRole(
                                        enabled = true,
                                        overrides = NativeHookOverrides(kernel = listOf("sock_ioctl")),
                                    ),
                            ),
                    ),
            )

        val updated = canonicalConfigWithSelfTarget(cfg, "dev.okhsunrog.vpnhide")

        assertEquals(
            CanonicalApp(java = true, native = NativeRole.All, hidden = true),
            updated.apps.getValue("dev.okhsunrog.vpnhide"),
        )
    }

    @Test
    fun `import parser accepts canonical json and adds self target`() {
        val cfg =
            requireNotNull(
                parseImportedCanonicalConfig(
                    """
                    {
                      "version": 1,
                      "debug": true,
                      "apps": {
                        "com.bank": { "java": true }
                      },
                      "settings": { "rememberSuperkey": true }
                    }
                    """.trimIndent(),
                    selfPkg = "dev.okhsunrog.vpnhide",
                ),
            )

        assertTrue(cfg.debug)
        assertTrue(cfg.apps.getValue("com.bank").java)
        assertEquals(CanonicalSettings(rememberSuperkey = true), cfg.settings)
        assertEquals(
            CanonicalApp(java = true, native = NativeRole.All, hidden = true),
            cfg.apps.getValue("dev.okhsunrog.vpnhide"),
        )
    }

    @Test
    fun `hook role masks honor partial java hook selections`() {
        val mask =
            hookSelectionMask(
                enabled = true,
                hooks = listOf("lsposed_network_capabilities", "unknown_hook"),
                entries = LsposedJavaHookEntries,
            )

        assertEquals(1L shl 11, mask)
        assertEquals(
            (1L shl 10) or (1L shl 11) or (1L shl 12) or (1L shl 13) or
                (1L shl 14) or (1L shl 15) or (1L shl 16),
            hookSelectionMask(enabled = true, hooks = null, entries = LsposedJavaHookEntries),
        )
    }

    @Test
    fun `import parser rejects invalid or unsupported json`() {
        assertEquals(null, parseImportedCanonicalConfig("", "dev.okhsunrog.vpnhide"))
        assertEquals(null, parseImportedCanonicalConfig("{not-json", "dev.okhsunrog.vpnhide"))
        assertEquals(null, parseImportedCanonicalConfig("""{"version": 999, "apps": {}}""", "dev.okhsunrog.vpnhide"))
    }

    @Test
    fun `snapshot builder folds legacy roles into canonical config`() {
        val snapshot =
            TargetsSnapshot(
                kmodModuleInstalled = true,
                kpmModuleInstalled = false,
                zygiskModuleInstalled = false,
                portsModuleInstalled = true,
                kmodTargets = setOf("com.native"),
                kpmTargets = emptySet(),
                zygiskTargets = emptySet(),
                lsposedTargets = setOf("com.java"),
                hiddenPkgs = setOf("com.hidden"),
                observerUids = setOf(10123),
                portsObservers = setOf("com.ports"),
                uidToPkg = mapOf(10123 to "com.observer"),
                canonicalConfig = null,
            )

        val cfg = buildCanonicalConfigFromTargetsSnapshot(snapshot, debug = true)

        assertTrue(cfg.debug)
        assertTrue(cfg.apps.getValue("com.java").java)
        assertEquals(NativeRole.All, cfg.apps.getValue("com.native").native)
        assertTrue(cfg.apps.getValue("com.hidden").hidden)
        assertTrue(cfg.apps.getValue("com.observer").appHiding)
        assertTrue(cfg.apps.getValue("com.ports").ports)
    }

    @Test
    fun `legacy cleanup removes retired config inputs only`() {
        val cmd = buildLegacyConfigCleanupCommand()

        listOf(
            KMOD_TARGETS,
            KPM_TARGETS,
            ZYGISK_TARGETS,
            LSPOSED_TARGETS,
            PORTS_OBSERVERS_FILE,
            SS_HIDDEN_PKGS_FILE,
            SS_OBSERVER_UIDS_FILE,
            "/data/system/vpnhide_uids.txt",
        ).forEach { path -> assertTrue(cmd.contains(path)) }

        assertTrue(!cmd.contains(CANONICAL_CONFIG_FILE))
        assertTrue(!cmd.contains(SUPERKEY_FILE))
    }

    private fun sharedStorageFixture(): String =
        listOf(
            File("../../testdata/storage_config_v1.json"),
            File("../testdata/storage_config_v1.json"),
            File("testdata/storage_config_v1.json"),
        ).first(File::isFile).readText()
}
