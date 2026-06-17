# aic-flash

Cross-platform CLI flasher for ArtInChip SoCs.  Communicates with the
device over USB using the CBW/CSW-based UPG protocol (reverse-engineered
from the Luban-Lite SDK).

## Build

Prerequisites: [Rust] 1.70+.

```sh
cargo build --release
```

The static binary is placed at `target/release/aic-flash`.

[Rust]: https://rustup.rs

## Usage

```
aic-flash scan          # list connected ArtInChip devices
aic-flash info          # query connected device (HWINFO, storage media)
aic-flash info <img>    # parse .img file header and META entries
aic-flash burn <img>    # burn firmware image to device
aic-flash burn <img> --no-reset  # burn without resetting
```

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
