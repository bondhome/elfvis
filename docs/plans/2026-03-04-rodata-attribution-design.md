---
title: "elfvis: .rodata Attribution"
subtitle: Source-file attribution for read-only data sections
date: March 4, 2026
abstract: |
  elfvis currently attributes .text symbols to source files via DWARF line tables, but
  .rodata (string literals, jump tables, const arrays) is either invisible or lands in
  <unknown>. In a typical firmware ELF, this hides ~20% of flash usage. This design adds
  gap synthesis for anonymous .rodata regions, DWARF variable DIE attribution, CU range
  fallback for unresolved .text symbols, and architecture-specific cross-reference analysis
  to trace .rodata back to source files through instruction scanning.
---

## Problem

In a Bond Mate firmware ELF (455 KB flash):

| Category | Bytes | % of flash |
|----------|-------|------------|
| `.text` (code) | 333,000 | 73.2% |
| `.rodata` (data) | 122,072 | 26.8% |
| Named `.rodata` symbols | 28,574 | 6.3% |
| **Anonymous `.rodata`** | **93,498** | **20.5%** |

The anonymous `.rodata` includes compiler-generated jump tables, string literals, and
const arrays with no symbol table entry. Named `.rodata` symbols exist but go to
`<unknown>` because DWARF line tables only cover `.text` addresses.

Example: `Bond_Error_Get_Message` is a 2,900-line switch returning error strings. The
compiler optimizes it to 28 bytes of code + a 5,728-byte jump table + ~15 KB of string
literals — all in `.rodata` with no attribution.

## Design

### Data Model

Add section kind tracking to distinguish code from data:

```rust
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SymbolKind {
    Code,         // .text
    ReadOnlyData, // .rodata, .rodata.str1.4, etc.
}

pub struct FlashSymbol {
    pub name: String,
    pub address: u64,
    pub size: u64,
    pub kind: SymbolKind,  // new
}

pub struct ResolvedSymbol {
    pub name: String,
    pub size: u64,
    pub source_path: Option<String>,
    pub kind: SymbolKind,  // new — drives visual distinction
}
```

### Code Organization

```
src/
├── parse.rs          # ELF symbol extraction + gap synthesis
├── dwarf.rs          # DWARF attribution: line tables, variable DIEs, CU ranges
├── xref/
│   ├── mod.rs        # Cross-reference trait, dispatcher, attribution logic
│   └── arm_thumb2.rs # ARM Thumb-2 PC-relative load scanner
├── tree.rs           # Tree building (unchanged interface)
├── layout.rs
├── color.rs
└── render.rs
```

**`parse.rs`** — structural extraction. Answers "what symbols exist, where, and how big?"
Extracts named symbols from `.text` and `.rodata` sections. Synthesizes gap entries for
anonymous `.rodata` regions. Orchestrates the full pipeline: extract → DWARF attribute →
xref attribute → return resolved symbols.

**`dwarf.rs`** — DWARF-based attribution. Answers "which source file owns this address?"
Contains the existing line table logic (moved from parse.rs) plus new variable DIE walking
and CU range fallback. Architecture-agnostic.

**`xref/`** — cross-reference attribution. Answers "which source file references this data?"
Architecture-specific instruction scanning behind a common trait. The dispatcher selects
the scanner based on ELF machine type.

### Gap Synthesis

After extracting named `.rodata` symbols, scan for unaccounted regions:

1. Sort named `.rodata` symbols by address
2. Walk the `.rodata` section address range, identify gaps between named symbols
3. Create synthetic `FlashSymbol` entries for gaps >= 32 bytes
4. Name by address: `data@0x08066414`
5. These enter the attribution pipeline as unresolved symbols

### Attribution Pipeline

Three levels, each refining the previous. A symbol attributed at any level keeps that
attribution and is not re-evaluated by later levels.

**Level 1 — DWARF line tables** (existing)

Map `.text` addresses to source files via DWARF `.debug_line`. This already works and
handles most `.text` symbols. Does not cover `.rodata` addresses.

**Level 2 — DWARF variable DIEs + CU ranges**

Walk all compilation units. For each CU:

- Record the CU's source file path (`DW_AT_name` + `DW_AT_comp_dir`)
- Find `DW_TAG_variable` DIEs with `DW_AT_location` containing `DW_OP_addr`[^1]
- If the address falls in `.rodata` → attribute to this CU's source file
- For still-unattributed `.text` symbols: check `DW_AT_ranges` / `DW_AT_low_pc` +
  `DW_AT_high_pc`. If the symbol falls within exactly one CU's ranges → attribute

