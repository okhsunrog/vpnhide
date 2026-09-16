# Keep Xposed entry points — LSPosed loads these by reflection via assets/xposed_init
-keep class dev.okhsunrog.vpnhide.hook.HookEntry { *; }
-keepnames class dev.okhsunrog.vpnhide.** { *; }

# Root app_process probe entry point (scripts/network-view-probe.py runs it by name)
-keep class dev.okhsunrog.vpnhide.debug.NetworkViewProbeMain { public static void main(java.lang.String[]); }

# Keep Xposed API types
-keep class de.robv.android.xposed.** { *; }
-dontwarn de.robv.android.xposed.**
