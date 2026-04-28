---
title: Click-to-Select with Clipboard Export
subtitle: Implementation Plan
date: March 04, 2026
abstract: |
  Step-by-step implementation plan for adding click-to-select with clipboard
  export to elfvis. Follows TDD where testable (pure logic), with integration
  steps for browser-dependent code.
---

# Click-to-Select Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Let users click treemap nodes to select them, highlighting them visually and copying a structured text description to the clipboard for pasting into Claude Code.

**Architecture:** Selection state stored as `HashSet<Vec<String>>` in the existing `AppState`. Hit-test upgraded to be header-aware so directories/files can be selected via their header bars. Rendering checks selection set to boost saturation. A pure `format_clipboard` function builds the clipboard string from the selection set and layout tree. Browser clipboard API writes on each selection change.

**Tech Stack:** Rust/WASM, web-sys (Clipboard API, Navigator), HTML5 Canvas

---

### Task 1: Add `selected_color()` to color.rs

**Files:**
- Modify: `src/color.rs:18-23`

**Step 1: Write the failing test**

Add to `src/color.rs` in the `tests` module:

```rust
#[test]
fn test_selected_color_more_vivid() {
    let normal = pastel_color(120.0, 2);
    let selected = selected_color(120.0, 2);
    // Selected should be more saturated (lower lightness sum)
    let lum_normal = normal.r as u32 + normal.g as u32 + normal.b as u32;
    let lum_selected = selected.r as u32 + selected.g as u32 + selected.b as u32;
    assert!(lum_selected < lum_normal, "selected should be more vivid/darker");
}

#[test]
fn test_selected_color_same_hue_family() {
    // Red hue: selected should still be reddish (r > g, r > b)
    let c = selected_color(0.0, 2);
    assert!(c.r > c.g && c.r > c.b, "red hue should stay reddish: ({},{},{})", c.r, c.g, c.b);
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test test_selected_color -v`
Expected: FAIL — `selected_color` not defined

**Step 3: Write minimal implementation**

Add to `src/color.rs` after `pastel_color`:

```rust
/// Generate a vivid highlight color for selected nodes.
/// Same hue mapping but boosted saturation and lower lightness.
pub fn selected_color(hue: f64, depth: usize) -> Color {
    let saturation = (0.55 + depth as f64 * 0.04).min(0.75);
    let lightness = (0.65 - depth as f64 * 0.015).max(0.50);
    hsl_to_rgb(hue % 360.0, saturation, lightness)
}
```

**Step 4: Run test to verify it passes**

Run: `cargo test test_selected_color -v`
Expected: PASS (both tests)

**Step 5: Commit**

```bash
git add src/color.rs
git commit -m "feat(elfvis): add selected_color for highlight rendering"
```

---

### Task 2: Header-aware hit-test in layout.rs

**Files:**
- Modify: `src/layout.rs:210-226`

**Step 1: Write the failing test**

The current `hit_test` always returns the deepest node. We need a variant that stops at the header level when the click Y falls in the header region. Add to `src/layout.rs` tests:

```rust
#[test]
fn test_hit_test_header_returns_directory() {
    let root = layout(&sample_tree(), 800.0, 600.0);
    // The "big" directory child should have a header.
    // Find it and click in its header region (first 14px of its rect).
    let big = &root.children[0];
    let x = big.rect.x + big.rect.w / 2.0;
    let y = big.rect.y + 5.0; // within HEADER_HEIGHT (14px)

    let result = hit_test(&root, x, y);
    assert!(result.is_some());
    let path = result.unwrap();
    // Should stop at "big" directory, not drill into a child
    assert_eq!(path.last().unwrap(), "big", "header click should select directory, got: {path:?}");
}

#[test]
fn test_hit_test_body_returns_leaf() {
    let root = layout(&sample_tree(), 800.0, 600.0);
    // Click in the body area (below header) of the "big" directory
    // should still return a leaf node (a.c or b.c)
    let big = &root.children[0];
    let x = big.rect.x + big.rect.w / 2.0;
    let y = big.rect.y + big.rect.h / 2.0; // well below header

    let result = hit_test(&root, x, y);
    assert!(result.is_some());
    let path = result.unwrap();
    assert!(path.len() > 2, "body click should reach a leaf: {path:?}");
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test test_hit_test_header -v && cargo test test_hit_test_body -v`
Expected: `test_hit_test_header` FAILS (returns leaf instead of directory). `test_hit_test_body` should pass already.

