# Troubleshooting

## The screen stays on "Searching for otoscope..."

Make sure the otoscope is switched on and close to the Pi. The otoscope accepts only
one connection at a time, so close the phone app or switch off WiFi on any phone that
was connected to it.

## The screen stays black

Use the Pi's mini-HDMI port and select the right input on the screen. Some screens
need a few seconds to find the signal. If that does not help, try another screen or
cable.

## The picture rotates unexpectedly

Set `rotate = off` in `otopi.conf` (see [Configuration](../README.md#configuration)).

## The picture freezes

When the video stops, otopi keeps the last picture for up to 10 seconds, the status in
the bottom bar turns red, and it reconnects automatically. If the video does not come
back, switch the otoscope off and on again.
