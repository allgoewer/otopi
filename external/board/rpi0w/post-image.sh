#!/bin/sh
# Builds sdcard.img: one FAT partition with firmware, DTB and the kernel
# (which contains the initramfs).
set -e

BOARD_DIR="$(dirname "$0")"
GENIMAGE_TMP="${BUILD_DIR}/genimage.tmp"
ROOTPATH_TMP="$(mktemp -d)"
trap 'rm -rf "${ROOTPATH_TMP}"' EXIT

cp "${BOARD_DIR}/otopi.conf" "${BINARIES_DIR}/otopi.conf"

rm -rf "${GENIMAGE_TMP}"
genimage \
	--rootpath "${ROOTPATH_TMP}" \
	--tmppath "${GENIMAGE_TMP}" \
	--inputpath "${BINARIES_DIR}" \
	--outputpath "${BINARIES_DIR}" \
	--config "${BOARD_DIR}/genimage.cfg"

# The same boot files as a directory, for USB boot: make usbboot
rm -rf "${BINARIES_DIR}/usbboot"
mkdir -p "${BINARIES_DIR}/usbboot/overlays"
cp "${BINARIES_DIR}"/rpi-firmware/bootcode.bin \
	"${BINARIES_DIR}"/rpi-firmware/start_cd.elf \
	"${BINARIES_DIR}"/rpi-firmware/fixup_cd.dat \
	"${BINARIES_DIR}"/rpi-firmware/config.txt \
	"${BINARIES_DIR}"/rpi-firmware/cmdline.txt \
	"${BINARIES_DIR}"/bcm2708-rpi-zero-w.dtb \
	"${BINARIES_DIR}"/zImage \
	"${BINARIES_DIR}"/otopi.conf \
	"${BINARIES_DIR}/usbboot/"
for o in vc4-kms-v3d disable-bt dwc2; do
	cp "${BINARIES_DIR}/rpi-firmware/overlays/${o}.dtbo" "${BINARIES_DIR}/usbboot/overlays/"
done
