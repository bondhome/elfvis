use web_sys::CanvasRenderingContext2d;

use crate::color::pastel_color;
use crate::layout::{LayoutNode, HEADER_HEIGHT, MIN_HEADER_HEIGHT};

pub fn render(ctx: &CanvasRenderingContext2d, root: &LayoutNode, num_groups: usize) {
    ctx.set_fill_style_str("#ffffff");
    ctx.fill_rect(root.rect.x, root.rect.y, root.rect.w, root.rect.h);
    render_node(ctx, root, num_groups);
}

fn render_node(ctx: &CanvasRenderingContext2d, node: &LayoutNode, num_groups: usize) {
    if node.rect.w < 1.0 || node.rect.h < 1.0 {
        return;
    }

    if node.is_leaf {
        let c = pastel_color(node.color_group, num_groups, node.depth);
        ctx.set_fill_style_str(&c.to_css());
        ctx.fill_rect(node.rect.x, node.rect.y, node.rect.w, node.rect.h);

        ctx.set_stroke_style_str("rgba(0,0,0,0.15)");
        ctx.set_line_width(0.5);
        ctx.stroke_rect(node.rect.x, node.rect.y, node.rect.w, node.rect.h);

        render_label(ctx, node);
    } else {
        let show_header = node.rect.h >= MIN_HEADER_HEIGHT && node.depth > 0;
        if show_header {
            let c = pastel_color(node.color_group, num_groups, node.depth);
            let header_color = darken(&c, 0.15);
            ctx.set_fill_style_str(&header_color.to_css());
            ctx.fill_rect(node.rect.x, node.rect.y, node.rect.w, HEADER_HEIGHT);

            ctx.set_fill_style_str("#333333");
            ctx.set_font("bold 11px -apple-system, sans-serif");
            ctx.set_text_baseline("middle");
            ctx.fill_text_with_max_width(
                &node.name,
                node.rect.x + 4.0,
                node.rect.y + HEADER_HEIGHT / 2.0,
                node.rect.w - 8.0,
            ).ok();
        }

        for child in &node.children {
            render_node(ctx, child, num_groups);
        }

        if node.depth > 0 {
            ctx.set_stroke_style_str("rgba(0,0,0,0.25)");
            ctx.set_line_width(1.0);
            ctx.stroke_rect(node.rect.x, node.rect.y, node.rect.w, node.rect.h);
        }
    }
}

fn render_label(ctx: &CanvasRenderingContext2d, node: &LayoutNode) {
    if node.rect.w < 30.0 || node.rect.h < 14.0 {
        return;
    }

    ctx.set_fill_style_str("#333333");
    ctx.set_font("11px -apple-system, sans-serif");
    ctx.set_text_baseline("middle");

    ctx.fill_text_with_max_width(
        &node.name,
        node.rect.x + 3.0,
        node.rect.y + node.rect.h / 2.0,
        node.rect.w - 6.0,
    ).ok();
}

pub fn render_tooltip(ctx: &CanvasRenderingContext2d, x: f64, y: f64, text: &str, canvas_w: f64, canvas_h: f64) {
    let lines: Vec<&str> = text.lines().collect();
    let line_height = 16.0;
    let padding = 8.0;
    let tooltip_w = 280.0;
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
    ctx.set_font("12px monospace");
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

fn darken(c: &crate::color::Color, amount: f64) -> crate::color::Color {
    crate::color::Color {
        r: (c.r as f64 * (1.0 - amount)) as u8,
        g: (c.g as f64 * (1.0 - amount)) as u8,
        b: (c.b as f64 * (1.0 - amount)) as u8,
    }
}
