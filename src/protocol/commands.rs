#![allow(dead_code)]

pub const CMD_GET_HWINFO: u8 = 0x00;
pub const CMD_GET_TRACEINFO: u8 = 0x01;
pub const CMD_WRITE: u8 = 0x02;
pub const CMD_READ: u8 = 0x03;
pub const CMD_EXEC: u8 = 0x04;
pub const CMD_RUN_SHELL_STR: u8 = 0x05;
pub const CMD_GET_MEM_BUF: u8 = 0x08;
pub const CMD_FREE_MEM_BUF: u8 = 0x09;
pub const CMD_SET_UPG_CFG: u8 = 0x0A;
pub const CMD_SET_UPG_END: u8 = 0x0B;
pub const CMD_GET_LOG_SIZE: u8 = 0x0C;
pub const CMD_GET_LOG_DATA: u8 = 0x0D;
pub const CMD_SET_FWC_META: u8 = 0x10;
pub const CMD_GET_BLOCK_SIZE: u8 = 0x11;
pub const CMD_SEND_FWC_DATA: u8 = 0x12;
pub const CMD_GET_FWC_CRC: u8 = 0x13;
pub const CMD_GET_FWC_BURN_RESULT: u8 = 0x14;
pub const CMD_GET_FWC_RUN_RESULT: u8 = 0x15;
pub const CMD_GET_STORAGE_MEDIA: u8 = 0x16;
pub const CMD_GET_PARTITION_TABLE: u8 = 0x17;
pub const CMD_READ_FWC_DATA: u8 = 0x18;
pub const CMD_SET_UART_ARGS: u8 = 0x19;

pub const UPG_MODE_FULL_DISK_UPGRADE: u8 = 0x00;
pub const UPG_MODE_PARTITION_UPGRADE: u8 = 0x01;
pub const UPG_MODE_BURN_USER_ID: u8 = 0x02;
pub const UPG_MODE_DUMP_PARTITION: u8 = 0x03;
pub const UPG_MODE_BURN_IMG_FORCE: u8 = 0x04;
pub const UPG_MODE_BURN_FROZEN: u8 = 0x05;

pub const FWC_META_SIZE: usize = 512;

/// FWC Meta entry (512 bytes)
pub struct FwcMeta {
    pub bytes: Vec<u8>,
}

impl FwcMeta {
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < FWC_META_SIZE {
            return None;
        }
        Some(Self {
            bytes: bytes[..FWC_META_SIZE].to_vec(),
        })
    }

    pub fn magic_str(&self) -> &str {
        std::str::from_utf8(&self.bytes[0..8])
            .unwrap_or("")
            .trim_end_matches('\0')
    }

    pub fn name_str(&self) -> &str {
        std::str::from_utf8(&self.bytes[8..72])
            .unwrap_or("")
            .trim_end_matches('\0')
    }

    pub fn partition_str(&self) -> &str {
        std::str::from_utf8(&self.bytes[72..136])
            .unwrap_or("")
            .trim_end_matches('\0')
    }

    pub fn offset_val(&self) -> u32 {
        u32::from_le_bytes(self.bytes[136..140].try_into().unwrap())
    }

    pub fn size_val(&self) -> u32 {
        u32::from_le_bytes(self.bytes[140..144].try_into().unwrap())
    }

    pub fn crc_val(&self) -> u32 {
        u32::from_le_bytes(self.bytes[144..148].try_into().unwrap())
    }

    pub fn ram_val(&self) -> u32 {
        u32::from_le_bytes(self.bytes[148..152].try_into().unwrap())
    }

    pub fn attr_str(&self) -> &str {
        std::str::from_utf8(&self.bytes[152..216])
            .unwrap_or("")
            .trim_end_matches('\0')
    }

    pub fn to_bytes(&self) -> &[u8] {
        &self.bytes
    }
}
