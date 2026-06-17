use std::time::Duration;

use rusb::{DeviceHandle, UsbContext};

use crate::protocol::cbw_csw::*;
use crate::protocol::commands::*;

const AIC_VID: u16 = 0x33C3;
const AIC_PID: u16 = 0x6677;
const BULK_OUT_EP: u8 = 0x02;
const BULK_IN_EP: u8 = 0x81;
const TIMEOUT_MS: Duration = Duration::from_secs(30);
const CHUNK_SIZE: u32 = 1024 * 1024;

pub struct AicDevice {
    handle: DeviceHandle<rusb::Context>,
    tag: u32,
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
                handle
                    .claim_interface(0)
                    .map_err(|e| format!("Failed to claim interface: {}", e))?;

                return Ok(Self { handle, tag: 1 });
            }
        }
        Err("No ArtInChip device found (VID=0x33C3, PID=0x6677)".to_string())
    }

    fn transact_write(&mut self, payload: &[u8]) -> Result<AicCsw, String> {
        let tag = self.tag;
        self.tag += 1;

        let cbw = AicCbw::new_write(tag, payload.len() as u32);
        let cbw_bytes = cbw.to_bytes();
        self.write_bulk(cbw_bytes)?;

        if !payload.is_empty() {
            self.write_bulk(payload)?;
        }

        let csw = self.read_csw()?;

        if csw.tag_val() != tag {
            return Err(format!(
                "CSW tag mismatch: got {}, expected {}",
                csw.tag_val(),
                tag
            ));
        }
        if !csw.is_ok() {
            return Err(format!(
                "CSW status: {} (residue: {})",
                csw.status_val(),
                csw.data_residue_val()
            ));
        }
        Ok(csw)
    }

    fn transact_read(&mut self, read_len: u32) -> Result<(AicCsw, Vec<u8>), String> {
        let tag = self.tag;
        self.tag += 1;

        let cbw = AicCbw::new_read(tag, read_len);
        let cbw_bytes = cbw.to_bytes();
        self.write_bulk(cbw_bytes)?;

        let mut data = vec![0u8; read_len as usize];
        let mut total = 0usize;
        while total < read_len as usize {
            let mut buf = vec![0u8; 64 * 1024];
            let max_read = buf.len().min(read_len as usize - total);
            match self
                .handle
                .read_bulk(BULK_IN_EP, &mut buf[..max_read], TIMEOUT_MS)
            {
                Ok(n) => {
                    data[total..total + n].copy_from_slice(&buf[..n]);
                    total += n;
                }
                Err(rusb::Error::Timeout) => break,
                Err(e) => return Err(format!("Bulk read failed: {}", e)),
            }
        }
        data.truncate(total);

        let csw = self.read_csw()?;
        if csw.tag_val() != tag {
            return Err(format!(
                "CSW tag mismatch: got {}, expected {}",
                csw.tag_val(),
                tag
            ));
        }
        Ok((csw, data))
    }

    fn write_bulk(&self, data: &[u8]) -> Result<(), String> {
        let mut written = 0usize;
        while written < data.len() {
            let n = self
                .handle
                .write_bulk(BULK_OUT_EP, &data[written..], TIMEOUT_MS)
                .map_err(|e| format!("Bulk write failed: {}", e))?;
            written += n;
        }
        Ok(())
    }

    fn read_csw(&self) -> Result<AicCsw, String> {
        let mut buf = [0u8; 13];
        let mut total = 0usize;
        while total < 13 {
            let n = self
                .handle
                .read_bulk(BULK_IN_EP, &mut buf[total..], TIMEOUT_MS)
                .map_err(|e| format!("CSW read failed: {}", e))?;
            total += n;
        }
        AicCsw::from_bytes(&buf).ok_or_else(|| "Failed to parse CSW".to_string())
    }

    pub fn send_cmd_header(&mut self, cmd: u8, data_len: u32) -> Result<(), String> {
        let hdr = CmdHeader::new(cmd, data_len);
        let bytes = hdr.to_bytes();
        self.transact_write(bytes)?;
        Ok(())
    }

    pub fn send_cmd_with_data(&mut self, cmd: u8, payload: &[u8]) -> Result<(), String> {
        let hdr = CmdHeader::new(cmd, payload.len() as u32);
        let mut data = hdr.to_bytes().to_vec();
        data.extend_from_slice(payload);
        self.transact_write(&data)?;
        Ok(())
    }

    pub fn read_response(&mut self, read_len: u32) -> Result<Vec<u8>, String> {
        let (_csw, data) = self.transact_read(read_len)?;
        Ok(data)
    }

    pub fn send_command_read_response(
        &mut self,
        cmd: u8,
        payload: &[u8],
        read_len: u32,
    ) -> Result<Vec<u8>, String> {
        let hdr = CmdHeader::new(cmd, payload.len() as u32);
        let mut tx_data = hdr.to_bytes().to_vec();
        if !payload.is_empty() {
            tx_data.extend_from_slice(payload);
        }
        self.transact_write(&tx_data)?;

        if read_len > 0 {
            self.read_response(read_len)
        } else {
            Ok(Vec::new())
        }
    }

    pub fn get_hwinfo(&mut self) -> Result<HwInfo, String> {
        self.send_cmd_header(CMD_GET_HWINFO, 0)?;

        let resp_size = 24usize + 104usize;
        let data = self.read_response(resp_size as u32)?;

        if data.len() < 24usize {
            return Err("Response too short".to_string());
        }

        let resp = RespHeader::from_bytes(&data)
            .ok_or_else(|| "Failed to parse response header".to_string())?;

        if !resp.is_ok() {
            return Err(format!("GET_HWINFO failed, status: {}", resp.status_val()));
        }

        let hwinfo_offset = 24usize;
        if data.len() < hwinfo_offset + 104usize {
            return Err("HWINFO data too short".to_string());
        }

        HwInfo::from_bytes(&data[hwinfo_offset..])
            .ok_or_else(|| "Failed to parse HWINFO".to_string())
    }

    pub fn set_upg_cfg(&mut self, mode: u8) -> Result<(), String> {
        let cfg = [mode, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8,
                   0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8,
                   0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8,
                   0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8];
        let payload = &[32u8, 0, 0, 0];
        let mut data = payload.to_vec();
        data.extend_from_slice(&cfg);
        self.send_cmd_with_data(CMD_SET_UPG_CFG, &data)
    }

    pub fn set_fwc_meta(&mut self, meta: &FwcMeta) -> Result<(), String> {
        let payload = meta.to_bytes();
        self.send_cmd_with_data(CMD_SET_FWC_META, payload)
    }

    pub fn get_block_size(&mut self) -> Result<u32, String> {
        self.send_cmd_header(CMD_GET_BLOCK_SIZE, 0)?;
        let resp_size = 24usize + 4;
        let data = self.read_response(resp_size as u32)?;
        if data.len() < 24usize + 4 {
            return Err("GET_BLOCK_SIZE response too short".to_string());
        }
        let resp = RespHeader::from_bytes(&data)
            .ok_or_else(|| "Failed to parse response header".to_string())?;
        if !resp.is_ok() {
            return Err(format!(
                "GET_BLOCK_SIZE failed, status: {}",
                resp.status_val()
            ));
        }
        let block_size = u32::from_le_bytes(
            data[24usize..24usize + 4]
                .try_into()
                .unwrap(),
        );
        Ok(block_size)
    }

    pub fn send_fwc_data(&mut self, chunk: &[u8]) -> Result<(), String> {
        self.send_cmd_with_data(CMD_SEND_FWC_DATA, chunk)
    }

    pub fn set_upg_end(&mut self) -> Result<(), String> {
        let mut data = [0u8; 36];
        data[0..4].copy_from_slice(&[32u8, 0, 0, 0]);
        self.send_cmd_with_data(CMD_SET_UPG_END, &data)
    }

    pub fn exec(&mut self, addr: u32) -> Result<(), String> {
        let payload = addr.to_le_bytes().to_vec();
        self.send_cmd_with_data(CMD_EXEC, &payload)
    }

    pub fn run_shell(&mut self, cmd_str: &str) -> Result<(), String> {
        let cmd_bytes = cmd_str.as_bytes();
        let len_bytes = (cmd_bytes.len() as u32).to_le_bytes();
        let mut payload = len_bytes.to_vec();
        payload.extend_from_slice(cmd_bytes);
        self.send_cmd_with_data(CMD_RUN_SHELL_STR, &payload)
    }

    pub fn get_storage_media(&mut self) -> Result<String, String> {
        self.send_cmd_header(CMD_GET_STORAGE_MEDIA, 0)?;
        let resp_size = 24usize + 64;
        let data = self.read_response(resp_size as u32)?;
        if data.len() < 24usize {
            return Err("GET_STORAGE_MEDIA response too short".to_string());
        }
        let _resp = RespHeader::from_bytes(&data)
            .ok_or_else(|| "Failed to parse response header".to_string())?;
        let offset = 24usize;
        let media = String::from_utf8_lossy(&data[offset..])
            .trim_end_matches('\0')
            .to_string();
        Ok(media)
    }

    pub fn burn_image(
        &mut self,
        img_data: &[u8],
        metas: &[FwcMeta],
        _header: &crate::image::parser::FwHeader,
    ) -> Result<(), String> {
        eprintln!("Setting upgrade mode to FULL_DISK_UPGRADE...");
        self.set_upg_cfg(UPG_MODE_FULL_DISK_UPGRADE)?;

        for meta in metas {
            let name = meta.name_str();
            let size = meta.size_val() as usize;
            let offset = meta.offset_val() as usize;
            let crc_expected = meta.crc_val();

            eprintln!(
                "  Meta: {} (offset={:#x}, size={}, crc=0x{:08x})",
                name, offset, size, crc_expected
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

                self.send_fwc_data(chunk_data)?;
                data_sent += chunk_size;
                let pct = (data_sent as f64 / size as f64) * 100.0;
                eprintln!("    {}: {}/{} ({:.1}%)", name, data_sent, size, pct);
            }

            let actual_crc = crc32fast::hash(&img_data[offset..offset + size]);
            if actual_crc != crc_expected {
                eprintln!(
                    "    WARNING: CRC mismatch! expected=0x{:08x}, actual=0x{:08x}",
                    crc_expected, actual_crc
                );
            } else {
                eprintln!("    CRC OK (0x{:08x})", actual_crc);
            }
        }

        eprintln!("Ending upgrade...");
        self.set_upg_end()?;

        Ok(())
    }

    pub fn reset(&mut self) -> Result<(), String> {
        self.run_shell("reset")
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
}

impl Drop for AicDevice {
    fn drop(&mut self) {
        let _ = self.handle.release_interface(0);
    }
}
