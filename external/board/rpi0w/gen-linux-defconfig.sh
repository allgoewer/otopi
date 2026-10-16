#!/bin/sh
# Regenerates linux.defconfig from bcmrpi_defconfig: every module ("=m") is
# dropped, then linux.fragment adds the builtin drivers otopi needs.
# Run after a kernel version change:  make linux-extract && external/board/rpi0w/gen-linux-defconfig.sh
set -e

TOP="$(cd "$(dirname "$0")/../../.." && pwd)"
BOARD="${TOP}/external/board/rpi0w"
SRC="${TOP}/output/build/linux-custom"
TMP="$(mktemp -d)"
trap 'rm -rf "${TMP}"' EXIT
CFG="${TMP}/config"

[ -f "${SRC}/Makefile" ] || { echo "run 'make linux-extract' first" >&2; exit 1; }

# Buildroot builds the kernel in-tree; KCONFIG_CONFIG keeps its .config untouched.
kmake() {
	make -s -C "${SRC}" ARCH=arm KCONFIG_CONFIG="${CFG}" \
		CROSS_COMPILE="${TOP}/output/host/bin/arm-linux-" "$@"
}

kmake bcmrpi_defconfig
sed -i 's/^\(CONFIG_[A-Za-z0-9_]*\)=m$/# \1 is not set/' "${CFG}"
# Later assignments win when kconfig reads the file.
cat "${BOARD}/linux.fragment" >> "${CFG}"
kmake olddefconfig 2>/dev/null
kmake savedefconfig
mv "${SRC}/defconfig" "${BOARD}/linux.defconfig"

# Report fragment options that did not survive (missing dependencies).
grep -E "^CONFIG_[A-Za-z0-9_]*=y" "${BOARD}/linux.fragment" | grep -v "^CONFIG_INITRAMFS_" | while read -r opt; do
	grep -qx "${opt}" "${CFG}" || echo "warning: ${opt} not set in result" >&2
done
echo "wrote ${BOARD}/linux.defconfig"
