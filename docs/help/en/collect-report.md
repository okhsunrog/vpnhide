# Collect a debug report

Use **Settings → Debugging**. Choose a snapshot for a configuration or module
problem, or a recording for a problem that happens while using another app.

## Snapshot: what is happening now

1. If possible, turn on your VPN and route VPN Hide through it so the self-test can run.
2. Open **Debug export** and leave **Verbose logs** enabled for a diagnostic report.
3. Leave the installed-app list and kernel image off unless requested.
4. Export, then save or share the ZIP.

You can still export when the VPN is off or the self-test cannot run. The bundle
records that limitation; it can still help diagnose installation and root problems.

## Recording: reproduce a problem in another app

1. In **Settings → Debugging**, use **Start recording** before reproducing the issue.
2. Switch to the target app, force-stop/reopen it if needed, and reproduce the problem.
3. Return to VPN Hide, stop recording, then save or share the resulting file.
4. In the report, describe the expected and actual behavior, the target app/version,
   the approximate time of the failure, and whether that app was inside the VPN tunnel.

A debug export temporarily enables logging for its own capture and runs self-tests.
It cannot recover detailed logs that were disabled when your target app failed.
It also clears the kernel log buffer before collecting a fresh self-test window;
a later snapshot is not a replacement for a recording of the failure.

**Debug logging** in **Settings → Developer** is useful when continuous logging
outside a capture is requested. Enable it before reproducing, and turn it off
when finished. A capture restores the logging state it found beforehand.

## Check the contents before sharing

The ZIP contains `state.json`, including configuration, module states and diagnostic
results. Verbose capture adds logs and network/system details. The installed-app
option adds the full inventory and profile information; leaving it off does **not**
remove configured package names from the configuration or all app/network details
from logs. Treat the report as potentially identifying information.

A kernel image is a separate, larger attachment for kernel investigation. Include
it only when requested. Review files before posting publicly, and use the contact
links in the app to share through the agreed support channel.
