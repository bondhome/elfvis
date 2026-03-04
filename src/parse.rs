use gimli::{RunTimeEndian, SectionId};
use object::{Object, ObjectSection, ObjectSymbol, SectionKind};

/// A symbol extracted from the ELF, located in a flash section.
#[derive(Debug, Clone)]
pub struct FlashSymbol {
    pub name: String,
    pub address: u64,
    pub size: u64,
}

/// Parse an ELF binary and return all symbols in flash sections (.text, .rodata).
pub fn extract_flash_symbols(data: &[u8]) -> Result<Vec<FlashSymbol>, String> {
    let obj = object::File::parse(data).map_err(|e| format!("Failed to parse ELF: {e}"))?;

    // Collect section indices for flash sections (.text, .rodata, etc.)
    let flash_sections: Vec<object::SectionIndex> = obj
        .sections()
        .filter(|s| {
            matches!(
                s.kind(),
                SectionKind::Text | SectionKind::ReadOnlyData | SectionKind::ReadOnlyString
            )
        })
        .map(|s| s.index())
        .collect();

    let mut symbols = Vec::new();
    for sym in obj.symbols() {
        let section_idx = match sym.section() {
            object::SymbolSection::Section(idx) => idx,
            _ => continue,
        };
        if !flash_sections.contains(&section_idx) {
            continue;
        }
        let name = match sym.name() {
            Ok(n) if !n.is_empty() => n.to_string(),
            _ => continue,
        };
        let size = sym.size();
        if size == 0 {
            continue;
        }
        symbols.push(FlashSymbol {
            name,
            address: sym.address(),
            size,
        });
    }

    Ok(symbols)
}

/// A symbol with its source file path resolved from DWARF.
#[derive(Debug, Clone)]
pub struct ResolvedSymbol {
    pub name: String,
    pub size: u64,
    /// Source file path from DWARF, or None if not found.
    pub source_path: Option<String>,
}

/// Parse ELF + DWARF and return symbols with source paths.
pub fn parse_elf(data: &[u8]) -> Result<Vec<ResolvedSymbol>, String> {
    let flash_symbols = extract_flash_symbols(data)?;
    if flash_symbols.is_empty() {
        return Ok(Vec::new());
    }

    let obj = object::File::parse(data).map_err(|e| format!("Failed to parse ELF: {e}"))?;

    let endian = if obj.is_little_endian() {
        RunTimeEndian::Little
    } else {
        RunTimeEndian::Big
    };

    let load_section =
        |id: SectionId| -> Result<gimli::EndianSlice<'_, RunTimeEndian>, gimli::Error> {
            let data = obj
                .section_by_name(id.name())
                .and_then(|s| s.data().ok())
                .unwrap_or(&[]);
            Ok(gimli::EndianSlice::new(data, endian))
        };
    let dwarf =
        gimli::Dwarf::load(&load_section).map_err(|e| format!("Failed to load DWARF: {e}"))?;

    // Check that we actually have debug info
    let mut units = dwarf.units();
    if units
        .next()
        .map_err(|e| format!("DWARF error: {e}"))?
        .is_none()
    {
        return Err("Found your ELF but not your DWARF. Rebuild with `-g`.".to_string());
    }

    // Build address -> source path ranges from DWARF line programs
    let mut addr_to_path: Vec<(u64, u64, String)> = Vec::new();

    let mut units = dwarf.units();
    while let Some(header) = units.next().map_err(|e| format!("DWARF error: {e}"))? {
        let unit = dwarf
            .unit(header)
            .map_err(|e| format!("DWARF error: {e}"))?;
        if let Some(line_program) = unit.line_program.clone() {
            let mut rows = line_program.rows();
            let mut prev_row: Option<(u64, String)> = None;

            while let Some((header, row)) =
                rows.next_row().map_err(|e| format!("DWARF error: {e}"))?
            {
                let file_path = if let Some(file) = row.file(header) {
                    let mut path = String::new();
                    if let Some(dir) = file.directory(header) {
                        let dir_str = dwarf
                            .attr_string(&unit, dir)
                            .map_err(|e| format!("DWARF error: {e}"))?;
                        let dir_s = dir_str.to_string_lossy();
                        if !dir_s.is_empty() {
                            path.push_str(&dir_s);
                            path.push('/');
                        }
                    }
                    let file_str = dwarf
                        .attr_string(&unit, file.path_name())
                        .map_err(|e| format!("DWARF error: {e}"))?;
                    path.push_str(&file_str.to_string_lossy());
                    path
                } else {
                    continue;
                };

                if let Some((prev_addr, ref prev_path)) = prev_row {
                    let addr = row.address();
                    if addr > prev_addr {
                        addr_to_path.push((prev_addr, addr, prev_path.clone()));
                    }
                }

                if row.end_sequence() {
                    // End-of-sequence: close range, don't carry forward
                    prev_row = None;
                } else {
                    prev_row = Some((row.address(), file_path));
                }
            }
        }
    }

    addr_to_path.sort_by_key(|&(low, _, _)| low);

    // Strip longest common directory prefix from all paths so the tree shows
    // relative paths (e.g. "ctrl/Sidekick/Sidekick.h" instead of
    // "/Users/.../bond-core/ctrl/Sidekick/Sidekick.h").
    strip_common_prefix(&mut addr_to_path);

    let resolved: Vec<ResolvedSymbol> = flash_symbols
        .into_iter()
        .map(|sym| {
            // Binary search: find the last range whose low <= sym.address
            let source_path = addr_to_path
                .binary_search_by_key(&sym.address, |&(low, _, _)| low)
                .map_or_else(
                    |i| i.checked_sub(1),
                    |i| Some(i),
                )
                .and_then(|i| {
                    let (low, high, ref path) = addr_to_path[i];
                    if sym.address >= low && sym.address < high {
                        Some(path.clone())
                    } else {
                        None
                    }
                });
            ResolvedSymbol {
                name: sym.name,
                size: sym.size,
                source_path,
            }
        })
        .collect();

    Ok(resolved)
}

