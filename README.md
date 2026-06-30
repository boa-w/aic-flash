# aic-flash

Cross-platform CLI flasher for ArtInChip SoCs.  Communicates with the
device over USB using the CBW/CSW-based UPG protocol (reverse-engineered
from the Luban-Lite SDK).

## Build

Prerequisites: [Rust] 1.70+.

```sh
cargo build --release
```

The CLI binary is placed at `target/release/aic-flash`.

To build the GUI:

```sh
cargo build --release --bin aic-flash-gui
```

The GUI binary is placed at `target/release/aic-flash-gui`.

[Rust]: https://rustup.rs

## Usage

```
aic-flash scan          # list connected ArtInChip devices
aic-flash info          # query connected device (HWINFO, storage media)
aic-flash info <img>    # parse .img file header and META entries
aic-flash burn <img>    # burn firmware image to device
aic-flash burn <img> --no-reset  # burn without resetting
```

## GUI

The GUI mirrors the official AiBurn workflow and reads the official default
configuration from `C:\ArtInChip\AiBurn\AiBurn.ini` when present:

```sh
cargo run --bin aic-flash-gui
```

Implemented GUI features:

- USB device scan and device info display for VID `0x33C3`, PID `0x6677`.
- AiBurn-compatible image loading, header display, image history, component
  table, component extraction, and target partition selection.
- AiBurn-style online burn flow with updater stage, reconnect wait,
  `FULL_DISK_UPGRADE`, `image.info`, selected target components, upgrade end,
  progress events, CRC checks, and optional reset.
- Settings compatible with the official `AiBurn.ini` fields:
  `auto_burn`, `is_verbose`, `read_device_log`, `adb_scan`, `retry_cnt`,
  `block_err_log`, `burn_timeout`, `language`, `image_path`, and
  `selected_parts`.
- Real GUI internationalization with Simplified Chinese (`zh_cn`) and English
  (`en`), controlled by the `language` setting and saved back to `AiBurn.ini`.
- An official tools page that wraps `upgcmd.exe` for the advanced functions
  exposed by the ArtInChip package: list devices, parse/extract image, device
  log, shell command, continue boot, go to bootloader, memory read/write,
  32-bit register read/write, memory test, execute, hexdump, fill/clear,
  full image upgrade through the official tool, dump partition, list media,
  list partition table, flash erase, RAM boot, JTAG unlock data, JTAG unlock,
  and raw `upgcmd` arguments.

By default the GUI looks for official tools under `C:\ArtInChip\AiBurn`, but
the path can be changed in Settings.

### Examples

```sh
# Scan for devices
aic-flash scan

# Inspect a firmware image
aic-flash info firmware_d21x_demo128-nand.img

# Flash the device
aic-flash burn firmware_d21x_demo128-nand.img
```

### Typical burn output

```
Image: artinchip d21x_demo128-nand v1.0.0 (4 components, 8388608 bytes)
  Magic:        AIC.FW
  Init mode:    0x0
  Current mode: 0x4
  Boot stage:   2
  Chip ID:      ...
Setting upgrade mode to FULL_DISK_UPGRADE...
  Meta: SPL (offset=0x800, size=131072, crc=0x...)
    Block size: 2048
    SPL: 131072/131072 (100.0%)
    CRC OK (0x...)
  Meta: U-Boot (offset=0x20800, size=524288, crc=0x...)
    ...
Burn completed successfully!
Device reset.
```

## Protocol

The USB protocol is fully documented in the Luban-Lite SDK
(`application/baremetal/bootloader/include/`) under Apache 2.0:

| Layer | File | Notes |
|-------|------|-------|
| Transport | `data_trans_layer.h` | CBW (USBC, 31 B) / CSW (USBS, 13 B), EP 0x02/0x81 |
| Application | `aicupg.h` | cmd_header (UPGC, 24 B), resp_header (UPGR, 24 B) |
| Commands | `basic_cmd.c`, `fwc_cmd.c` | GET_HWINFO, SET_FWC_META, SEND_FWC_DATA, ... |
| Image | `mk_image.py` | 2048 B header (AIC.FW), 512 B META entries |

- VID = `0x33C3`, PID = `0x6677`
- Bulk endpoints, no alternative setting
- Checksum: `magic + (reserved<<24|cmd<<16|ver<<8|protocol) + data_length`
