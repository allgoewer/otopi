# Wrapper around Buildroot with the BR2_EXTERNAL tree in external/.
#   make            download Buildroot (once), configure, build
#   make menuconfig / linux-menuconfig / busybox-menuconfig / <any Buildroot target>
#   make flash DEV=/dev/sdX
#   make usbboot        boot a Pi Zero (no SD card) over its USB OTG port

TOP        := $(CURDIR)
BR_VERSION := $(shell cat $(TOP)/buildroot.version)
BR_DIR     := $(TOP)/buildroot/buildroot-$(BR_VERSION)
BR_TARBALL := $(TOP)/buildroot/buildroot-$(BR_VERSION).tar.xz
OUTPUT     := $(TOP)/output
DEFCONFIG  := otopi_rpi0w_defconfig

BR_MAKE = $(MAKE) -C $(BR_DIR) O=$(OUTPUT) BR2_EXTERNAL=$(TOP)/external BR2_DL_DIR=$(TOP)/dl

.PHONY: all buildroot defconfig savedefconfig flash usbboot clean-output

BOARD_DIR := $(TOP)/external/board/rpi0w
FW_DIR    := $(OUTPUT)/images/rpi-firmware

all: $(OUTPUT)/.config
	@# rpi-firmware copies config.txt/cmdline.txt only when it is installed.
	@for f in config.txt cmdline.txt; do \
		if [ -f $(FW_DIR)/$$f ] && [ $(BOARD_DIR)/$$f -nt $(FW_DIR)/$$f ]; then \
			$(BR_MAKE) rpi-firmware-reinstall || exit 1; break; \
		fi; \
	done
	@# The app is a local package: Buildroot copies its source only once.
	$(BR_MAKE) otopi-rebuild
	$(BR_MAKE)

buildroot: $(BR_DIR)/Makefile

$(BR_DIR)/Makefile:
	mkdir -p $(TOP)/buildroot
	curl -fL -o $(BR_TARBALL) https://buildroot.org/downloads/buildroot-$(BR_VERSION).tar.xz
	tar -C $(TOP)/buildroot -xf $(BR_TARBALL)
	touch $@

$(OUTPUT)/.config: $(BR_DIR)/Makefile $(TOP)/external/configs/$(DEFCONFIG)
	$(BR_MAKE) $(DEFCONFIG)

defconfig: $(BR_DIR)/Makefile
	$(BR_MAKE) $(DEFCONFIG)

# Write the current configuration back to external/configs/.
savedefconfig: $(OUTPUT)/.config
	$(BR_MAKE) savedefconfig BR2_DEFCONFIG=$(TOP)/external/configs/$(DEFCONFIG)

flash: $(OUTPUT)/images/sdcard.img
	@test -n "$(DEV)" || { echo "usage: make flash DEV=/dev/sdX"; exit 1; }
	@lsblk $(DEV)
	@read -p "Overwrite $(DEV)? [y/N] " a && [ "$$a" = y ]
	sudo dd if=$< of=$(DEV) bs=4M conv=fsync status=progress

# Serves output/images/usbboot to the Pi's boot ROM. Connect the Pi's "USB"
# port (not "PWR") without an SD card. rpiboot needs root for raw USB access.
# RPIBOOT_OPTS="-v -l": verbose, and keep serving boot files across reboots.
usbboot: $(OUTPUT)/images/usbboot/zImage
	sudo $(OUTPUT)/host/bin/rpiboot $(RPIBOOT_OPTS) -d $(OUTPUT)/images/usbboot

clean-output:
	rm -rf $(OUTPUT)

# Everything else goes to Buildroot.
Makefile: ;
$(TOP)/external/configs/$(DEFCONFIG): ;

%: $(BR_DIR)/Makefile
	$(BR_MAKE) $@
