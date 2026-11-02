# Building vpnhide-kmod

Users install the complete Core ZIP and its matching APK from [Releases](https://github.com/okhsunrog/vpnhide/releases). Core selects from the bundled backends automatically. This contributor guide builds internal payload inputs for full Core assembly; its component ZIPs are not separate user installations.

## Quick build

One command — same script CI runs, no container invocation to memorize:

```bash
./kmod/build.py --kmi android14-6.1     # one variant
./kmod/build.py --all                   # every supported GKI
```

The script auto-detects whether to build natively (you're already inside the DDK image, or you've pointed `--kdir` at a kernel source tree) or to spawn a `ghcr.io/ylarod/ddk-min:<kmi>-<TAG>` container via Docker or podman. Docker is preferred when both are installed, matching CI and device-test workflows. On rootless podman (Fedora etc) it adds `--userns=keep-id` and `:Z` automatically. The output is `vpnhide-kmod-<kmi>.zip` at the repo root.

Requires `docker` or `podman` and the pinned Rust toolchain for shared artifact validation/provenance. The wrapper prepares a static Linux `vpnhide-tooling` executor and mounts it into the minimal DDK image. CI supplies the same-checkout prebuilt executor through `VPNHIDE_TOOLING`; a direct minimal-container invocation must supply it too. See [shared host tooling](../docs/development.md#shared-host-tooling).

### Identifying the source-build KMI

```bash
adb shell uname -r
```

The output looks like `6.1.75-android14-11-g...` — the generation is `android14-6.1`.

The `android14` component identifies the kernel generation, independently of the installed Android version. This table is for contributor build inputs; the app does not require a manual GKI choice.

| `uname -r` pattern | GKI generation |
|---|---|
| `5.10.xxx-android12-...` | android12-5.10 |
| `5.10.xxx-android13-...` | android13-5.10 |
| `5.15.xxx-android13-...` | android13-5.15 |
| `5.15.xxx-android14-...` | android14-5.15 |
| `6.1.xxx-android14-...` | android14-6.1 |
| `6.6.xxx-android15-...` | android15-6.6 |
| `6.12.xxx-android16-...` | android16-6.12 |

## Local build with kernel source

If you prefer building against a local kernel source tree (e.g. for development or debugging), point `--kdir` at it. The script then runs natively without spinning up a container:

```bash
./kmod/build.py --kdir ~/kernels/android14-6.1 --kmi android14-6.1
```

You can also drop a `kmod/.env` file with `KDIR=` / `KERNEL_SRC=` / `CLANG_DIR=` (see `.env.example`) and use [`direnv`](https://direnv.net/) to load it automatically. The script picks those up via env, no flag needed.

## Install and test

```bash
adb push vpnhide-kmod-<kmi>.zip /sdcard/Download/
# Install via KernelSU-Next manager -> Modules -> Install from storage
# Reboot
```

Verify after reboot:

```bash
adb shell "su -c 'lsmod | grep vpnhide'"
adb shell "su -c 'dmesg | grep vpnhide'"
adb shell "su -c 'cat /proc/vpnhide_ctl'"
```

The optional `.ko` filesystem-hiding probes are controlled from the app's
Settings screen and applied on the next reboot. For direct development loads,
pass `filesystem_hiding=1` to `insmod`; omitting it leaves the global VFS hooks
unregistered.

## Troubleshooting

**`insmod: Exec format error`** — symvers CRC mismatch. Rebuild via the DDK container (`./kmod/build.py --kmi <kmi>`); the container image carries matched symvers.

**`insmod: File exists`** — module already loaded. The module is intentionally
non-unloadable because root module managers replace or remove it across a
reboot; reboot before loading a different build.

**kretprobe not firing** — check `dmesg | grep vpnhide` for registration messages and `/proc/vpnhide_ctl` for correct UIDs. Target app UIDs change on reinstall — re-resolve via the VPN Hide app.

**`./kmod/build.py` says "neither podman nor docker found"** — install one (`apt install docker.io` / `dnf install podman`), or build natively against a local kernel source via `--kdir`.

**Bumping the DDK image tag** — single source of truth is `DDK_IMAGE_TAG` in `kmod/build.py`. Both this script and `.github/workflows/ci.yml`'s kmod matrix pin to the same value, so update both together.
