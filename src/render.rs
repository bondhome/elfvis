use std::collections::{HashMap, HashSet};

use web_sys::CanvasRenderingContext2d;

use crate::color::{delta_color, pastel_color};
use crate::diff::Delta;
use crate::layout::{LayoutNode, HEADER_HEIGHT, MIN_HEADER_HEIGHT};
use crate::tree::SymbolKey;

const MONO_FONT_STACK: &str =
    "\"SF Mono\", \"Cascadia Code\", \"Fira Code\", Consolas, Menlo, monospace";

pub fn render(ctx: &CanvasRenderingContext2d, root: &LayoutNode) {
    ctx.set_fill_style_str("#ffffff");
    ctx.fill_rect(root.rect.x, root.rect.y, root.rect.w, root.rect.h);
    render_node(ctx, root);
}

fn render_node(ctx: &CanvasRenderingContext2d, node: &LayoutNode) {
    if node.rect.w < 1.0 || node.rect.h < 1.0 {
        return;
    }

    if node.is_leaf {
        let c = pastel_color(node.hue, node.depth);
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
            let header_color = darken(&c, 0.15);
            ctx.set_fill_style_str(&header_color.to_css());
            ctx.fill_rect(node.rect.x, node.rect.y, node.rect.w, HEADER_HEIGHT);

            draw_header_label(ctx, &node.name, node.rect.x, node.rect.w, node.rect.y);
        }

        for child in &node.children {
            render_node(ctx, child);
        }

        if node.depth > 0 {
            ctx.set_stroke_style_str("rgba(0,0,0,1)");
            ctx.set_line_width(1.0);
            ctx.stroke_rect(node.rect.x, node.rect.y, node.rect.w, node.rect.h);
        }
    }
}

fn render_label(ctx: &CanvasRenderingContext2d, node: &LayoutNode) {
    if node.rect.w < 30.0 || node.rect.h < 14.0 {
        return;
    }

    let pad = 3.0;
    let max_w = node.rect.w - pad * 2.0;
    let y_mid = node.rect.y + node.rect.h / 2.0;

    let font_main = format!("7px {MONO_FONT_STACK}");
    let font_ellipsis = format!("5px {MONO_FONT_STACK}");

    ctx.set_fill_style_str("#333333");
    ctx.set_font(&font_main);
    ctx.set_text_baseline("middle");

    if let Ok(m) = ctx.measure_text(&node.name) {
        if m.width() <= max_w {
            ctx.fill_text(&node.name, node.rect.x + pad, y_mid).ok();
        } else {
            let name = strip_extension(&node.name);
            let ellipsis = "\u{2026}";
            ctx.set_font(&font_ellipsis);
            let ellipsis_w = ctx.measure_text(ellipsis).map(|m| m.width()).unwrap_or(3.0);
            ctx.fill_text(ellipsis, node.rect.x + pad, y_mid).ok();

            let tail_budget = max_w - ellipsis_w;
            if tail_budget > 0.0 {
                ctx.set_font(&font_main);
                let tail = fit_tail(ctx, &name, tail_budget);
                if !tail.is_empty() {
                    ctx.fill_text(&tail, node.rect.x + pad + ellipsis_w, y_mid).ok();
                }
            }
        }
    }
}

pub fn render_tooltip(ctx: &CanvasRenderingContext2d, x: f64, y: f64, text: &str, canvas_w: f64, canvas_h: f64) {
    let lines: Vec<&str> = text.lines().collect();
    let line_height = 16.0;
    let padding = 8.0;

    // Set font before measuring
    let font = "12px \"SF Mono\", \"Cascadia Code\", \"Fira Code\", Consolas, Menlo, monospace";
    ctx.set_font(font);

    // Measure widest line to size tooltip dynamically
    let max_line_w = lines.iter()
        .filter_map(|line| ctx.measure_text(line).ok().map(|m| m.width()))
        .fold(0.0_f64, f64::max);
    let tooltip_w = max_line_w + padding * 2.0;
    let tooltip_h = lines.len() as f64 * line_height + padding * 2.0;

    let mut tx = x + 12.0;
    let mut ty = y + 12.0;
    if tx + tooltip_w > canvas_w { tx = x - tooltip_w - 12.0; }
    if ty + tooltip_h > canvas_h { ty = y - tooltip_h - 12.0; }
    tx = tx.max(0.0);
    ty = ty.max(0.0);

    ctx.set_fill_style_str("rgba(0,0,0,0.85)");
    ctx.begin_path();
    round_rect(ctx, tx, ty, tooltip_w, tooltip_h, 4.0);
    ctx.fill();

    ctx.set_fill_style_str("#ffffff");
    ctx.set_text_baseline("top");
    for (i, line) in lines.iter().enumerate() {
        ctx.fill_text(line, tx + padding, ty + padding + i as f64 * line_height).ok();
    }
}

