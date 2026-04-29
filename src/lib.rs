pub mod parse;
pub mod tree;
pub mod layout;
pub mod color;
pub mod diff;
mod render;

use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::{
    window, CanvasRenderingContext2d, Document, DragEvent, Event, FileReader,
    HtmlCanvasElement, HtmlElement, HtmlInputElement, MouseEvent,
};
use std::cell::RefCell;

struct AppState {
    layout_root: Option<layout::LayoutNode>,
    filename: String,
    total_size: u64,
    canvas_width: f64,
    canvas_height: f64,
    dpr: f64,
    // Comparison mode
    size_tree: Option<tree::SizeNode>,
    symbol_sizes: Option<std::collections::HashMap<String, u64>>,
    compare_layout: Option<layout::LayoutNode>,
    compare_filename: String,
    compare_total_size: u64,
    diff_map: Option<std::collections::HashMap<String, diff::Delta>>,
}

thread_local! {
    static STATE: RefCell<AppState> = RefCell::new(AppState {
        layout_root: None,
        filename: String::new(),
        total_size: 0,
        canvas_width: 0.0,
        canvas_height: 0.0,
        dpr: 1.0,
        size_tree: None,
        symbol_sizes: None,
        compare_layout: None,
        compare_filename: String::new(),
        compare_total_size: 0,
        diff_map: None,
    });
}

#[wasm_bindgen(start)]
pub fn main() -> Result<(), JsValue> {
    let win = window().unwrap();
    let document = win.document().unwrap();

    // Populate build info on landing page
    if let Some(el) = document.get_element_by_id("build-info") {
        let version = env!("CARGO_PKG_VERSION");
        let build_time = env!("ELFVIS_BUILD_TIME");
        el.set_inner_html(&format!(
            "v{version} &middot; built {build_time}"
        ));
    }

    setup_drop_zone(&document)?;
    setup_file_input(&document)?;
    setup_canvas_events(&document)?;

    // Reset link
    let reset = document.get_element_by_id("reset").unwrap();
    let cb = Closure::wrap(Box::new(move |e: Event| {
        e.prevent_default();
        let doc = window().unwrap().document().unwrap();

        STATE.with(|s| {
            let mut state = s.borrow_mut();
            state.layout_root = None;
            state.size_tree = None;
            state.symbol_sizes = None;
            state.compare_layout = None;
            state.compare_filename = String::new();
            state.compare_total_size = 0;
            state.diff_map = None;
        });

        doc.get_element_by_id("header").unwrap()
            .unchecked_ref::<HtmlElement>().style().set_property("display", "none").ok();
        doc.get_element_by_id("error").unwrap()
            .unchecked_ref::<HtmlElement>().style().set_property("display", "none").ok();
        doc.get_element_by_id("drop-zone").unwrap()
            .unchecked_ref::<HtmlElement>().class_list().remove_1("hidden").ok();
        // Show corner GitHub link again on splash
        if let Some(corner) = doc.get_element_by_id("corner-gh") {
            corner.unchecked_ref::<HtmlElement>().class_list().remove_1("hidden").ok();
        }

        // Hide comparison UI
        doc.get_element_by_id("canvas-b").unwrap()
            .unchecked_ref::<HtmlElement>().style().set_property("display", "none").ok();
        doc.get_element_by_id("compare-divider").unwrap()
            .unchecked_ref::<HtmlElement>().style().set_property("display", "none").ok();
        doc.get_element_by_id("canvas-container").unwrap()
            .unchecked_ref::<HtmlElement>().class_list().remove_1("compare-mode").ok();

        let canvas: HtmlCanvasElement = doc.get_element_by_id("canvas").unwrap().unchecked_into();
        let ctx = canvas.get_context("2d").unwrap().unwrap()
            .unchecked_into::<CanvasRenderingContext2d>();
        ctx.clear_rect(0.0, 0.0, canvas.width() as f64, canvas.height() as f64);

        let canvas_b: HtmlCanvasElement = doc.get_element_by_id("canvas-b").unwrap().unchecked_into();
        let ctx_b = canvas_b.get_context("2d").unwrap().unwrap()
            .unchecked_into::<CanvasRenderingContext2d>();
        ctx_b.clear_rect(0.0, 0.0, canvas_b.width() as f64, canvas_b.height() as f64);
    }) as Box<dyn FnMut(_)>);
    reset.add_event_listener_with_callback("click", cb.as_ref().unchecked_ref())?;
    cb.forget();

    // Compare button
    let compare_btn = document.get_element_by_id("compare-btn").unwrap();
    let cb = Closure::wrap(Box::new(move |_: Event| {
        let doc = window().unwrap().document().unwrap();
        let input: HtmlInputElement = doc.get_element_by_id("file-input-b").unwrap().unchecked_into();
        input.click();
    }) as Box<dyn FnMut(_)>);
    compare_btn.add_event_listener_with_callback("click", cb.as_ref().unchecked_ref())?;
    cb.forget();

    // Compare file input
    let input_b: HtmlInputElement = document.get_element_by_id("file-input-b").unwrap().unchecked_into();
    let cb = Closure::wrap(Box::new(move |_: Event| {
        let doc = window().unwrap().document().unwrap();
        let input: HtmlInputElement = doc.get_element_by_id("file-input-b").unwrap().unchecked_into();
        if let Some(files) = input.files() {
            if let Some(file) = files.get(0) {
                load_compare_file(file);
            }
        }
    }) as Box<dyn FnMut(_)>);
    input_b.add_event_listener_with_callback("change", cb.as_ref().unchecked_ref())?;
    cb.forget();

    Ok(())
}

