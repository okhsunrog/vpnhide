use super::*;
use std::collections::{BTreeMap, BTreeSet};
use std::os::unix::process::ExitStatusExt;

use vpnhide_apatch_abi::{
    CommandStyle as ApatchCommandStyle, encode_command as supercall_cmd,
    parse_kernel_version_hint as parse_apatch_kernel_version_hint,
};
use vpnhide_protocol::hook_ids::{Hook, ZYGISK_HOOK_MASK};
use vpnhide_protocol::{KPM_ARGS_LEN, Target, format_config};

#[test]
fn parses_android_kernel_release_families_without_prefix_matches() {
    assert_eq!(parse_kernel_family("4.9"), Some((4, 9)));
    assert_eq!(parse_kernel_family("4.9.337-gki"), Some((4, 9)));
    assert_eq!(
        parse_kernel_family("6.1.128-android14-11-g123456789abc-ab12345678"),
        Some((6, 1)),
    );
    assert_eq!(parse_kernel_family("6.12+"), Some((6, 12)));
    assert_eq!(parse_kernel_family("6.10.2"), Some((6, 10)));
    assert_eq!(parse_kernel_family("5.100.1"), Some((5, 100)));
    assert_eq!(parse_kernel_family("Linux 6.1.2"), None);
    assert_eq!(parse_kernel_family("6.1rc1"), None);
    assert_eq!(parse_kernel_family("6"), None);
    assert!(kernel_release_supports_kpm("6.1.128-android14-ab123"));
    assert!(!kernel_release_supports_kpm("6.10.2-mainline"));
    assert!(!kernel_release_supports_kpm("Linux 6.1.2"));
}

#[test]
fn kpm_supported_families_match_the_offset_selector() {
    let header = include_str!("../../../kmod/kpm/kver_offsets.h");
    let selector_families = header
        .match_indices("VPNHIDE_KVER_FAMILY(kver, ")
        .filter_map(|(start, _)| {
            let args = header[start..].split_once(')')?.0;
            let mut values = args.rsplitn(3, ',');
            let minor = values.next()?.trim().parse::<u32>().ok()?;
            let major = values.next()?.trim().parse::<u32>().ok()?;
            Some((major, minor))
        })
        .collect::<BTreeSet<_>>();
    let activator_families = KPM_SUPPORTED_KERNEL_PAIRS
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    assert_eq!(activator_families, selector_families);
}

#[test]
fn parses_pm_package_uids_for_all_profiles() {
    let map = parse_pm_packages(
        "package:com.example.one uid:10123,1010123\n\
         package:com.example.two uid:10234\n\
         package:bad.without.uid\n",
    );
    assert_eq!(map.uids_for("com.example.one"), &[10123, 1010123]);
    assert_eq!(map.uids_for("com.example.two"), &[10234]);
    assert!(map.uids_for("bad.without.uid").is_empty());
}

#[test]
fn parses_android_user_ids_without_treating_names_as_syntax() {
    let users = parse_pm_user_ids(
        "Users:\n\
         \tUserInfo{0:Owner:c13} running\n\
         \tUserInfo{10:Work:1030} running\n\
         \tUserInfo{11:Second:Space:10} running\n",
    );

    assert_eq!(users, vec![0, 10, 11]);
}

#[test]
fn merges_repeated_per_user_rows_and_accepts_apk_paths() {
    let map = parse_pm_packages(
        "package:/data/app/example/base.apk=com.example uid:10123\n\
         package:/data/app/example/base.apk=com.example uid:1010123\n",
    );

    assert_eq!(map.uids_for("com.example"), &[10123, 1010123]);
}

#[test]
fn projects_native_roles_to_wire() {
    let cfg = parse_canonical(
        r#"{
          "version": 1,
          "debug": true,
          "apps": {
            "com.example.disabled": { "native": false },
            "com.example.full": { "native": true },
            "com.example.partial": { "native": ["sock_ioctl"] }
          }
        }"#,
    )
    .unwrap();
    let resolver = parse_pm_packages(
        "package:com.example.full uid:10123,1010123\n\
         package:com.example.partial uid:10234\n\
         package:com.example.disabled uid:10345\n",
    );
    assert_eq!(
        project_native_with_resolver(&cfg, &resolver),
        "vpnhide 2 config\n\
         debug 1\n\
         targets 40 27fa\n\
         targets a0003ff 278b f69cb\n\
         end 3\n",
    );
}