**Step 3: Modify hit_test**

Replace the `hit_test` function in `src/layout.rs`:

```rust
/// Find the deepest node at the given point.
/// If the click lands in a non-leaf node's header region (top HEADER_HEIGHT pixels),
/// return that node instead of drilling into children.
pub fn hit_test(node: &LayoutNode, x: f64, y: f64) -> Option<Vec<String>> {
    if x < node.rect.x || x > node.rect.x + node.rect.w
        || y < node.rect.y || y > node.rect.y + node.rect.h
    {
        return None;
    }

    // Check if click is in this node's header region
    if !node.is_leaf && node.depth > 0 && node.rect.h >= MIN_HEADER_HEIGHT {
        let header_bottom = node.rect.y + HEADER_HEIGHT;
        if y <= header_bottom {
            return Some(vec![node.name.clone()]);
        }
    }

    for child in &node.children {
        if let Some(path) = hit_test(child, x, y) {
            let mut result = vec![node.name.clone()];
            result.extend(path);
            return Some(result);
        }
    }

    Some(vec![node.name.clone()])
}
```

**Step 4: Run all tests**

Run: `cargo test -v`
Expected: All 46+ tests pass (existing tests should still pass)

**Step 5: Commit**

```bash
git add src/layout.rs
git commit -m "feat(elfvis): header-aware hit-test for directory selection"
```

---

### Task 3: Clipboard formatting (pure logic, fully testable)

**Files:**
- Create: `src/clipboard_fmt.rs`

This is the most complex logic and is 100% pure — no browser deps. Build a module that takes a set of selected paths and produces formatted clipboard text.

**Step 1: Write the failing tests**

Create `src/clipboard_fmt.rs` with tests:

```rust
use std::collections::HashSet;

/// Format selected paths into clipboard text.
///
/// Paths are `["dir", "file.c", "symbol"]` style vectors.
/// Leaf paths (symbols) group under their parent file.
/// Non-leaf paths (directories/files) show with trailing `/` for dirs.
pub fn format_clipboard(selected: &HashSet<Vec<String>>) -> String {
    todo!()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set_of(paths: &[&[&str]]) -> HashSet<Vec<String>> {
        paths.iter().map(|p| p.iter().map(|s| s.to_string()).collect()).collect()
    }

    #[test]
    fn test_single_symbol() {
        let sel = set_of(&[&["src", "app/main.c", "func_a"]]);
        assert_eq!(format_clipboard(&sel), "src/app/main.c: func_a");
    }

    #[test]
    fn test_two_symbols_same_file() {
        let sel = set_of(&[
            &["src", "app/main.c", "func_a"],
            &["src", "app/main.c", "func_b"],
        ]);
        let result = format_clipboard(&sel);
        // Both symbols under same file, order may vary
        assert!(result.starts_with("src/app/main.c: "));
        assert!(result.contains("func_a"));
        assert!(result.contains("func_b"));
        assert!(result.contains(", "));
    }

    #[test]
    fn test_symbols_across_files_common_parent() {
        let sel = set_of(&[
            &["src", "app/main.c", "func_a"],
            &["src", "lib/util.c", "func_c"],
        ]);
        let result = format_clipboard(&sel);
        assert!(result.contains("src/"), "should have common parent: {result}");
        assert!(result.contains("app/main.c: func_a"), "should have file+sym: {result}");
        assert!(result.contains("lib/util.c: func_c"), "should have file+sym: {result}");
    }

    #[test]
    fn test_single_directory() {
        // A directory node has no leaf (symbol) component
        let sel = set_of(&[&["src", "app"]]);
        assert_eq!(format_clipboard(&sel), "src/app/");
    }

    #[test]
    fn test_two_directories_common_parent() {
        let sel = set_of(&[
            &["src", "app"],
            &["src", "lib"],
        ]);
        let result = format_clipboard(&sel);
        assert!(result.contains("src/"), "{result}");
        assert!(result.contains("  app/"), "{result}");
        assert!(result.contains("  lib/"), "{result}");
    }

    #[test]
    fn test_empty_selection() {
        let sel: HashSet<Vec<String>> = HashSet::new();
        assert_eq!(format_clipboard(&sel), "");
    }
}
```

