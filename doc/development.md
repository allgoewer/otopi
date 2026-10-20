# Development

## Overview

otopi is a [Buildroot](https://buildroot.org/) image for the Raspberry Pi Zero W with a
viewer app written in Rust. The system runs from an initramfs, so the power can be
removed at any time. Settings and SSH keys live on a separate FAT partition (`OTOPI`),
which is also exported over USB as a mass storage device.

## Building

You need a Linux computer with the usual Buildroot build tools (on Debian or Ubuntu:
`build-essential bc cpio file rsync unzip wget xz-utils`) and about 10 GB of free disk
space. The first build takes a while.

```sh
make                       # downloads Buildroot, builds output/images/sdcard.img
make flash DEV=/dev/sdX    # writes the image to an SD card (asks first)
make usbboot               # boots a Pi without SD card over USB
cd app && cargo test       # runs the app's tests on the host
```

## Repository layout

| Folder | Contents |
| --- | --- |
| `app/` | The viewer, written in Rust |
| `external/` | Buildroot configuration, Raspberry Pi boot files and settings partition |
| `doc/` | Documentation and notes on the otoscope's WiFi protocol |
| `buildroot/`, `dl/`, `output/` | Created by the build (Buildroot, downloads, results) |

## Booting over USB

Each release contains `otopi-<version>-usbboot.tar.gz`. Unpack it, connect the Pi's
`USB` port (not `PWR`) to a computer and run
[rpiboot](https://github.com/raspberrypi/usbboot):

```sh
sudo rpiboot -d <unpacked folder>
```

## SSH login

With the Pi's `USB` port connected to a computer, the Pi shows up as a USB network
adapter (on Windows: *Remote NDIS Compatible Device*) and as a USB drive. Put your SSH
public key into `ssh/authorized_keys` on the `OTOPI` drive, restart the Pi, then:

```sh
ssh root@10.55.0.1
```

## Releases

GitHub Actions builds a release when a `v*` tag is pushed.
