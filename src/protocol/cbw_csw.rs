pub const AIC_USB_SIGN_USBC: u32 = 0x43425355;
pub const AIC_USB_SIGN_USBS: u32 = 0x53425355;

pub const TRANS_LAYER_CMD_WRITE: u8 = 0x01;
pub const TRANS_LAYER_CMD_READ: u8 = 0x02;

/// CBW - Command Block Wrapper (31 bytes)
pub struct AicCbw {
    bytes: [u8; 31],
}

impl AicCbw {
    pub fn new_write(tag: u32, data_len: u32) -> Self {
        let mut b = [0u8; 31];
        b[0..4].copy_from_slice(&AIC_USB_SIGN_USBC.to_le_bytes());
        b[4..8].copy_from_slice(&tag.to_le_bytes());
        b[8..12].copy_from_slice(&data_len.to_le_bytes());
        // flags = 0x00 (host->dev)
        // cb_length = 0
        b[15] = TRANS_LAYER_CMD_WRITE;
        Self { bytes: b }
    }

    pub fn new_read(tag: u32, data_len: u32) -> Self {
        let mut b = [0u8; 31];
        b[0..4].copy_from_slice(&AIC_USB_SIGN_USBC.to_le_bytes());
        b[4..8].copy_from_slice(&tag.to_le_bytes());
        b[8..12].copy_from_slice(&data_len.to_le_bytes());
        b[12] = 0x80; // flags = dev->host
        b[15] = TRANS_LAYER_CMD_READ;
        Self { bytes: b }
    }

    pub fn to_bytes(&self) -> &[u8] {
        &self.bytes
    }
}

/// CSW - Command Status Wrapper (13 bytes)
pub struct AicCsw {
    bytes: [u8; 13],
}

impl AicCsw {
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < 13 {
            return None;
        }
        let mut b = [0u8; 13];
        b.copy_from_slice(&bytes[..13]);
        Some(Self { bytes: b })
    }

    pub fn signature(&self) -> u32 {
        u32::from_le_bytes(self.bytes[0..4].try_into().unwrap())
    }

    pub fn tag_val(&self) -> u32 {
        u32::from_le_bytes(self.bytes[4..8].try_into().unwrap())
    }

    pub fn data_residue_val(&self) -> u32 {
        u32::from_le_bytes(self.bytes[8..12].try_into().unwrap())
    }

    pub fn status_val(&self) -> u8 {
        self.bytes[12]
    }

    pub fn is_ok(&self) -> bool {
        self.signature() == AIC_USB_SIGN_USBS && self.status_val() == 0
    }
}

/// UPG Command Header (24 bytes)
pub struct CmdHeader {
    pub bytes: [u8; 24],
}

impl CmdHeader {
    pub fn new(command: u8, data_length: u32) -> Self {
        let mut b = [0u8; 24];
        b[0..4].copy_from_slice(&0x43475055u32.to_le_bytes()); // "UPGC"
        b[4] = 0x01; // protocol
        b[5] = 0x01; // version
        b[6] = command;
        // b[7] = reserved
        b[8..12].copy_from_slice(&data_length.to_le_bytes());

        // checksum = magic + (reserved<<24|command<<16|version<<8|protocol) + data_length
        let mut sum: u32 = 0;
        sum = sum.wrapping_add(0x43475055);
        sum = sum.wrapping_add(
            (0u32 << 24) | (command as u32) << 16 | (0x01u32 << 8) | 0x01u32,
        );
        sum = sum.wrapping_add(data_length);
        b[12..16].copy_from_slice(&sum.to_le_bytes());

        Self { bytes: b }
    }

    pub fn to_bytes(&self) -> &[u8] {
        &self.bytes
    }
}

/// UPG Response Header (24 bytes)
pub struct RespHeader {
    bytes: [u8; 24],
}

impl RespHeader {
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < 24 {
            return None;
        }
        let mut b = [0u8; 24];
        b.copy_from_slice(&bytes[..24]);
        Some(Self { bytes: b })
    }

    fn magic(&self) -> u32 {
        u32::from_le_bytes(self.bytes[0..4].try_into().unwrap())
    }

    pub fn status_val(&self) -> u8 {
        self.bytes[7]
    }

    pub fn is_ok(&self) -> bool {
        self.magic() == 0x52475055 && self.status_val() == 0
    }
}

/// HWINFO response data
pub struct HwInfo {
    bytes: Vec<u8>,
}

impl HwInfo {
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < 104 {
            return None;
        }
        Some(Self {
            bytes: bytes.to_vec(),
        })
    }

    pub fn magic_str(&self) -> &str {
        std::str::from_utf8(&self.bytes[0..8])
            .unwrap_or("")
            .trim_end_matches('\0')
    }

    pub fn chipid_val(&self) -> [u32; 4] {
        let mut ids = [0u32; 4];
        for i in 0..4 {
            let off = 48 + i * 4;
            ids[i] = u32::from_le_bytes(self.bytes[off..off + 4].try_into().unwrap());
        }
        ids
    }

    pub fn init_mode(&self) -> u8 {
        self.bytes[38]
    }

    pub fn curr_mode(&self) -> u8 {
        self.bytes[39]
    }

    pub fn boot_stage(&self) -> u8 {
        self.bytes[40]
    }
}