#[test]
fn native_projection_targets_only_the_main_profile_copy_of_vpnhide() {
    let cfg = parse_canonical(
        r#"{
          "version": 1,
          "apps": {
            "dev.okhsunrog.vpnhide": { "native": true },
            "com.example.profiled": { "native": true }
          }
        }"#,
    )
    .unwrap();
    let resolver = parse_pm_packages(
        "package:dev.okhsunrog.vpnhide uid:10123,1010123\n\
         package:com.example.profiled uid:10234,1010234\n",
    );

    assert_eq!(
        project_native_with_resolver(&cfg, &resolver),
        "vpnhide 2 config\n\
         debug 0\n\
         targets a0003ff 278b 27fa f6a3a\n\
         end 3\n",
    );
}

#[test]
fn native_projection_drops_platform_aids_but_keeps_preinstalled_apps() {
    // A package sharing "android.uid.system" resolves to 1000 — the same uid as
    // system_server — so listing it would mean "hide from everything running as
    // 1000", not "hide from that app". Vendor-preinstalled apps are a different
    // set: FLAG_SYSTEM but an ordinary 10xxx uid, and they stay targetable.
    let cfg = parse_canonical(
        r#"{
          "version": 1,
          "apps": {
            "com.oem.sharesSystemUid": { "native": true },
            "com.oem.preinstalled": { "native": true }
          }
        }"#,
    )
    .unwrap();
    let resolver = parse_pm_packages(
        "package:com.oem.sharesSystemUid uid:1000\n\
         package:com.oem.preinstalled uid:10234,1010234\n",
    );

    assert_eq!(
        project_native_with_resolver(&cfg, &resolver),
        "vpnhide 2 config\n\
         debug 0\n\
         targets a0003ff 27fa f6a3a\n\
         end 2\n",
    );
}

#[test]
fn kmod_projection_includes_the_optional_filesystem_hook() {
    let cfg = parse_canonical(
        r#"{
          "version": 1,
          "settings": { "optionalFeatures": ["filesystem_iface_paths"] },
          "apps": { "com.example.full": { "native": true } }
        }"#,
    )
    .unwrap();
    let resolver = parse_pm_packages("package:com.example.full uid:10123\n");

    assert!(
        cfg.settings
            .optional_features
            .contains(OPTIONAL_FEATURE_FILESYSTEM_IFACE_PATHS)
    );
    assert_eq!(
        project_native_with_resolver_for_family(&cfg, &resolver, NativeHookFamily::Kmod),
        "vpnhide 2 config\ndebug 0\ntargets a0003ff 278b\nend 1\n",
    );
    assert_eq!(
        project_native_with_resolver_for_family(&cfg, &resolver, NativeHookFamily::Kpm),
        "vpnhide 2 config\ndebug 0\ntargets a0003ff 278b\nend 1\n",
    );
}

#[test]
fn zygisk_projection_gates_filesystem_hook_on_the_optional_feature() {
    let disabled = parse_canonical(
        r#"{
          "version": 1,
          "apps": { "com.example.full": { "native": true } }
        }"#,
    )
    .unwrap();
    let enabled = parse_canonical(
        r#"{
          "version": 1,
          "settings": { "optionalFeatures": ["filesystem_iface_paths"] },
          "apps": { "com.example.full": { "native": true } }
        }"#,
    )
    .unwrap();
    let resolver = parse_pm_packages("package:com.example.full uid:10123\n");
    let filesystem_bit = Hook::FilesystemIfacePaths.bit();

    let disabled_wire =
        project_native_with_resolver_for_family(&disabled, &resolver, NativeHookFamily::Zygisk);
    let enabled_wire =
        project_native_with_resolver_for_family(&enabled, &resolver, NativeHookFamily::Zygisk);
    let disabled_config = vpnhide_protocol::parse_config(disabled_wire.as_bytes()).unwrap();
    let enabled_config = vpnhide_protocol::parse_config(enabled_wire.as_bytes()).unwrap();

    assert_eq!(disabled_config.targets[0].hookmask & filesystem_bit, 0);
    assert_ne!(enabled_config.targets[0].hookmask & filesystem_bit, 0);
}

