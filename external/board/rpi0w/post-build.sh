#!/bin/sh
# Trims the root filesystem: it is an initramfs, every byte costs boot time.
set -e

# WiFi firmware: keep only the BCM43430 (Zero W) files. In the package they
# are symlinks into cypress/, so copy the targets before deleting the rest.
fw="${TARGET_DIR}/lib/firmware"
if [ -d "${fw}/brcm" ]; then
	keep="$(mktemp -d)"
	for ext in bin clm_blob txt; do
		cp -L "${fw}/brcm/brcmfmac43430-sdio.${ext}" "${keep}/"
	done
	rm -rf "${fw}/brcm" "${fw}/cypress"
	mkdir -p "${fw}/brcm"
	cp "${keep}"/* "${fw}/brcm/"
	rm -rf "${keep}"
fi

# Libraries that nothing in the image links against.
rm -f "${TARGET_DIR}"/usr/lib/libstdc++.so* \
	"${TARGET_DIR}"/usr/lib/libgomp.so* \
	"${TARGET_DIR}"/lib/libatomic.so* \
	"${TARGET_DIR}"/usr/lib/libnl-idiag-3.so* \
	"${TARGET_DIR}"/usr/lib/libnl-nf-3.so* \
	"${TARGET_DIR}"/usr/lib/libnl-xfrm-3.so*

# Not needed at runtime; init is our own inittab, no rcS scripts.
rm -rf "${TARGET_DIR}/usr/share/man" "${TARGET_DIR}/usr/share/doc"
rm -f "${TARGET_DIR}/etc/init.d/S"*

# dropbear: /etc/dropbear is a symlink to /var/run/dropbear; otopi-read-settings
# fills a real directory instead.
[ -L "${TARGET_DIR}/etc/dropbear" ] && rm -f "${TARGET_DIR}/etc/dropbear"
mkdir -p "${TARGET_DIR}/etc/dropbear"
chmod 700 "${TARGET_DIR}/etc/dropbear"

# No SSH keys are built into the image; they come from /boot/ssh. Remove
# leftovers of older builds.
rm -rf "${TARGET_DIR}/etc/otopi"