fn setup_drop_zone(document: &Document) -> Result<(), JsValue> {
    let drop_zone = document.get_element_by_id("drop-zone").unwrap();
    let drop_zone_el: HtmlElement = drop_zone.unchecked_into();

    // Click to open file picker
    let cb = Closure::wrap(Box::new(move |_: Event| {
        let doc = window().unwrap().document().unwrap();
        let input: HtmlInputElement = doc.get_element_by_id("file-input").unwrap().unchecked_into();
        input.click();
    }) as Box<dyn FnMut(_)>);
    drop_zone_el.add_event_listener_with_callback("click", cb.as_ref().unchecked_ref())?;
    cb.forget();

    // Dragover
    let dz = drop_zone_el.clone();
    let cb = Closure::wrap(Box::new(move |e: DragEvent| {
        e.prevent_default();
        dz.class_list().add_1("dragover").ok();
    }) as Box<dyn FnMut(_)>);
    drop_zone_el.add_event_listener_with_callback("dragover", cb.as_ref().unchecked_ref())?;
    cb.forget();

    // Dragleave
    let dz = drop_zone_el.clone();
    let cb = Closure::wrap(Box::new(move |_: DragEvent| {
        dz.class_list().remove_1("dragover").ok();
    }) as Box<dyn FnMut(_)>);
    drop_zone_el.add_event_listener_with_callback("dragleave", cb.as_ref().unchecked_ref())?;
    cb.forget();

    // Drop
    let cb = Closure::wrap(Box::new(move |e: DragEvent| {
        e.prevent_default();
        if let Some(dt) = e.data_transfer() {
            if let Some(files) = dt.files() {
                if let Some(file) = files.get(0) {
                    load_file(file);
                }
            }
        }
    }) as Box<dyn FnMut(_)>);
    drop_zone_el.add_event_listener_with_callback("drop", cb.as_ref().unchecked_ref())?;
    cb.forget();

    Ok(())
}

fn setup_file_input(document: &Document) -> Result<(), JsValue> {
    let input: HtmlInputElement = document.get_element_by_id("file-input").unwrap().unchecked_into();
    let cb = Closure::wrap(Box::new(move |_: Event| {
        let doc = window().unwrap().document().unwrap();
        let input: HtmlInputElement = doc.get_element_by_id("file-input").unwrap().unchecked_into();
        if let Some(files) = input.files() {
            if let Some(file) = files.get(0) {
                load_file(file);
            }
        }
    }) as Box<dyn FnMut(_)>);
    input.add_event_listener_with_callback("change", cb.as_ref().unchecked_ref())?;
    cb.forget();
    Ok(())
}

fn load_file(file: web_sys::File) {
    let filename = file.name();
    let reader = FileReader::new().unwrap();

    let r = reader.clone();
    let cb = Closure::wrap(Box::new(move |_: Event| {
        let array_buffer = r.result().unwrap();
        let uint8_array = js_sys::Uint8Array::new(&array_buffer);
        let data = uint8_array.to_vec();
        process_elf(&filename, &data);
    }) as Box<dyn FnMut(_)>);
    reader.set_onload(Some(cb.as_ref().unchecked_ref()));
    cb.forget();

    reader.read_as_array_buffer(&file).unwrap();
}

