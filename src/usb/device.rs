use std::thread;
use std::time::{Duration, Instant};

use rusb::{DeviceHandle, UsbContext};

use crate::protocol::cbw_csw::*;
use crate::protocol::commands::*;

const AIC_VID: u16 = 0x33C3;
const AIC_PID: u16 = 0x6677;
const BULK_OUT_EP: u8 = 0x02;
const BULK_IN_EP: u8 = 0x81;
const TIMEOUT_MS: Duration = Duration::from_secs(30);
const SHORT_TIMEOUT: Duration = Duration::from_millis(500);
const CHUNK_SIZE: u32 = 1024 * 1024;
const BULK_WRITE_CHUNK: usize = 16 * 1024;
const DEFAULT_BURN_TIMEOUT: Duration = Duration::from_secs(60);
const DEFAULT_SELECTED_PARTS: &[&str] = &["spl", "env", "os"];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CswPolicy {
    Required,
    AllowMissing,
}

#[derive(Clone, Debug)]
struct UpgResponse {
    payload: Vec<u8>,
}

pub struct AicDevice {
    handle: DeviceHandle<rusb::Context>,
    tag: u32,
    in_buf: Vec<u8>,
}

impl AicDevice {
    pub fn open_first() -> Result<Self, String> {
        let context = rusb::Context::new().map_err(|e| format!("Failed to init USB: {}", e))?;

        let devices = context
            .devices()
            .map_err(|e| format!("Failed to list USB devices: {}", e))?;

        for device in devices.iter() {
            let desc = device
                .device_descriptor()
                .map_err(|e| format!("Failed to get device descriptor: {}", e))?;
            if desc.vendor_id() == AIC_VID && desc.product_id() == AIC_PID {
                let handle = device
                    .open()
                    .map_err(|e| format!("Failed to open device: {}", e))?;
                if let Ok(active) = handle.kernel_driver_active(0) {
                    if active {
                        let _ = handle.detach_kernel_driver(0);
                    }
                }
                if let Ok(desc) = device.config_descriptor(0) {
                    for iface in desc.interfaces() {
                        for desc in iface.descriptors() {
                            eprintln!(
                                "  Interface {}: {} endpoints",
                                desc.interface_number(),
                                desc.num_endpoints()
                            );
                            for ep in desc.endpoint_descriptors() {
                                eprintln!(
                                    "    EP 0x{:02x} {} max_packet={}",
                                    ep.address(),
                                    if ep.direction() == rusb::Direction::In {
                                        "IN"
                                    } else {
                                        "OUT"
                                    },
                                    ep.max_packet_size()
                                );
                            }
                        }
                    }
                }

                handle
                    .claim_interface(0)
                    .map_err(|e| format!("Failed to claim interface: {}", e))?;

                let _ = handle.clear_halt(BULK_OUT_EP);
                let _ = handle.clear_halt(BULK_IN_EP);
                eprintln!(
                    "  Cleared halt on EP 0x{:02x} and 0x{:02x}",
                    BULK_OUT_EP, BULK_IN_EP
                );

                let mut dev = Self {
                    handle,
                    tag: 1,
                    in_buf: Vec::new(),
                };
                dev.drain_in_endpoint(Duration::from_millis(50), 64 * 1024)?;
                return Ok(dev);
            }
        }
        Err("No ArtInChip device found (VID=0x33C3, PID=0x6677)".to_string())
    }

    fn reopen(&mut self) -> Result<(), String> {
        let replacement = Self::open_first()?;
        *self = replacement;
        Ok(())
    }

    fn wait_reconnect(&mut self, timeout: Duration) -> Result<(), String> {
        eprintln!("Waiting for ArtInChip device to reconnect...");
        let deadline = Instant::now() + timeout;
        let mut last_err = String::new();
        while Instant::now() < deadline {
            match self.reopen() {
                Ok(()) => {
                    eprintln!("Device reconnected.");
                    return Ok(());
                }
                Err(e) => last_err = e,
            }
            thread::sleep(Duration::from_millis(500));
        }
        Err(format!(
            "Timed out waiting for device reconnect: {}",
            last_err
        ))
    }

