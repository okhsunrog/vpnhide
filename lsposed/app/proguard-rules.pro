# Keep Xposed entry points — LSPosed loads these by reflection via assets/xposed_init
-keep class dev.okhsunrog.vpnhide.hook.HookEntry { *; }
-keepnames class dev.okhsunrog.vpnhide.** { *; }

# Root app_process probe entry point (scripts/network-view-probe.py runs it by name)
-keep class dev.okhsunrog.vpnhide.debug.NetworkViewProbeMain { public static void main(java.lang.String[]); }

# Keep Xposed API types
-keep class de.robv.android.xposed.** { *; }
-dontwarn de.robv.android.xposed.**

# core-ffi client: JNA (the UniFFI runtime) calls back into Java by reflection and from
# native code, and maps Structure fields by name. Keep JNA and the generated bindings.
-keep class com.sun.jna.** { *; }
-keepclassmembers class * extends com.sun.jna.** { public *; }
-keep class uniffi.vpnhide_core_ffi.** { *; }
-dontwarn java.awt.**