fn process_elf(filename: &str, data: &[u8]) {
    let document = window().unwrap().document().unwrap();

    match parse::parse_elf(data) {
        Ok(symbols) => {
            let sym_map = symbols_to_map(&symbols);
            let size_tree = tree::build_tree(&symbols);
            let total_size = size_tree.size;
            let win = window().unwrap();
            let w = win.inner_width().unwrap().as_f64().unwrap();
            let h = win.inner_height().unwrap().as_f64().unwrap() - 36.0;

            let dpr = win.device_pixel_ratio();
            let layout_root = layout::layout(&size_tree, w, h);

            STATE.with(|s| {
                let mut state = s.borrow_mut();
                state.size_tree = Some(size_tree);
                state.symbol_sizes = Some(sym_map);
                state.layout_root = Some(layout_root);
                state.filename = filename.to_string();
                state.total_size = total_size;
                state.canvas_width = w;
                state.canvas_height = h;
                state.dpr = dpr;
            });

            show_header(&document, filename, total_size);
            document.get_element_by_id("drop-zone").unwrap()
                .unchecked_ref::<HtmlElement>().class_list().add_1("hidden").ok();

            let canvas: HtmlCanvasElement = document.get_element_by_id("canvas").unwrap().unchecked_into();
            // Set backing store to physical pixels for crisp rendering
            canvas.set_width((w * dpr) as u32);
            canvas.set_height((h * dpr) as u32);
            // Set CSS display size to logical pixels
            canvas.style().set_property("width", &format!("{w}px")).ok();
            canvas.style().set_property("height", &format!("{h}px")).ok();
            let ctx = canvas.get_context("2d").unwrap().unwrap()
                .unchecked_into::<CanvasRenderingContext2d>();
            // Scale context so draw calls use CSS pixel coordinates
            ctx.scale(dpr, dpr).ok();
            STATE.with(|s| {
                let state = s.borrow();
                if let Some(ref root) = state.layout_root {
                    render::render(&ctx, root);
                }
            });
        }
        Err(msg) => {
            show_error(&document, &msg);
        }
    }
}

fn show_header(document: &Document, filename: &str, total_size: u64) {
    let header = document.get_element_by_id("header").unwrap();
    header.unchecked_ref::<HtmlElement>().style().set_property("display", "flex").ok();
    // Hide corner GitHub link (header has its own)
    if let Some(corner) = document.get_element_by_id("corner-gh") {
        corner.unchecked_ref::<HtmlElement>().class_list().add_1("hidden").ok();
    }
    document.get_element_by_id("filename").unwrap().set_text_content(Some(filename));
    let size_str = format_size(total_size);
    document.get_element_by_id("totalsize").unwrap().set_text_content(Some(&format!("Flash: {size_str}")));

    if let Some(btn) = document.get_element_by_id("compare-btn") {
        btn.unchecked_ref::<HtmlElement>().style().set_property("display", "").ok();
    }
}

fn show_error(document: &Document, msg: &str) {
    let error = document.get_element_by_id("error").unwrap();
    error.unchecked_ref::<HtmlElement>().style().set_property("display", "flex").ok();
    document.get_element_by_id("error-msg").unwrap().set_text_content(Some(msg));
}

fn format_size(bytes: u64) -> String {
    if bytes >= 1_048_576 {
        format!("{:.1} MB", bytes as f64 / 1_048_576.0)
    } else if bytes >= 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{bytes} B")
    }
}

fn collect_leaf_names(node: &layout::LayoutNode) -> Vec<String> {
    let mut names = Vec::new();
    collect_leaf_names_recursive(node, &mut names);
    names
}

fn collect_leaf_names_recursive(node: &layout::LayoutNode, names: &mut Vec<String>) {
    if node.is_leaf {
        names.push(node.name.clone());
    } else {
        for child in &node.children {
            collect_leaf_names_recursive(child, names);
        }
    }
}

fn symbols_to_map(symbols: &[parse::ResolvedSymbol]) -> std::collections::HashMap<String, u64> {
    symbols.iter().map(|s| (s.name.clone(), s.size)).collect()
}