#[test]
fn builtin_projection_gates_filesystem_hook_on_the_optional_feature() {
    // The compiled-in built-in driver has no load-time gate (unlike the .ko's
    // module_param and KPM's ensure_loaded), so its runtime mask is the ONLY
    // gate: the filesystem toggle must be reflected there, or it never turns
    // off. Kmod/Kpm deliberately keep the bit (they gate at load), so assert
    // both the built-in gating AND that the load-gated families do not.
    let disabled = parse_canonical(
        r#"{
          "version": 1,
          "apps": { "com.example.full": { "native": true } }
        }"#,
    )
    .unwrap();
    let enabled = parse_canonical(
        r#"{
          "version": 1,
          "settings": { "optionalFeatures": ["filesystem_iface_paths"] },
          "apps": { "com.example.full": { "native": true } }
        }"#,
    )
    .unwrap();
    let resolver = parse_pm_packages("package:com.example.full uid:10123\n");
    let filesystem_bit = Hook::FilesystemIfacePaths.bit();
    let mask = |cfg: &_, family| {
        let wire = project_native_with_resolver_for_family(cfg, &resolver, family);
        vpnhide_protocol::parse_config(wire.as_bytes()).unwrap().targets[0].hookmask
    };

    // Built-in: gated in the mask, exactly like zygisk.
    assert_eq!(mask(&disabled, NativeHookFamily::Builtin) & filesystem_bit, 0);
    assert_ne!(mask(&enabled, NativeHookFamily::Builtin) & filesystem_bit, 0);

    // Load-gated families keep the bit even when the feature is off.
    assert_ne!(mask(&disabled, NativeHookFamily::Kmod) & filesystem_bit, 0);
    assert_ne!(mask(&disabled, NativeHookFamily::Kpm) & filesystem_bit, 0);
}

#[test]
fn native_projection_ignores_non_kernel_hook_names() {
    let cfg = parse_canonical(
        r#"{
          "version": 1,
          "debug": false,
          "apps": {
            "com.example.java": { "native": ["lsposed_network"] }
          }
        }"#,
    )
    .unwrap();
    let resolver = parse_pm_packages("package:com.example.java uid:10123\n");

    assert_eq!(
        project_native_with_resolver(&cfg, &resolver),
        "vpnhide 2 config\ndebug 0\nend 0\n",
    );
}

#[test]
fn projects_backend_specific_native_hook_overrides() {
    let cfg = parse_canonical(
        r#"{
          "version": 1,
          "apps": {
            "com.example.app": {
              "native": {
                "enabled": true,
                "kernel": ["sock_ioctl"],
                "zygisk": ["zygisk_ioctl", "zygisk_recvfrom_chk", "zygisk_setsockopt"]
              }
            }
          }
        }"#,
    )
    .unwrap();
    let resolver = parse_pm_packages("package:com.example.app uid:10234\n");

    assert_eq!(
        project_native_with_resolver(&cfg, &resolver),
        "vpnhide 2 config\n\
         debug 0\n\
         targets 40 27fa\n\
         end 1\n",
    );
    assert_eq!(
        project_native_with_resolver_for_family(&cfg, &resolver, NativeHookFamily::Zygisk),
        "vpnhide 2 config\n\
         debug 0\n\
         targets 5040000 27fa\n\
         end 1\n",
    );
}

