# Per-KMI image for the kmod QEMU test harness.
#
#   ghcr.io/ylarod/ddk-min:<kmi>   (toolchain + module-build headers)
#   + qemu-system-aarch64          (boot the kernel; TCG, no KVM needed)
#   + a prebuilt virtio GKI kernel that boots in `qemu -machine virt`
#   + the Alpine minirootfs tarball (so the test needs no download at run time)
#
# Stock GKI boot images can't boot in plain QEMU (no virtio/console drivers),
# so we build kernel/common@<kmi> with kmod/test/qemu.config merged onto
# gki_defconfig, using the DDK container's matching clang. The module itself is
# NOT baked — it's built per-PR by the kmod CI job and passed in via
# VPNHIDE_QEMU_KO (a kdir-built module loads fine here: same source, same
# vermagic). Built rarely by qemu-image.yml since kernels change seldom.
ARG KMI

# --- builder: the bionic getifaddrs probe ----------------------------------
# The gai_getifaddrs vector must run a *bionic* binary (it validates how
# Android's getifaddrs parses our filtered netlink dump — glibc differs and is
# the wrong libc to test). Build it from gai-probe.c with the NDK here; only
# the ~420 KB static binary is copied into the final image (no NDK retained).
# The DDK clang can't build it (no bionic userspace sysroot), hence the NDK.
FROM debian:trixie-slim AS gai
ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update && apt-get install -y --no-install-recommends \
        curl ca-certificates unzip && rm -rf /var/lib/apt/lists/*
ARG NDK_VER=r28c
RUN curl -fsSL "https://dl.google.com/android/repository/android-ndk-${NDK_VER}-linux.zip" \
        -o /tmp/ndk.zip && \
    unzip -q /tmp/ndk.zip "android-ndk-${NDK_VER}/toolchains/llvm/prebuilt/linux-x86_64/*" \
        -d /opt && rm /tmp/ndk.zip
COPY kmod/test/gai-probe.c /tmp/gai-probe.c
COPY kmod/test/ifconf-probe.c /tmp/ifconf-probe.c
RUN TC="/opt/android-ndk-${NDK_VER}/toolchains/llvm/prebuilt/linux-x86_64/bin" && \
    "$TC/aarch64-linux-android24-clang" -static -O2 -o /opt/gai /tmp/gai-probe.c && \
    "$TC/aarch64-linux-android24-clang" -static -O2 -o /opt/ifconf /tmp/ifconf-probe.c && \
    "$TC/llvm-strip" /opt/gai /opt/ifconf

FROM ghcr.io/ylarod/ddk-min:${KMI}-20260313
ARG KMI

ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update && apt-get install -y --no-install-recommends \
        qemu-system-arm cpio gzip curl ca-certificates git \
        bc bison flex libelf-dev libssl-dev \
        gcc-aarch64-linux-gnu && \
    rm -rf /var/lib/apt/lists/*

# Build the kernel and KEEP its tree: the test module must be built against the
# kernel it runs on (matching CFI, vermagic, struct layouts) — building against
# the GKI kdir and running on this separately-built kernel mismatches and panics
# on some KMIs (CFI / struct kretprobe on android16-6.12). Prune .cmd files
# (incremental-rebuild artifacts, ~half the tree, not needed for external
# modules) to keep the image smaller.
COPY kmod/test/qemu.config /tmp/qemu.config
RUN CLANG_BIN="$(ls -d /opt/ddk/clang/*/bin | head -1)" && export PATH="$CLANG_BIN:$PATH" && \
    mkdir -p /opt/qemu && \
    git clone --depth=1 -b "${KMI}" \
        https://android.googlesource.com/kernel/common /opt/qemu/linux && \
    cd /opt/qemu/linux && \
    make ARCH=arm64 LLVM=1 gki_defconfig && \
    ./scripts/kconfig/merge_config.sh -m .config /tmp/qemu.config && \
    make ARCH=arm64 LLVM=1 olddefconfig && \
    make ARCH=arm64 LLVM=1 -j"$(nproc)" Image && \
    make ARCH=arm64 LLVM=1 modules_prepare && \
    cp arch/arm64/boot/Image /opt/qemu/Image && \
    cp vmlinux.symvers Module.symvers && \
    rm -f /tmp/qemu.config && \
    find /opt/qemu/linux -name '*.cmd' -delete

RUN curl -fsSL \
        https://dl-cdn.alpinelinux.org/alpine/v3.21/releases/aarch64/alpine-minirootfs-3.21.2-aarch64.tar.gz \
        -o /opt/qemu/alpine-minirootfs.tar.gz

# KernelPatch host tool (kptools) + runtime (kpimg) for the KPM harness
# (run-kpm.sh) — baked at a pinned version so the per-PR kpm-qemu job is
# hermetic (no per-run release download). Same binaries for every KMI.
ARG KP_VER=0.13.1
RUN mkdir -p /opt/qemu/kp && \
    for a in kptools-linux kpimg-linux; do \
        curl -fsSL "https://github.com/bmax121/KernelPatch/releases/download/${KP_VER}/$a" \
            -o "/opt/qemu/kp/$a"; \
    done && chmod +x /opt/qemu/kp/kptools-linux

# Prebuilt bionic getifaddrs + SIOCGIFCONF probes (built from *-probe.c with the
# NDK in the `gai` stage). run.sh / run-kpm.sh pick them up via VPNHIDE_GAI_BIN /
# VPNHIDE_IFC_BIN so the addr-fill hook and the ifconf size-query path are
# validated in CI on every KMI — not skipped.
COPY --from=gai /opt/gai /opt/qemu/gai
COPY --from=gai /opt/ifconf /opt/qemu/ifconf

# run.sh / run-kpm.sh read these to use the baked artifacts instead of the local
# cache. VPNHIDE_QEMU_KSRC is the kernel tree to build the test module against;
# VPNHIDE_KP_BIN is the baked KernelPatch tool/runtime dir (KPM only);
# VPNHIDE_GAI_BIN is the prebuilt bionic getifaddrs probe;
# VPNHIDE_IFC_BIN is the prebuilt SIOCGIFCONF size-query probe.
ENV VPNHIDE_QEMU_IMAGE=/opt/qemu/Image
ENV VPNHIDE_QEMU_ROOTFS=/opt/qemu/alpine-minirootfs.tar.gz
ENV VPNHIDE_QEMU_KSRC=/opt/qemu/linux
ENV VPNHIDE_KP_BIN=/opt/qemu/kp
ENV VPNHIDE_GAI_BIN=/opt/qemu/gai
ENV VPNHIDE_IFC_BIN=/opt/qemu/ifconf
