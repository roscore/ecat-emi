//! Browser glue: DOM panels, file input, events, WebGL / 2D rendering. All state stays in memory.
use crate::doc::Doc;
use crate::gl::{Mesh, Paint, Renderer};
use crate::i18n::{fix_text, path_text, tr};
use crate::math::{self, Orbit, OrbitCam};
use crate::{plots, scene};
use ee_core::board::P2;
use ee_core::couple::{self, RunKind, PATHS};
use ee_core::gerber::LayerGuess;
use ee_core::link::Verdict;
use ee_core::roles::{AggrKind, AggrRole, Side};
use ee_core::settings::{self, Settings};
use std::cell::RefCell;
use std::fmt::Write;
use std::rc::Rc;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::{CanvasRenderingContext2d as C2, Document, Element, Event, HtmlCanvasElement, HtmlElement, HtmlInputElement, HtmlSelectElement};

const FOV: f32 = 0.8;
const INVERTER_KEYS: [&str; 5] = ["vbus", "tr", "iph", "fpwm", "csw"];

struct Gpu {
    r: Renderer,
    layers: Vec<(Option<Mesh>, Option<Mesh>)>,
    vias: Option<Mesh>,
    slabs: Option<Mesh>,
    bands: Option<Mesh>,
    hatch: Option<Mesh>,
    outline: Option<Mesh>,
}

enum Mode {
    Orbit,
    Pan,
    Drag(crate::doc::DragStart, P2),
}

struct Ptr {
    x: f64,
    y: f64,
    x0: f64,
    y0: f64,
    mode: Mode,
    moved: bool,
    delta: Option<P2>,
}

struct App {
    doc: Option<Doc>,
    set: Settings,
    gpu: Option<Gpu>,
    ctx2d: Option<C2>,
    cam: OrbitCam,
    /// 2D view: px per mm and offset.
    v2: [f64; 3],
    vis: Vec<bool>,
    opacity: f32,
    xray: bool,
    sel_pair: Option<usize>,
    sel_run: Option<usize>,
    hover: Option<usize>,
    sel_net: Option<usize>,
    ptr: Option<Ptr>,
    dirty_geo: bool,
    dirty_an: bool,
    need_fix: bool,
    last_t: f64,
}

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

fn with<R>(f: impl FnOnce(&mut App) -> R) -> Option<R> {
    APP.with(|c| c.borrow_mut().as_mut().map(f))
}

fn doc() -> Document {
    web_sys::window().expect("window").document().expect("document")
}

fn el(id: &str) -> Option<Element> {
    doc().get_element_by_id(id)
}

fn set_html(id: &str, html: &str) {
    if let Some(e) = el(id) {
        e.set_inner_html(html);
    }
}

fn show(id: &str, on: bool) {
    if let Some(e) = el(id).and_then(|e| e.dyn_into::<HtmlElement>().ok()) {
        e.set_hidden(!on);
    }
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&#39;")
}

fn on(target: &web_sys::EventTarget, ev: &str, f: impl FnMut(Event) + 'static) {
    let c = Closure::<dyn FnMut(Event)>::new(f);
    let _ = target.add_event_listener_with_callback(ev, c.as_ref().unchecked_ref());
    c.forget();
}

fn on_id(id: &str, ev: &str, f: impl FnMut(Event) + 'static) {
    if let Some(e) = el(id) {
        on(&e, ev, f);
    }
}

/// The element (or its ancestor) carrying data-act, with its action and data-i.
fn act(e: &Event) -> Option<(Element, String, usize)> {
    let t: Element = e.target()?.dyn_into().ok()?;
    let a = t.closest("[data-act]").ok()??;
    let name = a.get_attribute("data-act")?;
    let i = a.get_attribute("data-i").and_then(|s| s.parse().ok()).unwrap_or(0);
    Some((a, name, i))
}

fn input_value(e: &Element) -> String {
    if let Some(i) = e.dyn_ref::<HtmlInputElement>() {
        i.value()
    } else if let Some(s) = e.dyn_ref::<HtmlSelectElement>() {
        s.value()
    } else {
        String::new()
    }
}

fn checked(e: &Element) -> bool {
    e.dyn_ref::<HtmlInputElement>().is_some_and(|i| i.checked())
}