    fn drain_in_endpoint(&mut self, timeout: Duration, max_bytes: usize) -> Result<(), String> {
        let mut total = 0usize;
        let mut buf = [0u8; 512];
        loop {
            match self.handle.read_bulk(BULK_IN_EP, &mut buf, timeout) {
                Ok(n) => {
                    if n == 0 {
                        break;
                    }
                    total += n;
                    eprintln!("  Flushed {} stale bytes from IN EP", n);
                    if total >= max_bytes {
                        eprintln!("  Stopped IN flush after {} bytes", total);
                        break;
                    }
                }
                Err(rusb::Error::Timeout) => break,
                Err(e) => return Err(format!("IN EP flush error: {}", e)),
            }
        }
        Ok(())
    }

    // ── Low-level transactions ─────────────────────────────────────────

    /// Full write transaction: CBW → data → CSW
    fn write_txn(&mut self, payload: &[u8]) -> Result<AicCsw, String> {
        self.write_txn_policy(payload, CswPolicy::Required)?
            .ok_or_else(|| "CSW unexpectedly missing".to_string())
    }

    fn write_txn_policy(
        &mut self,
        payload: &[u8],
        policy: CswPolicy,
    ) -> Result<Option<AicCsw>, String> {
        let tag = self.next_tag();

        let cbw = AicCbw::new_write(tag, payload.len() as u32);
        let cbw_bytes = cbw.to_bytes();
        eprintln!(
            "  >> WRITE CBW tag={} len={} cbw={:02x?}",
            tag,
            payload.len(),
            cbw_bytes
        );
        self.write_bulk(cbw_bytes)?;
        if !payload.is_empty() {
            eprintln!("  >> DATA len={}", payload.len());
            self.write_bulk(payload)?;
        }
        let csw = self.read_csw(tag, policy)?;
        if let Some(csw) = &csw {
            eprintln!(
                "  << CSW tag={} status={} residue={} sig=0x{:08x}",
                csw.tag_val(),
                csw.status_val(),
                csw.data_residue_val(),
                csw.signature()
            );
            self.check_csw(csw, tag)?;
        }
        Ok(csw)
    }

    /// Full read transaction: CBW → data → CSW
    fn read_txn(&mut self, read_len: u32) -> Result<Vec<u8>, String> {
        let tag = self.next_tag();

        let cbw = AicCbw::new_read(tag, read_len);
        eprintln!(
            "  >> READ CBW tag={} len={} cbw={:02x?}",
            tag,
            read_len,
            cbw.to_bytes()
        );
        self.write_bulk(cbw.to_bytes())?;

        let data = self.read_exact_from_in(read_len as usize, TIMEOUT_MS)?;
        eprintln!("  << DATA {} bytes", data.len());

        let csw = self
            .read_csw(tag, CswPolicy::Required)?
            .ok_or_else(|| "CSW unexpectedly missing".to_string())?;
        eprintln!(
            "  << CSW tag={} status={} residue={} sig=0x{:08x}",
            csw.tag_val(),
            csw.status_val(),
            csw.data_residue_val(),
            csw.signature()
        );
        self.check_csw(&csw, tag)?;
        Ok(data)
    }

    fn next_tag(&mut self) -> u32 {
        let tag = self.tag;
        self.tag = self.tag.wrapping_add(1).max(1);
        tag
    }

    fn write_bulk(&self, data: &[u8]) -> Result<(), String> {
        let mut written = 0usize;
        while written < data.len() {
            let end = (written + BULK_WRITE_CHUNK).min(data.len());
            let n = self
                .handle
                .write_bulk(BULK_OUT_EP, &data[written..end], TIMEOUT_MS)
                .map_err(|e| format!("Bulk write failed at {}/{}: {}", written, data.len(), e))?;
            if n == 0 {
                return Err("Bulk write made no progress".to_string());
            }
            written += n;
        }
        Ok(())
    }

    fn read_bulk_to_buffer(&mut self, timeout: Duration) -> Result<usize, rusb::Error> {
        let mut buf = [0u8; 64 * 1024];
        let n = self.handle.read_bulk(BULK_IN_EP, &mut buf, timeout)?;
        if n > 0 {
            eprintln!("  << IN EP raw {} bytes: {:02x?}", n, &buf[..n.min(128)]);
            self.in_buf.extend_from_slice(&buf[..n]);
        }
        Ok(n)
    }

