use tracing::{debug, warn};

/// Result of gcode validation against a target printer model.
#[derive(Debug)]
pub struct ValidationResult {
    pub is_valid: bool,
    pub detected_printer: Option<String>,
    pub error_message: Option<String>,
}

impl ValidationResult {
    fn valid(detected_printer: String) -> Self {
        Self {
            is_valid: true,
            detected_printer: Some(detected_printer),
            error_message: None,
        }
    }

    fn invalid(detected_printer: Option<String>, message: String) -> Self {
        Self {
            is_valid: false,
            detected_printer,
            error_message: Some(message),
        }
    }
}

/// Map from Bambu gcode `printer_model` values to our internal model names.
fn normalize_printer_model(gcode_model: &str) -> Option<&'static str> {
    match gcode_model.trim() {
        "Bambu Lab A1" => Some("A1"),
        "Bambu Lab A1 Mini" => Some("A1 Mini"),
        "Bambu Lab P1P" => Some("P1P"),
        "Bambu Lab P1S" => Some("P1S"),
        "Bambu Lab X1" | "Bambu Lab X1 Carbon" => Some("X1C"),
        "Bambu Lab X1E" => Some("X1E"),
        _ => None,
    }
}

/// Check if a gcode printer model is compatible with the target printer.
/// Only exact matches are allowed — cross-family gcode is NOT compatible
/// (e.g., A1 gcode won't work on P1S due to different kinematics/bed size).
fn is_compatible(gcode_model: &str, target_model: &str) -> bool {
    gcode_model == target_model
}

