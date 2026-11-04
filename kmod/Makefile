obj-m += vpnhide_kmod.o

# Kernel code cannot call compiler-rt userspace outline-atomic helpers.
ccflags-y += -mno-outline-atomics

# Kbuild includes the external module Makefile in its modpost stage. Replace
# only the checker executable here, preserving this kernel's own recursive
# MODPOST flags (including $@, symbol inputs and later += additions). Passing
# MODPOST on make's command line would silently discard those flags.
ifdef VPNHIDE_MODPOST
ifdef modpost-args
# Newer Kbuild keeps flags separately. Its executable assignment may precede
# or follow this include; accept only those known executable-only definitions.
ifneq ($(strip $(value MODPOST)),)
ifneq ($(strip $(value MODPOST)),scripts/mod/modpost)
ifneq ($(strip $(value MODPOST)),$$(objtree)/scripts/mod/modpost)
$(error Unsupported Kbuild MODPOST executable)
endif
endif
endif
override MODPOST = $(VPNHIDE_MODPOST)
else
ifdef MODPOST
ifneq ($(firstword $(value MODPOST)),scripts/mod/modpost)
$(error Unsupported Kbuild MODPOST command)
endif
$(eval MODPOST = $(subst scripts/mod/modpost,$(VPNHIDE_MODPOST),$(value MODPOST)))
endif
endif
endif

ifeq ($(KERNELRELEASE),)

# Default target stays the LKM (.ko) build, even though `kpm` is defined first.
.DEFAULT_GOAL := all

# clang is needed by both backends; prefer $(CLANG_DIR) if the .env set it.
CLANG := $(if $(CLANG_DIR),$(CLANG_DIR)/clang,clang)
VPNHIDE_KPM_BUILD_VERSION ?= $(shell sed -n '1p' $(CURDIR)/../VERSION)

# ================================================================== #
#  KPM backend — relocatable `.kpm`, KernelPatch headers, NO kernel  #
#  source / Module.symvers.  Build: make kpm KP_DIR=/path/KernelPatch #
# ================================================================== #
# KernelPatch header tree — vendored as a submodule by default; override
# KP_DIR to point at your own checkout. Run `git submodule update --init`.
KP_DIR ?= $(CURDIR)/third_party/KernelPatch
# Include dirs mirror KernelPatch's own kpms/*/Makefile INCLUDE_DIRS.
KPM_INCLUDE_DIRS := . include patch/include linux/include \
	linux/arch/arm64/include linux/tools/arch/arm64/include
KPM_INCLUDES := $(foreach d,$(KPM_INCLUDE_DIRS),-I$(KP_DIR)/kernel/$(d))

# -fuse-ld=lld: the `-r` link targets aarch64; lld handles the cross link with
# one binary. DDK clang bundles lld and defaults to it, but a plain distro clang
# (the legacy KPM image) defaults to the host GNU ld, which can't do the aarch64
# relocatable link — force lld so both toolchains build the .kpm identically.
# -nostdlib: this is a freestanding relocatable kernel object; older Android
# clang drivers (e.g. clang-12 on android12-5.10) otherwise add CRT startup
# files + -lc/-ldl to the `-r` link and fail (crtbegin_dynamic.o / -lc not found).
# -mcmodel=large: REQUIRED, do not change to small. KernelPatch loads the .kpm
# in a vmalloc region that can sit >4 GB from where ADRP-relative literals were
# assumed, so -mcmodel=small's ADR_PREL_PG_HI21 relocations for .rodata (string
# literals) overflow and resolve to garbage addresses — the ctl0 status/stats
# replies came back corrupted on a real APatch device until this was switched to
# large (absolute MOVW_UABS materialisation, no ±4 GB range limit). The .ko is
# unaffected because Kbuild handles module addressing itself.
KPM_CFLAGS := -target aarch64-linux-android -fuse-ld=lld -nostdlib \
	-O2 -fno-pic -fno-pie -fno-plt -mcmodel=large \
	-fvisibility=hidden -mno-outline-atomics \
	-fno-stack-protector -fno-common -fno-builtin -ffreestanding \
	-fno-asynchronous-unwind-tables -fno-unwind-tables -nostdinc \
	-Wall -Wextra -Werror -Wno-unused-parameter \
	-D__KERNEL__ -DCONFIG_ARM64 -DMODNAME=\"vpnhide\" \
	-DVPNHIDE_KPM_BUILD_VERSION=\"$(VPNHIDE_KPM_BUILD_VERSION)\"

.PHONY: all strip clean kpm kpm-clean

kpm:
	@if [ ! -f "$(KP_DIR)/kernel/include/kpmodule.h" ]; then \
		echo "KernelPatch headers not found at $(KP_DIR)."; \
		echo "Run: git submodule update --init kmod/third_party/KernelPatch"; \
		echo "(or override KP_DIR=/path/to/KernelPatch)"; \
		exit 1; \
	fi
	$(CLANG) $(KPM_CFLAGS) $(KPM_INCLUDES) -I$(CURDIR) -r \
		-o vpnhide.kpm kpm/vpnhide_kpm.c

kpm-clean:
	rm -f vpnhide.kpm

# ================================================================== #
#  LKM backend — `.ko` via Kbuild. Requires the GKI kernel headers.  #
#  These requirements must NOT fire for the `kpm` targets above.     #
# ================================================================== #
ifeq ($(filter kpm kpm-clean,$(MAKECMDGOALS)),)

# KERNEL_SRC and CLANG_DIR come from the environment (set via .env + direnv).
ifndef KERNEL_SRC
$(error KERNEL_SRC is not set — copy .env.example to .env and fill in paths)
endif
ifndef CLANG_DIR
$(error CLANG_DIR is not set — copy .env.example to .env and fill in paths)
endif

ARCH           := arm64
CROSS_COMPILE  := aarch64-linux-gnu-
CC             := $(CLANG_DIR)/clang
LD             := $(CLANG_DIR)/ld.lld
AR             := $(CLANG_DIR)/llvm-ar
NM             := $(CLANG_DIR)/llvm-nm
OBJCOPY        := $(CLANG_DIR)/llvm-objcopy
OBJDUMP        := $(CLANG_DIR)/llvm-objdump
STRIP          := $(CLANG_DIR)/llvm-strip

MAKE_ARGS := -C $(KERNEL_SRC) M=$(CURDIR) \
	ARCH=$(ARCH) LLVM=1 LLVM_IAS=1 \
	CROSS_COMPILE=$(CROSS_COMPILE) \
	CC=$(CC) LD=$(LD) AR=$(AR) NM=$(NM) \
	OBJCOPY=$(OBJCOPY) OBJDUMP=$(OBJDUMP) STRIP=$(STRIP)

all:
	$(MAKE) $(MAKE_ARGS) modules

strip: all
	$(STRIP) -d $(CURDIR)/vpnhide_kmod.ko

clean:
	$(MAKE) $(MAKE_ARGS) clean

endif # not a kpm target

endif # KERNELRELEASE
