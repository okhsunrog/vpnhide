# CI image for vpnhide.
#
# Bakes in everything needed for all three modules:
#   - Rust + NDK + cargo-ndk (zygisk)
#   - JDK 17 (lsposed, test-app)
#   - Google AOSP clang + cross-compile tools (kmod)
#
# Rebuilt by ci-image.yml when this Dockerfile changes (and monthly).

FROM ubuntu:24.04

ENV DEBIAN_FRONTEND=noninteractive

RUN apt-get update && \
    apt-get install -y --no-install-recommends \
        ca-certificates curl unzip zip xz-utils \
        build-essential pkg-config \
        cmake ninja-build \
        openjdk-17-jdk-headless \
        bc kmod cpio flex bison libssl-dev libelf-dev \
        binutils-aarch64-linux-gnu \
        clang-format \
        git && \
    rm -rf /var/lib/apt/lists/*

# ── ktlint (for lsposed / test-app Kotlin) ──────────────────────────
ENV KTLINT_VERSION=1.8.0
RUN curl -fsSL -o /usr/local/bin/ktlint \
        "https://github.com/pinterest/ktlint/releases/download/${KTLINT_VERSION}/ktlint" && \
    chmod +x /usr/local/bin/ktlint

# ── Android SDK (for lsposed) ────────────────────────────────────────
ENV ANDROID_HOME=/opt/android-sdk
ENV PATH="${ANDROID_HOME}/cmdline-tools/latest/bin:${ANDROID_HOME}/platform-tools:${PATH}"

RUN mkdir -p "${ANDROID_HOME}/cmdline-tools" && \
    curl -fsSL -o /tmp/cmdline-tools.zip \
        "https://dl.google.com/android/repository/commandlinetools-linux-11076708_latest.zip" && \
    unzip -q /tmp/cmdline-tools.zip -d /tmp/cmdline-tools && \
    mv /tmp/cmdline-tools/cmdline-tools "${ANDROID_HOME}/cmdline-tools/latest" && \
    rm -rf /tmp/cmdline-tools.zip /tmp/cmdline-tools && \
    yes | sdkmanager --licenses > /dev/null 2>&1 && \
    sdkmanager "platform-tools" "platforms;android-35" "build-tools;35.0.0" && \
    chmod -R a+rx "${ANDROID_HOME}"

# ── Rust toolchain (for zygisk) ──────────────────────────────────────
ENV RUSTUP_HOME=/usr/local/rustup \
    CARGO_HOME=/usr/local/cargo \
    PATH=/usr/local/cargo/bin:$PATH

RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | \
        sh -s -- -y --default-toolchain stable --profile minimal --no-modify-path && \
    rustup target add aarch64-linux-android && \
    rustup component add rustfmt clippy && \
    cargo install cargo-ndk --locked && \
    chmod -R a+w /usr/local/rustup /usr/local/cargo

# ── Android NDK (for zygisk) ─────────────────────────────────────────
ENV ANDROID_NDK_VERSION=r28b
ENV ANDROID_NDK_HOME=/opt/android-ndk
# Gobley's cargo plugin reads ANDROID_NDK_ROOT (not _HOME) to find the NDK
# when looking up the toolchain library directory for Android Rust targets.
ENV ANDROID_NDK_ROOT=/opt/android-ndk

RUN curl -fsSL -o /tmp/ndk.zip \
        "https://dl.google.com/android/repository/android-ndk-${ANDROID_NDK_VERSION}-linux.zip" && \
    unzip -q /tmp/ndk.zip -d /opt && \
    mv "/opt/android-ndk-${ANDROID_NDK_VERSION}" "${ANDROID_NDK_HOME}" && \
    rm /tmp/ndk.zip && \
    chmod -R a+rx "${ANDROID_NDK_HOME}"

# ── Google AOSP clang (for kmod) ─────────────────────────────────────
# Same toolchain that built the Pixel GKI kernel — avoids ABI mismatches
# that cause bootloops with distro clang.
ENV AOSP_CLANG_VERSION=clang-r487747c
ENV AOSP_CLANG_DIR=/opt/aosp-clang

RUN git clone --depth=1 --filter=blob:none --sparse \
        -b android-gs-shusky-6.1-android16 \
        https://android.googlesource.com/platform/prebuilts/clang/host/linux-x86 \
        /tmp/clang-repo && \
    cd /tmp/clang-repo && \
    git sparse-checkout set ${AOSP_CLANG_VERSION} && \
    mv ${AOSP_CLANG_VERSION} ${AOSP_CLANG_DIR} && \
    rm -rf /tmp/clang-repo && \
    chmod -R a+rx ${AOSP_CLANG_DIR}