#[test]
fn legacy_native_hook_list_is_kernel_only_and_zygisk_defaults_to_all() {
    let cfg = parse_canonical(
        r#"{
          "version": 1,
          "apps": {
            "com.example.app": { "native": ["sock_ioctl"] }
          }
        }"#,
    )
    .unwrap();
    let resolver = parse_pm_packages("package:com.example.app uid:10234\n");

    assert_eq!(
        project_native_with_resolver(&cfg, &resolver),
        "vpnhide 2 config\n\
         debug 0\n\
         targets 40 27fa\n\
         end 1\n",
    );
    assert_eq!(
        project_native_with_resolver_for_family(&cfg, &resolver, NativeHookFamily::Zygisk),
        "vpnhide 2 config\n\
         debug 0\n\
         targets 5fc0000 27fa\n\
         end 1\n",
    );
}

#[test]
fn empty_legacy_native_hook_list_is_disabled_for_every_backend() {
    let cfg = parse_canonical(
        r#"{
          "version": 1,
          "apps": {
            "com.example.app": { "native": [] }
          }
        }"#,
    )
    .unwrap();
    let resolver = parse_pm_packages("package:com.example.app uid:10234\n");

    assert_eq!(
        project_native_with_resolver(&cfg, &resolver),
        "vpnhide 2 config\ndebug 0\nend 0\n",
    );
    assert_eq!(
        project_native_with_resolver_for_family(&cfg, &resolver, NativeHookFamily::Zygisk),
        "vpnhide 2 config\ndebug 0\nend 0\n",
    );
}

#[test]
fn zygisk_rejects_a_nonzero_default_mask() {
    let blacklist = format_config(false, NO_DEFAULT_MASK, &[]);
    assert!(validate_zygisk_config_wire(&blacklist).is_ok());

    let whitelist = format_config(false, ZYGISK_HOOK_MASK, &[]);
    let error = validate_zygisk_config_wire(&whitelist)
        .unwrap_err()
        .to_string();
    assert!(error.contains("non-zero default hookmask"));
    assert!(error.contains("requires kmod or KPM"));
}

#[test]
fn parses_shared_storage_fixture() {
    let cfg = parse_canonical(include_str!("../../../testdata/storage_config_v1.json")).unwrap();

    assert!(cfg.debug);
    assert!(cfg.settings.remember_superkey);
    assert_eq!(
        cfg.apps.get("com.example.bank").unwrap().native,
        NativeSelection::Enabled(true),
    );
    let proxy = cfg.apps.get("org.example.proxy").unwrap();
    assert_eq!(
        proxy.native,
        NativeSelection::Hooks(vec![
            "fib_route_seq_show".to_owned(),
            "sock_ioctl".to_owned()
        ]),
    );
    // Per-hook Java selection in the fixture: the array form must parse and
    // collapse to "java enabled" without breaking the native config read.
    assert!(proxy.java);
}

#[test]
fn parses_per_hook_java_selection_without_breaking_native() {
    // The canonical the app writes when a user picks individual Java hooks:
    // "java" is a string array, not a bool. A bool-only field used to make
    // serde reject the whole config, silently disabling every native target.
    let cfg = parse_canonical(
        r#"{
          "version": 1,
          "apps": {
            "com.example.partialjava": {
              "java": ["lsposed_network", "lsposed_network_info"],
              "native": true
            },
            "com.example.emptyjava": { "java": [], "native": true }
          }
        }"#,
    )
    .unwrap();
    assert!(cfg.apps.get("com.example.partialjava").unwrap().java);
    // An empty array means no Java hooks -> role disabled.
    assert!(!cfg.apps.get("com.example.emptyjava").unwrap().java);

    let resolver = parse_pm_packages(
        "package:com.example.partialjava uid:10123\n\
         package:com.example.emptyjava uid:10124\n",
    );
    // Native projection is unaffected: both apps still get the kernel mask.
    assert_eq!(
        project_native_with_resolver(&cfg, &resolver),
        "vpnhide 2 config\n\
         debug 0\n\
         targets a0003ff 278b 278c\n\
         end 2\n",
    );
}