[^1]: In the Bond Mate ELF, 1,577 variable DIEs point to `.rodata` addresses. These cover
    named const variables like `AHBPrescTable`, `APBPrescTable`, etc. String literals and
    compiler-generated tables do NOT get variable DIEs.

**Level 3 — Cross-reference analysis** (architecture-specific)

For each `.text` function with a known source file:

1. Read the function's raw instruction bytes
2. Identify PC-relative loads that reference `.rodata` addresses
3. Record `(source_file, rodata_address)` edges
4. Follow one level of pointer indirection: if the loaded word is itself a `.rodata`
   pointer, transitively record a reference to that target too[^2]

Final attribution pass: for each unattributed `.rodata` address, if referenced from
exactly ONE source file → attribute. If referenced from multiple files → keep unattributed.

[^2]: This captures the jump table → string literal chain. `Bond_Error_Get_Message` loads
    from a literal pool pointing to 0x08066414 (jump table base). Each jump table entry
    is a pointer to a string literal. One level of indirection attributes both the table
    and all its string targets to `Bond_Error.c`.

### Cross-Reference Trait

```rust
// xref/mod.rs
pub struct RodataRef {
    pub from_addr: u64,  // .text address of referencing instruction
    pub to_addr: u64,    // .rodata address being referenced
}

pub trait XrefScanner {
    fn scan_function(&self, code: &[u8], base_addr: u64, full_image: &[u8]) -> Vec<RodataRef>;
}
```

Dispatcher selects scanner based on ELF `e_machine`:

| `e_machine` | Scanner | Notes |
|-------------|---------|-------|
| `EM_ARM` | `ArmThumb2Scanner` | 16-bit `ldr Rt,[PC,#imm]` (0x48xx) + 32-bit variants |
| other | `None` (skip level 3) | Graceful degradation |

Future scanners (Xtensa for ESP32, RISC-V, x86) add a file and a match arm.

### ARM Thumb-2 Scanner

Key instruction patterns:

- **16-bit `ldr Rt, [PC, #imm8*4]`**: opcode bits `[15:11] = 01001`, offset = `imm8 * 4`,
  target = `(PC & ~3) + 4 + offset`
- **32-bit `ldr.w Rt, [PC, #imm12]`**: encoding T3, target = `PC + 4 ± imm12`

For each identified literal pool reference:
1. Compute the literal pool address
2. Read the 4-byte value at that address from the full ELF image
3. If the value falls within `.rodata` address range → emit `RodataRef`
4. Optionally follow one indirection level: read the word at the `.rodata` target and check
   if it also points to `.rodata`

### Tree Structure

Attributed `.rodata` appears under source files alongside code. Unattributed `.rodata`
goes under a `<data>` top-level node:

```
root
├── sys/SysLog/Bond_Error.c
│   ├── Bond_Error_Get_Message        28B  (code)
│   ├── data@0x08066414            5,728B  (data, xref-attributed)
│   └── data@0x0805XXXX          ~15,000B  (data, xref-attributed)
├── src/app/main.c
│   ├── func_a                      100B  (code)
│   └── AHBPrescTable                64B  (data, DWARF-attributed)
├── <unknown>                              (unattributed .text, clustered)
│   ├── __
│   └── ...
└── <data>                                 (unattributed .rodata)
    └── remaining gaps
```

The tree builder receives `Vec<ResolvedSymbol>` with `kind` set. No changes to the tree
building interface — `.rodata` symbols with `source_path: Some(...)` are placed under their
source file, those with `source_path: None` go to `<data>` instead of `<unknown>`.

### Visual Distinction

`.rodata` symbols are rendered with **higher color saturation** than `.text` symbols from
the same file. Same hue (preserving the per-file color), boosted saturation. This makes
data blocks immediately distinguishable in the treemap without changing the overall color
scheme.

### Implementation Phases

**Phase 1:** Data model + gap synthesis + `dwarf.rs` (line tables moved + variable DIEs +
CU ranges) + tree/visual changes. All architecture-agnostic.

**Phase 2:** `xref/` module with ARM Thumb-2 scanner + pointer chain following +
cross-reference attribution.

### Validation

Test against Bond Mate ELF. Success criteria:

| Metric | Before | Target |
|--------|--------|--------|
| Visible flash in treemap | 333 KB (.text only) | ~440 KB (.text + .rodata) |
| Bond_Error.c apparent size | 28 B | ~20 KB |
| `<unknown>` + `<data>` symbols | ~1,564 | fewer (better attribution) |