/// Render a treemap with delta-based coloring for comparison mode.
pub fn render_diff(
    ctx: &CanvasRenderingContext2d,
    root: &LayoutNode,
    deltas: &HashMap<SymbolKey, Delta>,
) {
    ctx.set_fill_style_str("#ffffff");
    ctx.fill_rect(root.rect.x, root.rect.y, root.rect.w, root.rect.h);
    render_diff_node(ctx, root, deltas);
}

fn render_diff_node(ctx: &CanvasRenderingContext2d, node: &LayoutNode, deltas: &HashMap<SymbolKey, Delta>) {
    if node.rect.w < 1.0 || node.rect.h < 1.0 {
        return;
    }

    if node.is_leaf {
        // Look up by stable symbol key (source + name), not bare name — two
        // translation units may legitimately define a same-named local symbol.
        let color = if let Some(delta) = node.key.as_ref().and_then(|k| deltas.get(k)) {
            delta_color(delta.diff_pct())
        } else {
            delta_color(0.0)
        };
        ctx.set_fill_style_str(&color.to_css());
        ctx.fill_rect(node.rect.x, node.rect.y, node.rect.w, node.rect.h);

        ctx.set_stroke_style_str("rgba(0,0,0,1)");
        ctx.set_line_width(0.5);
        ctx.stroke_rect(node.rect.x, node.rect.y, node.rect.w, node.rect.h);

        render_label(ctx, node);
    } else {
        let show_header = node.rect.h >= MIN_HEADER_HEIGHT && node.depth > 0;
        if show_header {
            ctx.set_fill_style_str("rgb(220,220,220)");
            ctx.fill_rect(node.rect.x, node.rect.y, node.rect.w, HEADER_HEIGHT);

            draw_header_label(ctx, &node.name, node.rect.x, node.rect.w, node.rect.y);
        }

        for child in &node.children {
            render_diff_node(ctx, child, deltas);
        }

        if node.depth > 0 {
            ctx.set_stroke_style_str("rgba(0,0,0,1)");
            ctx.set_line_width(1.0);
            ctx.stroke_rect(node.rect.x, node.rect.y, node.rect.w, node.rect.h);
        }
    }
}

/// Highlight every leaf in `root` whose stable symbol key is in `keys`.
///
/// Matches by symbol identity rather than display path: the two comparison
/// trees are built and clustered independently, so the same symbol can end
/// up at a different collapsed directory path in each one (unknown-symbol
/// clustering, a moved source file, an added/removed sibling that changes
/// whether a chain collapses). A path-based walk fails silently whenever
/// that happens; a key lookup does not, and works the same way whether one
/// leaf is being cross-highlighted or a whole hovered directory's worth.
pub fn render_highlight(ctx: &CanvasRenderingContext2d, root: &LayoutNode, keys: &HashSet<SymbolKey>) {
    for leaf in matching_leaves(root, keys) {
        ctx.set_stroke_style_str("rgba(59, 130, 246, 0.9)");
        ctx.set_line_width(2.5);
        ctx.stroke_rect(leaf.rect.x, leaf.rect.y, leaf.rect.w, leaf.rect.h);
    }
}

/// Every leaf under `node` whose stable symbol key is in `keys` — the
/// selection half of cross-highlighting, kept canvas-free so it can be unit
/// tested. Matches by identity, not display path: the two comparison trees
/// are built and clustered independently, so the same symbol can end up at a
/// different collapsed directory path in each one (unknown-symbol
/// clustering, a moved source file, an added/removed sibling that changes
/// whether a chain collapses). A path-based walk fails silently whenever
/// that happens; a key lookup does not, and works the same way whether one
/// leaf is being cross-highlighted or a whole hovered directory's worth.
fn matching_leaves<'a>(node: &'a LayoutNode, keys: &HashSet<SymbolKey>) -> Vec<&'a LayoutNode> {
    if keys.is_empty() {
        return Vec::new();
    }
    let mut matches = Vec::new();
    collect_matching_leaves(node, keys, &mut matches);
    matches
}