fn load_compare_file(file: web_sys::File) {
    let filename = file.name();
    let reader = FileReader::new().unwrap();
    let r = reader.clone();
    let cb = Closure::wrap(Box::new(move |_: Event| {
        let array_buffer = r.result().unwrap();
        let uint8_array = js_sys::Uint8Array::new(&array_buffer);
        let data = uint8_array.to_vec();
        process_compare_elf(&filename, &data);
    }) as Box<dyn FnMut(_)>);
    reader.set_onload(Some(cb.as_ref().unchecked_ref()));
    cb.forget();
    reader.read_as_array_buffer(&file).unwrap();
}

fn process_compare_elf(filename: &str, data: &[u8]) {
    let document = window().unwrap().document().unwrap();

    match parse::parse_elf(data) {
        Ok(symbols) => {
            let size_tree_b = tree::build_tree(&symbols);
            let total_b = size_tree_b.size;
            let win = window().unwrap();
            let full_w = win.inner_width().unwrap().as_f64().unwrap();
            let h = win.inner_height().unwrap().as_f64().unwrap() - 36.0;
            let half_w = (full_w - 2.0) / 2.0;
            let dpr = win.device_pixel_ratio();

            let layout_b = layout::layout(&size_tree_b, half_w, h);

            // Build diff by symbol name (not tree path, which varies due to clustering)
            let paths_b = symbols_to_map(&symbols);

            // Re-layout tree A at half width and compute diff
            STATE.with(|s| {
                let mut state = s.borrow_mut();
                let paths_a = state.symbol_sizes.clone().unwrap_or_default();

                if let Some(ref tree_a) = state.size_tree {
                    state.layout_root = Some(layout::layout(tree_a, half_w, h));
                }

                let diff_map = diff::compute_diff(&paths_a, &paths_b);

                state.compare_layout = Some(layout_b);
                state.compare_filename = filename.to_string();
                state.compare_total_size = total_b;
                state.diff_map = Some(diff_map);
                state.canvas_width = half_w;
                state.canvas_height = h;
                state.dpr = dpr;
            });

            // Update header
            STATE.with(|s| {
                let state = s.borrow();
                let size_a = format_size(state.total_size);
                let size_b = format_size(total_b);
                let diff = total_b as i64 - state.total_size as i64;
                let (sign, abs_diff) = if diff >= 0 { ("+", diff as u64) } else { ("-", (-diff) as u64) };
                let diff_str = format!("{sign}{}", format_size(abs_diff));
                let pct = if state.total_size > 0 {
                    diff as f64 / state.total_size as f64 * 100.0
                } else {
                    0.0
                };
                document.get_element_by_id("filename").unwrap()
                    .set_text_content(Some(&format!("{} vs {}", state.filename, filename)));
                document.get_element_by_id("totalsize").unwrap()
                    .set_text_content(Some(&format!("{size_a} → {size_b} ({diff_str}, {pct:+.1}%)")));
            });

            // Hide compare button
            if let Some(btn) = document.get_element_by_id("compare-btn") {
                btn.unchecked_ref::<HtmlElement>().style().set_property("display", "none").ok();
            }

            // Show comparison UI
            document.get_element_by_id("compare-divider").unwrap()
                .unchecked_ref::<HtmlElement>().style().set_property("display", "").ok();
            let canvas_b: HtmlCanvasElement = document.get_element_by_id("canvas-b").unwrap().unchecked_into();
            canvas_b.style().set_property("display", "").ok();
            document.get_element_by_id("canvas-container").unwrap()
                .unchecked_ref::<HtmlElement>().class_list().add_1("compare-mode").ok();

            // Resize canvas A
            let canvas_a: HtmlCanvasElement = document.get_element_by_id("canvas").unwrap().unchecked_into();
            canvas_a.set_width((half_w * dpr) as u32);
            canvas_a.set_height((h * dpr) as u32);
            canvas_a.style().set_property("width", &format!("{half_w}px")).ok();
            canvas_a.style().set_property("height", &format!("{h}px")).ok();

            // Set up canvas B
            canvas_b.set_width((half_w * dpr) as u32);
            canvas_b.set_height((h * dpr) as u32);
            canvas_b.style().set_property("width", &format!("{half_w}px")).ok();
            canvas_b.style().set_property("height", &format!("{h}px")).ok();

            // Render both
            render_comparison(dpr);
        }
        Err(msg) => {
            show_error(&document, &msg);
        }
    }
}

