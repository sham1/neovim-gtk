use std::cell::RefCell;
use std::thread;
use std::collections::HashMap;
use std::mem;

use rmp::Value;
use rmp::value::Integer;

use cairo;
use gtk;
use gtk::prelude::*;
use gtk::{Window, WindowType, DrawingArea, Grid, ToolButton, ButtonBox, Orientation, Image};
use gdk;
use gdk::EventKey;
use neovim_lib::{Neovim, NeovimApi};

use ui_model::{UiModel, Attrs, Color};
use nvim::RedrawEvents;

thread_local!(pub static UI: RefCell<Ui> = {
    let thread = thread::current();
    let current_thread_name = thread.name();
    if current_thread_name != Some("<main>") {
        panic!("Can create UI  only from main thread, {:?}", current_thread_name);
    }
    RefCell::new(Ui::new())
});

pub struct Ui {
    pub model: UiModel,
    nvim: Option<Neovim>,
    drawing_area: DrawingArea,
    cur_attrs: Option<Attrs>,
}

impl Ui {
    pub fn new() -> Ui {
        Ui {
            model: UiModel::empty(),
            drawing_area: DrawingArea::new(),
            nvim: None,
            cur_attrs: None,
        }
    }

    pub fn set_nvim(&mut self, nvim: Neovim) {
        self.nvim = Some(nvim);
    }

    pub fn nvim(&mut self) -> &mut Neovim {
        self.nvim.as_mut().unwrap()
    }

    pub fn init(&mut self) {

        let window = Window::new(WindowType::Toplevel);

        let grid = Grid::new();

        let button_bar = ButtonBox::new(Orientation::Horizontal);
        button_bar.set_hexpand(true);
        button_bar.set_layout(gtk::ButtonBoxStyle::Start);

        let open_image = Image::new_from_icon_name("document-open", 50);
        let open_btn = ToolButton::new(Some(&open_image), None);
        button_bar.add(&open_btn);

        let save_image = Image::new_from_icon_name("document-save", 50);
        let save_btn = ToolButton::new(Some(&save_image), None);
        button_bar.add(&save_btn);

        let exit_image = Image::new_from_icon_name("application-exit", 50);
        let exit_btn = ToolButton::new(Some(&exit_image), None);
        button_bar.add(&exit_btn);

        grid.attach(&button_bar, 0, 0, 1, 1);

        self.drawing_area.set_size_request(500, 500);
        self.drawing_area.set_hexpand(true);
        self.drawing_area.set_vexpand(true);
        grid.attach(&self.drawing_area, 0, 1, 1, 1);
        self.drawing_area.connect_draw(gtk_draw);

        window.add(&grid);
        window.show_all();
        window.connect_key_press_event(gtk_key_press);
        window.connect_delete_event(|_, _| {
            gtk::main_quit();
            Inhibit(false)
        });
    }
}

fn gtk_key_press(_: &Window, ev: &EventKey) -> Inhibit {
    let keyval = ev.get_keyval();
    if let Some(keyval_name) = gdk::keyval_name(keyval) {
        UI.with(|ui_cell| {
            let mut ui = ui_cell.borrow_mut();
            let input = if keyval_name.starts_with("KP_") {
                keyval_name.chars().skip(3).collect()
            } else {
                keyval_name
            };
            ui.nvim().input(&input).expect("Error run input command to nvim");
        });
    }
    Inhibit(true)
}

fn gtk_draw(drawing_area: &DrawingArea, ctx: &cairo::Context) -> Inhibit {
    let width = drawing_area.get_allocated_width() as f64;
    let height = drawing_area.get_allocated_height() as f64;

    ctx.set_source_rgb(0.0, 0.0, 0.0);
    ctx.paint();
    ctx.set_source_rgb(1.0, 1.0, 1.0);



    let font_extents = ctx.font_extents();
    UI.with(|ui_cell| {
        let ui = ui_cell.borrow();

        let mut line_y = font_extents.height;
        for line in ui.model.model() {
            ctx.move_to(0.0, line_y - font_extents.descent);
            for cell in line {
                let slant = if cell.attrs.italic {
                    cairo::enums::FontSlant::Italic
                } else {
                    cairo::enums::FontSlant::Normal
                };

                let weight = if cell.attrs.bold {
                    cairo::enums::FontWeight::Bold
                } else {
                    cairo::enums::FontWeight::Normal
                };

                let font_face = cairo::FontFace::toy_create("", slant, weight);
                ctx.set_font_face(font_face);

                let bg = &cell.attrs.background;
                ctx.set_source_rgb(bg.0, bg.1, bg.2);
                // ctx.set_source_rgb(1.0, 0.0 , 0.0);
                let text_extents = ctx.text_extents(&cell.ch.to_string());
                let current_point = ctx.get_current_point();
                ctx.rectangle(current_point.0,
                              line_y - font_extents.height,
                              text_extents.width,
                              font_extents.height);
                ctx.fill();

                ctx.move_to(current_point.0, current_point.1);
                let fg = &cell.attrs.foreground;
                ctx.set_source_rgb(fg.0, fg.1, fg.2);
                ctx.show_text(&cell.ch.to_string());
            }
            line_y += font_extents.height;
        }
    });

    Inhibit(true)
}

impl RedrawEvents for Ui {
    fn on_cursor_goto(&mut self, row: u64, col: u64) {
        self.model.set_cursor(row, col);
    }

    fn on_put(&mut self, text: &str) {
        self.model.put(text, &self.cur_attrs);
    }

    fn on_clear(&mut self) {
        self.model.clear();
    }

    fn on_resize(&mut self, columns: u64, rows: u64) {
        self.model = UiModel::new(rows, columns);
    }

    fn on_redraw(&self) {
        self.drawing_area.queue_draw();
    }

    fn on_highlight_set(&mut self, attrs: &HashMap<String, Value>) {
        let mut model_attrs = Attrs::new();
        if let Some(&Value::Integer(Integer::U64(fg))) = attrs.get("foreground") {
            model_attrs.foreground = split_color(fg);
        }
        if let Some(&Value::Integer(Integer::U64(fg))) = attrs.get("background") {
            model_attrs.background = split_color(fg);
        }
        if attrs.contains_key("reverse") {
            mem::swap(&mut model_attrs.foreground, &mut model_attrs.background);
        }
        model_attrs.bold = attrs.contains_key("bold");
        model_attrs.italic = attrs.contains_key("italic");
        self.cur_attrs = Some(model_attrs);
    }
}

fn split_color(indexed_color: u64) -> Color {
    let r = ((indexed_color >> 16) & 0xff) as f64;
    let g = ((indexed_color >> 8) & 0xff) as f64;
    let b = (indexed_color & 0xff) as f64;
    Color(255.0 / r, 255.0 / g, 255.0 / b)
}