    fn read_exact_from_in(&mut self, len: usize, timeout: Duration) -> Result<Vec<u8>, String> {
        let deadline = Instant::now() + timeout;
        while self.in_buf.len() < len {
            let now = Instant::now();
            if now >= deadline {
                return Err(format!(
                    "Bulk read timed out with {}/{} bytes buffered",
                    self.in_buf.len(),
                    len
                ));
            }
            let remaining = deadline.saturating_duration_since(now);
            match self.read_bulk_to_buffer(remaining.min(TIMEOUT_MS)) {
                Ok(0) => {}
                Ok(n) => eprintln!(
                    "  << DATA buffered {} bytes (buffer={}/{})",
                    n,
                    self.in_buf.len(),
                    len
                ),
                Err(rusb::Error::Timeout) => {
                    return Err(format!(
                        "Bulk read timed out with {}/{} bytes buffered",
                        self.in_buf.len(),
                        len
                    ));
                }
                Err(e) => return Err(format!("Bulk read failed: {}", e)),
            }
        }
        Ok(self.in_buf.drain(..len).collect())
    }

    fn find_csw_signature(&self) -> Option<usize> {
        let sig = AIC_USB_SIGN_USBS.to_le_bytes();
        self.in_buf.windows(4).position(|w| w == sig)
    }

    fn read_csw(&mut self, expected_tag: u32, policy: CswPolicy) -> Result<Option<AicCsw>, String> {
        let deadline = Instant::now() + TIMEOUT_MS;
        loop {
            if let Some(pos) = self.find_csw_signature() {
                if pos > 0 {
                    eprintln!("  << Dropping {} non-CSW stale bytes before USBS", pos);
                    self.in_buf.drain(..pos);
                }
                if self.in_buf.len() < 13 {
                    self.fill_until(deadline, 13)?;
                    continue;
                }
                let csw = AicCsw::from_bytes(&self.in_buf[..13])
                    .ok_or_else(|| "Failed to parse CSW".to_string())?;
                eprintln!(
                    "  << CSW candidate sig=0x{:08x} tag={} status={} residue={}",
                    csw.signature(),
                    csw.tag_val(),
                    csw.status_val(),
                    csw.data_residue_val()
                );
                self.in_buf.drain(..13);

                if csw.tag_val() == expected_tag {
                    return Ok(Some(csw));
                }

                eprintln!(
                    "  << Discarding stale CSW tag={} while expecting tag={}",
                    csw.tag_val(),
                    expected_tag
                );
                continue;
            }

            if !self.in_buf.is_empty() && self.in_buf.len() > 3 {
                let keep = self.in_buf.split_off(self.in_buf.len() - 3);
                let dropped = std::mem::replace(&mut self.in_buf, keep).len();
                eprintln!("  << Dropping {} bytes without CSW signature", dropped);
            }

            let now = Instant::now();
            if now >= deadline {
                if policy == CswPolicy::AllowMissing {
                    eprintln!("  << No CSW before timeout; accepted by transaction policy");
                    return Ok(None);
                }
                return Err(format!("No CSW for tag {} before timeout", expected_tag));
            }
            match self
                .read_bulk_to_buffer(deadline.saturating_duration_since(now).min(SHORT_TIMEOUT))
            {
                Ok(_) => {}
                Err(rusb::Error::Timeout) if policy == CswPolicy::AllowMissing => {
                    eprintln!("  << No CSW after short timeout; accepted by transaction policy");
                    return Ok(None);
                }
                Err(rusb::Error::Timeout) => {}
                Err(e) => return Err(format!("IN EP read failed: {}", e)),
            }
        }
    }

    fn fill_until(&mut self, deadline: Instant, min_len: usize) -> Result<(), String> {
        while self.in_buf.len() < min_len {
            let now = Instant::now();
            if now >= deadline {
                return Err(format!(
                    "Timed out waiting for {} buffered bytes (have {})",
                    min_len,
                    self.in_buf.len()
                ));
            }
            match self
                .read_bulk_to_buffer(deadline.saturating_duration_since(now).min(SHORT_TIMEOUT))
            {
                Ok(_) => {}
                Err(rusb::Error::Timeout) => {}
                Err(e) => return Err(format!("IN EP read failed: {}", e)),
            }
        }
        Ok(())
    }

    fn check_csw(&self, csw: &AicCsw, expected_tag: u32) -> Result<(), String> {
        if csw.tag_val() != expected_tag {
            return Err(format!(
                "CSW tag mismatch: got {}, expected {}",
                csw.tag_val(),
                expected_tag
            ));
        }
        if !csw.is_ok() {
            return Err(format!(
                "CSW failed: status={} residue={}",
                csw.status_val(),
                csw.data_residue_val()
            ));
        }
        Ok(())
    }