fn render_comparison(dpr: f64) {
    let document = window().unwrap().document().unwrap();

    STATE.with(|s| {
        let state = s.borrow();
        if let (Some(ref root_a), Some(ref root_b), Some(ref deltas)) =
            (&state.layout_root, &state.compare_layout, &state.diff_map)
        {
            let canvas_a: HtmlCanvasElement = document.get_element_by_id("canvas").unwrap().unchecked_into();
            let ctx_a = canvas_a.get_context("2d").unwrap().unwrap()
                .unchecked_into::<CanvasRenderingContext2d>();
            ctx_a.set_transform(dpr, 0.0, 0.0, dpr, 0.0, 0.0).ok();
            render::render_diff(&ctx_a, root_a, deltas);

            let canvas_b: HtmlCanvasElement = document.get_element_by_id("canvas-b").unwrap().unchecked_into();
            let ctx_b = canvas_b.get_context("2d").unwrap().unwrap()
                .unchecked_into::<CanvasRenderingContext2d>();
            ctx_b.set_transform(dpr, 0.0, 0.0, dpr, 0.0, 0.0).ok();
            render::render_diff(&ctx_b, root_b, deltas);
        }
    });
}

fn handle_compare_hover(x: f64, y: f64, is_canvas_b: bool) {
    STATE.with(|s| {
        let state = s.borrow();

        let (hovered_root, other_root) = if is_canvas_b {
            (&state.compare_layout, &state.layout_root)
        } else {
            (&state.layout_root, &state.compare_layout)
        };

        if let (Some(hovered), Some(other), Some(deltas)) =
            (hovered_root, other_root, &state.diff_map)
        {
            let doc = window().unwrap().document().unwrap();
            let dpr = state.dpr;

            // Redraw both canvases clean (clears previous highlights)
            let canvas_a: HtmlCanvasElement = doc.get_element_by_id("canvas").unwrap().unchecked_into();
            let ctx_a = canvas_a.get_context("2d").unwrap().unwrap()
                .unchecked_into::<CanvasRenderingContext2d>();
            ctx_a.set_transform(dpr, 0.0, 0.0, dpr, 0.0, 0.0).ok();
            render::render_diff(&ctx_a, state.layout_root.as_ref().unwrap(), deltas);

            let canvas_b: HtmlCanvasElement = doc.get_element_by_id("canvas-b").unwrap().unchecked_into();
            let ctx_b = canvas_b.get_context("2d").unwrap().unwrap()
                .unchecked_into::<CanvasRenderingContext2d>();
            ctx_b.set_transform(dpr, 0.0, 0.0, dpr, 0.0, 0.0).ok();
            render::render_diff(&ctx_b, state.compare_layout.as_ref().unwrap(), deltas);

            if let Some(path) = layout::hit_test(hovered, x, y) {
                // Highlight matching node in OTHER canvas
                let other_ctx = if is_canvas_b { &ctx_a } else { &ctx_b };
                render::render_highlight(other_ctx, other, &path[1..]);

                // Walk to hovered node
                let mut node = hovered;
                for name in &path[1..] {
                    if let Some(child) = node.children.iter().find(|c| c.name == *name) {
                        node = child;
                    } else {
                        break;
                    }
                }

                let display_name = path.last().map(|s| s.as_str()).unwrap_or("");
                let tooltip = if node.is_leaf {
                    // Leaf: look up by symbol name
                    if let Some(delta) = deltas.get(display_name) {
                        let before_str = delta.before.map(format_size).unwrap_or_else(|| "\u{2014}".into());
                        let after_str = delta.after.map(format_size).unwrap_or_else(|| "\u{2014}".into());
                        let diff = delta.diff_bytes();
                        let (sign, abs_diff) = if diff >= 0 { ("+", diff as u64) } else { ("-", (-diff) as u64) };
                        let diff_str = format!("{sign}{}", format_size(abs_diff));
                        format!("{display_name}\n{before_str} \u{2192} {after_str}\n{diff_str}")
                    } else {
                        display_name.to_string()
                    }
                } else {
                    // Parent: sum diffs of all descendant leaves
                    let leaf_names = collect_leaf_names(node);
                    let mut total_before: u64 = 0;
                    let mut total_after: u64 = 0;
                    for name in &leaf_names {
                        if let Some(delta) = deltas.get(name.as_str()) {
                            total_before += delta.before.unwrap_or(0);
                            total_after += delta.after.unwrap_or(0);
                        }
                    }
                    let diff = total_after as i64 - total_before as i64;
                    let (sign, abs_diff) = if diff >= 0 { ("+", diff as u64) } else { ("-", (-diff) as u64) };
                    let diff_str = format!("{sign}{}", format_size(abs_diff));
                    let pct = if total_before > 0 {
                        diff as f64 / total_before as f64 * 100.0
                    } else if total_after > 0 {
                        f64::INFINITY
                    } else {
                        0.0
                    };
                    let pct_str = if pct.is_finite() {
                        format!(" ({pct:+.1}%)")
                    } else {
                        " (new)".to_string()
                    };
                    format!(
                        "{display_name}\n{} \u{2192} {}\n{diff_str}{pct_str}",
                        format_size(total_before),
                        format_size(total_after),
                    )
                };

                // Show tooltip on hovered canvas
                let hovered_ctx = if is_canvas_b { &ctx_b } else { &ctx_a };
                render::render_tooltip(hovered_ctx, x, y, &tooltip, state.canvas_width, state.canvas_height);
            }
        }
    });
}

