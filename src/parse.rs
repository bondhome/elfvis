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
}
