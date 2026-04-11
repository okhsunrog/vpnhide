obj-m += vpnhide_kmod.o

ifeq ($(KERNELRELEASE),)

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

clean:
	$(MAKE) $(MAKE_ARGS) clean

endif