fn setup_canvas_events(document: &Document) -> Result<(), JsValue> {
    let canvas: HtmlCanvasElement = document.get_element_by_id("canvas").unwrap().unchecked_into();

    let cb = Closure::wrap(Box::new(move |e: MouseEvent| {
        let x = e.offset_x() as f64;
        let y = e.offset_y() as f64;

        // Check if in comparison mode
        let in_compare = STATE.with(|s| s.borrow().diff_map.is_some());
        if in_compare {
            handle_compare_hover(x, y, false);
            return;
        }

        STATE.with(|s| {
            let state = s.borrow();
            if let Some(ref root) = state.layout_root {
                let doc = window().unwrap().document().unwrap();
                let canvas: HtmlCanvasElement = doc.get_element_by_id("canvas").unwrap().unchecked_into();
                let ctx = canvas.get_context("2d").unwrap().unwrap()
                    .unchecked_into::<CanvasRenderingContext2d>();

                // Reset transform and re-apply DPR scaling
                ctx.set_transform(state.dpr, 0.0, 0.0, state.dpr, 0.0, 0.0).ok();
                render::render(&ctx, root);

                if let Some(path) = layout::hit_test(root, x, y) {
                    let mut node = root;
                    for name in &path[1..] {
                        if let Some(child) = node.children.iter().find(|c| c.name == *name) {
                            node = child;
                        } else {
                            break;
                        }
                    }

                    let pct = if state.total_size > 0 {
                        node.size as f64 / state.total_size as f64 * 100.0
                    } else {
                        0.0
                    };
                    let size_str = format_size(node.size);

                    // Show "filename\nsymbol  size (pct%)"
                    // The path is [root, ..dirs.., file, symbol] for leaves
                    let parts = &path[1..];
                    let tooltip = if parts.len() >= 2 {
                        let file_node = &parts[parts.len() - 2];
                        // basename: last component after any '/' from collapsed paths
                        let basename = file_node.rsplit('/').next().unwrap_or(file_node);
                        let sym_name = &parts[parts.len() - 1];
                        format!("{basename}\n{sym_name}\n{size_str} ({pct:.1}%)")
                    } else {
                        let display_path = parts.join("/");
                        format!("{display_path}\n{size_str} ({pct:.1}%)")
                    };

                    render::render_tooltip(&ctx, x, y, &tooltip, state.canvas_width, state.canvas_height);
                }
            }
        });
    }) as Box<dyn FnMut(_)>);
    canvas.add_event_listener_with_callback("mousemove", cb.as_ref().unchecked_ref())?;
    cb.forget();

    // Canvas B mousemove (comparison mode)
    let canvas_b: HtmlCanvasElement = document.get_element_by_id("canvas-b").unwrap().unchecked_into();
    let cb = Closure::wrap(Box::new(move |e: MouseEvent| {
        let in_compare = STATE.with(|s| s.borrow().diff_map.is_some());
        if in_compare {
            handle_compare_hover(e.offset_x() as f64, e.offset_y() as f64, true);
        }
    }) as Box<dyn FnMut(_)>);
    canvas_b.add_event_listener_with_callback("mousemove", cb.as_ref().unchecked_ref())?;
    cb.forget();

    Ok(())
}
