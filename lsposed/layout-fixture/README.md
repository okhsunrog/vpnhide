# Adaptive layout fixture

Opt-in, root-free Activity using the production `TargetActionBar`, `MetricGrid`
and `ModuleHeading`. No configuration writes or native backend are needed.
The Activity is absent from normal APK builds.

Build from the repository root:

```sh
JAVA_HOME=/usr/lib/jvm/java-21-openjdk ./lsposed/gradlew -p lsposed \
  -I "$PWD/lsposed/layout-fixture/fixture.init.gradle" -PvpnhideEmulatorX86 :app:assembleRawDebug
adb -s emulator-5554 install -r lsposed/app/build/outputs/apk/rawDebug/app-rawDebug.apk
adb -s emulator-5554 shell cmd locale set-app-locales dev.okhsunrog.vpnhide --locales ru
```

At density 320, physical widths 640, 720 and 824 px give widths 320,
360 and 412 dp. Use height 1600 px and test each with `font_scale` 1.0, 1.3, 1.5, 2.0.
Restart the fixture after changing a setting:

```sh
adb -s emulator-5554 shell wm size 720x1600
adb -s emulator-5554 shell wm density 320
adb -s emulator-5554 shell settings put system font_scale 1.3
adb -s emulator-5554 shell am force-stop dev.okhsunrog.vpnhide
adb -s emulator-5554 shell am start -W -n dev.okhsunrog.vpnhide/.settings.LayoutHarness
```

Use `--ez long_count true` for the hidden-app counter or `--ez metrics true`
for dashboard tiles and the module heading. Inspect screenshots for complete
text, usable buttons and available list space. At normal sizes the footer stays
in one horizontal row (its counter can still wrap to two lines). At larger font
sizes the counter moves above the actions; actions stack only if both do not fit.
The metric grid uses one column when any tile needs more width.

Restore the emulator afterwards:

```sh
adb -s emulator-5554 shell wm size reset
adb -s emulator-5554 shell wm density reset
adb -s emulator-5554 shell settings put system font_scale 1.0
```

This checks the actual layout components with fixture data, not root/module
integration. Run the standard Android quality gates separately.

## KPM runtime help scenario

For a physical ARM phone, build with the same init script but omit
`-PvpnhideEmulatorX86`. Install the rawDebug APK and launch:

```sh
adb shell am start -W -n dev.okhsunrog.vpnhide/.settings.KpmHelpHarness
```

The preview uses the production module-problem renderer, Dashboard message
section and HelpScreen. Tap Learn more to check the localized installation
anchor, follow guide links, and use Back to return. The preview is visibly
labelled as test data and does not change module state or canonical settings.
Links to app settings show a preview notice instead of mutating the real app.
This Activity is absent from ordinary APK builds.
