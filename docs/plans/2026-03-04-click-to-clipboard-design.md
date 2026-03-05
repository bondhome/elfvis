---
title: Click-to-Select with Clipboard Export
subtitle: elfvis Feature Design
date: March 04, 2026
abstract: |
  Adds click-to-select interaction to the elfvis treemap. Clicking nodes highlights
  them and copies a structured text description to the clipboard, enabling quick
  paste into Claude Code for flash optimization guidance.
---

## Selection Model

Selection state is a `HashSet<Vec<String>>` in `AppState`, where each entry is a
node path from root (e.g. `["src", "app/main.c", "func_a"]`).

**Interaction:**
- **Click**: replace selection with clicked node, or deselect if it's the sole selected item
- **Shift+click**: toggle clicked node in/out of selection

**Hit targets:**
- Leaf symbols: click anywhere in the cell
- Directories/files: click on the header bar (14px region)
- `hit_test` modified to return the header-level node when click Y falls within the header region

## Highlight Rendering

Selected nodes rendered with boosted saturation (~0.7) and lower lightness (~0.60)
compared to the pastel defaults (0.30–0.55 sat, 0.70–0.88 light). A `selected_color()`
variant in `color.rs` provides this.

For selected directories, the header bar is highlighted. For selected symbols, the
leaf cell is highlighted. Children of a selected directory are NOT individually
highlighted.

## Clipboard Format

On each selection change, structured text is written to the clipboard via the
browser Clipboard API.

**Single symbol:**
```
src/app/main.c: func_a
```

**Multiple symbols in same file:**
```
src/app/main.c: func_a, func_b
```

**Symbols across files with common parent:**
```
src/
  app/main.c: func_a, func_b
  lib/util.c: func_c
```

**Directory selection:**
```
src/app/
```

**Mixed directories:**
```
src/
  app/
  lib/
```

When selection is cleared, the clipboard is left alone.

## Notification

A brief "Copied to clipboard" message flashes in the header bar (right side),
fading after ~1.5s. Implemented as a `<span>` in the existing header.

## Files Changed

- `src/lib.rs` — add `selected` to `AppState`, click handler, clipboard write, notification
- `src/layout.rs` — modify `hit_test` to be header-aware
- `src/render.rs` — accept selection set, render highlights
- `src/color.rs` — add `selected_color()` function
- `www/index.html` — add clipboard notification `<span>` in header