/// Extract the `printer_model` from gcode header comments.
/// Bambu Studio gcode headers look like:
/// ```gcode
/// ; BambuStudio 01.09.07.50
/// ; printer_model = Bambu Lab P1S
/// ; printer_variant = 0.4
/// ```
fn extract_printer_model_from_gcode(gcode: &str) -> Option<String> {
    for line in gcode.lines().take(500) {
        let trimmed_line = line.trim();
        // Skip blank lines (don't break — the header may have blank lines)
        if trimmed_line.is_empty() {
            continue;
        }
        // Stop scanning once we hit actual gcode commands (G0, G1, M104, etc.)
        if !trimmed_line.starts_with(';') {
            break;
        }
        if let Some(value) = trimmed_line.strip_prefix("; printer_model =") {
            let trimmed = value.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
        // Also check alternate format (no spaces around =)
        if let Some(value) = trimmed_line.strip_prefix("; printer_model=") {
            let trimmed = value.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}

/// Minimal ZIP reader for extracting files from 3MF archives.
/// We only need to read a single entry, so this avoids pulling in the full `zip` crate.
mod mini_zip {
    use std::io::Read;

    /// End of central directory record signature
    const EOCD_SIGNATURE: u32 = 0x06054b50;
    /// Local file header signature
    const LOCAL_FILE_HEADER_SIGNATURE: u32 = 0x04034b50;
    /// Central directory file header signature
    const CENTRAL_DIR_SIGNATURE: u32 = 0x02014b50;

    /// Read a little-endian u16 from a byte slice
    fn read_u16(data: &[u8], offset: usize) -> u16 {
        u16::from_le_bytes([data[offset], data[offset + 1]])
    }

    /// Read a little-endian u32 from a byte slice
    fn read_u32(data: &[u8], offset: usize) -> u32 {
        u32::from_le_bytes([data[offset], data[offset + 1], data[offset + 2], data[offset + 3]])
    }

    /// Find the End of Central Directory record
    fn find_eocd(data: &[u8]) -> Option<usize> {
        // EOCD is at least 22 bytes, search backwards
        let min_offset = data.len().saturating_sub(65535 + 22);
        for i in (min_offset..data.len().saturating_sub(21)).rev() {
            if read_u32(data, i) == EOCD_SIGNATURE {
                return Some(i);
            }
        }
        None
    }

    /// A file entry found in the central directory
    #[allow(dead_code)]
    struct CentralDirEntry {
        compression: u16,
        compressed_size: u32,
        uncompressed_size: u32,
        local_header_offset: usize,
        name: String,
    }

    /// Parse all entries from the central directory
    fn parse_central_directory(data: &[u8]) -> Result<Vec<CentralDirEntry>, String> {
        let eocd_offset = find_eocd(data).ok_or("Not a valid ZIP/3MF file: EOCD not found")?;

        let num_entries = read_u16(data, eocd_offset + 10) as usize;
        let central_dir_offset = read_u32(data, eocd_offset + 16) as usize;

        let mut entries = Vec::new();
        let mut offset = central_dir_offset;

        for _ in 0..num_entries {
            if offset + 46 > data.len() {
                return Err("ZIP central directory truncated".to_string());
            }
            if read_u32(data, offset) != CENTRAL_DIR_SIGNATURE {
                return Err("Invalid central directory entry".to_string());
            }

            let compression = read_u16(data, offset + 10);
            let compressed_size = read_u32(data, offset + 20);
            let uncompressed_size = read_u32(data, offset + 24);
            let name_len = read_u16(data, offset + 28) as usize;
            let extra_len = read_u16(data, offset + 30) as usize;
            let comment_len = read_u16(data, offset + 32) as usize;
            let local_header_offset = read_u32(data, offset + 42) as usize;

            if offset + 46 + name_len > data.len() {
                return Err("ZIP entry name truncated".to_string());
            }
            let name = std::str::from_utf8(&data[offset + 46..offset + 46 + name_len])
                .unwrap_or("")
                .to_string();

            entries.push(CentralDirEntry {
                compression,
                compressed_size,
                uncompressed_size,
                local_header_offset,
                name,
            });

            offset += 46 + name_len + extra_len + comment_len;
        }

        Ok(entries)
    }

    /// Extract a file from a ZIP archive by name.
    /// Returns the decompressed content of the first matching entry.
    /// Uses the central directory for sizes (more reliable than local headers
    /// when data descriptors are present).
    pub fn extract_file(data: &[u8], target_name: &str) -> Result<String, String> {
        let entries = parse_central_directory(data)?;

        for entry in &entries {
            if entry.name == target_name {
                return read_local_file(data, entry);
            }
        }

        // Try case-insensitive match as fallback
        for entry in &entries {
            if entry.name.to_lowercase() == target_name.to_lowercase() {
                return read_local_file(data, entry);
            }
        }

        Err(format!("File '{target_name}' not found in ZIP archive"))
    }

    /// Read a file from its local file header, using sizes from the central directory
    fn read_local_file(data: &[u8], entry: &CentralDirEntry) -> Result<String, String> {
        let offset = entry.local_header_offset;

        if offset + 30 > data.len() {
            return Err("Local file header truncated".to_string());
        }
        if read_u32(data, offset) != LOCAL_FILE_HEADER_SIGNATURE {
            return Err("Invalid local file header".to_string());
        }

        let name_len = read_u16(data, offset + 26) as usize;
        let extra_len = read_u16(data, offset + 28) as usize;

        let data_start = offset + 30 + name_len + extra_len;

        // Use the compressed size from the central directory entry,
        // which is always correct (local header may have 0 if data descriptor is used)
        let compressed_size = entry.compressed_size as usize;

        if compressed_size == 0 {
            // Data descriptor mode: sizes are 0 in both local and central headers.
            // Try to find the data descriptor signature (0x08074b50) after the compressed data.
            // The data descriptor contains: signature(4) + crc32(4) + compressed(4) + uncompressed(4)
            let dd_sig: u32 = 0x08074b50;
            let search_start = data_start;
            let search_end = data.len().min(data_start + 50_000_000); // limit search to 50MB

            let found = false;
            for i in (search_start..search_end.saturating_sub(4)).step_by(1) {
                if read_u32(data, i) == dd_sig {
                    // Found data descriptor — compressed size is at i+8
                    let _cs = read_u32(data, i + 8) as usize;
                    // The actual compressed data is between data_start and i
                    let actual_compressed = &data[data_start..i];

                    return match entry.compression {
                        0 => std::str::from_utf8(actual_compressed)
                            .map(|s| s.to_string())
                            .map_err(|e| format!("Invalid UTF-8 in stored file: {e}")),
                        8 => {
                            let mut decoder = flate2::read::DeflateDecoder::new(actual_compressed);
                            let mut result = String::new();
                            decoder.read_to_string(&mut result)
                                .map_err(|e| format!("Deflate decompression failed: {e}"))?;
                            Ok(result)
                        }
                        _ => Err(format!("Unsupported compression method: {}", entry.compression)),
                    };
                }
            }

            if !found {
                return Err("Could not find data descriptor in ZIP entry".to_string());
            }
        }

        if data_start + compressed_size > data.len() {
            return Err(format!(
                "Compressed data truncated (need {} bytes at offset {}, but file is {} bytes)",
                compressed_size, data_start, data.len()
            ));
        }

        let compressed = &data[data_start..data_start + compressed_size];

        match entry.compression {
            0 => {
                // Stored (no compression)
                std::str::from_utf8(compressed)
                    .map(|s| s.to_string())
                    .map_err(|e| format!("Invalid UTF-8 in stored file: {e}"))
            }
            8 => {
                // Deflate
                let mut decoder = flate2::read::DeflateDecoder::new(compressed);
                let mut result = String::new();
                decoder.read_to_string(&mut result)
                    .map_err(|e| format!("Deflate decompression failed: {e}"))?;
                Ok(result)
            }
            _ => Err(format!("Unsupported compression method: {}", entry.compression)),
        }
    }
}

/// Extract gcode from a .3mf file (which is a ZIP archive).
/// Bambu 3MF files contain gcode at `Metadata/plate_1.gcode` (or plate_2, etc.)
fn extract_gcode_from_3mf(data: &[u8]) -> Result<String, String> {
    // Strategy 1: Try proper ZIP extraction via central directory
    let mut zip_parse_ok = false;
    for plate_num in 1..=10 {
        let path = format!("Metadata/plate_{plate_num}.gcode");
        match mini_zip::extract_file(data, &path) {
            Ok(gcode) if !gcode.is_empty() => {
                debug!(plate = plate_num, "extracted gcode from 3MF via central directory");
                return Ok(gcode);
            }
            Ok(_) => {
                zip_parse_ok = true;
                continue;
            }
            Err(e) => {
                let err = e.to_lowercase();
                if err.contains("not found") {
                    // File simply doesn't exist in the archive — try next plate
                    continue;
                }
                // ZIP-level parse error — stop trying plates, fall back to raw scan
                debug!(plate = plate_num, error = %e, "ZIP parse error, falling back to raw scan");
                break;
            }
        }
    }

    // If ZIP parsing worked but no gcode plates were found, don't bother with raw scan
    if zip_parse_ok {
        return Err("No gcode found inside 3MF file".to_string());
    }

    // Strategy 2: Raw byte scan — search for the printer_model string directly in the 3MF.
    // Bambu Studio embeds gcode as text inside the ZIP, so the `; printer_model =` string
    // is present as raw UTF-8 bytes even if we can't properly parse the ZIP structure.
    // This handles ZIP files with data descriptors, ZIP64, or other features our mini parser
    // doesn't support.
    if let Some(model) = scan_raw_bytes_for_printer_model(data) {
        debug!("found printer_model via raw byte scan in 3MF");
        // Return a minimal gcode string with just the printer_model line
        return Ok(format!("; printer_model = {model}"));
    }

    Err("No gcode found inside 3MF file".to_string())
}

/// Scan raw bytes of a file for the `; printer_model =` pattern.
/// This is a fallback for when proper ZIP parsing fails (e.g., data descriptors, ZIP64).
fn scan_raw_bytes_for_printer_model(data: &[u8]) -> Option<String> {
    let needle = b"; printer_model =";
    let needle_alt = b"; printer_model=";

    // Search for the needle in the raw bytes
    for i in 0..data.len().saturating_sub(needle.len()) {
        if data[i..].starts_with(needle) || data[i..].starts_with(needle_alt) {
            // Found the prefix — extract the value until end of line
            let prefix_len = if data[i..].starts_with(needle) {
                needle.len()
            } else {
                needle_alt.len()
            };
            let start = i + prefix_len;
            let end = data[start..]
                .iter()
                .position(|&b| b == b'\n' || b == b'\r')
                .map(|pos| start + pos)
                .unwrap_or(data.len().min(start + 200));

            if start < end {
                let value = std::str::from_utf8(&data[start..end]).ok()?;
                let trimmed = value.trim();
                if !trimmed.is_empty() {
                    return Some(trimmed.to_string());
                }
            }
        }
    }
    None
}

/// Validate an uploaded file against a target printer model.
///
/// Supports:
/// - `.gcode` / `.gco` files — parsed directly
/// - `.3mf` files — ZIP archive with embedded gcode
///
/// Returns a `ValidationResult` indicating whether the file is compatible.
pub fn validate_file(data: &[u8], filename: &str, target_printer_model: &str) -> ValidationResult {
    let extension = filename
        .rsplit('.')
        .next()
        .map(|e| e.to_lowercase())
        .unwrap_or_default();

    let gcode = match extension.as_str() {
        "gcode" | "gco" => {
            // Plain gcode file — parse directly
            std::str::from_utf8(data)
                .map_err(|e| format!("Invalid UTF-8 in gcode file: {e}"))
                .map(|s| s.to_string())
        }
        "3mf" => {
            // 3MF ZIP archive — extract embedded gcode
            extract_gcode_from_3mf(data)
        }
        _ => Err(format!(
            "Unsupported file type '.{extension}'. Please upload a .gcode or .3mf file."
        )),
    };

    let gcode = match gcode {
        Ok(g) => g,
        Err(e) => {
            return ValidationResult::invalid(
                None,
                format!("Could not read file: {e}. Please re-upload or contact a TA or Teacher."),
            );
        }
    };

    // Extract printer model from gcode header
    let detected = match extract_printer_model_from_gcode(&gcode) {
        Some(model) => model,
        None => {
            warn!(%filename, "no printer_model found in gcode header");
            return ValidationResult::invalid(
                None,
                "Could not detect printer model from file. The file may not be a valid Bambu Studio slice. \
                 Please re-upload a properly sliced file or contact a TA or Teacher."
                    .to_string(),
            );
        }
    };

    debug!(%filename, %detected, %target_printer_model, "validating gcode printer model");

    // Normalize the detected model
    let normalized = normalize_printer_model(&detected);

    match normalized {
        Some(norm) => {
            if is_compatible(norm, target_printer_model) {
                ValidationResult::valid(detected)
            } else {
                ValidationResult::invalid(
                    Some(detected.clone()),
                    format!(
                        "This file was sliced for '{detected}' but you selected '{target_printer_model}'. \
                         The gcode is not compatible with this printer. \
                         Please re-slice your model for the correct printer or contact a TA or Teacher."
                    ),
                )
            }
        }
        None => {
            ValidationResult::invalid(
                Some(detected.clone()),
                format!(
                    "Unrecognized printer model '{detected}' in file. \
                     Please re-slice your model for a supported printer or contact a TA or Teacher."
                ),
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_printer_model_from_gcode() {
        let gcode = "; BambuStudio 01.09.07.50\n; printer_model = Bambu Lab P1S\n; printer_variant = 0.4\nG28\n";
        assert_eq!(
            extract_printer_model_from_gcode(gcode),
            Some("Bambu Lab P1S".to_string())
        );
    }

    #[test]
    fn test_extract_printer_model_no_space() {
        let gcode = "; printer_model=Bambu Lab A1\nG28\n";
        assert_eq!(
            extract_printer_model_from_gcode(gcode),
            Some("Bambu Lab A1".to_string())
        );
    }

    #[test]
    fn test_extract_printer_model_missing() {
        let gcode = "; some comment\nG28\n";
        assert_eq!(extract_printer_model_from_gcode(gcode), None);
    }

    #[test]
    fn test_validate_gcode_valid() {
        let gcode = "; BambuStudio 01.09.07.50\n; printer_model = Bambu Lab P1S\nG28\n";
        let result = validate_file(gcode.as_bytes(), "test.gcode", "P1S");
        assert!(result.is_valid);
        assert_eq!(result.detected_printer, Some("Bambu Lab P1S".to_string()));
    }

    #[test]
    fn test_validate_gcode_wrong_printer() {
        let gcode = "; BambuStudio 01.09.07.50\n; printer_model = Bambu Lab A1\nG28\n";
        let result = validate_file(gcode.as_bytes(), "test.gcode", "P1S");
        assert!(!result.is_valid);
        assert!(result.error_message.is_some());
        assert!(result.error_message.unwrap().contains("not compatible"));
    }

    #[test]
    fn test_validate_unsupported_extension() {
        let result = validate_file(b"data", "test.stl", "P1S");
        assert!(!result.is_valid);
        assert!(result.error_message.unwrap().contains("Unsupported file type"));
    }

    #[test]
    fn test_normalize_printer_model() {
        assert_eq!(normalize_printer_model("Bambu Lab A1"), Some("A1"));
        assert_eq!(normalize_printer_model("Bambu Lab A1 Mini"), Some("A1 Mini"));
        assert_eq!(normalize_printer_model("Bambu Lab P1S"), Some("P1S"));
        assert_eq!(normalize_printer_model("Bambu Lab X1 Carbon"), Some("X1C"));
        assert_eq!(normalize_printer_model("Unknown"), None);
    }

    #[test]
    fn test_extract_printer_model_with_blank_lines() {
        // Real Bambu Studio gcode can have blank lines in the header
        let gcode = "; BambuStudio 01.09.07.50\n\n; printer_model = Bambu Lab A1 Mini\n; printer_variant = 0.4\nG28\n";
        assert_eq!(
            extract_printer_model_from_gcode(gcode),
            Some("Bambu Lab A1 Mini".to_string())
        );
    }

    #[test]
    fn test_extract_printer_model_deep_header() {
        // printer_model might not be the first comment
        let gcode = "; BambuStudio 01.09.07.50\n; generated by BambuStudio\n; some other comment\n; printer_model = Bambu Lab P1S\n; printer_variant = 0.4\nG28\n";
        assert_eq!(
            extract_printer_model_from_gcode(gcode),
            Some("Bambu Lab P1S".to_string())
        );
    }

    /// Helper to create a minimal valid ZIP file in memory
    fn create_minimal_zip(filename: &str, content: &[u8]) -> Vec<u8> {
        use std::io::Write;
        let mut buf = Vec::new();
        let name_bytes = filename.as_bytes();
        let name_len = name_bytes.len() as u16;
        let content_len = content.len() as u32;

        // Local file header
        buf.write_all(&0x04034b50u32.to_le_bytes()).unwrap(); // signature
        buf.write_all(&20u16.to_le_bytes()).unwrap(); // version needed
        buf.write_all(&0u16.to_le_bytes()).unwrap(); // flags
        buf.write_all(&0u16.to_le_bytes()).unwrap(); // compression (stored)
        buf.write_all(&0u16.to_le_bytes()).unwrap(); // mod time
        buf.write_all(&0u16.to_le_bytes()).unwrap(); // mod date
        buf.write_all(&0u32.to_le_bytes()).unwrap(); // crc32 (skip for test)
        buf.write_all(&content_len.to_le_bytes()).unwrap(); // compressed size
        buf.write_all(&content_len.to_le_bytes()).unwrap(); // uncompressed size
        buf.write_all(&name_len.to_le_bytes()).unwrap(); // name length
        buf.write_all(&0u16.to_le_bytes()).unwrap(); // extra length
        buf.write_all(name_bytes).unwrap();
        buf.write_all(content).unwrap();

        let local_header_offset = 0u32;
        let central_dir_offset = buf.len() as u32;

        // Central directory entry
        buf.write_all(&0x02014b50u32.to_le_bytes()).unwrap(); // signature
        buf.write_all(&20u16.to_le_bytes()).unwrap(); // version made by
        buf.write_all(&20u16.to_le_bytes()).unwrap(); // version needed
        buf.write_all(&0u16.to_le_bytes()).unwrap(); // flags
        buf.write_all(&0u16.to_le_bytes()).unwrap(); // compression
        buf.write_all(&0u16.to_le_bytes()).unwrap(); // mod time
        buf.write_all(&0u16.to_le_bytes()).unwrap(); // mod date
        buf.write_all(&0u32.to_le_bytes()).unwrap(); // crc32
        buf.write_all(&content_len.to_le_bytes()).unwrap(); // compressed size
        buf.write_all(&content_len.to_le_bytes()).unwrap(); // uncompressed size
        buf.write_all(&name_len.to_le_bytes()).unwrap(); // name length
        buf.write_all(&0u16.to_le_bytes()).unwrap(); // extra length
        buf.write_all(&0u16.to_le_bytes()).unwrap(); // comment length
        buf.write_all(&0u16.to_le_bytes()).unwrap(); // disk number
        buf.write_all(&0u16.to_le_bytes()).unwrap(); // internal attrs
        buf.write_all(&0u32.to_le_bytes()).unwrap(); // external attrs
        buf.write_all(&local_header_offset.to_le_bytes()).unwrap(); // local header offset
        buf.write_all(name_bytes).unwrap();

        let central_dir_size = (buf.len() as u32) - central_dir_offset;

        // End of central directory
        buf.write_all(&0x06054b50u32.to_le_bytes()).unwrap(); // signature
        buf.write_all(&0u16.to_le_bytes()).unwrap(); // disk number
        buf.write_all(&0u16.to_le_bytes()).unwrap(); // disk with central dir
        buf.write_all(&1u16.to_le_bytes()).unwrap(); // entries on disk
        buf.write_all(&1u16.to_le_bytes()).unwrap(); // total entries
        buf.write_all(&(central_dir_size as u16).to_le_bytes()).unwrap(); // central dir size
        buf.write_all(&central_dir_offset.to_le_bytes()).unwrap(); // central dir offset
        buf.write_all(&0u16.to_le_bytes()).unwrap(); // comment length

        buf
    }

    #[test]
    fn test_validate_3mf_file() {
        let gcode = "; BambuStudio 01.09.07.50\n; printer_model = Bambu Lab A1\nG28\n";
        let zip_data = create_minimal_zip("Metadata/plate_1.gcode", gcode.as_bytes());
        let result = validate_file(&zip_data, "test.3mf", "A1");
        assert!(result.is_valid, "Expected valid, got error: {:?}", result.error_message);
        assert_eq!(result.detected_printer, Some("Bambu Lab A1".to_string()));
    }

    #[test]
    fn test_validate_3mf_wrong_printer() {
        let gcode = "; BambuStudio 01.09.07.50\n; printer_model = Bambu Lab P1S\nG28\n";
        let zip_data = create_minimal_zip("Metadata/plate_1.gcode", gcode.as_bytes());
        let result = validate_file(&zip_data, "test.3mf", "A1");
        assert!(!result.is_valid);
        assert!(result.error_message.unwrap().contains("not compatible"));
    }

    #[test]
    fn test_raw_byte_scan_fallback() {
        // Simulate a 3MF file where ZIP parsing fails but the gcode text is still
        // present as raw bytes — this is the real-world case with data descriptors
        let gcode = b"; BambuStudio 01.09.07.50\n; printer_model = Bambu Lab P1S\nG28\n";
        // Just embed the gcode in some random bytes (not a valid ZIP)
        let mut data = vec![0x50, 0x4B, 0x03, 0x04]; // PK header
        data.extend_from_slice(gcode);
        data.extend_from_slice(b"\x00\x00\x00\x00"); // padding

        // The raw byte scan should find the printer_model
        let model = scan_raw_bytes_for_printer_model(&data);
        assert_eq!(model, Some("Bambu Lab P1S".to_string()));
    }

    #[test]
    fn test_validate_3mf_raw_scan_fallback() {
        // Create a "broken" 3MF that can't be parsed as ZIP but contains
        // the printer_model string — the fallback should find it
        let mut data = vec![0x50, 0x4B, 0x03, 0x04]; // PK header (not a valid ZIP)
        data.extend_from_slice(b"random bytes ; printer_model = Bambu Lab A1 Mini\nmore data");
        data.extend_from_slice(b"\x00\x00");

        let result = validate_file(&data, "test.3mf", "A1 Mini");
        assert!(result.is_valid, "Expected valid, got error: {:?}", result.error_message);
        assert_eq!(result.detected_printer, Some("Bambu Lab A1 Mini".to_string()));
    }
}
