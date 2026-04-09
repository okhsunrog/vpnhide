# Keep Xposed entry points — LSPosed loads these by reflection via assets/xposed_init
-keep class dev.okhsunrog.vpnhide.HookEntry { *; }
-keepnames class dev.okhsunrog.vpnhide.** { *; }

# Keep Xposed API types
-keep class de.robv.android.xposed.** { *; }
-dontwarn de.robv.android.xposed.**