/// Strip the longest common directory prefix from all paths in the address map.
/// Only strips at directory boundaries (i.e. at '/' characters).
fn strip_common_prefix(entries: &mut Vec<(u64, u64, String)>) {
    if entries.is_empty() {
        return;
    }
    // Find longest common byte prefix
    let first = entries[0].2.as_bytes();
    let mut prefix_len = first.len();
    for entry in entries.iter().skip(1) {
        let other = entry.2.as_bytes();
        prefix_len = prefix_len.min(other.len());
        for i in 0..prefix_len {
            if first[i] != other[i] {
                prefix_len = i;
                break;
            }
        }
    }
    // Trim back to last '/' boundary
    if let Some(pos) = first[..prefix_len].iter().rposition(|&b| b == b'/') {
        let strip = pos + 1; // include the '/'
        if strip > 0 {
            for entry in entries.iter_mut() {
                entry.2 = entry.2[strip..].to_string();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    static ARM_ELF: &[u8] = include_bytes!("../tests/fixtures/arm.elf");

    #[test]
    fn test_extracts_symbols_from_arm_elf() {
        let symbols = extract_flash_symbols(ARM_ELF).unwrap();
        assert!(!symbols.is_empty(), "should find at least one flash symbol");
    }

    #[test]
    fn test_finds_known_function() {
        let symbols = extract_flash_symbols(ARM_ELF).unwrap();
        let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"app_init"), "should find app_init symbol");
    }

    #[test]
    fn test_symbols_have_nonzero_size() {
        let symbols = extract_flash_symbols(ARM_ELF).unwrap();
        let app_init = symbols.iter().find(|s| s.name == "app_init").unwrap();
        assert!(app_init.size > 0, "app_init should have nonzero size");
    }

    #[test]
    fn test_finds_rodata() {
        let symbols = extract_flash_symbols(ARM_ELF).unwrap();
        let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"version"), "should find version string in .rodata");
    }

    #[test]
    fn test_rejects_non_elf() {
        let result = extract_flash_symbols(b"not an elf");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_elf_resolves_source_paths() {
        let symbols = parse_elf(ARM_ELF).unwrap();
        let app_init = symbols.iter().find(|s| s.name == "app_init").unwrap();
        let path = app_init
            .source_path
            .as_ref()
            .expect("app_init should have a source path");
        assert!(
            path.ends_with("main.c"),
            "app_init should come from main.c, got: {path}"
        );
    }

    #[test]
    fn test_parse_elf_resolves_different_files() {
        let symbols = parse_elf(ARM_ELF).unwrap();
        let util_add = symbols.iter().find(|s| s.name == "util_add").unwrap();
        let path = util_add
            .source_path
            .as_ref()
            .expect("util_add should have a source path");
        assert!(
            path.ends_with("util.c"),
            "util_add should come from util.c, got: {path}"
        );
    }

    #[test]
    fn test_parse_elf_succeeds_with_dwarf() {
        let result = parse_elf(ARM_ELF);
        assert!(result.is_ok());
    }

    #[test]
    fn test_strip_common_prefix_absolute_paths() {
        let mut entries = vec![
            (0, 10, "/Users/me/eng/bond-core/ctrl/Sidekick/Sidekick.h".into()),
            (10, 20, "/Users/me/eng/bond-core/target/mate/Sidekick.h".into()),
            (20, 30, "/Users/me/eng/bond-core/sys/SysLog.c".into()),
        ];
        strip_common_prefix(&mut entries);
        assert_eq!(entries[0].2, "ctrl/Sidekick/Sidekick.h");
        assert_eq!(entries[1].2, "target/mate/Sidekick.h");
        assert_eq!(entries[2].2, "sys/SysLog.c");
    }

    #[test]
    fn test_strip_common_prefix_no_common() {
        let mut entries = vec![
            (0, 10, "src/main.c".into()),
            (10, 20, "/opt/gcc/include/stdio.h".into()),
        ];
        strip_common_prefix(&mut entries);
        // No common prefix — paths unchanged
        assert_eq!(entries[0].2, "src/main.c");
        assert_eq!(entries[1].2, "/opt/gcc/include/stdio.h");
    }

    #[test]
    fn test_strip_common_prefix_empty() {
        let mut entries: Vec<(u64, u64, String)> = vec![];
        strip_common_prefix(&mut entries);
        assert!(entries.is_empty());
    }

    #[test]
    fn test_strip_common_prefix_single() {
        let mut entries = vec![
            (0, 10, "/a/b/c/file.c".into()),
        ];
        strip_common_prefix(&mut entries);
        // Single entry: prefix is entire path, trimmed to last dir boundary = "file.c"
        assert_eq!(entries[0].2, "file.c");
    }

    #[test]
    fn test_variable_die_attributes_rodata_symbol() {
        // The arm.elf fixture has 'version' in .rodata defined in main.c.
        // Line tables don't cover .rodata addresses, but DW_TAG_variable
        // DIEs should provide the attribution.
        let symbols = parse_elf(ARM_ELF).unwrap();
        let version = symbols.iter().find(|s| s.name == "version").unwrap();
        assert!(
            version.source_path.is_some(),
            "version (.rodata) should be attributed via DW_TAG_variable DIE, got None"
        );
        let path = version.source_path.as_ref().unwrap();
        assert!(
            path.contains("main.c"),
            "version should be attributed to main.c, got: {path}"
        );
    }
}
