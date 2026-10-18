# otopi

![otopi searching for the otoscope](doc/searching.png)

**otopi turns a Raspberry Pi Zero W and any HDMI screen into a viewer for the NE3
WiFi otoscope (ear camera).** No phone, no app: put in the SD card, power on,
switch on the otoscope, and the live picture shows up on the screen.

## What it does

- Boots in about 6 seconds.
- Finds the otoscope's WiFi (`hndec-…` or `HNDEC_…`) and connects to it by itself.
- Shows the live video in an LCARS-style frame.
- Reconnects by itself when the otoscope is switched off and on again or loses the
  connection.
- Keeps the picture level with the otoscope's tilt sensor, so "up" stays up while you
  turn the otoscope.
- Shows the connection status, CPU load, temperature and the otopi version in the
  frame's bottom bar.
- Runs from memory only and never writes to the SD card. You can pull the power at any
  time.

## What you need

- A **Raspberry Pi Zero W** (the "W" is important: it needs WiFi)
- A **microSD card**. Any size is fine, otopi needs only about 32 MB.
- A **mini-HDMI to HDMI** cable or adapter, and a screen or TV
- A **micro-USB power supply** for the Pi's `PWR` port
- An **NE3 WiFi otoscope**

## Getting started

1. Download `otopi-<version>-sdcard.img.xz` from the
   [latest release](https://github.com/allgoewer/otopi/releases/latest).
2. Write it to the microSD card with
   [Raspberry Pi Imager](https://www.raspberrypi.com/software/) ("Use custom") or
   [balenaEtcher](https://etcher.balena.io/). Both read the `.xz` file directly.
   On Linux you can also run
   `xzcat otopi-*-sdcard.img.xz | sudo dd of=/dev/sdX bs=4M conv=fsync`
   (check `/dev/sdX` first, it is overwritten).
3. Put the card into the Pi and connect the screen.
4. Connect the power supply. After a few seconds the screen says
   *Searching for otoscope...*
5. Switch on the otoscope. otopi connects, and the video starts.

To check the download, compare it with `SHA256SUMS` from the same release:
`sha256sum -c SHA256SUMS --ignore-missing`.

## Settings

The SD card has a file `otopi.conf`. Your computer can open the card (it is a normal
FAT partition), so you can edit the file with any text editor. Changes take effect at
the next boot.

| Setting | Values | Meaning |
| --- | --- | --- |
| `rotate` | `snap` (default) | Keep the picture level, but lock to 0°, 90°, 180° or 270° when it is close to one of them (within about 8°). |
| | `free` | Keep the picture level at any angle. |
| | `off` | Never rotate the picture. |

Example:

```
rotate = off
```

## Troubleshooting

**The screen stays on "Searching for otoscope...".**
Make sure the otoscope is switched on. The otoscope accepts only one
connection at a time: close the phone app or switch off WiFi on the phone that was
connected to it. Keep the otoscope near the Pi.

**The screen stays black.**
Use the Pi's mini-HDMI port and select the right input on the screen. Some screens
need a few seconds to find the signal. If that does not help, try another screen or
cable.

**The picture turns when I do not want it to.**
Set `rotate = off` in `otopi.conf` (see [Settings](#settings)).

**The picture freezes.**
When the video stops, otopi keeps the last picture for up to 10 seconds, with the
status in the bottom bar turning red, and reconnects by itself. If it does not come back,
switch the otoscope off and on again.

## Advanced

**Boot without an SD card.** Each release also has `otopi-<version>-usbboot.tar.gz`.
Unpack it, connect the Pi's `USB` port (not `PWR`) to a computer, and run
[rpiboot](https://github.com/raspberrypi/usbboot) with
`sudo rpiboot -d <unpacked folder>`. The Pi loads otopi over the cable.

**Log in over USB.** Connect the Pi's `USB` port to a computer: the Pi shows up as a USB
network adapter. Put your SSH public key into `ssh/authorized_keys` on the SD card,
then log in with `ssh root@10.55.0.1`.

## Building from source

You need a Linux computer with the usual
[Buildroot](https://buildroot.org/) build tools (on Debian or Ubuntu:
`build-essential bc cpio file rsync unzip wget xz-utils`) and about 10 GB of free disk
space. The first build takes a while.

```sh
make                       # downloads Buildroot, builds output/images/sdcard.img
make flash DEV=/dev/sdX    # writes the image to an SD card (asks first)
make usbboot               # boots a Pi without SD card over USB
cd app && cargo test       # runs the app's tests on your computer
```

Folders:

| Folder | Contents |
| --- | --- |
| `app/` | The viewer, written in Rust |
| `external/` | Buildroot configuration, Raspberry Pi boot files and settings |
| `doc/` | Notes on the otoscope's WiFi protocol |
| `buildroot/`, `dl/`, `output/` | Created by the build (Buildroot, downloads, results) |

New releases are built by GitHub Actions when a `v*` tag is pushed.

## License

GPL-3.0-or-later, see [LICENSE](LICENSE).

otopi is not affiliated with the maker of the otoscope. It is not a medical device.
