# Otopi - Otoscope Viewer via Raspberry Pi

We want to implement a raspberry-pi based viewer for a cheap otoscope with wifi.

The spec can be found in @NE3-NE7-Earpick-Protocol-Specification.md .

The device we want to implement for is the NE3.

## Requirements

* buildroot-based OS
  - Use latest buildroot, not LTS
  - Run from initramfs only, no RW partition needed
  - ~ 10 s boot, so keep modules builtin, use small init, busybox etc
* Rust-based app
  - No display server if possible, display directly to framebuffer?
  - Search for earpick wifi, connect
  - Display image, fullscreen
  - proper reconnect functionality
