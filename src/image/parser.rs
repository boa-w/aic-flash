use crate::protocol::commands::{FwcMeta, FWC_META_SIZE};

pub const FW_HEADER_SIZE: usize = 2048;

/// ArtInChip firmware image header (struct image_header_pack, 2048 bytes)
///
/// Core header (struct image_header_upgrade, 348 bytes):
///   [0..8)     magic       "AIC.FW"
///   [8..72)    platform    64 bytes
///   [72..136)   product     64 bytes
///   [136..200)  version     64 bytes
///   [200..264)  media_type  64 bytes
///   [264..268)  media_dev_id  u32 LE
///   [268..332)  media_id    64 bytes (nand_array_org)
///   [332..336)  meta_offset  u32 LE
///   [336..340)  meta_size    u32 LE
///   [340..344)  file_offset  u32 LE
///   [344..348)  file_size    u32 LE
///   [348..2048) pad         1700 zero bytes
pub struct FwHeader {
    bytes: Vec<u8>,
}

impl FwHeader {
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < FW_HEADER_SIZE {
            return None;
        }
        Some(Self {
            bytes: bytes[..FW_HEADER_SIZE].to_vec(),
        })
    }

    pub fn magic_str(&self) -> &str {
        std::str::from_utf8(&self.bytes[0..8])
            .unwrap_or("")
            .trim_end_matches('\0')
    }

    pub fn platform_str(&self) -> &str {
        std::str::from_utf8(&self.bytes[8..72])
            .unwrap_or("")
            .trim_end_matches('\0')
    }

    pub fn product_str(&self) -> &str {
        std::str::from_utf8(&self.bytes[72..136])
            .unwrap_or("")
            .trim_end_matches('\0')
    }

    pub fn version_str(&self) -> &str {
        std::str::from_utf8(&self.bytes[136..200])
            .unwrap_or("")
            .trim_end_matches('\0')
    }

    pub fn media_type_str(&self) -> &str {
        std::str::from_utf8(&self.bytes[200..264])
            .unwrap_or("")
            .trim_end_matches('\0')
    }

    pub fn media_dev_id(&self) -> u32 {
        u32::from_le_bytes(self.bytes[264..268].try_into().unwrap())
    }

    pub fn meta_offset_val(&self) -> u32 {
        u32::from_le_bytes(self.bytes[332..336].try_into().unwrap())
    }

    pub fn meta_size_val(&self) -> u32 {
        u32::from_le_bytes(self.bytes[336..340].try_into().unwrap())
    }

    pub fn file_offset_val(&self) -> u32 {
        u32::from_le_bytes(self.bytes[340..344].try_into().unwrap())
    }

    pub fn file_size_val(&self) -> u32 {
        u32::from_le_bytes(self.bytes[344..348].try_into().unwrap())
    }
}

pub fn parse_image(data: &[u8]) -> Result<(FwHeader, Vec<FwcMeta>, &[u8]), String> {
    if data.len() < FW_HEADER_SIZE {
        return Err("File too small to contain image header".to_string());
    }

    let header = FwHeader::from_bytes(&data[..FW_HEADER_SIZE])
        .ok_or_else(|| "Failed to parse firmware header".to_string())?;

    if header.magic_str() != "AIC.FW" {
        return Err(format!(
            "Invalid image magic: '{}' (expected 'AIC.FW')",
            header.magic_str()
        ));
    }

    let meta_offset = header.meta_offset_val() as usize;
    let meta_size = header.meta_size_val() as usize;
    let meta_count = if meta_size > 0 {
        meta_size / FWC_META_SIZE
    } else {
        0
    };
    let file_offset = header.file_offset_val() as usize;

    if meta_offset == 0 || meta_count == 0 {
        return Err("No META entries in image header".to_string());
    }

    let meta_end = meta_offset + meta_count * FWC_META_SIZE;
    if data.len() < meta_end {
        return Err(format!(
            "File too short for META entries: need {} bytes, have {}",
            meta_end,
            data.len()
        ));
    }

    let mut metas = Vec::with_capacity(meta_count);
    for i in 0..meta_count {
        let entry_offset = meta_offset + i * FWC_META_SIZE;
        let entry_bytes = &data[entry_offset..entry_offset + FWC_META_SIZE];
        let meta = FwcMeta::from_bytes(entry_bytes)
            .ok_or_else(|| format!("Failed to parse META entry {}", i))?;
        metas.push(meta);
    }

    if file_offset > 0 {
        Ok((header, metas, &data[file_offset..]))
    } else {
        Ok((header, metas, &data[meta_end..]))
    }
}

pub fn print_image_info(data: &[u8]) -> Result<(), String> {
    let (header, metas, _payload) = parse_image(data)?;

    println!("=== Image Header ===");
    println!("  Magic:       {}", header.magic_str());
    println!("  Platform:    {}", header.platform_str());
    println!("  Product:     {}", header.product_str());
    println!("  Version:     {}", header.version_str());
    println!("  Media type:  {}", header.media_type_str());
    println!("  Media dev:   {:#x}", header.media_dev_id());
    println!("  Meta offset: {:#x}", header.meta_offset_val());
    println!(
        "  Meta count:  {}",
        meta_size_to_count(header.meta_size_val())
    );
    println!("  File offset: {:#x}", header.file_offset_val());
    println!("  File size:   {}", header.file_size_val());

    println!();
    println!("=== META Entries ({}) ===", metas.len());
    for (i, meta) in metas.iter().enumerate() {
        println!(
            "  [{:2}] {} (partition: {}, offset={:#x}, size={}, crc=0x{:08x}, ram={:#x}, attr='{}')",
            i,
            meta.name_str(),
            meta.partition_str(),
            meta.offset_val(),
            meta.size_val(),
            meta.crc_val(),
            meta.ram_val(),
            meta.attr_str(),
        );
    }

    Ok(())
}

fn meta_size_to_count(sz: u32) -> u32 {
    if sz > 0 {
        sz / FWC_META_SIZE as u32
    } else {
        0
    }
}