#[test]
fn projects_ports_roles_to_iptables_rulesets() {
    let cfg = parse_canonical(
        r#"{
          "version": 1,
          "apps": {
            "com.example.disabled": { "ports": false },
            "com.example.ports": { "ports": true },
            "com.example.system": { "ports": true }
          }
        }"#,
    )
    .unwrap();
    let resolver = parse_pm_packages(
        "package:com.example.ports uid:10123,1010123\n\
         package:com.example.system uid:999\n\
         package:com.example.disabled uid:10345\n",
    );

    let rules = project_ports_with_resolver(&cfg, &resolver);

    assert_eq!(rules.target_count, 2);
    assert_eq!(
        rules.ipv4,
        "*filter\n\
         :vpnhide_out - [0:0]\n\
         -A vpnhide_out -m owner --uid-owner 10123 -d 127.0.0.0/8 -p tcp -j REJECT --reject-with tcp-reset\n\
         -A vpnhide_out -m owner --uid-owner 10123 -d 127.0.0.0/8 -p udp -j REJECT --reject-with icmp-port-unreachable\n\
         -A vpnhide_out -m owner --uid-owner 1010123 -d 127.0.0.0/8 -p tcp -j REJECT --reject-with tcp-reset\n\
         -A vpnhide_out -m owner --uid-owner 1010123 -d 127.0.0.0/8 -p udp -j REJECT --reject-with icmp-port-unreachable\n\
         -A vpnhide_out -j RETURN\n\
         COMMIT\n",
    );
    assert_eq!(
        rules.ipv6,
        "*filter\n\
         :vpnhide_out6 - [0:0]\n\
         -A vpnhide_out6 -m owner --uid-owner 10123 -d ::1 -p tcp -j REJECT --reject-with tcp-reset\n\
         -A vpnhide_out6 -m owner --uid-owner 10123 -d ::1 -p udp -j REJECT --reject-with icmp6-port-unreachable\n\
         -A vpnhide_out6 -m owner --uid-owner 1010123 -d ::1 -p tcp -j REJECT --reject-with tcp-reset\n\
         -A vpnhide_out6 -m owner --uid-owner 1010123 -d ::1 -p udp -j REJECT --reject-with icmp6-port-unreachable\n\
         -A vpnhide_out6 -j RETURN\n\
         COMMIT\n",
    );
}

#[test]
fn projects_custom_ports_policy_to_dport_rules() {
    let cfg = parse_canonical(
        r#"{
          "version": 1,
          "apps": {
            "com.example.proxy": {
              "ports": true,
              "portPolicy": {
                "mode": "custom",
                "rules": [
                  { "protocol": "tcp", "start": 7890, "end": 7892 },
                  { "protocol": "udp", "start": 5353 },
                  { "start": 1080 }
                ]
              }
            }
          }
        }"#,
    )
    .unwrap();
    let resolver = parse_pm_packages("package:com.example.proxy uid:10123\n");

    let rules = project_ports_with_resolver(&cfg, &resolver);

    assert_eq!(rules.target_count, 1);
    assert_eq!(
        rules.ipv4,
        "*filter\n\
         :vpnhide_out - [0:0]\n\
         -A vpnhide_out -m owner --uid-owner 10123 -d 127.0.0.0/8 -p tcp --dport 1080 -j REJECT --reject-with tcp-reset\n\
         -A vpnhide_out -m owner --uid-owner 10123 -d 127.0.0.0/8 -p udp --dport 1080 -j REJECT --reject-with icmp-port-unreachable\n\
         -A vpnhide_out -m owner --uid-owner 10123 -d 127.0.0.0/8 -p udp --dport 5353 -j REJECT --reject-with icmp-port-unreachable\n\
         -A vpnhide_out -m owner --uid-owner 10123 -d 127.0.0.0/8 -p tcp --dport 7890:7892 -j REJECT --reject-with tcp-reset\n\
         -A vpnhide_out -j RETURN\n\
         COMMIT\n",
    );
}

