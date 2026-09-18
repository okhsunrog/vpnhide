# Kbuild stub for the ABI probe. Not a shippable module: only the object file's
# debug info is consumed, by kmod/test/verify-trim-abi.sh. Build it with
#
#   make -C <kernel-tree> M=$(pwd)/kmod/test/abi ARCH=arm64 LLVM=1 \
#        KCFLAGS=-g abi-probe.o
#
# KCFLAGS=-g is what makes this work against a kernel configured with
# CONFIG_DEBUG_INFO_NONE (which the trim fragment sets): the layouts come from
# the headers, so they are identical either way, but pahole needs the DWARF.
obj-m += abi-probe.o