fn fmt_num(v: f64) -> String {
    let s = format!("{v:.3}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

fn verdict_key(v: Verdict) -> &'static str {
    match v {
        Verdict::Ok => "ok",
        Verdict::Marginal => "marginal",
        Verdict::Fail => "fail",
    }
}

#[wasm_bindgen(start)]
pub fn start() {
    let hash = web_sys::window().and_then(|w| w.location().hash().ok()).unwrap_or_default();
    let set = settings::decode(&hash);
    let canvas: Option<HtmlCanvasElement> = el("gl").and_then(|e| e.dyn_into().ok());
    let gpu = canvas.as_ref().and_then(Renderer::new).map(|mut r| {
        r.fog = [3000.0, 12000.0];
        r.cell = 10.0;
        Gpu { r, layers: vec![], vias: None, slabs: None, bands: None, hatch: None, outline: None }
    });
    let ctx2d = if gpu.is_none() {
        show("gl", false);
        show("c2d", true);
        show("fallback", true);
        el("c2d").and_then(|e| e.dyn_into::<HtmlCanvasElement>().ok()).and_then(|c| c.get_context("2d").ok().flatten()).and_then(|c| c.dyn_into().ok())
    } else {
        None
    };
    let app = App {
        doc: None,
        set,
        gpu,
        ctx2d,
        cam: OrbitCam::new(Orbit { target: [0.0; 3], yaw: 0.5, pitch: 0.7, dist: 150.0 }),
        v2: [5.0, 0.0, 0.0],
        vis: vec![],
        opacity: 1.0,
        xray: false,
        sel_pair: None,
        sel_run: None,
        hover: None,
        sel_net: None,
        ptr: None,
        dirty_geo: false,
        dirty_an: false,
        need_fix: false,
        last_t: 0.0,
    };
    APP.with(|c| *c.borrow_mut() = Some(app));
    with(|a| {
        a.fill_inputs();
        a.texts();
    });
    wire();
    frame_loop();
}

fn wire() {
    on_id("sample", "click", |_| {
        with(|a| {
            let d = Doc::sample(&a.set.params);
            a.load(d);
        });
    });
    on_id("lang", "click", |_| {
        with(|a| {
            a.set.ko = !a.set.ko;
            a.texts();
            a.panels();
            a.hash();
        });
    });
    on_id("file", "change", |e| {
        if let Some(files) = e.target().and_then(|t| t.dyn_into::<HtmlInputElement>().ok()).and_then(|i| i.files()) {
            read_files(files);
        }
    });
    // Whole page accepts drops, so a file dropped on a side panel is not opened by the browser.
    if let Some(v) = doc().body() {
        on(&v, "dragover", |e| e.prevent_default());
        on(&v, "drop", |e| {
            e.prevent_default();
            if let Some(f) = e.dyn_ref::<web_sys::DragEvent>().and_then(|d| d.data_transfer()).and_then(|d| d.files()) {
                read_files(f);
            }
        });
    }
    // Settings inputs: live analysis on input, ranked fixes on change.
    let nodes = doc().query_selector_all("[data-k]").ok();
    for i in 0..nodes.as_ref().map(|n| n.length()).unwrap_or(0) {
        let Some(n) = nodes.as_ref().and_then(|n| n.item(i)) else { continue };
        let key = n.dyn_ref::<Element>().and_then(|e| e.get_attribute("data-k")).unwrap_or_default();
        let k2 = key.clone();
        on(&n, "input", move |_| {
            with(|a| a.read_inputs(&key));
        });
        on(&n, "change", move |_| {
            with(|a| {
                a.read_inputs(&k2);
                a.need_fix = true;
            });
        });
    }
    let views = doc().query_selector_all("[data-view]").ok();
    for i in 0..views.as_ref().map(|n| n.length()).unwrap_or(0) {
        let Some(n) = views.as_ref().and_then(|n| n.item(i)) else { continue };
        let v: u8 = n.dyn_ref::<Element>().and_then(|e| e.get_attribute("data-view")).and_then(|s| s.parse().ok()).unwrap_or(0);
        on(&n, "click", move |_| {
            with(|a| a.set_view(v));
        });
    }
    on_id("xray", "change", |e| {
        let on = e.target().and_then(|t| t.dyn_into::<Element>().ok()).is_some_and(|t| checked(&t));
        with(|a| a.xray = on);
    });
    on_id("opacity", "input", |e| {
        let v = e.target().and_then(|t| t.dyn_into::<Element>().ok()).map(|t| input_value(&t)).and_then(|s| s.parse().ok()).unwrap_or(1.0);
        with(|a| a.opacity = v);
    });
    on_id("layers", "change", |e| {
        let Some(t) = e.target().and_then(|t| t.dyn_into::<Element>().ok()) else { return };
        let l: usize = t.get_attribute("data-layer").and_then(|s| s.parse().ok()).unwrap_or(0);
        let c = checked(&t);
        with(|a| {
            if let Some(v) = a.vis.get_mut(l) {
                *v = c;
            }
        });
    });
    on_id("reset", "click", |_| {
        with(|a| {
            if let Some(d) = a.doc.as_mut() {
                d.reset_edits(&a.set.params);
                a.dirty_geo = true;
                a.dirty_an = true;
                a.need_fix = true;
            }
        });
    });
    on_id("roles", "change", |e| {
        let Some((t, name, i)) = act(&e) else { return };
        with(|a| a.role_edit(&name, i, &t));
    });
    on_id("pick", "click", |e| {
        let Some((_, name, _)) = act(&e) else { return };
        with(|a| a.tag(&name));
    });
    on_id("gfiles", "change", |e| {
        let Some((t, _, i)) = act(&e) else { return };
        let v = input_value(&t);
        let g = match v.strip_prefix('c').and_then(|s| s.parse().ok()) {
            Some(n) => LayerGuess::Copper(n),
            None => LayerGuess::Unknown,
        };
        with(|a| {
            if let Some(d) = a.doc.as_mut() {
                d.set_gerber_layer(i, g, &a.set.params);
                a.after_load();
            }
        });
    });
    on_id("cards", "click", |e| {
        let Some((t, name, i)) = act(&e) else { return };
        with(|a| match name.as_str() {
            "pair" => {
                a.sel_pair = Some(i);
                a.sel_run = None;
                a.panels();
            }
            "fix" => {
                let k: usize = t.get_attribute("data-k").and_then(|s| s.parse().ok()).unwrap_or(0);
                if let Some(d) = a.doc.as_mut() {
                    if let Some(f) = d.fixes.iter().find(|f| f.0 == i).and_then(|f| f.1.get(k)) {
                        d.edits = f.edits.clone();
                        d.analyze(&a.set.params);
                        a.dirty_geo = true;
                        a.dirty_an = true;
                        a.need_fix = true;
                    }
                }
            }
            _ => {}
        });
    });
    on_id("md", "click", |_| {
        with(|a| a.export(true));
    });
    on_id("csv", "click", |_| {
        with(|a| a.export(false));
    });
    for id in ["gl", "c2d"] {
        on_id(id, "pointerdown", |e| {
            if let Some(p) = e.dyn_ref::<web_sys::PointerEvent>() {
                if let Some(t) = e.target().and_then(|t| t.dyn_into::<Element>().ok()) {
                    let _ = t.set_pointer_capture(p.pointer_id());
                }
                with(|a| a.down(p));
            }
        });
        on_id(id, "pointermove", |e| {
            if let Some(p) = e.dyn_ref::<web_sys::MouseEvent>() {
                with(|a| a.moved(p));
            }
        });
        on_id(id, "pointerup", |e| {
            if let Some(p) = e.dyn_ref::<web_sys::MouseEvent>() {
                with(|a| a.up(p));
            }
        });
        on_id(id, "pointercancel", |_| {
            with(|a| {
                a.cam.held = false;
                if a.ptr.take().is_some() {
                    a.need_fix = true;
                }
            });
        });
        on_id(id, "pointerleave", |_| {
            with(|a| {
                a.hover = None;
                show("tip", false);
            });
        });
        on_id(id, "wheel", |e| {
            e.prevent_default();
            if let Some(w) = e.dyn_ref::<web_sys::WheelEvent>() {
                with(|a| a.wheel(w));
            }
        });
        on_id(id, "dblclick", |_| {
            with(|a| a.set_view(0));
        });
        on_id(id, "contextmenu", |e| e.prevent_default());
    }
}

/// Read every file with the File API (in memory only), then open them together.
fn read_files(list: web_sys::FileList) {
    let n = list.length();
    if n == 0 {
        return;
    }
    type Files = Rc<RefCell<Vec<(String, Vec<u8>)>>>;
    let got: Files = Rc::new(RefCell::new(Vec::new()));
    for i in 0..n {
        let Some(f) = list.get(i) else { continue };
        let Ok(r) = web_sys::FileReader::new() else { continue };
        let (got, r2, name) = (got.clone(), r.clone(), f.name());
        let c = Closure::<dyn FnMut()>::new(move || {
            let bytes = r2.result().map(|b| js_sys::Uint8Array::new(&b).to_vec()).unwrap_or_default();
            got.borrow_mut().push((name.clone(), bytes));
            if got.borrow().len() == n as usize {
                let files = std::mem::take(&mut *got.borrow_mut());
                with(|a| match Doc::open(files, &a.set.params, a.set.ko) {
                    Ok(d) => a.load(d),
                    Err(m) => {
                        set_html("err", &format!("<b>{}</b>\n{}", esc(tr(a.set.ko, "err_title")), esc(&m)));
                        show("err", true);
                    }
                });
            }
        });
        r.set_onload(Some(c.as_ref().unchecked_ref()));
        c.forget();
        let _ = r.read_as_array_buffer(&f);
    }
}

fn frame_loop() {
    type Tick = Rc<RefCell<Option<Closure<dyn FnMut(f64)>>>>;
    let f: Tick = Rc::new(RefCell::new(None));
    let g = f.clone();
    *g.borrow_mut() = Some(Closure::new(move |t: f64| {
        with(|a| a.frame(t));
        if let (Some(w), Some(c)) = (web_sys::window(), f.borrow().as_ref()) {
            let _ = w.request_animation_frame(c.as_ref().unchecked_ref());
        }
    }));
    let first = g.borrow();
    if let (Some(w), Some(c)) = (web_sys::window(), first.as_ref()) {
        let _ = w.request_animation_frame(c.as_ref().unchecked_ref());
    }
}

impl App {
    fn ko(&self) -> bool {
        self.set.ko
    }

    fn texts(&self) {
        let ko = self.ko();
        if let Ok(n) = doc().query_selector_all("[data-t]") {
            for i in 0..n.length() {
                if let Some(e) = n.item(i).and_then(|x| x.dyn_into::<Element>().ok()) {
                    let k = e.get_attribute("data-t").unwrap_or_default();
                    e.set_text_content(Some(tr(ko, &k)));
                }
            }
        }
        if let Some(e) = el("lang") {
            e.set_text_content(Some(if ko { "EN" } else { "한국어" }));
        }
        if let Some(e) = doc().document_element() {
            let _ = e.set_attribute("lang", if ko { "ko" } else { "en" });
        }
    }

    fn fill_inputs(&self) {
        let Ok(n) = doc().query_selector_all("[data-k]") else { return };
        for i in 0..n.length() {
            let Some(e) = n.item(i).and_then(|x| x.dyn_into::<Element>().ok()) else { continue };
            let k = e.get_attribute("data-k").unwrap_or_default();
            let Some(v) = settings::value(&self.set, &k) else { continue };
            if let Some(x) = e.dyn_ref::<HtmlInputElement>() {
                x.set_value(&fmt_num(v));
            } else if let Some(x) = e.dyn_ref::<HtmlSelectElement>() {
                x.set_value(&fmt_num(v));
            }
        }
    }

    /// Rebuild settings from every input through the tolerant hash decoder (one source of clamps).
    fn read_inputs(&mut self, key: &str) {
        let mut h = format!("lang={}&view={}", if self.ko() { "ko" } else { "en" }, self.set.view);
        if let Ok(n) = doc().query_selector_all("[data-k]") {
            for i in 0..n.length() {
                if let Some(e) = n.item(i).and_then(|x| x.dyn_into::<Element>().ok()) {
                    let _ = write!(h, "&{}={}", e.get_attribute("data-k").unwrap_or_default(), input_value(&e));
                }
            }
        }
        let old_exag = self.set.exag;
        self.set = settings::decode(&h);
        if self.set.exag != old_exag {
            self.dirty_geo = true;
            self.fit_target();
        }
        if let Some(d) = self.doc.as_mut() {
            if INVERTER_KEYS.contains(&key) {
                d.set_inverter(&self.set.params);
            } else {
                d.analyze(&self.set.params);
            }
            self.dirty_an = true;
        }
        self.hash();
    }

    fn hash(&self) {
        let url = format!("#{}", settings::encode(&self.set));
        if let Some(h) = web_sys::window().and_then(|w| w.history().ok()) {
            let _ = h.replace_state_with_url(&JsValue::NULL, "", Some(&url));
        }
    }

    fn load(&mut self, d: Doc) {
        self.doc = Some(d);
        self.after_load();
        self.set_view(self.set.view);
    }

    fn after_load(&mut self) {
        show("err", false);
        show("empty", false);
        let Some(d) = self.doc.as_ref() else { return };
        self.vis = vec![true; d.prep.board.layers.len()];
        self.sel_pair = None;
        self.sel_run = None;
        self.sel_net = None;
        self.hover = None;
        self.dirty_geo = true;
        self.dirty_an = true;
        self.need_fix = true;
        let mut lay = String::new();
        for (l, x) in d.prep.board.layers.iter().enumerate() {
            let _ = write!(lay, "<label><input type=\"checkbox\" data-layer=\"{l}\" checked> {}</label>", esc(&x.name));
        }
        set_html("layers", &lay);
        self.fit_2d();
    }

    fn set_view(&mut self, v: u8) {
        self.set.view = v.min(2);
        self.hash();
        if self.gpu.is_none() {
            self.fit_2d();
        }
        let Some(d) = self.doc.as_ref() else { return };
        let b = &d.prep.board;
        let bb = b.bbox();
        let size = (bb[2] - bb[0]).max(bb[3] - bb[1]) as f32;
        let (yaw, pitch) = if v == 0 { (0.5, 0.75) } else { (0.0, math::PITCH_MAX) };
        let dist = size * 0.62 / (FOV / 2.0).tan();
        self.cam.set_goal(Orbit { target: scene::centre(b, self.set.exag as f32), yaw, pitch, dist });
        self.dirty_geo = true;
    }

    fn fit_target(&mut self) {
        if let Some(d) = self.doc.as_ref() {
            let c = scene::centre(&d.prep.board, self.set.exag as f32);
            self.cam.goal.target[1] = c[1];
        }
    }

    fn fit_2d(&mut self) {
        let (Some(d), Some(c)) = (self.doc.as_ref(), el("c2d")) else { return };
        let bb = d.prep.board.bbox();
        let (w, h) = (c.client_width().max(1) as f64, c.client_height().max(1) as f64);
        let s = (w / (bb[2] - bb[0])).min(h / (bb[3] - bb[1])) * 0.9;
        self.v2 = [s, w / 2.0 - s * (bb[0] + bb[2]) / 2.0, h / 2.0 - s * (bb[1] + bb[3]) / 2.0];
    }

    // ---- picking ----

    fn canvas(&self) -> Option<HtmlCanvasElement> {
        el(if self.gpu.is_some() { "gl" } else { "c2d" }).and_then(|e| e.dyn_into().ok())
    }

    fn bottom(&self) -> bool {
        self.set.view == 2 && self.gpu.is_some()
    }

    /// Board point (mm) under canvas pixel (x, y), and the mm size of one pixel there.
    fn board_at(&self, x: f64, y: f64) -> Option<(P2, f64)> {
        let d = self.doc.as_ref()?;
        if self.gpu.is_none() {
            let [s, ox, oy] = self.v2;
            return Some(([(x - ox) / s, (y - oy) / s], 1.0 / s));
        }
        let c = self.canvas()?;
        let (w, h) = (c.client_width().max(1) as f32, c.client_height().max(1) as f32);
        let b = &d.prep.board;
        let ex = self.set.exag as f32;
        let mvp = math::mul(&self.vp(w / h), &scene::model(b, ex, self.bottom()));
        let inv = math::invert(&mvp)?;
        let l = if self.bottom() { b.layers.len().saturating_sub(1) } else { 0 };
        let p = scene::board_point(&inv, 2.0 * x as f32 / w - 1.0, 1.0 - 2.0 * y as f32 / h, scene::layer_y(b, l, ex))?;
        let px = self.cam.cur.dist * 2.0 * (FOV / 2.0).tan() / h;
        Some((p, px as f64))
    }

    fn vp(&self, aspect: f32) -> math::M4 {
        let o = self.cam.cur;
        let proj = math::perspective(FOV, aspect, 1.0, 20000.0);
        math::mul(&proj, &math::look_at(o.eye(), o.target, [0.0, 1.0, 0.0]))
    }

    fn drag_allowed(&self) -> bool {
        self.gpu.is_none() || self.set.view == 1
    }

    fn down(&mut self, e: &web_sys::PointerEvent) {
        let (x, y) = (e.offset_x() as f64, e.offset_y() as f64);
        let mut mode = if e.button() == 2 || e.shift_key() { Mode::Pan } else if self.gpu.is_some() { Mode::Orbit } else { Mode::Pan };
        if e.button() == 0 && !e.shift_key() && self.drag_allowed() {
            if let (Some((p, px)), Some(d)) = (self.board_at(x, y), self.doc.as_ref()) {
                if let Some(seg) = couple::pick_victim(&d.prep, &d.roles, &d.edits, p, 6.0 * px) {
                    mode = Mode::Drag(d.drag_start(seg), p);
                }
            }
        }
        self.cam.held = matches!(mode, Mode::Orbit | Mode::Pan);
        self.ptr = Some(Ptr { x, y, x0: x, y0: y, mode, moved: false, delta: None });
    }

    fn moved(&mut self, e: &web_sys::MouseEvent) {
        let (x, y) = (e.offset_x() as f64, e.offset_y() as f64);
        let Some(mut p) = self.ptr.take() else {
            self.hover_at(x, y);
            return;
        };
        let (dx, dy) = (x - p.x, y - p.y);
        p.x = x;
        p.y = y;
        if (x - p.x0).abs() + (y - p.y0).abs() > 3.0 {
            p.moved = true;
        }
        match &p.mode {
            Mode::Orbit => self.cam.rotate(-dx as f32 * 0.006, dy as f32 * 0.006),
            Mode::Pan if self.gpu.is_some() => {
                let o = self.cam.cur;
                let k = o.dist * 2.0 * (FOV / 2.0).tan() / self.canvas().map(|c| c.client_height().max(1)).unwrap_or(600) as f32;
                let (sy, cy) = o.yaw.sin_cos();
                let off = math::add(math::scale([cy, 0.0, -sy], -dx as f32 * k), math::scale([-sy, 0.0, -cy], -dy as f32 * k));
                self.cam.pan(off);
            }
            Mode::Pan => {
                self.v2[1] += dx;
                self.v2[2] += dy;
            }
            Mode::Drag(_, p0) => {
                if let Some((b, _)) = self.board_at(x, y) {
                    p.delta = Some([b[0] - p0[0], b[1] - p0[1]]);
                }
            }
        }
        self.ptr = Some(p);
    }

    fn up(&mut self, e: &web_sys::MouseEvent) {
        self.cam.held = false;
        let Some(p) = self.ptr.take() else { return };
        if let Mode::Drag(..) = p.mode {
            if p.moved {
                self.apply_drag(&p);
                self.need_fix = true;
                return;
            }
        }
        if p.moved {
            return;
        }
        // A click: select a run, else a net for tagging.
        let Some((b, px)) = self.board_at(e.offset_x() as f64, e.offset_y() as f64) else { return };
        let Some(d) = self.doc.as_ref() else { return };
        self.sel_run = couple::pick_run(&d.an, b, 4.0 * px);
        if let Some(r) = self.sel_run.and_then(|k| d.an.runs.get(k)) {
            self.sel_pair = Some(r.pair);
        } else {
            self.sel_net = scene::pick_net(&d.prep.board, b, &self.vis, 3.0 * px);
        }
        self.dirty_an = true;
    }

    fn apply_drag(&mut self, p: &Ptr) {
        if let (Mode::Drag(start, _), Some(delta), Some(d)) = (&p.mode, p.delta, self.doc.as_mut()) {
            d.drag(start, delta, &self.set.params);
            self.dirty_geo = true;
            self.dirty_an = true;
        }
    }

    fn hover_at(&mut self, x: f64, y: f64) {
        let Some((b, px)) = self.board_at(x, y) else { return };
        let Some(d) = self.doc.as_ref() else { return };
        let h = couple::pick_run(&d.an, b, 4.0 * px);
        if h != self.hover {
            self.hover = h;
            self.dirty_an = true;
        }
        let Some(r) = h.and_then(|k| d.an.runs.get(k)) else {
            show("tip", false);
            return;
        };
        let ko = self.ko();
        let who = match r.kind {
            RunKind::Parallel => r.aggr.and_then(|a| d.roles.aggressors.get(a)).map(|a| d.prep.board.nets[a.net].clone()).unwrap_or_default(),
            _ => tr(ko, "hot_loop").to_string(),
        };
        let txt = format!(
            "{} ↔ {}\n{} {:.1} mm\n{} {:.2} / {:.2} mm\n{} {:.3} mm\n{} {:.1} dB ({:.0} %)",
            d.roles.pairs.get(r.pair).map(|p| p.name.as_str()).unwrap_or("?"),
            who,
            tr(ko, "len_l"),
            r.len,
            tr(ko, "spacing"),
            r.s_p,
            r.s_n,
            tr(ko, "height"),
            r.h,
            tr(ko, "contrib"),
            r.db,
            r.share * 100.0
        );
        if let Some(t) = el("tip").and_then(|e| e.dyn_into::<HtmlElement>().ok()) {
            t.set_text_content(Some(&txt));
            let _ = t.style().set_property("left", &format!("{}px", x + 14.0));
            let _ = t.style().set_property("top", &format!("{}px", y + 14.0));
            t.set_hidden(false);
        }
    }

    fn wheel(&mut self, e: &web_sys::WheelEvent) {
        let f = (e.delta_y() * 0.0015).exp();
        if self.gpu.is_some() {
            self.cam.zoom(f as f32);
        } else {
            let (x, y) = (e.offset_x() as f64, e.offset_y() as f64);
            let k = 1.0 / f;
            self.v2 = [self.v2[0] * k, x - (x - self.v2[1]) * k, y - (y - self.v2[2]) * k];
        }
    }

    // ---- role editing ----

    fn role_edit(&mut self, name: &str, i: usize, t: &Element) {
        let Some(d) = self.doc.as_mut() else { return };
        let v = input_value(t);
        let num = v.parse::<f64>().ok().filter(|x| x.is_finite() && *x >= 0.0);
        let inv = self.set.params.inv;
        match name {
            "pen" => {
                if let Some(p) = d.roles.pairs.get_mut(i) {
                    p.enabled = checked(t);
                }
            }
            "pside" => {
                if let Some(p) = d.roles.pairs.get_mut(i) {
                    p.side = if v == "1" { Side::Line } else { Side::Phy };
                }
            }
            "aen" => {
                if let Some(a) = d.roles.aggressors.get_mut(i) {
                    a.enabled = checked(t);
                }
            }
            "akind" => {
                if let Some(a) = d.roles.aggressors.get_mut(i) {
                    let kind = match v.as_str() {
                        "1" => AggrKind::Gate,
                        "2" => AggrKind::Bus,
                        _ => AggrKind::SwitchNode,
                    };
                    let en = a.enabled;
                    *a = AggrRole::preset(a.net, kind, &inv);
                    a.enabled = en;
                }
            }
            "adv" | "atr" | "adi" => {
                if let (Some(a), Some(x)) = (d.roles.aggressors.get_mut(i), num) {
                    match name {
                        "adv" => a.dv = x,
                        "atr" => a.t_r = x.max(1.0) * 1e-9,
                        _ => a.di = x,
                    }
                }
            }
            _ => return,
        }
        d.analyze(&self.set.params);
        self.dirty_geo = true;
        self.dirty_an = true;
        self.need_fix = true;
    }

    fn tag(&mut self, name: &str) {
        let (Some(d), Some(net)) = (self.doc.as_mut(), self.sel_net) else { return };
        let p = &self.set.params;
        match name {
            "tag_aggr" => d.tag_aggr(net, p),
            "tag_p" => {
                d.tag_pair(net, true, p);
            }
            "tag_n" => {
                d.tag_pair(net, false, p);
            }
            _ => return,
        }
        self.dirty_geo = true;
        self.dirty_an = true;
        self.need_fix = true;
    }

    // ---- panels ----

    fn panels(&self) {
        let ko = self.ko();
        let Some(d) = self.doc.as_ref() else { return };
        let b = &d.prep.board;
        let mut info = tr(ko, "board_info")
            .replace("{n}", &b.nets.len().saturating_sub(1).to_string())
            .replace("{s}", &b.segs.len().to_string())
            .replace("{v}", &b.vias.len().to_string())
            .replace("{l}", &b.layers.len().to_string())
            .replace("{t}", &format!("{:.2}", b.thickness));
        if let Some(h) = &d.prep.hot_loop {
            let _ = write!(info, " · {} {:.0} mm²", tr(ko, "hot_loop"), h.area_mm2);
        }
        set_html("info", &esc(&info));
        // Gerber layer guesses.
        let mut g = String::new();
        if let Some(f) = &d.gerber {
            let _ = write!(g, "<p class=\"hint\">{}</p><h3>{}</h3>", esc(tr(ko, "gerber_note")), esc(tr(ko, "gerber_layers")));
            for (i, (name, guess, _)) in f.layers.iter().enumerate() {
                let cur = match guess {
                    LayerGuess::Copper(n) => format!("c{n}"),
                    _ => "x".into(),
                };
                let _ = write!(g, "<label><span class=\"name\">{}</span><select data-act=\"glayer\" data-i=\"{i}\">", esc(name));
                let opts = [("c0", "F.Cu"), ("c1", "In1"), ("c2", "In2"), ("c3", "In3"), ("c4", "In4"), ("c1000", "B.Cu"), ("x", tr(ko, "g_ignore"))];
                for (v, label) in opts {
                    let _ = write!(g, "<option value=\"{v}\"{}>{}</option>", if v == cur { " selected" } else { "" }, esc(label));
                }
                g.push_str("</select></label>");
            }
        }
        set_html("gfiles", &g);
        // Roles.
        let mut r = format!("<h3>{}</h3>", esc(tr(ko, "pairs")));
        if d.roles.pairs.is_empty() {
            let _ = write!(r, "<p class=\"hint\">{}</p>", esc(tr(ko, "no_pairs")));
        }
        for (i, p) in d.roles.pairs.iter().enumerate() {
            let line = p.side == Side::Line;
            let _ = write!(
                r,
                "<div class=\"role\"><input type=\"checkbox\" data-act=\"pen\" data-i=\"{i}\"{}><span class=\"sw v\"></span><span class=\"name\">{}</span><select data-act=\"pside\" data-i=\"{i}\"><option value=\"0\"{}>{}</option><option value=\"1\"{}>{}</option></select></div>",
                if p.enabled { " checked" } else { "" },
                esc(&p.name),
                if line { "" } else { " selected" },
                esc(tr(ko, "side_phy")),
                if line { " selected" } else { "" },
                esc(tr(ko, "side_line"))
            );
        }
        let _ = write!(r, "<h3>{}</h3>", esc(tr(ko, "aggressors")));
        for (i, a) in d.roles.aggressors.iter().enumerate() {
            let k = match a.kind {
                AggrKind::SwitchNode => 0,
                AggrKind::Gate => 1,
                AggrKind::Bus => 2,
            };
            let _ = write!(
                r,
                "<div class=\"role\"><input type=\"checkbox\" data-act=\"aen\" data-i=\"{i}\"{}><span class=\"sw a\"></span><span class=\"name\">{}</span><select data-act=\"akind\" data-i=\"{i}\">",
                if a.enabled { " checked" } else { "" },
                esc(b.nets.get(a.net).map(String::as_str).unwrap_or("?"))
            );
            for (v, key) in ["kind_sw", "kind_gate", "kind_bus"].iter().enumerate() {
                let _ = write!(r, "<option value=\"{v}\"{}>{}</option>", if v == k { " selected" } else { "" }, esc(tr(ko, key)));
            }
            let _ = write!(
                r,
                "</select><input type=\"number\" data-act=\"adv\" data-i=\"{i}\" value=\"{}\" step=\"any\" title=\"dV\">V<input type=\"number\" data-act=\"atr\" data-i=\"{i}\" value=\"{}\" step=\"any\" title=\"t_r\">ns<input type=\"number\" data-act=\"adi\" data-i=\"{i}\" value=\"{}\" step=\"any\" title=\"dI\">A</div>",
                fmt_num(a.dv),
                fmt_num(a.t_r * 1e9),
                fmt_num(a.di)
            );
        }
        set_html("roles", &r);
        // Selected net / tagging.
        let mut pk = String::new();
        if let Some(n) = self.sel_net {
            let _ = write!(
                pk,
                "{}: <b>{}</b><br><button type=\"button\" data-act=\"tag_aggr\">{}</button> <button type=\"button\" data-act=\"tag_p\">{}</button> <button type=\"button\" data-act=\"tag_n\">{}</button>",
                esc(tr(ko, "selected")),
                esc(b.nets.get(n).map(String::as_str).unwrap_or("?")),
                esc(tr(ko, "tag_aggr")),
                esc(tr(ko, "tag_p")),
                esc(tr(ko, "tag_n"))
            );
            if let Some(pn) = d.pending_p {
                let _ = write!(pk, " <span class=\"hint\">P = {}</span>", esc(b.nets.get(pn).map(String::as_str).unwrap_or("?")));
            }
        } else {
            pk = esc(tr(ko, "pick_hint"));
        }
        set_html("pick", &pk);
        self.cards();
        self.plots();
    }

    fn sel_pair(&self) -> Option<usize> {
        let d = self.doc.as_ref()?;
        self.sel_pair
            .filter(|k| d.result(*k).is_some())
            .or_else(|| d.an.pairs.iter().max_by(|a, b| a.link.ber.total_cmp(&b.link.ber)).map(|p| p.pair))
    }

    fn cards(&self) {
        let ko = self.ko();
        let Some(d) = self.doc.as_ref() else { return };
        let sel = self.sel_pair();
        let mut h = String::new();
        for p in &d.an.pairs {
            let l = &p.link;
            let v = verdict_key(l.verdict());
            let name = d.roles.pairs.get(p.pair).map(|x| x.name.as_str()).unwrap_or("?");
            let _ = write!(
                h,
                "<div class=\"card {v}{}\" data-act=\"pair\" data-i=\"{}\"><b>{}</b><b class=\"v\">{}</b><br>{} {:.2e} · {} {:.2e}<br>{} {:.3e}<br>{} {}<br>{} {:.1} mV · {} {:.1} mV<br>{} {:.2} mm · {} {:.1} dB",
                if sel == Some(p.pair) { " sel" } else { "" },
                p.pair,
                esc(name),
                esc(tr(ko, v)),
                tr(ko, "ber"),
                l.ber,
                tr(ko, "fer"),
                l.fer,
                esc(tr(ko, "lost")),
                l.lost_per_hour,
                esc(tr(ko, "dominant")),
                esc(path_text(ko, p.dominant)),
                esc(tr(ko, "margin")),
                l.margin * 1e3,
                esc(tr(ko, "peak")),
                l.noise_peak * 1e3,
                esc(tr(ko, "skew")),
                p.skew_mm,
                esc(tr(ko, "mode_conv")),
                20.0 * p.mode_conv.max(1e-12).log10()
            );
            let gaps = d.an.gaps.iter().filter(|g| g.pair == p.pair).count();
            if gaps > 0 {
                let _ = write!(h, "<br>{}: {gaps}", esc(tr(ko, "gaps")));
            }
            match d.fixes.iter().find(|f| f.0 == p.pair) {
                Some((_, list)) => {
                    let _ = write!(h, "<br><i>{}</i><ol>", esc(tr(ko, "fixes")));
                    for (k, f) in list.iter().enumerate() {
                        let _ = write!(
                            h,
                            "<li data-act=\"fix\" data-i=\"{}\" data-k=\"{k}\">{} → {:.1e} ({})</li>",
                            p.pair,
                            esc(&fix_text(ko, &f.kind, name)),
                            f.ber,
                            esc(tr(ko, verdict_key(f.verdict)))
                        );
                    }
                    h.push_str("</ol>");
                }
                None if l.verdict() == Verdict::Ok => {
                    let _ = write!(h, "<br><i>{}</i>", esc(tr(ko, "no_fix")));
                }
                None => {}
            }
            h.push_str("</div>");
        }
        set_html("cards", &h);
    }

    fn plots(&self) {
        let ko = self.ko();
        let (Some(d), Some(pair)) = (self.doc.as_ref(), self.sel_pair()) else { return };
        let Some(res) = d.result(pair) else { return };
        let prm = &self.set.params;
        let ctx = |id: &str| -> Option<(C2, f64, f64)> {
            let c: HtmlCanvasElement = el(id)?.dyn_into().ok()?;
            let x: C2 = c.get_context("2d").ok()??.dyn_into().ok()?;
            Some((x, c.width() as f64, c.height() as f64))
        };
        if let Some((c, w, h)) = ctx("spec") {
            let aggr: Vec<f64> = d.an.freqs.iter().map(|&f| ee_core::spectrum::edge_density(prm.inv.v_bus, d.an.t_r, f)).collect();
            let cols = ["#6aa9ff", "#b48cff", "#ff6ad5", "#ff5050", "#ffd040", "#40e0d0"];
            let mut curves: Vec<(&[f64], &str, &str)> = vec![(&aggr, "#ff8c1a", tr(ko, "aggr_spec"))];
            for (k, path) in PATHS.iter().enumerate() {
                if res.path_energy[k] > 0.0 {
                    curves.push((&res.path_noise[k], cols[k], path_text(ko, *path)));
                }
            }
            if let Some(r) = self.sel_run.and_then(|k| d.an.runs.get(k)).filter(|r| r.pair == pair) {
                curves.push((&r.noise, "#ffffff", tr(ko, "noise")));
            }
            plots::spectrum(&c, w, h, &d.an.freqs, &curves, prm.link.rx_bw, tr(ko, "rx_band"));
        }
        if let Some((c, w, h)) = ctx("eye") {
            plots::eye(&c, w, h, res.link.eye_rx, res.link.noise_peak, [tr(ko, "eye_rx"), tr(ko, "peak"), tr(ko, "margin")]);
        }
        if let Some((c, w, h)) = ctx("burst") {
            let labels = [tr(ko, "frame"), tr(ko, "pwm_period"), tr(ko, "exposure"), tr(ko, "frame_hit")];
            plots::burst(&c, w, h, prm.inv.f_pwm, prm.inv.duty, res.link.burst_s, prm.link.frame_s(), labels, res.link.exposure, res.link.frame_hit);
        }
    }

    fn export(&self, md: bool) {
        let Some(d) = self.doc.as_ref() else { return };
        let (text, name, mime) = if md {
            (ee_core::report::markdown(&d.prep, &d.roles, &d.an, &d.fixes), "ecat-emi-report.md", "text/markdown;charset=utf-8")
        } else {
            (ee_core::report::csv(&d.prep, &d.roles, &d.an), "ecat-emi-runs.csv", "text/csv;charset=utf-8")
        };
        let parts = js_sys::Array::of1(&JsValue::from_str(&text));
        let opt = web_sys::BlobPropertyBag::new();
        opt.set_type(mime);
        let Ok(blob) = web_sys::Blob::new_with_str_sequence_and_options(&parts, &opt) else { return };
        let Ok(url) = web_sys::Url::create_object_url_with_blob(&blob) else { return };
        if let Some(a) = doc().create_element("a").ok().and_then(|e| e.dyn_into::<web_sys::HtmlAnchorElement>().ok()) {
            a.set_href(&url);
            a.set_download(name);
            a.click();
        }
        let _ = web_sys::Url::revoke_object_url(&url);
    }

    // ---- frame ----

    fn frame(&mut self, t: f64) {
        let dt = if self.last_t > 0.0 { ((t - self.last_t) / 1000.0).min(0.1) } else { 1.0 / 60.0 };
        self.last_t = t;
        // Live drag: one analysis per frame at most.
        if let Some(p) = self.ptr.as_mut() {
            if matches!(p.mode, Mode::Drag(..)) && p.delta.is_some() {
                let snapshot = Ptr { x: p.x, y: p.y, x0: p.x0, y0: p.y0, mode: std::mem::replace(&mut p.mode, Mode::Orbit), moved: p.moved, delta: p.delta };
                self.apply_drag(&snapshot);
                if let Some(p) = self.ptr.as_mut() {
                    p.mode = snapshot.mode;
                    p.delta = None;
                }
            }
        }
        if self.dirty_an && !self.need_fix {
            self.panels();
        }
        if self.need_fix && self.ptr.is_none() {
            if let Some(d) = self.doc.as_mut() {
                d.refix(&self.set.params);
            }
            self.need_fix = false;
            self.panels();
        }
        if self.gpu.is_some() {
            self.render3d(dt as f32);
        } else {
            self.render2d();
        }
        self.dirty_an = false;
    }

    fn rebuild(&mut self) {
        let (Some(g), Some(d)) = (self.gpu.as_mut(), self.doc.as_ref()) else { return };
        let ex = self.set.exag as f32;
        if self.dirty_geo {
            for (a, b) in g.layers.drain(..) {
                for m in [a, b].into_iter().flatten() {
                    g.r.free(m);
                }
            }
            for m in [g.vias.take(), g.slabs.take()].into_iter().flatten() {
                g.r.free(m);
            }
            for lg in scene::copper(&d.prep, &d.roles, &d.edits, ex) {
                g.layers.push((g.r.mesh(&lg.base), g.r.mesh(&lg.hot)));
            }
            g.vias = g.r.mesh(&scene::vias(&d.prep.board, ex));
            g.slabs = g.r.mesh(&scene::slabs(&d.prep.board, ex));
        }
        if self.dirty_geo || self.dirty_an {
            for m in [g.bands.take(), g.hatch.take(), g.outline.take()].into_iter().flatten() {
                g.r.free(m);
            }
            g.bands = g.r.mesh(&scene::bands(&d.prep.board, &d.an, ex));
            g.hatch = g.r.mesh(&scene::hatch(&d.prep, &d.an, ex));
            if let Some(k) = self.hover.or(self.sel_run) {
                g.outline = g.r.mesh(&scene::band_outline(&d.prep.board, &d.an, k, ex));
            }
        }
        self.dirty_geo = false;
    }

    fn render3d(&mut self, dt: f32) {
        self.rebuild();
        self.cam.step(dt);
        let Some(c) = self.canvas() else { return };
        let dpr = web_sys::window().map(|w| w.device_pixel_ratio()).unwrap_or(1.0);
        let (w, h) = ((c.client_width() as f64 * dpr) as u32, (c.client_height() as f64 * dpr) as u32);
        if w == 0 || h == 0 {
            return;
        }
        if c.width() != w || c.height() != h {
            c.set_width(w);
            c.set_height(h);
        }
        let o = self.cam.cur;
        let proj = math::perspective(FOV, w as f32 / h as f32, 1.0, 20000.0);
        let view = math::look_at(o.eye(), o.target, [0.0, 1.0, 0.0]);
        let (Some(g), Some(d)) = (self.gpu.as_ref(), self.doc.as_ref()) else {
            if let Some(g) = self.gpu.as_ref() {
                g.r.begin(&view, &proj, o.eye(), [0, 0, w as i32, h as i32]);
            }
            return;
        };
        let r = &g.r;
        r.begin(&view, &proj, o.eye(), [0, 0, w as i32, h as i32]);
        let m = scene::model(&d.prep.board, self.set.exag as f32, self.bottom());
        if let Some(s) = &g.slabs {
            r.shadow(s, &m);
        }
        let (base_a, hot_a) = if self.xray { (0.06, 1.0) } else { (self.opacity, self.opacity) };
        let vis = |l: usize| self.vis.get(l).copied().unwrap_or(true);
        // Opaque first, translucent after (they do not write depth).
        for pass in [true, false] {
            for (l, (base, hot)) in g.layers.iter().enumerate().filter(|(l, _)| vis(*l)) {
                let _ = l;
                for (mesh, a) in [(base, base_a), (hot, hot_a)] {
                    if let Some(mesh) = mesh {
                        if (a >= 1.0) == pass {
                            r.draw(mesh, &m, Paint::lit([1.0, 1.0, 1.0, a]));
                        }
                    }
                }
            }
            if pass {
                if let Some(v) = &g.vias {
                    if !self.xray {
                        r.draw(v, &m, Paint::lit([1.0; 4]));
                    }
                }
            } else if !self.xray {
                if let Some(s) = &g.slabs {
                    r.draw(s, &m, Paint::lit([1.0, 1.0, 1.0, 0.3]));
                }
            }
        }
        // Overlays on top of everything.
        r.gl.disable(web_sys::WebGl2RenderingContext::DEPTH_TEST);
        let pulse = 0.5 + 0.5 * (self.last_t / 400.0).sin() as f32;
        if let Some(b) = &g.bands {
            r.draw(b, &m, Paint { glow: 0.15 * pulse, ..Paint::flat([1.0, 1.0, 1.0, 0.45 + 0.3 * pulse]) });
        }
        if let Some(hm) = &g.hatch {
            r.draw(hm, &m, Paint::flat([1.0, 0.2, 0.2, 0.95]));
        }
        if let Some(om) = &g.outline {
            r.draw(om, &m, Paint::flat([1.0; 4]));
        }
        r.gl.enable(web_sys::WebGl2RenderingContext::DEPTH_TEST);
    }

    fn render2d(&mut self) {
        let (Some(c), Some(cv)) = (self.ctx2d.as_ref(), self.canvas()) else { return };
        let (w, h) = (cv.client_width().max(1) as u32, cv.client_height().max(1) as u32);
        if cv.width() != w || cv.height() != h {
            cv.set_width(w);
            cv.set_height(h);
        }
        c.set_fill_style_str("#0e1116");
        c.fill_rect(0.0, 0.0, w as f64, h as f64);
        let Some(d) = self.doc.as_ref() else { return };
        let [s, ox, oy] = self.v2;
        let tx = |p: P2| (p[0] * s + ox, p[1] * s + oy);
        let b = &d.prep.board;
        let rgb = |v: [f32; 3], a: f64| format!("rgba({},{},{},{a})", (v[0] * 255.0) as u8, (v[1] * 255.0) as u8, (v[2] * 255.0) as u8);
        let mut role = std::collections::HashMap::new();
        for a in d.roles.aggressors.iter().filter(|a| a.enabled) {
            role.insert(a.net, scene::AGGR);
        }
        for p in d.roles.pairs.iter().filter(|p| p.enabled) {
            role.insert(p.p, scene::VICTIM);
            role.insert(p.n, scene::VICTIM);
        }
        c.set_line_cap("round");
        for l in (0..b.layers.len()).rev().filter(|&l| self.vis.get(l).copied().unwrap_or(true)) {
            for (k, sg) in b.segs.iter().enumerate().filter(|(_, x)| x.layer == l) {
                let sg = if role.contains_key(&sg.net) { couple::edited_seg(&d.prep, &d.edits, k) } else { *sg };
                let col = role.get(&sg.net).copied().unwrap_or(scene::layer_color(b, l));
                c.set_stroke_style_str(&rgb(col, if role.contains_key(&sg.net) { 1.0 } else { 0.55 * self.opacity as f64 }));
                c.set_line_width((sg.w * s).max(1.0));
                c.begin_path();
                let (a, e) = (tx(sg.a), tx(sg.b));
                c.move_to(a.0, a.1);
                c.line_to(e.0, e.1);
                c.stroke();
            }
        }
        c.set_fill_style_str("rgba(220,205,150,0.8)");
        for p in &b.pads {
            let (x, y) = tx(p.at);
            c.fill_rect(x - p.size[0] * s / 2.0, y - p.size[1] * s / 2.0, p.size[0] * s, p.size[1] * s);
        }
        for (k, r) in d.an.runs.iter().enumerate().filter(|(_, r)| r.kind != RunKind::Cable) {
            c.set_fill_style_str(&rgb(scene::share_color(r.share), 0.6));
            c.begin_path();
            for (i, p) in r.band.iter().enumerate() {
                let (x, y) = tx(*p);
                if i == 0 { c.move_to(x, y) } else { c.line_to(x, y) }
            }
            c.close_path();
            c.fill();
            if Some(k) == self.hover.or(self.sel_run) {
                c.set_stroke_style_str("#fff");
                c.set_line_width(1.5);
                c.stroke();
            }
        }
        c.set_stroke_style_str("rgba(255,60,60,0.95)");
        c.set_line_width(1.0);
        for g in &d.an.gaps {
            let (x0, y0) = tx([g.bbox[0], g.bbox[1]]);
            let (x1, y1) = tx([g.bbox[2], g.bbox[3]]);
            c.stroke_rect(x0, y0, x1 - x0, y1 - y0);
            c.save();
            c.begin_path();
            c.rect(x0, y0, x1 - x0, y1 - y0);
            c.clip();
            c.begin_path();
            let mut k = -(y1 - y0);
            while k < x1 - x0 {
                c.move_to(x0 + k, y1);
                c.line_to(x0 + k + (y1 - y0), y0);
                k += 6.0;
            }
            c.stroke();
            c.restore();
        }
    }
}
