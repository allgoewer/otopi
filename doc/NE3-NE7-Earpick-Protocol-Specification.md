# NE3 / NE7 Earpick Protocol Specification

Oct 16, 2026 · @Maik

## Scope and confidence

This specification gives the wire protocols of two WiFi earpick cameras (otoscopes). It contains sufficient data to write a new client for the NE3 video stream. The NE7 part is a first draft only.

- **NE3** (Natfire NE3, SSID `hndec-xxxx`): UDP, custom framing, the client acknowledges each frame. The device sends JPEG scan data without JPEG headers. The client adds the headers.
- **NE7** ("HNDvisual version 2" app family): UDP, broadcast discovery, the device sends complete JPEG files in chunks, plus a separate sensor stream.

Source: [haxko/NE3-Scope](https://github.com/haxko/NE3-Scope), commit `f1a5642` (2023-10-06), files `ne3.py`, `client.py`, `pcap_parser.py`, `ne7.py`, `client2.py` and the git history. The authors got the protocol from packet captures of the original phone app. No vendor document exists in the repository.

Each statement in this document has one of these confidence levels:

| Level | Meaning |
| --- | --- |
| **Confirmed** | The reference client sends or parses this value, and the client works with real devices. |
| **Constant** | The reference client sends this value unchanged. The meaning is not known. Send it as given. |
| **Inferred** | The code does not prove it. It is my interpretation of the code or of the data. Test it before you rely on it. |
| **Unknown** | The repository gives no data. |

All multi-byte integers are little-endian, except where a field says big-endian (BE). Offsets are in bytes, from the start of the UDP payload. Hex bytes are written as `ef 02`.

## NE3: network and transport

The NE3 is a WiFi access point. All traffic is UDP over IPv4. There is no TCP, no encryption and no authentication.

| Item | Value | Level |
| --- | --- | --- |
| WiFi SSID | `hndec-xxxx` (open network) | Confirmed (README). A real NE3 announced `HNDEC_55-967668`: match the prefix `hndec` without case. |
| WiFi band and channel | 2.4 GHz, channel 1 only | Confirmed (README) |
| Device IP | `192.168.169.1` | Confirmed |
| Client IP | `192.168.169.2`, from the device DHCP server | Confirmed (README) |
| Device control and video port | UDP `8800` | Confirmed |
| Device "null" port | UDP `1234` (see below) | Confirmed |
| Client port | UDP `36000` (the reference client binds this port) | Confirmed. Inferred: the device replies to the source port of the request, so other ports can also work. |
| Number of clients | 1. A second phone or PC cannot connect at the same time. | Confirmed (README) |
| Native image size | 640 × 360 pixels | Confirmed (README, old hard-coded header) |

The device sends all video packets from `192.168.169.1`. The reference PCAP parser uses only this source IP as a filter. It does not check the source port.

**Null packet.** The client sends a UDP datagram with a payload of 0 bytes to `192.168.169.1:1234`. It sends one at start and then one each 5 s. The purpose is unknown. Inferred: it keeps the session or the WiFi association alive. The reference client always sends it, so send it too.

## NE3: common framing

All NE3 datagrams on port 8800 start with the same 4-byte header. The first byte gives the direction. The second byte gives the packet type.

| Offset | Size | Field | Value |
| --- | --- | --- | --- |
| 0 | 1 | `magic` | `ef` = client to device. `93` = device to client. |
| 1 | 1 | `type` | See the table below. |
| 2 | 2 | `length` | u16 LE. Total datagram length in bytes, including this 4-byte header. |

| Direction | `magic` | `type` | Name | Level |
| --- | --- | --- | --- | --- |
| Client → device | `ef` | `00` | Connect (start stream) | Confirmed |
| Client → device | `ef` | `02` | Message batch (ACK, request, continue) | Confirmed |
| Device → client | `93` | `01` | Video chunk | Confirmed |
| Device → client | `93` | `04` | Control message. The reference client ignores it. Content unknown. | Confirmed that it exists |

The `length` field is confirmed for client packets: the client calculates it as payload length + 4. For device packets, the client reads the field but does not use it. Inferred: it has the same meaning in both directions.

A receiver must drop all datagrams where `magic` is not `93`, and all `93` datagrams where `type` is not `01`. It can log type `04` for analysis.

## NE3: client-to-device packets

The client sends 2 packet types to `192.168.169.1:8800`. Type `00` starts the stream. Type `02` carries 0 or more messages that control the frame flow.

### Connect (`ef 00`)

The packet has 4 bytes and no payload: `ef 00 04 00`. Send it at start. Send it again if no video packet comes for 0.5 s. Level: confirmed.

### Message batch (`ef 02`)

| Offset | Size | Field | Value | Level |
| --- | --- | --- | --- | --- |
| 0 | 4 | header | `ef 02` + `length` (u16 LE) | Confirmed |
| 4 | 4 | `prefix` | `02 02 00 01` | Constant |
| 8 | 8 | `msg_count` | u64 LE. Number of messages that follow. | Confirmed |
| 16 | 8 | reserved | `00` × 8 | Constant |
| 24 | 8 | `magic2` | `0a 4b 14 2d 00 00 00 00`. Inferred: a session or app identifier (as u64 LE: 0x2D144B0A). | Constant |
| 32 | var | `messages` | `msg_count` messages, back to back | Confirmed |
| end − 8 | 8 | trailer | `00` × 8 | Constant |

Total length = 40 + sum of message lengths.

Each message starts with a 16-byte header:

| Offset in message | Size | Field | Value |
| --- | --- | --- | --- |
| 0 | 8 | `img_number` | u64 LE. The frame that the message refers to. |
| 8 | 4 | `msg_type` | u32 LE. `1` = ACK, `3` = request. |
| 12 | 4 | `msg_len` | u32 LE. Message length in bytes, including this 16-byte header. |
| 16 | var | `msg_data` | `msg_len − 16` bytes. |

| `msg_type` | Name | `msg_len` | `msg_data` | Meaning |
| --- | --- | --- | --- | --- |
| 1 | ACK | 20 | `ff ff ff ff` | The client received all chunks of frame `img_number`. |
| 3 | Request | 16 | none | The client wants frame `img_number`. |

The 4 bytes of ACK data are not known. The reference client sends `ff ff ff ff`. The original app sends other values here (comment in `client.py`). Inferred: it is a bitmask of the received chunks, or a statistic. `ff ff ff ff` works.

### The 3 batches that the client uses

1. **Continue** (`msg_count` = 0). The client sends this while a frame is incomplete. It tells the device to continue the transmission. Length 40.
2. **ACK + Request** (`msg_count` = 2). The client sends this when frame *n* is complete: ACK for *n*, then Request for *n* + 1. Length 76.
3. **Connect** (`ef 00`, see above).

Example: Continue, 40 bytes.

```
0000  ef 02 28 00 02 02 00 01 00 00 00 00 00 00 00 00
0010  00 00 00 00 00 00 00 00 0a 4b 14 2d 00 00 00 00
0020  00 00 00 00 00 00 00 00
```

Example: ACK for frame 7 + Request for frame 8, 76 bytes.

```
0000  ef 02 4c 00 02 02 00 01 02 00 00 00 00 00 00 00
0010  00 00 00 00 00 00 00 00 0a 4b 14 2d 00 00 00 00
0020  07 00 00 00 00 00 00 00 01 00 00 00 14 00 00 00
0030  ff ff ff ff 08 00 00 00 00 00 00 00 03 00 00 00
0040  10 00 00 00 00 00 00 00 00 00 00 00
```

Note: an old version of the client encoded `img_number` as a 4-byte integer. Commit `0f58bc7` changed it to 8 bytes. Use 8 bytes.

## NE3: video packets (`93 01`)

The device divides each frame into chunks of 1024 bytes of JPEG scan data. It sends each chunk in one datagram with a 56-byte header. Chunk 0 also carries the tilt sensor value.

### Header (56 bytes)

Python `struct` format: `<xxHxxxxQQxxxxxxxxIIIHHBxxxxxxx`.

| Offset | Size | Field | Type | Meaning | Level |
| --- | --- | --- | --- | --- | --- |
| 0 | 1 | `magic` | u8 | `93` | Confirmed |
| 1 | 1 | `type` | u8 | `01` | Confirmed |
| 2 | 2 | `length` | u16 LE | Datagram length | Read, not used. Inferred meaning. |
| 4 | 4 | — | — | Not decoded | Unknown |
| 8 | 8 | `img_number` | u64 LE | Frame number. It increases by 1 for each frame. | Confirmed |
| 16 | 8 | `img_number2` | u64 LE | Second frame number. Read, not used. | Unknown |
| 24 | 8 | — | — | Not decoded | Unknown |
| 32 | 4 | `packet_number` | u32 LE | Chunk index, 0-based | Confirmed |
| 36 | 4 | `packet_count` | u32 LE | Number of chunks in this frame | Confirmed |
| 40 | 4 | `img_size` | u32 LE | Read, not used. Inferred: total scan data length in bytes. | Inferred |
| 44 | 2 | `img_width` | u16 LE | Image width in pixels (640) | Confirmed |
| 46 | 2 | `img_height` | u16 LE | Image height in pixels (360) | Confirmed |
| 48 | 1 | `img_quality` | u8 | JPEG quality: 5, 10, 25, 50, 75 or 100 | Confirmed |
| 49 | 7 | — | — | Not decoded | Unknown |

### Payload (offset 56 to end)

| Chunk | Payload content |
| --- | --- |
| `packet_number` = 0 | 1024 bytes of scan data, then the tilt string (see "Tilt sensor") |
| `packet_number` > 0 | Up to 1024 bytes of scan data. Inferred: only the last chunk is shorter than 1024 bytes. |

The reference client keeps the first 1024 bytes of each payload (`payload[0:1024]`). It drops the rest.

### Reassembly

1. Read `img_number`. If it is larger than the number of the current frame, drop the current frame and start a new frame.
2. If `img_number` is smaller than the number of the current frame, drop the packet. It is late.
3. Store the chunk at index `packet_number`. A duplicate chunk replaces the previous copy.
4. The frame is complete when the number of stored chunks equals `packet_count`.
5. Join the chunks in index order 0 … `packet_count` − 1. Then make the JPEG file (see "JPEG reconstruction").

The reference client does not use `img_size`. If the joined data is longer than `img_size`, cut it to `img_size` (inferred, not tested). A frame of only 1 chunk shorter than 1024 bytes cannot contain the tilt string at offset 1024. The reference client does not handle this case.

## NE3: JPEG reconstruction

The scan data is a baseline JPEG entropy-coded segment without markers before it. The client must put a 605-byte JPEG header before the data and `ff d9` (EOI) after it. Then a standard JPEG decoder can read the file.

JPEG file = header (605 bytes) + chunk 0 + chunk 1 + … + chunk (`packet_count` − 1) + `ff d9`.

### Header segments, in this order

| # | Marker | Bytes | Content |
| --- | --- | --- | --- |
| 1 | SOI | 2 | `ff d8` |
| 2 | DQT, table 0 | 69 | `ff db 00 43 00` + 64 luminance values, zigzag order |
| 3 | DQT, table 1 | 69 | `ff db 00 43 01` + 64 chrominance values, zigzag order |
| 4 | SOF0 | 19 | `ff c0 00 11 08` + height (u16 **BE**) + width (u16 **BE**) + `03 01 11 00 02 11 01 03 11 01` |
| 5 | DHT DC 0 | 33 | Standard luminance DC table (JPEG Annex K) |
| 6 | DHT AC 0 | 183 | Standard luminance AC table (Annex K) |
| 7 | DHT DC 1 | 33 | Standard chrominance DC table (Annex K) |
| 8 | DHT AC 1 | 183 | Standard chrominance AC table (Annex K) |
| 9 | SOS | 14 | `ff da 00 0c 03 01 00 02 11 03 11 00 3f 00` |

SOF0 gives 8-bit samples and 3 components. All components use 1×1 sampling, which is 4:4:4. Y uses quantization table 0. Cb and Cr use table 1. SOS selects Huffman tables 0/0 for Y and 1/1 for Cb and Cr. There is no DRI marker, so there are no restart intervals. Take height and width from the packet header (offset 46 and 44). Note that SOF0 has height first.

### Quantization tables

The repository contains fixed tables for `img_quality` 5, 10, 25, 50, 75 and 100. For other values it uses the quality 100 table. I compared all 12 tables with the IJG (libjpeg) quality scaling of the Annex K base tables. All 12 tables are equal to the IJG result. Thus you can calculate the tables for each quality value q as follows:

```
S   = floor(5000 / q)   if q < 50
S   = 200 - 2*q         if q >= 50
T_i = min(255, max(1, floor((B_i * S + 50) / 100)))
```

B is the Annex K base table (luminance or chrominance). Write the 64 values of T in zigzag order. Level: confirmed for q = 5, 10, 25, 50, 75, 100. Inferred for other q values; the device was not seen to send them.

### Fixed bytes

The 4 DHT segments are the same for all frames. Copy them unchanged:

```
ffc4001f0000010501010101010100000000000000000102030405060708090a0b
ffc400b5100002010303020403050504040000017d01020300041105122131410613516107227114328191a1082342b1c11552d1f02433627282090a161718191a25262728292a3435363738393a434445464748494a535455565758595a636465666768696a737475767778797a838485868788898a92939495969798999aa2a3a4a5a6a7a8a9aab2b3b4b5b6b7b8b9bac2c3c4c5c6c7c8c9cad2d3d4d5d6d7d8d9dae1e2e3e4e5e6e7e8e9eaf1f2f3f4f5f6f7f8f9fa
ffc4001f0100030101010101010101010000000000000102030405060708090a0b
ffc400b51100020102040403040705040400010277000102031104052131061241510761711322328108144291a1b1c109233352f0156272d10a162434e125f11718191a262728292a35363738393a434445464748494a535455565758595a636465666768696a737475767778797a82838485868788898a92939495969798999aa2a3a4a5a6a7a8a9aab2b3b4b5b6b7b8b9bac2c3c4c5c6c7c8c9cad2d3d4d5d6d7d8d9dae2e3e4e5e6e7e8e9eaf2f3f4f5f6f7f8f9fa
```

The DQT values for quality 50 (zigzag order) are:

```
ffdb004300100b0c0e0c0a100e0d0e1211101318281a181616183123251d283a333d3c3933383740485c4e404457453738506d51575f626768673e4d71797064785c656763
ffdb0043011112121815182f1a1a2f634238426363636363636363636363636363636363636363636363636363636363636363636363636363636363636363636363636363
```

For the other 5 quality values, see `Frame.assemble_jpeg_header` in `ne3.py`, or use the formula above.

## NE3: tilt sensor and image rotation

Chunk 0 of each frame carries 2 ASCII integers, x and y, after the 1024 bytes of scan data. The client uses them to turn the image so that it stays level.

| Datagram offset | Payload offset | Size | Content |
| --- | --- | --- | --- |
| 1080 | 1024 | 5 | x, ASCII decimal |
| 1085 | 1029 | 1 | Separator. The client skips it. Character not known. |
| 1086 | 1030 | to end | y, ASCII decimal |

The reference client parses each field with Python `int()`. Thus a field can have a sign and spaces before or after the digits. A field must not contain NUL bytes. Level: confirmed for the position. The exact text format (padding, sign, separator) is unknown, because the repository has no capture.

### Angle calculation (client side)

1. If x = 0 and y = 1024, set angle = 90°. Commit `59dff13` adds this case with the message "turn off rotation if tilted up or down". Inferred: the device sends (0, 1024) when the tilt is not usable.
2. Else, angle = atan2(x, y) in degrees. Note the argument order: x first.
3. Turn the image by (−angle − 90)° about its center. The output size stays the same. The reference client uses OpenCV `getRotationMatrix2D`, where a positive value turns the image counterclockwise.

Inferred: 1024 is the value for 1 g, so x and y are accelerometer values in the image plane.

## NE3: session flow and timers

The client requests each frame. After it receives frame *n*, it sends ACK *n* + Request *n* + 1. While a frame is incomplete, it sends Continue batches. Silence on the socket causes a resend, a reconnect, or an exit. Inferred: the device waits for these requests, because the client never receives frames without them.

```
            +--------------------+     +------------------------+     +--------------------------+
            | Start or reconnect | --> | Receive frame n        | --> | Frame n complete         |
            | null pkt -> :1234  |     | store chunk            |     | build JPEG, show image   |
            | ef 00 04 00 ->8800 |     | Continue, max 1/25 ms  |     | send ACK n + Request n+1 |
            +--------------------+     +------------------------+     +--------------------------+
                      ^                   |        ^       ^                       |
                      |       no datagram |        |       +---- next frame n+1 ---+
                      |                   v        | resend
  No datagram   +---------------+   +------------------------------+   +-----------------------+
  received for: | 0.5 s or more |   | 25 ms                        |   | 120 s                 |
                | send Connect, |   | frame done: ACK + Request    |   | stop: connection lost |
                | drop frame    |   | frame open: Continue         |   | (client exits)        |
                +---------------+   +------------------------------+   +-----------------------+
```

The 25 ms rule keeps the stream moving. The 0.5 s rule starts a new session. Only 120 s of silence ends the client.

| Timer | Value | Action of the reference client |
| --- | --- | --- |
| Receive timeout | 25 ms | Check the silence rules below. |
| Continue rate | ≥ 25 ms between 2 Continue batches | Sent after a received chunk while the frame is incomplete. |
| Silence ≥ 25 ms, last frame complete | — | Resend ACK *n* + Request *n* + 1. |
| Silence ≥ 25 ms, frame incomplete | — | Send Continue. |
| Silence > 0.5 s | — | Send Connect. Drop the current frame. Set the last complete frame to 0. Repeat at each 25 ms timeout until data comes. |
| Silence > 120 s | — | Stop: "Connection lost". |
| Null packet | 5 s | Empty datagram to port 1234. The client checks this timer only after a received datagram. |

The silence rules use the time of the last received datagram of any type, also non-video datagrams. The first frame number after Connect is not documented. The client accepts any first `img_number`.

## NE7 protocol (draft)

The NE7 uses a different protocol. The client finds the device with a UDP broadcast and then starts 2 streams: video on port 44506 and sensor data on port 52219. The device sends complete JPEG files, so the client does not add a header. The repository has only an "initial" client for it (`client2.py`, commit `372c5fc`). Thus most of this section is less certain than the NE3 sections.

### Network

| Item | Value | Level |
| --- | --- | --- |
| Subnet | 192.168.1.0/24 (the client broadcasts to `192.168.1.255`) | Inferred |
| Client port | UDP `36123`, with `SO_BROADCAST` set | Confirmed |
| Discovery ports | UDP `58090` and `46526` on the broadcast address | Confirmed |
| Video port | UDP `44506` on the device | Confirmed |
| Sensor port | UDP `52219` on the device | Confirmed |
| SSID, DHCP | Not documented | Unknown |

### Session start

1. Send `66 39 01 01` to `192.168.1.255:58090`. Send the same 4 bytes to `192.168.1.255:46526`.
2. Wait up to 1 s for a reply. The reply is UTF-8 JSON. The keys are not documented. Use the reply source IP as the device IP.
3. Send `86 06 01` to `<device>:52219`. This starts the sensor stream (inferred).
4. Send `20 36` to `<device>:44506`. This starts the video stream (inferred).
5. Receive datagrams. Sort them by source port: 44506 = video, 52219 = sensor.

The reference client sends no ACK and no keep-alive after step 4. Unknown: whether the device stops after some time without client packets.

### Video datagram (source port 44506)

| Offset | Size | Field | Meaning |
| --- | --- | --- | --- |
| 0 | 1 | `frame_number` | u8. Frame counter. It wraps at 255 (inferred). |
| 1 | 1 | `flags` | Bit 0 = 1: last chunk of the frame. Other bits unknown. |
| 2 | 1 | `chunk_number` | u8. Chunk index, **1-based**. |
| 3 | 1 | `flags2` | Unknown. |
| 4 | var | data | JPEG bytes. If bit 0 of `flags` is 1, the last 5 bytes are a trailer, not JPEG data. |

The content of the 5-byte trailer is not known. The reference client prints it in hex and drops it.

Reassembly:

1. If `frame_number` is different from the current frame, start a new frame.
2. Store the data at index `chunk_number` − 1. On the last chunk, remove the 5-byte trailer first.
3. The frame is complete when the last chunk has arrived and the number of stored chunks equals its `chunk_number`.
4. Join the chunks in index order. The result is a complete JPEG file. Decode it directly.

### Sensor datagram (source port 52219, 24 bytes)

Python `struct` format: `>BxBxBxxxxxxxxxxxxxHxxxx`.

| Offset | Size | Field | Meaning |
| --- | --- | --- | --- |
| 0 | 1 | x | u8. Inferred: accelerometer X. |
| 1 | 1 | — | Not decoded. Inferred: maybe the other byte of a 16-bit x. |
| 2 | 1 | y | u8. Inferred: accelerometer Y. |
| 3 | 1 | — | Not decoded |
| 4 | 1 | z | u8. Inferred: accelerometer Z. |
| 5 | 13 | — | Not decoded |
| 18 | 2 | `angle` | u16 **BE**. Roll angle in degrees (inferred from use). |
| 20 | 4 | — | Not decoded |

The reference client turns the image by −`angle` degrees (OpenCV, positive = counterclockwise). It shows the last complete frame each time a sensor datagram arrives.

### Known faults of the NE7 reference client

- It stops with an exception when no datagram arrives for 0.25 s. There is no reconnect.
- It fails if a sensor datagram arrives before the first video datagram.
- A late chunk of the previous frame starts a new frame and discards the current one.

## Unknowns and test checklist

The NE3 video path is complete enough to reimplement. The gaps below do not stop a working viewer, but a full client must solve them with packet captures.

### Open questions

- NE3 `93 04` control messages: content, period, and whether they carry battery level or button events.
- NE3 header bytes 4–7, 16–31 and 49–55, including `img_number2`.
- NE3 ACK data: the 4 bytes that the original app sends in place of `ff ff ff ff`.
- NE3 batch constants `02 02 00 01` and `0a 4b 14 2d`: are they fixed, per app, or per session?
- NE3 port 1234: what the device does with the empty datagram, and what happens if the client does not send it.
- NE3 tilt string: the exact text format and the separator character.
- NE3 frame rate, and whether the device can change quality or resolution on request.
- NE7: the discovery JSON keys, the 5-byte trailer, `flags2`, and most of the sensor datagram.

### Test checklist for a new NE3 client

- [ ] Bind a UDP socket. Send the null packet to port 1234 and `ef 00 04 00` to port 8800.
- [ ] Receive `93 01` packets. Check that `packet_number` < `packet_count` and that width × height = 640 × 360.
- [ ] Make a JPEG from 1 complete frame. Check that a standard decoder (libjpeg, OpenCV) opens it.
- [ ] Send Continue every 25 ms while a frame is incomplete. Check that the device sends the missing chunks.
- [ ] Send ACK + Request after each complete frame. Check that `img_number` increases.
- [ ] Stop sending for 1 s. Check that a new Connect starts the stream again.
- [ ] Parse the tilt values. Turn the device and check the sign of the rotation.
- [ ] Log all `93 04` packets for later analysis.
