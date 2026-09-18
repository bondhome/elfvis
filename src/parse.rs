use gimli::{RunTimeEndian, SectionId};
use object::{Object, ObjectSection, ObjectSymbol, SectionKind, SymbolKind};

/// A symbol extracted from the ELF, located in a flash section.
#[derive(Debug, Clone)]
pub struct FlashSymbol {
    pub name: String,
    pub address: u64,
    pub size: u64,
    /// For a local symbol, the object file / translation unit it belongs to,
    /// taken from the `STT_FILE` symbol that precedes it in `.symtab` (the ELF
    /// ABI's provenance record for locals). `None` for global/weak symbols,
    /// whose names are unique across the link.
    pub tu: Option<String>,
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
    let mut current_file: Option<String> = None;
    for sym in obj.symbols() {
        // `STT_FILE` entries precede the local symbols of each object file.
        if sym.kind() == SymbolKind::File {
            current_file = sym.name().ok().filter(|n| !n.is_empty()).map(str::to_string);
            continue;
        }
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
            tu: if sym.is_local() { current_file.clone() } else { None },
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

/// A symbol with everything comparison mode needs to give it a stable
/// identity: the raw (untrimmed) source path alongside the display-trimmed one,
/// and translation-unit provenance for locals.
#[derive(Debug, Clone)]
pub struct SymbolDetail {
    pub name: String,
    pub size: u64,
    /// Source path exactly as DWARF reports it. Unlike `display_path`, it does
    /// not depend on which other files happen to be in this ELF, so two ELFs
    /// can be normalized against each other consistently.
    pub raw_path: Option<String>,
    /// Source path with this ELF's own longest common directory prefix removed.
    pub display_path: Option<String>,
    /// See [`FlashSymbol::tu`].
    pub tu: Option<String>,
}

/// Parse ELF + DWARF and return symbols with source paths.
pub fn parse_elf(data: &[u8]) -> Result<Vec<ResolvedSymbol>, String> {
    Ok(parse_elf_detailed(data)?
        .into_iter()
        .map(|d| ResolvedSymbol { name: d.name, size: d.size, source_path: d.display_path })
        .collect())
}

/// Like [`parse_elf`], but keeps raw paths and translation-unit provenance.
pub fn parse_elf_detailed(data: &[u8]) -> Result<Vec<SymbolDetail>, String> {
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
    // (low, high, display path, raw path): the raw path is resolved against the
    // CU's comp_dir, since line-table directories may be relative to it.
    let mut addr_to_path: Vec<(u64, u64, String, String)> = Vec::new();

    let mut units = dwarf.units();
    while let Some(header) = units.next().map_err(|e| format!("DWARF error: {e}"))? {
        let unit = dwarf
            .unit(header)
            .map_err(|e| format!("DWARF error: {e}"))?;
        let comp_dir = unit.comp_dir.as_ref().map(|d| d.to_string_lossy().into_owned());
        if let Some(line_program) = unit.line_program.clone() {
            let mut rows = line_program.rows();
            let mut prev_row: Option<(u64, String, String)> = None;

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

                if let Some((prev_addr, ref prev_path, ref prev_raw)) = prev_row {
                    let addr = row.address();
                    if addr > prev_addr {
                        addr_to_path.push((prev_addr, addr, prev_path.clone(), prev_raw.clone()));
                    }
                }

                if row.end_sequence() {
                    // End-of-sequence: close range, don't carry forward
                    prev_row = None;
                } else {
                    let raw = resolve_against(&file_path, comp_dir.as_deref());
                    prev_row = Some((row.address(), file_path, raw));
                }
            }
        }
    }

    // Level 2: Walk DW_TAG_variable DIEs to attribute data symbols.
    // For each variable with DW_AT_location containing DW_OP_addr,
    // resolve DW_AT_decl_file to a source path and add a point range.
    let mut var_units = dwarf.units();
    while let Some(header) = var_units.next().map_err(|e| format!("DWARF error: {e}"))? {
        let unit = dwarf
            .unit(header)
            .map_err(|e| format!("DWARF error: {e}"))?;

        let var_comp_dir = unit.comp_dir.as_ref().map(|d| d.to_string_lossy().into_owned());

        // We need the line program header to resolve DW_AT_decl_file indices
        let line_header = match &unit.line_program {
            Some(lp) => lp.header().clone(),
            None => continue,
        };

        let mut entries = unit.entries();
        while let Some((_, entry)) = entries.next_dfs().map_err(|e| format!("DWARF error: {e}"))? {
            if entry.tag() != gimli::DW_TAG_variable {
                continue;
            }

            // Need DW_AT_location with DW_OP_addr
            let address = match entry
                .attr_value(gimli::DW_AT_location)
                .map_err(|e| format!("DWARF error: {e}"))?
            {
                Some(gimli::AttributeValue::Exprloc(expr)) => {
                    let mut ops = expr.operations(unit.encoding());
                    match ops.next() {
                        Ok(Some(gimli::Operation::Address { address })) => address,
                        _ => continue,
                    }
                }
                _ => continue,
            };

            // Resolve DW_AT_decl_file to source path
            let file_index = match entry
                .attr_value(gimli::DW_AT_decl_file)
                .map_err(|e| format!("DWARF error: {e}"))?
            {
                Some(gimli::AttributeValue::FileIndex(idx)) => idx,
                _ => continue,
            };

            let file_path = match line_header.file(file_index) {
                Some(file) => {
                    let mut path = String::new();
                    if let Some(dir) = file.directory(&line_header) {
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
                }
                None => continue,
            };

            // Add point range so binary search picks up this address
            let raw = resolve_against(&file_path, var_comp_dir.as_deref());
            addr_to_path.push((address, address + 1, file_path, raw));
        }
    }

    addr_to_path.sort_by_key(|&(low, _, _, _)| low);

    // Strip longest common directory prefix from all paths so the tree shows
    // relative paths (e.g. "ctrl/Sidekick/Sidekick.h" instead of
    // "/Users/.../bond-core/ctrl/Sidekick/Sidekick.h"). The untrimmed paths are
    // kept too: this prefix depends on which files this ELF contains, so it is
    // fine for display but not for matching symbols across two ELFs.
    let mut display_map: Vec<(u64, u64, String)> =
        addr_to_path.iter().map(|r| (r.0, r.1, r.2.clone())).collect();
    strip_common_prefix(&mut display_map);

    let resolved: Vec<SymbolDetail> = flash_symbols
        .into_iter()
        .map(|sym| {
            // Binary search: find the last range whose low <= sym.address
            let range = addr_to_path
                .binary_search_by_key(&sym.address, |&(low, _, _, _)| low)
                .map_or_else(
                    |i| i.checked_sub(1),
                    Some,
                )
                .filter(|&i| {
                    let (low, high, _, _) = addr_to_path[i];
                    sym.address >= low && sym.address < high
                });
            SymbolDetail {
                name: sym.name,
                size: sym.size,
                raw_path: range.map(|i| addr_to_path[i].3.clone()),
                display_path: range.map(|i| display_map[i].2.clone()),
                tu: sym.tu,
            }
        })
        .collect();

    Ok(resolved)
}

/// `path` made absolute against the compilation unit's `comp_dir` when it is
/// relative. DWARF line-table directories are relative to `comp_dir`, so
/// without this one ELF mixes relative and absolute spellings of paths that
/// live under the same build root.
fn resolve_against(path: &str, comp_dir: Option<&str>) -> String {
    match comp_dir {
        Some(dir) if !path.starts_with('/') => format!("{}/{path}", dir.trim_end_matches('/')),
        _ => path.to_string(),
    }
}

/// Byte length of the longest common directory prefix of `paths` (ending just
/// after a `/`), or 0 if there is none. Only cuts at directory boundaries.
pub fn common_dir_prefix_len<'a>(paths: impl IntoIterator<Item = &'a str>) -> usize {
    let mut iter = paths.into_iter();
    let Some(first) = iter.next() else { return 0 };
    let first = first.as_bytes();
    let mut prefix_len = first.len();
    for other in iter {
        let other = other.as_bytes();
        prefix_len = prefix_len.min(other.len());
        for i in 0..prefix_len {
            if first[i] != other[i] {
                prefix_len = i;
                break;
            }
        }
    }
    first[..prefix_len].iter().rposition(|&b| b == b'/').map_or(0, |pos| pos + 1)
}

/// Strip the longest common directory prefix from all paths in the address map.
/// Only strips at directory boundaries (i.e. at '/' characters).
fn strip_common_prefix(entries: &mut [(u64, u64, String)]) {
    let strip = common_dir_prefix_len(entries.iter().map(|e| e.2.as_str()));
    if strip > 0 {
        for entry in entries.iter_mut() {
            entry.2 = entry.2[strip..].to_string();
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