**Step 2: Register the module and run tests to verify they fail**

Add `pub mod clipboard_fmt;` to `src/lib.rs` (after the existing module declarations).

Run: `cargo test clipboard_fmt -v`
Expected: FAIL — `todo!()` panics

**Step 3: Implement format_clipboard**

Replace the `todo!()` in `src/clipboard_fmt.rs`:

```rust
pub fn format_clipboard(selected: &HashSet<Vec<String>>) -> String {
    if selected.is_empty() {
        return String::new();
    }

    let mut paths: Vec<Vec<String>> = selected.iter().cloned().collect();
    paths.sort();

    // Determine if paths are leaf (symbol) selections or non-leaf (dir/file)
    // by checking if any sibling shares the same parent prefix.
    // We group by the parent path (all but last component).

    // Collect entries: (parent_path_components, leaf_name_or_none)
    // For a symbol: parent = ["src", "app/main.c"], leaf = "func_a"
    // For a directory: parent = ["src"], leaf = "app" (rendered with trailing /)
    // Heuristic: if the path has depth >= 3 in a typical tree, the last element
    // is likely a symbol. But we can't know for sure without the tree.
    // Instead: we'll treat every path as-is. The last component is the "item",
    // everything before it is the "container".

    // Group by container (all but last element)
    let mut groups: std::collections::BTreeMap<Vec<String>, Vec<String>> = std::collections::BTreeMap::new();
    for path in &paths {
        if path.len() <= 1 {
            // Top-level node
            groups.entry(vec![]).or_default().push(path.last().unwrap().clone());
        } else {
            let container = path[..path.len() - 1].to_vec();
            let item = path.last().unwrap().clone();
            groups.entry(container).or_default().push(item);
        }
    }

    // Find common prefix across all container paths
    let all_containers: Vec<&Vec<String>> = groups.keys().collect();
    let common = common_prefix(&all_containers);
    let common_len = common.len();

    // If there's only one group
    if groups.len() == 1 {
        let (container, items) = groups.into_iter().next().unwrap();
        let container_str = container.join("/");
        if items.len() == 1 {
            let item = &items[0];
            if container.is_empty() {
                // Top-level directory
                return format!("{item}/");
            }
            // Check if this looks like a symbol (container ends with a file-like name)
            // or a directory selection
            if looks_like_file(container.last().unwrap()) {
                return format!("{container_str}: {item}");
            } else {
                return format!("{container_str}/{item}/");
            }
        } else {
            // Multiple items in same container
            let items_str = items.join(", ");
            if !container.is_empty() && looks_like_file(container.last().unwrap()) {
                return format!("{container_str}: {items_str}");
            } else {
                let prefix = if container.is_empty() { String::new() } else { format!("{container_str}/\n") };
                let lines: Vec<String> = items.iter().map(|i| format!("  {i}/")).collect();
                return format!("{prefix}{}", lines.join("\n"));
            }
        }
    }

    // Multiple groups: show common prefix, then indented sub-paths
    let common_str = if common_len > 0 {
        common.join("/")
    } else {
        String::new()
    };

    let mut lines: Vec<String> = Vec::new();
    if !common_str.is_empty() {
        lines.push(format!("{common_str}/"));
    }

    for (container, items) in &groups {
        let suffix: Vec<String> = container[common_len..].to_vec();
        let suffix_str = suffix.join("/");

        if items.len() == 1 {
            let item = &items[0];
            if !suffix_str.is_empty() && looks_like_file(&suffix_str) {
                lines.push(format!("  {suffix_str}: {item}"));
            } else if !suffix_str.is_empty() {
                lines.push(format!("  {suffix_str}/{item}/"));
            } else if looks_like_file(container.last().unwrap_or(&String::new())) {
                lines.push(format!("  {item}"));
            } else {
                lines.push(format!("  {item}/"));
            }
        } else {
            let items_str = items.join(", ");
            if !suffix_str.is_empty() && looks_like_file(&suffix_str) {
                lines.push(format!("  {suffix_str}: {items_str}"));
            } else if !suffix_str.is_empty() {
                let item_lines: Vec<String> = items.iter().map(|i| format!("  {i}/")).collect();
                lines.push(format!("  {suffix_str}/"));
                lines.extend(item_lines.iter().map(|l| format!("  {l}")));
            } else {
                lines.push(format!("  {items_str}"));
            }
        }
    }

    lines.join("\n")
}

fn looks_like_file(name: &str) -> bool {
    // Check if name contains a dot suggesting a file extension
    if let Some(dot_pos) = name.rfind('.') {
        let ext = &name[dot_pos + 1..];
        ext.len() <= 4 && ext.chars().all(|c| c.is_alphanumeric())
    } else {
        false
    }
}

fn common_prefix(paths: &[&Vec<String>]) -> Vec<String> {
    if paths.is_empty() {
        return vec![];
    }
    if paths.len() == 1 {
        return paths[0].clone();
    }
    let mut prefix = paths[0].clone();
    for path in &paths[1..] {
        let mut i = 0;
        while i < prefix.len() && i < path.len() && prefix[i] == path[i] {
            i += 1;
        }
        prefix.truncate(i);
    }
    prefix
}
```