    fn build_cmd_hdr(&self, cmd: u8, data_len: u32) -> Vec<u8> {
        let hdr = CmdHeader::new(cmd, data_len);
        hdr.to_bytes().to_vec()
    }

    // ── High-level protocol helpers ────────────────────────────────────

    /// Send only a command header (no payload, no response read).
    /// Used as first step in multi-transaction commands.
    fn send_hdr(&mut self, cmd: u8, data_len: u32) -> Result<(), String> {
        self.write_txn(&self.build_cmd_hdr(cmd, data_len))?;
        Ok(())
    }

    fn cmd_hdr_data_resp(
        &mut self,
        cmd: u8,
        payload: &[u8],
        resp_extra: usize,
    ) -> Result<UpgResponse, String> {
        self.cmd_hdr_data_resp_policy(cmd, payload, resp_extra, CswPolicy::Required)
    }

    fn cmd_hdr_data_resp_policy(
        &mut self,
        cmd: u8,
        payload: &[u8],
        resp_extra: usize,
        policy: CswPolicy,
    ) -> Result<UpgResponse, String> {
        self.send_hdr(cmd, payload.len() as u32)?;
        let csw = self.write_txn_policy(payload, policy)?;
        if policy == CswPolicy::AllowMissing && csw.is_none() {
            return Ok(UpgResponse {
                payload: Vec::new(),
            });
        }
        self.read_upg_response(cmd, resp_extra, policy)
    }

    fn cmd_hdr_resp(&mut self, cmd: u8, resp_extra: usize) -> Result<UpgResponse, String> {
        self.send_hdr(cmd, 0)?;
        self.read_upg_response(cmd, resp_extra, CswPolicy::Required)
    }

    fn read_upg_response(
        &mut self,
        cmd: u8,
        expected_payload_len: usize,
        policy: CswPolicy,
    ) -> Result<UpgResponse, String> {
        // Official AiBurn reads the 16-byte RESP packet first, then reads the
        // optional data packet separately. Reading header+payload in one CBW
        // makes some bootloaders answer the READ CBW with a failed CSW only.
        let header_data = match self.read_txn(RESP_MIN_HDR_LEN as u32) {
            Ok(data) => data,
            Err(e) if policy == CswPolicy::AllowMissing => {
                eprintln!("  << No UPG response accepted by transaction policy: {}", e);
                return Ok(UpgResponse {
                    payload: Vec::new(),
                });
            }
            Err(e) => return Err(e),
        };
        let header = self.parse_resp_header(cmd, &header_data)?;
        let declared_len = header.data_length_val() as usize;
        let payload_len = if declared_len > 0 {
            declared_len
        } else {
            expected_payload_len
        };
        let payload = if payload_len > 0 {
            match self.read_txn(payload_len as u32) {
                Ok(data) => data,
                Err(e) if policy == CswPolicy::AllowMissing => {
                    eprintln!("  << No UPG payload accepted by transaction policy: {}", e);
                    Vec::new()
                }
                Err(e) => return Err(e),
            }
        } else {
            Vec::new()
        };
        Ok(UpgResponse { payload })
    }

    /// Parse the 16-byte UPG response packet.
    fn parse_resp_header(&self, expected_cmd: u8, data: &[u8]) -> Result<RespHeader, String> {
        if data.len() < RESP_MIN_HDR_LEN {
            return Err("Response too short".to_string());
        }
        let resp = RespHeader::from_bytes(data)
            .ok_or_else(|| "Failed to parse response header".to_string())?;
        if !resp.is_ok() {
            return Err(format!(
                "Command 0x{:02x} failed, status: {}, magic=0x{:08x}",
                expected_cmd,
                resp.status_val(),
                resp.magic()
            ));
        }
        if resp.command() != 0 && resp.command() != expected_cmd {
            eprintln!(
                "  << Warning: response command 0x{:02x} does not match request 0x{:02x}",
                resp.command(),
                expected_cmd
            );
        }
        Ok(resp)
    }

    // ── Public API ─────────────────────────────────────────────────────

    pub fn get_hwinfo(&mut self) -> Result<HwInfo, String> {
        let resp = self.cmd_hdr_resp(CMD_GET_HWINFO, 104)?;
        HwInfo::from_bytes(&resp.payload).ok_or_else(|| {
            format!(
                "Failed to parse HWINFO ({} payload bytes)",
                resp.payload.len()
            )
        })
    }