#[test]
fn shared_uid_full_ports_policy_wins_over_ranges() {
    let cfg = parse_canonical(
        r#"{
          "version": 1,
          "apps": {
            "com.example.full": { "ports": true },
            "com.example.range": {
              "ports": true,
              "portPolicy": {
                "mode": "custom",
                "rules": [{ "protocol": "tcp", "start": 7890 }]
              }
            }
          }
        }"#,
    )
    .unwrap();
    let resolver = parse_pm_packages(
        "package:com.example.full uid:10123\n\
         package:com.example.range uid:10123\n",
    );

    let rules = project_ports_with_resolver(&cfg, &resolver);

    assert_eq!(rules.target_count, 1);
    assert!(
        rules
            .ipv4
            .contains("-p tcp -j REJECT --reject-with tcp-reset")
    );
    assert!(!rules.ipv4.contains("--dport"));
}

#[test]
fn rejects_invalid_ports_policy_ranges() {
    assert!(
        parse_canonical(
            r#"{
              "version": 1,
              "apps": {
                "com.example.bad": {
                  "ports": true,
                  "portPolicy": {
                    "mode": "custom",
                    "rules": [{ "start": 0 }]
                  }
                }
              }
            }"#,
        )
        .is_err(),
    );
    assert!(
        parse_canonical(
            r#"{
              "version": 1,
              "apps": {
                "com.example.bad": {
                  "ports": true,
                  "portPolicy": {
                    "mode": "custom",
                    "rules": [{ "start": 9000, "end": 8000 }]
                  }
                }
              }
            }"#,
        )
        .is_err(),
    );
}

#[test]
fn projects_shared_fixture_ports_role() {
    let cfg = parse_canonical(include_str!("../../../testdata/storage_config_v1.json")).unwrap();
    let resolver = parse_pm_packages(
        "package:org.example.proxy uid:10177\n\
         package:com.example.bank uid:10178\n",
    );

    let rules = project_ports_with_resolver(&cfg, &resolver);

    assert_eq!(rules.target_count, 1);
    assert!(rules.ipv4.contains("--uid-owner 10177"));
    assert!(!rules.ipv4.contains("--uid-owner 10178"));
}

#[test]
fn absent_canonical_projects_to_empty_config_without_pm() {
    assert_eq!(
        project_native(empty_canonical_json()).unwrap(),
        "vpnhide 2 config\ndebug 0\nend 0\n",
    );
}

#[test]
fn pm_ready_check_matches_literal_package_token() {
    assert!(pm_output_has_package(
        "package:dev.okhsunrog.vpnhide uid:10123\n",
        APP_PACKAGE,
    ));
    assert!(!pm_output_has_package(
        "package:dev.okhsunrog.vpnhide.extra uid:10123\n",
        APP_PACKAGE,
    ));
}

#[test]
fn apatch_supercall_command_keeps_kpm_command_in_low_bits() {
    assert_eq!(
        supercall_cmd(
            ApatchCommandStyle::Versioned(
                vpnhide_apatch_abi::APATCH_SUPERCALL_DEFAULT_VERSION_CODE
            ),
            SUPERCALL_KPM_CONTROL,
        ),
        (vpnhide_apatch_abi::APATCH_SUPERCALL_DEFAULT_VERSION_CODE << 32)
            | (vpnhide_apatch_abi::APATCH_SUPERCALL_MAGIC << 16)
            | SUPERCALL_KPM_CONTROL,
    );
    assert_eq!(
        supercall_cmd(
            ApatchCommandStyle::Versioned(0x000c02),
            SUPERCALL_KPM_CONTROL
        ) & 0xffff,
        0x1022,
    );
    assert_eq!(
        supercall_cmd(ApatchCommandStyle::Raw, SUPERCALL_KPM_CONTROL),
        0x1022
    );
    assert_eq!(
        supercall_cmd(ApatchCommandStyle::Versioned(0x000c02), SUPERCALL_HELLO) & 0xffff,
        0x1000,
    );
    assert_eq!(SUPERCALL_HELLO_MAGIC, 0x11581158);
}