**Step 4: Run tests**

Run: `cargo test clipboard_fmt -v`
Expected: All 6 tests pass

**Step 5: Commit**

```bash
git add src/clipboard_fmt.rs src/lib.rs
git commit -m "feat(elfvis): clipboard text formatting for selected nodes"
```

---

### Task 4: Selection-aware rendering in render.rs

**Files:**
- Modify: `src/render.rs:1-89`

This is browser-dependent (Canvas API), so no unit tests — we verify visually and via the existing test suite staying green.

**Step 1: Update render signature to accept selection**

Change `render` and `render_node` to take a selection set:

```rust
use std::collections::HashSet;
use crate::color::{pastel_color, selected_color};

pub fn render(ctx: &CanvasRenderingContext2d, root: &LayoutNode, selected: &HashSet<Vec<String>>) {
    ctx.set_fill_style_str("#ffffff");
    ctx.fill_rect(root.rect.x, root.rect.y, root.rect.w, root.rect.h);
    render_node(ctx, root, selected, &vec![]);
}
```

Update `render_node` to track the current path and check selection:

```rust
fn render_node(ctx: &CanvasRenderingContext2d, node: &LayoutNode, selected: &HashSet<Vec<String>>, parent_path: &Vec<String>) {
    if node.rect.w < 1.0 || node.rect.h < 1.0 {
        return;
    }

    let mut current_path = parent_path.clone();
    if node.depth > 0 {
        current_path.push(node.name.clone());
    }
    let is_selected = selected.contains(&current_path);

    if node.is_leaf {
        let c = if is_selected {
            selected_color(node.hue, node.depth)
        } else {
            pastel_color(node.hue, node.depth)
        };
        ctx.set_fill_style_str(&c.to_css());
        ctx.fill_rect(node.rect.x, node.rect.y, node.rect.w, node.rect.h);

        ctx.set_stroke_style_str("rgba(0,0,0,1)");
        ctx.set_line_width(0.5);
        ctx.stroke_rect(node.rect.x, node.rect.y, node.rect.w, node.rect.h);

        render_label(ctx, node);
    } else {
        let show_header = node.rect.h >= MIN_HEADER_HEIGHT && node.depth > 0;
        if show_header {
            let c = pastel_color(node.hue, node.depth);
            let header_color = if is_selected {
                selected_color(node.hue, node.depth)
            } else {
                darken(&c, 0.15)
            };
            ctx.set_fill_style_str(&header_color.to_css());
            ctx.fill_rect(node.rect.x, node.rect.y, node.rect.w, HEADER_HEIGHT);

            // ... existing label rendering code unchanged ...
        }

        for child in &node.children {
            render_node(ctx, child, selected, &current_path);
        }

        if node.depth > 0 {
            ctx.set_stroke_style_str("rgba(0,0,0,1)");
            ctx.set_line_width(1.0);
            ctx.stroke_rect(node.rect.x, node.rect.y, node.rect.w, node.rect.h);
        }
    }
}
```

