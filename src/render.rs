use web_sys::CanvasRenderingContext2d;

use crate::color::pastel_color;
use crate::layout::{LayoutNode, HEADER_HEIGHT, MIN_HEADER_HEIGHT};

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

            let pad = 4.0;
            let max_w = node.rect.w - pad * 2.0;
            let y_mid = node.rect.y + HEADER_HEIGHT / 2.0;
            let mono = "\"SF Mono\", \"Cascadia Code\", \"Fira Code\", Consolas, Menlo, monospace";
            let font_full = format!("bold 11px {mono}");
            let font_small = format!("bold 9px {mono}");
            let font_ellipsis = format!("bold 6px {mono}");

            ctx.set_fill_style_str("#333333");
            ctx.set_text_baseline("middle");

            // Try 11px first
            ctx.set_font(&font_full);
            let fits_full = ctx.measure_text(&node.name).map(|m| m.width() <= max_w).unwrap_or(false);

            if fits_full {
                ctx.fill_text(&node.name, node.rect.x + pad, y_mid).ok();
            } else {
                let name = strip_extension(&node.name);
                // Try 9px full
                ctx.set_font(&font_small);
                let fits_small = ctx.measure_text(&name).map(|m| m.width() <= max_w).unwrap_or(false);

                if fits_small {
                    ctx.fill_text(&name, node.rect.x + pad, y_mid).ok();
                } else {
                    // Ellipsis + tail at 9px
                    let ellipsis = "\u{2026}";
                    ctx.set_font(&font_ellipsis);
                    let ellipsis_w = ctx.measure_text(ellipsis).map(|m| m.width()).unwrap_or(4.0);
                    ctx.fill_text(ellipsis, node.rect.x + pad, y_mid).ok();

                    let tail_budget = max_w - ellipsis_w;
                    if tail_budget > 0.0 {
                        ctx.set_font(&font_small);
                        let tail = fit_tail(ctx, &name, tail_budget);
                        if !tail.is_empty() {
                            ctx.fill_text(&tail, node.rect.x + pad + ellipsis_w, y_mid).ok();
                        }
                    }
                }
            }
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

    let mono = "\"SF Mono\", \"Cascadia Code\", \"Fira Code\", Consolas, Menlo, monospace";
    let font_main = format!("7px {mono}");
    let font_ellipsis = format!("5px {mono}");

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