#[test]
fn apatch_command_candidates_include_current_and_folkpatch_versions() {
    let candidates = apatch_command_candidates();
    assert_eq!(
        candidates.first(),
        Some(&ApatchCommandStyle::Versioned(0x000d00))
    );
    assert!(candidates.contains(&ApatchCommandStyle::Versioned(0x000d02)));
    assert!(candidates.contains(&ApatchCommandStyle::Versioned(0x000d01)));
    assert!(candidates.contains(&ApatchCommandStyle::Versioned(0x000d00)));
    assert_eq!(
        candidates
            .iter()
            .filter(|style| **style == ApatchCommandStyle::Versioned(0x000d01))
            .count(),
        1,
    );
}

#[test]
fn apatch_kernel_version_hint_parses_dmesg() {
    let log = "\
[    0.000000] KP KernelPatch Version: c02
[    0.000000] KP KernelPatch Config: 2
";
    assert_eq!(parse_apatch_kernel_version_hint(log), Some(0xc02));
}

#[test]
fn projection_is_bounded_to_backend_target_capacity() {
    let projected = MAX_NATIVE_TARGETS + 10;
    let apps = (0..projected)
        .map(|i| {
            (
                format!("com.example.{i:02}"),
                AppConfig {
                    native: NativeSelection::Enabled(true),
                    ..AppConfig::default()
                },
            )
        })
        .collect::<BTreeMap<_, _>>();
    let cfg = CanonicalConfig {
        version: 1,
        debug: false,
        apps,
        settings: Settings::default(),
    };
    let pm = (0..projected)
        .map(|i| format!("package:com.example.{i:02} uid:{}", 10_000 + i))
        .collect::<Vec<_>>()
        .join("\n");
    let wire = project_native_with_resolver(&cfg, &parse_pm_packages(&pm));

    // Every app shares one mask, so the whole set rides one `targets` record
    // and the count lives in the `end` fuse — which the backend checks, so the
    // producer has to cap itself here rather than let the backend reject it.
    assert_eq!(
        wire.lines().filter(|l| l.starts_with("targets ")).count(),
        1
    );
    assert!(wire.ends_with(&format!("end {MAX_NATIVE_TARGETS:x}\n")));
    let uids = wire
        .lines()
        .find(|l| l.starts_with("targets "))
        .unwrap()
        .split_whitespace()
        .count()
        - 2; // keyword + mask
    assert_eq!(uids, MAX_NATIVE_TARGETS);

    // And what it produces must survive its own reader: the parser rejects a
    // payload carrying more uids than a backend can hold.
    assert!(vpnhide_protocol::parse_config(wire.as_bytes()).is_some());
    assert_eq!(
        native_target_capacity_warning(projected),
        "vpnhide-warning native_target_cap total=170 cap=160 dropped=10",
    );
}

#[test]
fn kpm_rejects_a_valid_config_that_exceeds_its_argument_buffer() {
    // MAX_TARGET_UIDS alone is not enough to prove transport fit: distinct
    // per-app masks each need their own group header. Keep this valid at the
    // protocol layer and make the KPM transport boundary reject it clearly.
    let targets = (0..MAX_NATIVE_TARGETS as u32)
        .map(|offset| Target {
            uid: u32::MAX - offset,
            hookmask: offset + 1,
        })
        .collect::<Vec<_>>();
    let wire = format_config(false, NO_DEFAULT_MASK, &targets);

    assert!(parse_config(wire.as_bytes()).is_some());
    assert!(wire.len() >= KPM_ARGS_LEN);
    let error = validate_kpm_config_wire(&wire).unwrap_err().to_string();
    assert!(error.contains(&wire.len().to_string()));
    assert!(error.contains(&(KPM_ARGS_LEN - 1).to_string()));
    assert!(error.contains("distinct per-app hook selections"));
}

#[test]
fn kpm_accepts_the_largest_argument_that_leaves_room_for_nul() {
    assert!(validate_kpm_config_wire(&"x".repeat(KPM_ARGS_LEN - 1)).is_ok());
    assert!(validate_kpm_config_wire(&"x".repeat(KPM_ARGS_LEN)).is_err());
}