**Step 2: Run tests to verify nothing breaks**

Run: `cargo test -v`
Expected: All existing tests pass (render.rs has no unit tests, but compilation must succeed)

**Step 3: Commit**

```bash
git add src/render.rs
git commit -m "feat(elfvis): highlight selected nodes in treemap rendering"
```

---

### Task 5: Add web-sys Clipboard/Navigator features to Cargo.toml

**Files:**
- Modify: `Cargo.toml:19-39`

**Step 1: Add features**

Add these to the `web-sys` features list in `Cargo.toml`:

```
"Clipboard",
"Navigator",
```

**Step 2: Verify it compiles**

Run: `cargo check`
Expected: Compiles without errors

**Step 3: Commit**

```bash
git add Cargo.toml
git commit -m "chore(elfvis): add web-sys Clipboard and Navigator features"
```

---

### Task 6: Add notification span to HTML

**Files:**
- Modify: `www/index.html`

**Step 1: Add the notification span and CSS**

In the `#header` div, before the `<span class="spacer">`, add:

```html
<span id="clipboard-msg" class="clipboard-msg"></span>
```

Add CSS for the notification:

```css
#header .clipboard-msg {
  color: var(--accent);
  font-size: 12px;
  opacity: 0;
  transition: opacity 0.3s;
  margin-right: 8px;
}
#header .clipboard-msg.show {
  opacity: 1;
}
```

**Step 2: Verify page loads**

Open `www/index.html` in browser, confirm no visual regressions.

**Step 3: Commit**

```bash
git add www/index.html
git commit -m "feat(elfvis): add clipboard notification span to header"
```

---

### Task 7: Wire up click handler, selection state, and clipboard in lib.rs

**Files:**
- Modify: `src/lib.rs`

This is the integration task that ties everything together.

**Step 1: Add selection state to AppState**

```rust
use std::collections::HashSet;

struct AppState {
    layout_root: Option<layout::LayoutNode>,
    filename: String,
    total_size: u64,
    canvas_width: f64,
    canvas_height: f64,
    dpr: f64,
    selected: HashSet<Vec<String>>,
}
```

Initialize `selected: HashSet::new()` in the `STATE` thread_local.

**Step 2: Update all render calls to pass selection**

In `process_elf`, change:
```rust
render::render(&ctx, root);
```
to:
```rust
render::render(&ctx, root, &state.selected);
```

In the mousemove handler, same change.

**Step 3: Clear selection on reset**

In the reset click handler, add `state.selected.clear();` alongside `state.layout_root = None;`.

**Step 4: Add click handler**

In `setup_canvas_events`, add a `click` event listener alongside the existing `mousemove`:

