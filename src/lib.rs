pub mod parse;
pub mod tree;
pub mod layout;
pub mod color;
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
    num_color_groups: usize,
    filename: String,
    total_size: u64,
    canvas_width: f64,
    canvas_height: f64,
}

thread_local! {
    static STATE: RefCell<AppState> = RefCell::new(AppState {
        layout_root: None,
        num_color_groups: 0,
        filename: String::new(),
        total_size: 0,
        canvas_width: 0.0,
        canvas_height: 0.0,
    });
}

#[wasm_bindgen(start)]
pub fn main() -> Result<(), JsValue> {
    let win = window().unwrap();
    let document = win.document().unwrap();

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
        });

        doc.get_element_by_id("header").unwrap()
            .unchecked_ref::<HtmlElement>().style().set_property("display", "none").ok();
        doc.get_element_by_id("error").unwrap()
            .unchecked_ref::<HtmlElement>().style().set_property("display", "none").ok();
        doc.get_element_by_id("drop-zone").unwrap()
            .unchecked_ref::<HtmlElement>().class_list().remove_1("hidden").ok();

        let canvas: HtmlCanvasElement = doc.get_element_by_id("canvas").unwrap().unchecked_into();
        let ctx = canvas.get_context("2d").unwrap().unwrap()
            .unchecked_into::<CanvasRenderingContext2d>();
        ctx.clear_rect(0.0, 0.0, canvas.width() as f64, canvas.height() as f64);
    }) as Box<dyn FnMut(_)>);
    reset.add_event_listener_with_callback("click", cb.as_ref().unchecked_ref())?;
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
            let size_tree = tree::build_tree(&symbols);
            let total_size = size_tree.size;
            let num_groups = size_tree.children.len();

            let win = window().unwrap();
            let w = win.inner_width().unwrap().as_f64().unwrap();
            let h = win.inner_height().unwrap().as_f64().unwrap() - 36.0;

            let layout_root = layout::layout(&size_tree, w, h);

            STATE.with(|s| {
                let mut state = s.borrow_mut();
                state.layout_root = Some(layout_root);
                state.num_color_groups = num_groups;
                state.filename = filename.to_string();
                state.total_size = total_size;
                state.canvas_width = w;
                state.canvas_height = h;
            });

            show_header(&document, filename, total_size);
            document.get_element_by_id("drop-zone").unwrap()
                .unchecked_ref::<HtmlElement>().class_list().add_1("hidden").ok();

            let canvas: HtmlCanvasElement = document.get_element_by_id("canvas").unwrap().unchecked_into();
            canvas.set_width(w as u32);
            canvas.set_height(h as u32);
            let ctx = canvas.get_context("2d").unwrap().unwrap()
                .unchecked_into::<CanvasRenderingContext2d>();
            STATE.with(|s| {
                let state = s.borrow();
                if let Some(ref root) = state.layout_root {
                    render::render(&ctx, root, state.num_color_groups);
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
    document.get_element_by_id("filename").unwrap().set_text_content(Some(filename));
    let size_str = format_size(total_size);
    document.get_element_by_id("totalsize").unwrap().set_text_content(Some(&format!("Flash: {size_str}")));
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

fn setup_canvas_events(document: &Document) -> Result<(), JsValue> {
    let canvas: HtmlCanvasElement = document.get_element_by_id("canvas").unwrap().unchecked_into();

    let cb = Closure::wrap(Box::new(move |e: MouseEvent| {
        let x = e.offset_x() as f64;
        let y = e.offset_y() as f64;

        STATE.with(|s| {
            let state = s.borrow();
            if let Some(ref root) = state.layout_root {
                let doc = window().unwrap().document().unwrap();
                let canvas: HtmlCanvasElement = doc.get_element_by_id("canvas").unwrap().unchecked_into();
                let ctx = canvas.get_context("2d").unwrap().unwrap()
                    .unchecked_into::<CanvasRenderingContext2d>();

                render::render(&ctx, root, state.num_color_groups);

                if let Some(path) = layout::hit_test(root, x, y) {
                    let mut node = root;
                    for name in &path[1..] {
                        if let Some(child) = node.children.iter().find(|c| c.name == *name) {
                            node = child;
                        } else {
                            break;
                        }
                    }

                    let display_path = path[1..].join("/");
                    let pct = if state.total_size > 0 {
                        node.size as f64 / state.total_size as f64 * 100.0
                    } else {
                        0.0
                    };
                    let size_str = format_size(node.size);
                    let tooltip = format!("{display_path}\n{size_str} ({pct:.1}%)");

                    render::render_tooltip(&ctx, x, y, &tooltip, state.canvas_width, state.canvas_height);
                }
            }
        });
    }) as Box<dyn FnMut(_)>);
    canvas.add_event_listener_with_callback("mousemove", cb.as_ref().unchecked_ref())?;
    cb.forget();

    Ok(())
}