#[test]
fn kpatch_ctl0_accepts_config_target_count_exit_codes() {
    let one_target = "vpnhide 2 config\ndebug 0\ntargets 1 123\nend 1\n";
    assert!(kpatch_ctl0_config_status_ok(
        std::process::ExitStatus::from_raw(0),
        "vpnhide 2 config\ndebug 0\nend 0\n"
    ));
    assert!(kpatch_ctl0_config_status_ok(
        std::process::ExitStatus::from_raw(1 << 8),
        one_target
    ));
    assert!(!kpatch_ctl0_config_status_ok(
        std::process::ExitStatus::from_raw(2 << 8),
        one_target
    ));
    assert!(!kpatch_ctl0_config_status_ok(
        std::process::ExitStatus::from_raw(1 << 8),
        "not vpnhide config\n"
    ));
    assert!(!kpatch_ctl0_config_status_ok(
        std::process::ExitStatus::from_raw(255 << 8),
        one_target
    ));
    assert!(!kpatch_ctl0_config_status_ok(
        std::process::ExitStatus::from_raw(15),
        one_target
    ));
}

#[test]
fn kpm_readback_marks_truncated_complete_line_prefixes() {
    let complete = "vpnhide 1 stats\n0x1 0x0:0x2\n";
    assert_eq!(
        normalize_kpm_reply("vpnhide 1 stats", complete.to_owned()).unwrap(),
        complete,
    );

    let partial = "vpnhide 1 stats\n0x1 0x0:0x2";
    assert_eq!(
        normalize_kpm_reply("vpnhide 1 stats", partial.to_owned()).unwrap(),
        "vpnhide 1 stats\n0x1 0x0:0x2\n# vpnhide truncated\n",
    );
}

#[test]
fn kpm_readback_rejects_empty_or_wrong_kind_replies() {
    assert!(normalize_kpm_reply("vpnhide 1 stats", String::new()).is_err());
    assert!(
        normalize_kpm_reply(
            "vpnhide 1 stats",
            "vpnhide 1 status\nbackend 0x1\n".to_owned(),
        )
        .is_err(),
    );
}

#[test]
fn kpm_stats_pages_are_validated_and_folded_into_legacy_telemetry() {
    let replies = [
        "vpnhide 1 stats\n0x2800 0x0:0x12 0x19:0x5\npage 0x0 0x2800 0x1",
        "vpnhide 1 stats\n0x2801 0x2:0x7\npage 0x2800 done 0x1\n",
    ];
    let mut replies = replies.into_iter();
    let mut requests = Vec::new();
    let stats = collect_kpm_stats_pages(|request| {
        requests.push(request.to_owned());
        Ok(replies.next().unwrap().to_owned())
    })
    .unwrap();

    assert_eq!(
        requests,
        ["vpnhide 1 stats", "vpnhide 1 stats\nafter 0x2800\n",]
    );
    assert_eq!(
        stats,
        "vpnhide 1 stats\n0x2800 0x0:0x12 0x19:0x5\n0x2801 0x2:0x7\n"
    );
}

#[test]
fn kpm_stats_reader_accepts_a_complete_legacy_reply() {
    let legacy = "vpnhide 1 stats\n0x2800 0x0:0x12\n";
    assert_eq!(
        collect_kpm_stats_pages(|_| Ok(legacy.to_owned())).unwrap(),
        legacy
    );
}

#[test]
fn kpm_stats_pages_reject_broken_integrity_signals() {
    let cases = [
        // Echoed cursor mismatch.
        "vpnhide 1 stats\n0x2800 0x0:0x1\npage 0x1 0x2800 0x1",
        // Declared row count mismatch.
        "vpnhide 1 stats\n0x2800 0x0:0x1\npage 0x0 0x2800 0x2",
        // A non-final page must retain the old-reader truncation signal.
        "vpnhide 1 stats\n0x2800 0x0:0x1\npage 0x0 0x2800 0x1\n",
        // A final page must be conventionally newline-terminated.
        "vpnhide 1 stats\n0x2800 0x0:0x1\npage 0x0 done 0x1",
    ];
    for reply in cases {
        assert!(
            collect_kpm_stats_pages(|_| Ok(reply.to_owned())).is_err(),
            "{reply:?}"
        );
    }
}