fn collect_matching_leaves<'a>(node: &'a LayoutNode, keys: &HashSet<SymbolKey>, matches: &mut Vec<&'a LayoutNode>) {
    if node.is_leaf {
        if node.key.as_ref().is_some_and(|k| keys.contains(k)) {
            matches.push(node);
        }
        return;
    }
    for child in &node.children {
        collect_matching_leaves(child, keys, matches);
    }
}

fn round_rect(ctx: &CanvasRenderingContext2d, x: f64, y: f64, w: f64, h: f64, r: f64) {
    ctx.move_to(x + r, y);
    ctx.line_to(x + w - r, y);
    ctx.arc_to(x + w, y, x + w, y + r, r).ok();
    ctx.line_to(x + w, y + h - r);
    ctx.arc_to(x + w, y + h, x + w - r, y + h, r).ok();
    ctx.line_to(x + r, y + h);
    ctx.arc_to(x, y + h, x, y + h - r, r).ok();
    ctx.line_to(x, y + r);
    ctx.arc_to(x, y, x + r, y, r).ok();
    ctx.close_path();
}

/// Draw a directory/file header label within `[x, x+w]`, falling back through a full
/// name, an extension-stripped name at a smaller size, then an ellipsis + best-fit
/// tail. Shared by the single-file view and comparison-mode panels so a header never
/// silently renders blank just because its panel is narrower, and so the header text
/// matches the node's real name (as shown in tooltips) whenever it fits.
fn draw_header_label(ctx: &CanvasRenderingContext2d, name: &str, x: f64, w: f64, y: f64) {
    let pad = 4.0;
    let max_w = w - pad * 2.0;
    let y_mid = y + HEADER_HEIGHT / 2.0;
    let font_full = format!("bold 11px {MONO_FONT_STACK}");
    let font_small = format!("bold 9px {MONO_FONT_STACK}");
    let font_ellipsis = format!("bold 6px {MONO_FONT_STACK}");

    ctx.set_fill_style_str("#333333");
    ctx.set_text_baseline("middle");

    ctx.set_font(&font_full);
    let fits_full = ctx.measure_text(name).map(|m| m.width() <= max_w).unwrap_or(false);
    if fits_full {
        ctx.fill_text(name, x + pad, y_mid).ok();
        return;
    }

    let stripped = strip_extension(name);
    ctx.set_font(&font_small);
    let fits_small = ctx.measure_text(&stripped).map(|m| m.width() <= max_w).unwrap_or(false);
    if fits_small {
        ctx.fill_text(&stripped, x + pad, y_mid).ok();
        return;
    }

    let ellipsis = "\u{2026}";
    ctx.set_font(&font_ellipsis);
    let ellipsis_w = ctx.measure_text(ellipsis).map(|m| m.width()).unwrap_or(4.0);
    ctx.fill_text(ellipsis, x + pad, y_mid).ok();

    let tail_budget = max_w - ellipsis_w;
    if tail_budget > 0.0 {
        ctx.set_font(&font_small);
        let tail = fit_tail(ctx, &stripped, tail_budget);
        if !tail.is_empty() {
            ctx.fill_text(&tail, x + pad + ellipsis_w, y_mid).ok();
        }
    }
}

/// Strip file extension (e.g. ".c", ".h", ".rs") if present.
fn strip_extension(name: &str) -> String {
    if let Some(pos) = name.rfind('.') {
        if pos > 0 && pos < name.len() - 1 {
            return name[..pos].to_string();
        }
    }
    name.to_string()
}

/// Return the longest tail (suffix) of text that fits within max_w pixels.
fn fit_tail(ctx: &CanvasRenderingContext2d, text: &str, max_w: f64) -> String {
    let chars: Vec<char> = text.chars().collect();
    let len = chars.len();
    // Binary search: find smallest start index where suffix fits
    let mut lo = 0usize;
    let mut hi = len;
    while lo < hi {
        let mid = (lo + hi) / 2;
        let suffix: String = chars[mid..].iter().collect();
        if let Ok(m) = ctx.measure_text(&suffix) {
            if m.width() <= max_w {
                hi = mid;
            } else {
                lo = mid + 1;
            }
        } else {
            lo = mid + 1;
        }
    }
    if lo >= len {
        return String::new();
    }
    chars[lo..].iter().collect()
}

