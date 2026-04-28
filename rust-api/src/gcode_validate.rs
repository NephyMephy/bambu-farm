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
    for line in gcode.lines().take(200) {
        // Only scan header comments
        if !line.starts_with(';') {
            break;
        }
        if let Some(value) = line.strip_prefix("; printer_model =") {
            let trimmed = value.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
        // Also check alternate format
        if let Some(value) = line.strip_prefix("; printer_model=") {
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

    /// Extract a file from a ZIP archive by name.
    /// Returns the decompressed content of the first matching entry.
    pub fn extract_file(data: &[u8], target_name: &str) -> Result<String, String> {
        let eocd_offset = find_eocd(data).ok_or("Not a valid ZIP/3MF file: EOCD not found")?;

        // Parse EOCD
        let num_entries = read_u16(data, eocd_offset + 10) as usize;
        let central_dir_offset = read_u32(data, eocd_offset + 16) as usize;

        // Walk the central directory entries
        let mut offset = central_dir_offset;
        for _ in 0..num_entries {
            if offset + 46 > data.len() {
                return Err("ZIP central directory truncated".to_string());
            }
            if read_u32(data, offset) != 0x02014b50 {
                return Err("Invalid central directory entry".to_string());
            }

            let name_len = read_u16(data, offset + 28) as usize;
            let extra_len = read_u16(data, offset + 30) as usize;
            let comment_len = read_u16(data, offset + 32) as usize;
            let local_header_offset = read_u32(data, offset + 42) as usize;

            // Read entry name
            if offset + 46 + name_len > data.len() {
                return Err("ZIP entry name truncated".to_string());
            }
            let name = std::str::from_utf8(&data[offset + 46..offset + 46 + name_len])
                .unwrap_or("");

            if name == target_name {
                // Found it — read from local file header
                return read_local_file(data, local_header_offset);
            }

            offset += 46 + name_len + extra_len + comment_len;
        }

        Err(format!("File '{target_name}' not found in ZIP archive"))
    }

    /// Read a file from its local file header
    fn read_local_file(data: &[u8], offset: usize) -> Result<String, String> {
        if offset + 30 > data.len() {
            return Err("Local file header truncated".to_string());
        }
        if read_u32(data, offset) != LOCAL_FILE_HEADER_SIGNATURE {
            return Err("Invalid local file header".to_string());
        }

        let compression = read_u16(data, offset + 8);
        let compressed_size = read_u32(data, offset + 18) as usize;
        let name_len = read_u16(data, offset + 26) as usize;
        let extra_len = read_u16(data, offset + 28) as usize;

        let data_start = offset + 30 + name_len + extra_len;
        if data_start + compressed_size > data.len() {
            return Err("Compressed data truncated".to_string());
        }

        let compressed = &data[data_start..data_start + compressed_size];

        match compression {
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
            _ => Err(format!("Unsupported compression method: {compression}")),
        }
    }
}

/// Extract gcode from a .3mf file (which is a ZIP archive).
/// Bambu 3MF files contain gcode at `Metadata/plate_1.gcode` (or plate_2, etc.)
fn extract_gcode_from_3mf(data: &[u8]) -> Result<String, String> {
    // Try plate_1 first, then plate_2, etc.
    for plate_num in 1..=10 {
        let path = format!("Metadata/plate_{plate_num}.gcode");
        match mini_zip::extract_file(data, &path) {
            Ok(gcode) if !gcode.is_empty() => {
                debug!(plate = plate_num, "extracted gcode from 3MF");
                return Ok(gcode);
            }
            Ok(_) => continue, // Empty gcode, try next plate
            Err(_) => continue, // Plate not found, try next
        }
    }

    Err("No gcode found inside 3MF file".to_string())
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
}