```rust
// Click handler for selection
let cb = Closure::wrap(Box::new(move |e: MouseEvent| {
    let x = e.offset_x() as f64;
    let y = e.offset_y() as f64;
    let shift = e.shift_key();

    STATE.with(|s| {
        let mut state = s.borrow_mut();
        if let Some(ref root) = state.layout_root {
            if let Some(path) = layout::hit_test(root, x, y) {
                // Remove the root name from the path (it's empty string)
                let path: Vec<String> = path.into_iter().skip(1).collect();
                if path.is_empty() {
                    return;
                }

                if shift {
                    // Toggle in/out of selection
                    if state.selected.contains(&path) {
                        state.selected.remove(&path);
                    } else {
                        state.selected.insert(path);
                    }
                } else {
                    // Replace selection, or deselect if clicking sole item
                    if state.selected.len() == 1 && state.selected.contains(&path) {
                        state.selected.clear();
                    } else {
                        state.selected.clear();
                        state.selected.insert(path);
                    }
                }

                // Re-render with updated selection
                let doc = window().unwrap().document().unwrap();
                let canvas: HtmlCanvasElement = doc.get_element_by_id("canvas").unwrap().unchecked_into();
                let ctx = canvas.get_context("2d").unwrap().unwrap()
                    .unchecked_into::<CanvasRenderingContext2d>();
                ctx.set_transform(state.dpr, 0.0, 0.0, state.dpr, 0.0, 0.0).ok();
                render::render(&ctx, root, &state.selected);

                // Write to clipboard
                if !state.selected.is_empty() {
                    let text = clipboard_fmt::format_clipboard(&state.selected);
                    let win = window().unwrap();
                    let nav = win.navigator();
                    let clipboard = nav.clipboard();
                    let _ = clipboard.write_text(&text);
                    show_clipboard_notification(&doc);
                }
            }
        }
    });
}) as Box<dyn FnMut(_)>);
canvas.add_event_listener_with_callback("click", cb.as_ref().unchecked_ref())?;
cb.forget();
```

**Step 5: Add notification helper**

```rust
fn show_clipboard_notification(document: &Document) {
    if let Some(el) = document.get_element_by_id("clipboard-msg") {
        let el: HtmlElement = el.unchecked_into();
        el.set_text_content(Some("Copied to clipboard"));
        el.class_list().add_1("show").ok();

        // Remove after 1.5s
        let cb = Closure::once(Box::new(move || {
            let doc = window().unwrap().document().unwrap();
            if let Some(msg) = doc.get_element_by_id("clipboard-msg") {
                msg.unchecked_ref::<HtmlElement>().class_list().remove_1("show").ok();
            }
        }) as Box<dyn FnOnce()>);
        window().unwrap().set_timeout_with_callback_and_timeout_and_arguments_0(
            cb.as_ref().unchecked_ref(), 1500
        ).ok();
        cb.forget();
    }
}
```

**Step 6: Run tests and verify compilation**

Run: `cargo test -v`
Expected: All tests pass

Run: `cargo check --target wasm32-unknown-unknown`
Expected: Compiles (verifies web-sys bindings are correct)

**Step 7: Commit**

```bash
git add src/lib.rs
git commit -m "feat(elfvis): wire up click-to-select with clipboard export"
```

---

### Task 8: Build WASM and manual smoke test

**Step 1: Build**

Run: `wasm-pack build --target web --out-dir www/pkg`

**Step 2: Serve and test**

Run: `python3 -m http.server 8080 -d www`

Test manually:
1. Drop an ELF file
2. Click a symbol → highlights, clipboard shows `path/file.c: symbol`
3. Shift+click another symbol → both highlighted, clipboard updated
4. Click the highlighted symbol again (sole selection) → deselects
5. Click a directory header → highlights header, clipboard shows `path/dir/`
6. Verify "Copied to clipboard" notification appears and fades

**Step 3: Final commit if any fixes needed**

```bash
git add -A
git commit -m "fix(elfvis): smoke test fixes for click-to-select"
```