/// Truncate text with ellipsis to fit within max_w pixels.
fn truncate_to_fit(ctx: &CanvasRenderingContext2d, text: &str, max_w: f64) -> String {
    if max_w <= 0.0 {
        return String::new();
    }
    if let Ok(m) = ctx.measure_text(text) {
        if m.width() <= max_w {
            return text.to_string();
        }
    }
    // Binary search for the longest prefix that fits with ellipsis
    let chars: Vec<char> = text.chars().collect();
    let mut lo = 0usize;
    let mut hi = chars.len();
    while lo < hi {
        let mid = (lo + hi + 1) / 2;
        let candidate: String = chars[..mid].iter().collect::<String>() + "\u{2026}";
        if let Ok(m) = ctx.measure_text(&candidate) {
            if m.width() <= max_w {
                lo = mid;
            } else {
                hi = mid - 1;
            }
        } else {
            hi = mid - 1;
        }
    }
    if lo == 0 {
        return String::new();
    }
    chars[..lo].iter().collect::<String>() + "\u{2026}"
}

fn darken(c: &crate::color::Color, amount: f64) -> crate::color::Color {
    crate::color::Color {
        r: (c.r as f64 * (1.0 - amount)) as u8,
        g: (c.g as f64 * (1.0 - amount)) as u8,
        b: (c.b as f64 * (1.0 - amount)) as u8,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::Rect;

    fn leaf(x: f64, name: &str, key: Option<SymbolKey>) -> LayoutNode {
        LayoutNode {
            rect: Rect { x, y: 0.0, w: 10.0, h: 10.0 },
            name: name.to_string(),
            size: 10,
            depth: 1,
            is_leaf: true,
            hue: 0.0,
            children: Vec::new(),
            key,
        }
    }

    fn dir(x: f64, name: &str, children: Vec<LayoutNode>) -> LayoutNode {
        LayoutNode {
            rect: Rect { x, y: 0.0, w: 20.0, h: 10.0 },
            name: name.to_string(),
            size: 10,
            depth: 0,
            is_leaf: false,
            hue: 0.0,
            children,
            key: None,
        }
    }

    fn key(source: &str, name: &str) -> SymbolKey {
        SymbolKey { source: source.to_string(), name: name.to_string() }
    }

    #[test]
    fn test_matching_leaves_finds_leaf_despite_different_display_path() {
        // Regression for the code-review finding: the two comparison trees
        // collapse directories independently, so the same symbol can sit at a
        // different display path in each ("src/a.c::keep" collapses to a
        // single "a.c" node when it's the only file, but doesn't once a
        // sibling "b.c" exists). A path-based walk would miss this; matching
        // by SymbolKey must not.
        let keep_key = key("src/a.c", "keep");

        // Tree where "src" collapsed with its only child "a.c".
        let collapsed = dir(0.0, "a.c", vec![leaf(0.0, "keep", Some(keep_key.clone()))]);

        // Tree where "src" has two children, so it did NOT collapse.
        let uncollapsed = dir(
            0.0,
            "src",
            vec![
                dir(0.0, "a.c", vec![leaf(0.0, "keep", Some(keep_key.clone()))]),
                dir(20.0, "b.c", vec![leaf(20.0, "extra", Some(key("src/b.c", "extra")))]),
            ],
        );

        let target: HashSet<SymbolKey> = [keep_key].into_iter().collect();

        assert_eq!(matching_leaves(&collapsed, &target).len(), 1);
        let found = matching_leaves(&uncollapsed, &target);
        assert_eq!(found.len(), 1, "key-based match must still find the leaf under the deeper path");
        assert_eq!(found[0].name, "keep");
    }

    #[test]
    fn test_matching_leaves_highlights_every_leaf_in_a_group() {
        // A directory-level hover highlights every leaf in the group, not
        // just one — the same primitive used for a single-leaf hover.
        let a = key("src/dir", "a");
        let b = key("src/dir", "b");
        let other = key("src/dir", "c");

        let tree = dir(
            0.0,
            "dir",
            vec![
                leaf(0.0, "a", Some(a.clone())),
                leaf(10.0, "b", Some(b.clone())),
                leaf(20.0, "c", Some(other)),
            ],
        );

        let target: HashSet<SymbolKey> = [a, b].into_iter().collect();
        let found = matching_leaves(&tree, &target);
        assert_eq!(found.len(), 2);
    }

    #[test]
    fn test_matching_leaves_empty_keys_matches_nothing() {
        let tree = leaf(0.0, "solo", Some(key("src/a.c", "solo")));
        assert!(matching_leaves(&tree, &HashSet::new()).is_empty());
    }
}
