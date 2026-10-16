################################################################################
#
# otopi
#
################################################################################

OTOPI_VERSION = 0.1.0
OTOPI_SITE = $(BR2_EXTERNAL_OTOPI_PATH)/../app
OTOPI_SITE_METHOD = local
OTOPI_LICENSE = GPL-3.0+
OTOPI_LICENSE_FILES = LICENSE
OTOPI_OVERRIDE_SRCDIR_RSYNC_EXCLUSIONS = --exclude target --exclude VENDOR --exclude .cargo

# The copied source has no .git, so pass the version shown on screen (build.rs).
OTOPI_GIT_VERSION := $(shell git -C $(BR2_EXTERNAL_OTOPI_PATH)/.. describe --tags --always --dirty 2>/dev/null)
OTOPI_CARGO_ENV = OTOPI_GIT_VERSION=$(OTOPI_GIT_VERSION)

# Buildroot vendors crates in the download step, which the local site method
# skips. Vendor them here instead. The first build needs network access; the
# crates are cached in $(BR_CARGO_HOME). This runs before configure because
# host-rustc is only guaranteed to be built by then.
define OTOPI_VENDOR_CRATES
	mkdir -p $(@D)/.cargo $(BR_CARGO_HOME)
	cd $(@D) && $(HOST_MAKE_ENV) CARGO_HOME=$(BR_CARGO_HOME) \
		flock $(BR_CARGO_HOME)/.br-lock \
		cargo vendor --locked VENDOR > .cargo/config.toml
endef
OTOPI_PRE_CONFIGURE_HOOKS += OTOPI_VENDOR_CRATES

$(eval $(cargo-package))
