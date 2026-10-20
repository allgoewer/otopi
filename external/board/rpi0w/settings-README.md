# otopi settings drive

This is the settings partition of the otopi SD card. You see it when the card is in
your computer, or as a USB drive when the card is in the Pi and the Pi's `USB` port
(not `PWR`) is connected to your computer.

Changes take effect after the Pi restarts. Eject the drive before you unplug the Pi
or switch it off.

## Files

- `otopi.conf`: settings, edit with any text editor.
  `rotate = snap` (default), `free` or `off`.
- `ssh/authorized_keys`: your SSH public key(s), one per line. Create it yourself.
- `ssh/dropbear_*_host_key`: the Pi's SSH host key, created on the first boot.
  Leave it alone.
- `README.md`: this file (read-only).

## SSH login

Connect the Pi's `USB` port to your computer. The Pi shows up as a USB network
adapter (on Windows: *Remote NDIS Compatible Device*) and gives your computer an
address. Then:

    ssh root@10.55.0.1

Login works with a key from `ssh/authorized_keys` only, no password.

More: https://github.com/allgoewer/otopi
