use crate::protocol::commands::{FwcMeta, FWC_META_SIZE};

pub const FW_HEADER_SIZE: usize = 2048;

/// ArtInChip firmware image header
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
        std::str::from_utf8(&self.bytes[8..28])
            .unwrap_or("")
            .trim_end_matches('\0')
    }

    pub fn product_str(&self) -> &str {
        std::str::from_utf8(&self.bytes[28..60])
            .unwrap_or("")
            .trim_end_matches('\0')
    }

    pub fn version_str(&self) -> &str {
        std::str::from_utf8(&self.bytes[60..124])
            .unwrap_or("")
            .trim_end_matches('\0')
    }

    pub fn media_type_val(&self) -> u32 {
        u32::from_le_bytes(self.bytes[128..132].try_into().unwrap())
    }

    pub fn meta_offset_val(&self) -> u32 {
        u32::from_le_bytes(self.bytes[132..136].try_into().unwrap())
    }

    pub fn meta_count_val(&self) -> u32 {
        u32::from_le_bytes(self.bytes[136..140].try_into().unwrap())
    }

    pub fn file_offset_val(&self) -> u32 {
        u32::from_le_bytes(self.bytes[140..144].try_into().unwrap())
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
    let meta_count = header.meta_count_val() as usize;
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
    println!("  Magic:     {}", header.magic_str());
    println!("  Platform:  {}", header.platform_str());
    println!("  Product:   {}", header.product_str());
    println!("  Version:   {}", header.version_str());
    println!("  Media type: {:#x}", header.media_type_val());
    println!("  Meta offset: {:#x}", header.meta_offset_val());
    println!("  Meta count: {}", header.meta_count_val());
    println!("  File offset: {:#x}", header.file_offset_val());

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