    pub fn set_upg_cfg(&mut self, mode: u8) -> Result<(), String> {
        let cfg = [
            mode, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8,
            0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8,
        ];
        let mut payload = [0u8; 36];
        payload[0..4].copy_from_slice(&32u32.to_le_bytes()); // cfglen = 32
        payload[4..].copy_from_slice(&cfg);
        let _resp = self.cmd_hdr_data_resp(CMD_SET_UPG_CFG, &payload, 0)?;
        Ok(())
    }

    pub fn set_fwc_meta(&mut self, meta: &FwcMeta) -> Result<(), String> {
        let _resp = self.cmd_hdr_data_resp(CMD_SET_FWC_META, meta.to_bytes(), 0)?;
        Ok(())
    }

    pub fn get_block_size(&mut self) -> Result<u32, String> {
        let resp = self.cmd_hdr_resp(CMD_GET_BLOCK_SIZE, 4)?;
        if resp.payload.len() < 4 {
            return Err(format!(
                "Block size response too short: {} bytes",
                resp.payload.len()
            ));
        }
        let block_size = u32::from_le_bytes(resp.payload[0..4].try_into().unwrap());
        Ok(block_size)
    }

    fn send_fwc_data_policy(&mut self, chunk: &[u8], policy: CswPolicy) -> Result<(), String> {
        let _resp = self.cmd_hdr_data_resp_policy(CMD_SEND_FWC_DATA, chunk, 0, policy)?;
        Ok(())
    }

    pub fn set_upg_end(&mut self) -> Result<(), String> {
        let mut payload = [0u8; 36];
        payload[0..4].copy_from_slice(&32u32.to_le_bytes());
        let _resp =
            self.cmd_hdr_data_resp_policy(CMD_SET_UPG_END, &payload, 0, CswPolicy::AllowMissing)?;
        Ok(())
    }

    pub fn run_shell(&mut self, cmd_str: &str) -> Result<(), String> {
        let cmd_bytes = cmd_str.as_bytes();
        let len = cmd_bytes.len() as u32;
        let mut payload = len.to_le_bytes().to_vec();
        payload.extend_from_slice(cmd_bytes);
        let _resp = self.cmd_hdr_data_resp(CMD_RUN_SHELL_STR, &payload, 0)?;
        Ok(())
    }

    pub fn reset(&mut self) -> Result<(), String> {
        self.run_shell("reset")
    }

    pub fn get_storage_media(&mut self) -> Result<String, String> {
        let resp = self.cmd_hdr_resp(CMD_GET_STORAGE_MEDIA, 64)?;
        let media = String::from_utf8_lossy(&resp.payload)
            .trim_end_matches('\0')
            .to_string();
        Ok(media)
    }

    pub fn show_info(&mut self) -> Result<(), String> {
        let hwinfo = self.get_hwinfo()?;
        let chipid = hwinfo.chipid_val();
        println!("  Magic:        {}", hwinfo.magic_str());
        println!("  Init mode:    {:#x}", hwinfo.init_mode());
        println!("  Current mode: {:#x}", hwinfo.curr_mode());
        println!("  Boot stage:   {}", hwinfo.boot_stage());
        println!(
            "  Chip ID:      {:08x} {:08x} {:08x} {:08x}",
            chipid[0], chipid[1], chipid[2], chipid[3]
        );
        Ok(())
    }

    pub fn burn_image(
        &mut self,
        img_data: &[u8],
        metas: &[FwcMeta],
        _header: &crate::image::parser::FwHeader,
    ) -> Result<(), String> {
        let classified = classify_components(metas);
        print_burn_plan(&classified);

        let updater_count = classified
            .iter()
            .filter(|c| c.kind == ComponentKind::Updater)
            .count();
        if updater_count > 0 {
            eprintln!("Start burn online: sending updater components...");
            for component in classified
                .iter()
                .filter(|c| c.kind == ComponentKind::Updater)
            {
                self.send_component(img_data, component, true)?;
            }
            eprintln!("Updater stage complete; waiting for bootloader upgrade reconnect...");
            if let Err(e) = self.wait_reconnect(DEFAULT_BURN_TIMEOUT) {
                eprintln!("Warning: updater reconnect was not observed: {}", e);
            }
        } else {
            eprintln!(
                "No updater components found; continuing with target stage on current connection."
            );
        }

        eprintln!("Setting upgrade mode to FULL_DISK_UPGRADE...");
        self.set_upg_cfg(UPG_MODE_FULL_DISK_UPGRADE)?;

        if let Some(info) = classified
            .iter()
            .find(|c| c.kind == ComponentKind::ImageInfo)
        {
            self.send_component(img_data, info, false)?;
        } else {
            eprintln!("Warning: no image.info component found");
        }

        let selected: Vec<_> = classified
            .iter()
            .filter(|c| c.kind == ComponentKind::Target && c.selected)
            .collect();
        if selected.is_empty() {
            return Err("No selected target components to burn".to_string());
        }
        for component in selected {
            self.send_component(img_data, component, false)?;
        }

        eprintln!("Ending upgrade...");
        self.set_upg_end()?;

        Ok(())
    }

    fn send_component(
        &mut self,
        img_data: &[u8],
        component: &FirmwareComponent<'_>,
        allow_final_no_csw: bool,
    ) -> Result<(), String> {
        let meta = component.meta;
        let name = meta.name_str();
        let size = meta.size_val() as usize;
        let offset = meta.offset_val() as usize;
        let crc_expected = meta.crc_val();
        let end = offset
            .checked_add(size)
            .ok_or_else(|| format!("{} offset/size overflow", name))?;
        if end > img_data.len() {
            return Err(format!(
                "{} image range out of bounds: offset={:#x}, size={}, image_len={}",
                name,
                offset,
                size,
                img_data.len()
            ));
        }

        eprintln!(
            "  Meta: {} (partition={}, offset={:#x}, size={}, crc=0x{:08x})",
            name,
            meta.partition_str(),
            offset,
            size,
            crc_expected
        );

        self.set_fwc_meta(meta)?;

        let block_size = self.get_block_size().unwrap_or(2048);
        eprintln!("    Block size: {}", block_size);

        let mut data_sent = 0usize;
        let chunk_max = CHUNK_SIZE as usize;
        while data_sent < size {
            let chunk_end = (data_sent + chunk_max).min(size);
            let chunk_offset = offset + data_sent;
            let chunk_size = chunk_end - data_sent;
            let chunk_data = &img_data[chunk_offset..chunk_offset + chunk_size];
            let final_chunk = chunk_end == size;
            let policy = if allow_final_no_csw && final_chunk {
                CswPolicy::AllowMissing
            } else {
                CswPolicy::Required
            };

            self.send_fwc_data_policy(chunk_data, policy)?;
            data_sent += chunk_size;
            let pct = (data_sent as f64 / size as f64) * 100.0;
            eprintln!("    {}: {}/{} ({:.1}%)", name, data_sent, size, pct);
        }

        let actual_crc = crc32fast::hash(&img_data[offset..end]);
        if actual_crc != crc_expected {
            eprintln!(
                "    WARNING: CRC mismatch! expected=0x{:08x}, actual=0x{:08x}",
                crc_expected, actual_crc
            );
        } else {
            eprintln!("    CRC OK (0x{:08x})", actual_crc);
        }
        Ok(())
    }
}

impl Drop for AicDevice {
    fn drop(&mut self) {
        let _ = self.handle.release_interface(0);
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ComponentKind {
    Updater,
    ImageInfo,
    Target,
    Other,
}

struct FirmwareComponent<'a> {
    meta: &'a FwcMeta,
    kind: ComponentKind,
    selected: bool,
}

fn classify_components(metas: &[FwcMeta]) -> Vec<FirmwareComponent<'_>> {
    metas
        .iter()
        .map(|meta| {
            let name = meta.name_str();
            let kind = if name.starts_with("image.updater.") {
                ComponentKind::Updater
            } else if name == "image.info" {
                ComponentKind::ImageInfo
            } else if name.starts_with("image.target.") {
                ComponentKind::Target
            } else {
                ComponentKind::Other
            };
            let selected = kind != ComponentKind::Target || target_part_selected(meta);
            FirmwareComponent {
                meta,
                kind,
                selected,
            }
        })
        .collect()
}

fn target_part_selected(meta: &FwcMeta) -> bool {
    let partition = meta.partition_str();
    let target_name = meta
        .name_str()
        .strip_prefix("image.target.")
        .unwrap_or_else(|| meta.name_str());
    DEFAULT_SELECTED_PARTS
        .iter()
        .any(|part| partition == *part || target_name == *part)
}

fn print_burn_plan(components: &[FirmwareComponent<'_>]) {
    eprintln!("AiBurn-style component plan:");
    for component in components {
        eprintln!(
            "  {:?}: {} partition={} selected={}",
            component.kind,
            component.meta.name_str(),
            component.meta.partition_str(),
            component.selected
        );
    }
}
