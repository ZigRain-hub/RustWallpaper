use super::*;
use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::thread;
use serde::{Deserialize, Serialize};

mod video_thumb { include!("video_thumb.rs"); }

pub(super) const APP_NAME: &str = "RustWallpaper";
const ABOUT_PROJECT: &str = "RustWallpaper помогает настроить живые обои на рабочем столе Windows.";
const ABOUT_AUTHOR: &str = "Проект создан с любовью к персонализации и удобному рабочему пространству.";
pub(super) const TELEGRAM_URL: &str = "https://t.me/ZigRainZ";
pub(super) const GITHUB_URL: &str = "https://github.com/ZigRain-hub";

const ID_HOTKEY_A: i32 = 340;
const ID_HOTKEY_B: i32 = 341;
const APPLY_FROM_GALLERY: u32 = 0x8002;
const WM_KEYDOWN: u32 = 0x0100;
const WM_HOTKEY: u32 = 0x0312;
const WM_LBUTTONDOWN: u32 = 0x0201;
const WM_MOUSEWHEEL: u32 = 0x020a;
const WM_GALLERY_CLOSE: u32 = 0x0010;
const WM_DESTROY_GALLERY: u32 = 0x0002;
const SIZE_CARDS: usize = 4;
const CARD_X: i32 = 25;
const CARD_Y: i32 = 77;
const CARD_W: i32 = 190;
const CARD_H: i32 = 204;
const CARD_GAP: i32 = 18;

#[derive(Serialize, Deserialize, Clone)]
struct ProjectLink { path: PathBuf, #[serde(default)] overrides: HashMap<String, serde_json::Value> }
#[derive(Clone)]
struct SceneProperty { key: String, label: String, kind: String, value: serde_json::Value, definition: serde_json::Value }
struct LoadedProject { wallpaper: PathBuf, json: serde_json::Value, properties: Vec<SceneProperty> }

#[derive(Serialize, Deserialize)]
struct Stored { videos: Vec<PathBuf>, hotkey: String, #[serde(default = "default_quality")] scene_height: u32, #[serde(default = "default_fps")] scene_fps: u32, #[serde(default = "default_sound")] sound: bool, #[serde(default)] span: bool, #[serde(default)] projects: HashMap<PathBuf, ProjectLink>, #[serde(default)] screen_mode: Option<u8>, #[serde(default)] second_wallpaper: Option<PathBuf>, #[serde(default = "default_transition")] transition: u8, #[serde(default = "default_transition_ms")] transition_ms: u32 }
fn default_quality() -> u32 { 1080 }
fn default_fps() -> u32 { 24 }
fn default_sound() -> bool { true }
fn default_transition() -> u8 { 1 }
fn default_transition_ms() -> u32 { 700 }
impl Default for Stored {
    fn default() -> Self { Self { videos: Vec::new(), hotkey: "Alt+Q".into(), scene_height: default_quality(), scene_fps: default_fps(), sound: default_sound(), span: false, projects: HashMap::new(), screen_mode: None, second_wallpaper: None, transition: default_transition(), transition_ms: default_transition_ms() } }
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Size { cx: i32, cy: i32 }
#[repr(C)]
struct Bitmap { kind: i32, width: i32, height: i32, width_bytes: i32, planes: u16, bits_pixel: u16, bits: Handle }
#[repr(C)]
struct ImageFactory { vtable: *const ImageFactoryVtable }
#[repr(C)]
struct ImageFactoryVtable {
    query: usize, add_ref: usize,
    release: unsafe extern "system" fn(*mut ImageFactory) -> u32,
    get_image: unsafe extern "system" fn(*mut ImageFactory, Size, u32, *mut Handle) -> i32,
}
const IID_IMAGE: Guid = Guid { a: 0xbcc18b79, b: 0xba16, c: 0x442f, d: [0x80,0xc4,0x8a,0x59,0xc3,0x0c,0x46,0x3b] };

#[link(name = "shell32")]
extern "system" {
    fn SHCreateItemFromParsingName(path: *const u16, context: Handle, iid: *const Guid, out: *mut *mut c_void) -> i32;
}
#[link(name = "user32")]
extern "system" {
    fn RegisterHotKey(hwnd: Handle, id: i32, modifiers: u32, key: u32) -> i32;
    fn UnregisterHotKey(hwnd: Handle, id: i32) -> i32;
    fn InvalidateRect(hwnd: Handle, rect: *const Rect, erase: i32) -> i32;
    fn FillRect(dc: Handle, rect: *const Rect, brush: Handle) -> i32;
    fn DrawTextW(dc: Handle, text: *const u16, count: i32, rect: *mut Rect, format: u32) -> i32;
    fn GetWindowTextW(hwnd: Handle, text: *mut u16, len: i32) -> i32;
    fn SetForegroundWindow(hwnd: Handle) -> i32;
}
#[link(name = "gdi32")]
extern "system" {
    fn CreateSolidBrush(color: u32) -> Handle;
    fn DeleteObject(object: Handle) -> i32;
    fn SetBkMode(dc: Handle, mode: i32) -> i32;
    fn SetTextColor(dc: Handle, color: u32) -> u32;
    fn SelectObject(dc: Handle, object: Handle) -> Handle;
    fn CreateCompatibleDC(dc: Handle) -> Handle;
    fn DeleteDC(dc: Handle) -> i32;
    fn GetObjectW(object: Handle, size: i32, buffer: *mut c_void) -> i32;
    fn SetStretchBltMode(dc: Handle, mode: i32) -> i32;
    fn StretchBlt(dest: Handle, x: i32, y: i32, w: i32, h: i32,
        source: Handle, sx: i32, sy: i32, sw: i32, sh: i32, rop: u32) -> i32;
    fn CreateDIBSection(dc: Handle, info: *const BitmapInfo, usage: u32, bits: *mut *mut c_void,
        section: Handle, offset: u32) -> Handle;
}
#[link(name = "kernel32")]
extern "system" { fn MoveFileExW(old: *const u16, new: *const u16, flags: u32) -> i32; }

struct Thumb { bitmap: isize, w: i32, h: i32 }
impl Drop for Thumb {
    fn drop(&mut self) { if self.bitmap != 0 { unsafe { DeleteObject(self.bitmap as Handle); } } }
}
struct ThumbResult { path: PathBuf, image: Thumb }

struct PickerCard { bitmap: isize, card: i32, skew: i32, height: i32 }
impl Drop for PickerCard {
    fn drop(&mut self) { if self.bitmap != 0 { unsafe { DeleteObject(self.bitmap as Handle); } } }
}

unsafe fn make_scene_thumb(path: &PathBuf) -> Thumb {
    let empty = || Thumb { bitmap: 0, w: 0, h: 0 };
    let Ok(scene) = scene_wallpaper::load(path.clone(), 1920, 1080) else { return empty(); };
    let frame = scene.frame;
    let info = BitmapInfo { header: BitmapInfoHeader {
        size: size_of::<BitmapInfoHeader>() as u32, width: frame.width, height: -frame.height,
        planes: 1, bit_count: 32, compression: 0, size_image: 0,
        x_pixels: 0, y_pixels: 0, colors_used: 0, colors_important: 0,
    }, colors: [0] };
    let mut bits: *mut c_void = null_mut();
    let bitmap = CreateDIBSection(null_mut(), &info, 0, &mut bits, null_mut(), 0);
    if bitmap.is_null() { return empty(); }
    if bits.is_null() { DeleteObject(bitmap); return empty(); }
    std::ptr::copy_nonoverlapping(frame.pixels.as_ptr(), bits as *mut u8, frame.pixels.len());
    Thumb { bitmap: bitmap as isize, w: frame.width, h: frame.height }
}

unsafe fn make_thumb(path: &PathBuf) -> Thumb {
    if path.extension().and_then(|x| x.to_str()).map_or(false, |x| x.eq_ignore_ascii_case("pkg")) {
        return make_scene_thumb(path);
    }
    let video = path.extension().and_then(|x| x.to_str()).map_or(false, |x| ["mp4", "m4v", "mov", "wmv", "avi", "mkv"].iter().any(|ext| x.eq_ignore_ascii_case(ext)));
    if video {
        if let Some(frame) = video_thumb::frame(path) { return frame; }
    }
    let mut image = Thumb { bitmap: 0, w: 0, h: 0 };
    let name: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut factory: *mut ImageFactory = null_mut();
    let hr = SHCreateItemFromParsingName(name.as_ptr(), null_mut(), &IID_IMAGE, &mut factory as *mut _ as *mut *mut c_void);
    if hr < 0 || factory.is_null() { return image; }
    let mut bitmap: Handle = null_mut();
    let hr = ((*(*factory).vtable).get_image)(factory, Size { cx: 1920, cy: 1080 }, 0x8, &mut bitmap);
    ((*(*factory).vtable).release)(factory);
    if hr >= 0 && !bitmap.is_null() {
        let mut data: Bitmap = zeroed();
        if GetObjectW(bitmap, size_of::<Bitmap>() as i32, &mut data as *mut _ as *mut c_void) > 0 && data.width > 0 && data.height > 0 {
            image = Thumb { bitmap: bitmap as isize, w: data.width, h: data.height };
        } else { DeleteObject(bitmap); }
    }
    image
}

fn worker() -> (SyncSender<PathBuf>, Receiver<ThumbResult>) {
    let (tx, requests) = mpsc::sync_channel::<PathBuf>(8);
    let (results, rx) = mpsc::sync_channel::<ThumbResult>(8);
    thread::spawn(move || unsafe {
        let initialized = CoInitializeEx(null_mut(), 0) >= 0;
        let mf_ready = initialized && MFStartup(0x00020070, 0) >= 0;
        while let Ok(path) = requests.recv() {
            let image = if initialized { make_thumb(&path) } else { Thumb { bitmap: 0, w: 0, h: 0 } };
            if results.send(ThumbResult { path, image }).is_err() { break; }
        }
        if mf_ready { MFShutdown(); }
        if initialized { CoUninitialize(); }
    });
    (tx, rx)
}

fn config_path() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA").map(PathBuf::from).map(|p| p.join("rustwallpapr").join("library.json"))
}
fn load() -> Result<Stored, String> {
    let path = config_path().ok_or("Не найдена папка LOCALAPPDATA")?;
    if !path.exists() { return Ok(Stored::default()); }
    let data = std::fs::read(&path).map_err(|e| format!("Не удалось прочитать библиотеку: {e}"))?;
    serde_json::from_slice(&data).map_err(|e| format!("Библиотека повреждена; она не будет перезаписана: {e}"))
}
fn save(stored: &Stored) -> Result<(), String> {
    let path = config_path().ok_or("Не найдена папка LOCALAPPDATA")?;
    let folder = path.parent().ok_or("Неверный путь библиотеки")?;
    std::fs::create_dir_all(folder).map_err(|e| e.to_string())?;
    let temp = folder.join(format!("library-{}.tmp", std::process::id()));
    let data = serde_json::to_vec_pretty(stored).map_err(|e| e.to_string())?;
    std::fs::write(&temp, data).map_err(|e| e.to_string())?;
    let from: Vec<u16> = temp.as_os_str().encode_wide().chain(Some(0)).collect();
    let to: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    if unsafe { MoveFileExW(from.as_ptr(), to.as_ptr(), 1 | 8) } == 0 {
        let error = std::io::Error::last_os_error();
        let _ = std::fs::remove_file(&temp);
        return Err(format!("Не удалось сохранить библиотеку: {error}"));
    }
    Ok(())
}

fn parse_hotkey(text: &str) -> Option<(u32, u32)> {
    let mut modifiers = 0u32;
    let mut key = None;
    for part in text.split('+').map(|x| x.trim().to_ascii_uppercase()) {
        match part.as_str() {
            "ALT" => modifiers |= 1,
            "CTRL" | "CONTROL" => modifiers |= 2,
            "SHIFT" => modifiers |= 4,
            _ if part.len() == 1 && part.as_bytes()[0].is_ascii_alphanumeric() && key.is_none() => key = Some(part.as_bytes()[0] as u32),
            _ => return None,
        }
    }
    if modifiers == 0 { return None; }
    Some((modifiers | 0x4000, key?))
}


#[link(name = "gdi32")]
extern "system" {
    fn CreateCompatibleBitmap(dc: Handle, width: i32, height: i32) -> Handle;
    fn BitBlt(dest: Handle, x: i32, y: i32, width: i32, height: i32, src: Handle, sx: i32, sy: i32, rop: u32) -> i32;
    fn PlgBlt(dest: Handle, points: *const Point, src: Handle, x: i32, y: i32, width: i32, height: i32, mask: Handle, mx: i32, my: i32) -> i32;
    fn CreatePolygonRgn(points: *const Point, count: i32, mode: i32) -> Handle;
    fn ExtSelectClipRgn(dc: Handle, region: Handle, mode: i32) -> i32;
    fn SaveDC(dc: Handle) -> i32;
    fn RestoreDC(dc: Handle, saved: i32) -> i32;
    fn CreatePen(style: i32, width: i32, color: u32) -> Handle;
    fn Polygon(dc: Handle, points: *const Point, count: i32) -> i32;
    fn RoundRect(dc: Handle, left: i32, top: i32, right: i32, bottom: i32, rx: i32, ry: i32) -> i32;
    fn MoveToEx(dc: Handle, x: i32, y: i32, old: *mut Point) -> i32;
    fn LineTo(dc: Handle, x: i32, y: i32) -> i32;
    fn CreateFontW(height: i32, width: i32, escapement: i32, orientation: i32, weight: i32,
        italic: u32, underline: u32, strike: u32, charset: u32, output_precision: u32,
        clip_precision: u32, quality: u32, pitch: u32, face: *const u16) -> Handle;
}

const INK: u32 = 0x00f3ece8;
const MUTED: u32 = 0x00aaa098;
const BACKGROUND: u32 = 0x001d1714;
const SURFACE: u32 = 0x00332a25;
const EDGE: u32 = 0x0053473b;
const ACCENT: u32 = 0x00eacaa6;

#[derive(Clone, Copy)]
pub(super) enum Action { None, Add, Apply, Stop, Remove, Collection, Settings, Optimization, Properties, Help, TransitionPage, OpenTelegram, OpenGithub, ImportJson, Property(usize, i32), PropertyPage(i32), Quality(u32), Fps(u32), ToggleAutostart, Transition(u8), TransitionDuration, ScreenMode(u8), AssignSecond, ToggleSound, ToggleScripts, Hide, Exit, Capture, Select(usize), Previous, Next }

pub(super) struct Gallery {
    stored: Stored,
    writable: bool,
    load_error: Option<String>,
    main: Handle,
    selected: usize,
    hotkey_id: Option<i32>,
    tx: SyncSender<PathBuf>,
    rx: Receiver<ThumbResult>,
    pending: HashSet<PathBuf>,
    thumbs: HashMap<PathBuf, Thumb>,
    picker_cards: HashMap<PathBuf, PickerCard>,
    pub(super) settings: bool,
    pub(super) optimization: bool,
    pub(super) properties: bool,
    pub(super) help: bool,
    pub(super) transition_page: bool,
    duration_draft: Option<u32>,
    pub(super) autostart: bool,
    fps_draft: Option<u32>,
    scroll_offset: f32,
    scroll_time: Option<Instant>,
    property_page: usize,
    project: Option<LoadedProject>,
    pub(super) scripts: bool,
    pub(super) capturing: bool,
}

impl Gallery {
    pub(super) fn new(main: Handle) -> Self {
        let (stored, writable, load_error) = match load() {
            Ok(s) => (s, true, None),
            Err(e) => (Stored::default(), false, Some(e)),
        };
        let (tx, rx) = worker();
        let mut gallery = Self { stored, writable, load_error, main, selected: 0, hotkey_id: None, tx, rx,
            pending: HashSet::new(), thumbs: HashMap::new(), picker_cards: HashMap::new(), settings: false, optimization: false, properties: false, help: false, transition_page: false, duration_draft: None, autostart: super::startup::is_enabled().unwrap_or(false), fps_draft: None, scroll_offset: 0.0, scroll_time: None, property_page: 0, project: None, scripts: false, capturing: false };
        gallery.refresh_project();
        gallery
    }
    pub(super) fn load_error(&self) -> Option<&str> { self.load_error.as_deref() }
    pub(super) fn scene_height(&self) -> u32 {
        if [480, 720, 1080, 1440, 2160].contains(&self.stored.scene_height) { self.stored.scene_height } else { default_quality() }
    }
    pub(super) fn transition(&self) -> u8 { if self.stored.transition <= 2 { self.stored.transition } else { default_transition() } }
    pub(super) fn set_transition(&mut self, mode: u8) -> Result<(), String> {
        if mode > 2 { return Err("Неизвестный эффект перехода.".into()); }
        let old = self.stored.transition;
        self.stored.transition = mode;
        if let Err(error) = self.persist() { self.stored.transition = old; return Err(error); }
        Ok(())
    }
    pub(super) fn transition_ms(&self) -> u32 {
        if (100..=3000).contains(&self.stored.transition_ms) { self.stored.transition_ms } else { default_transition_ms() }
    }
    pub(super) fn set_transition_ms(&mut self, ms: u32) -> Result<(), String> {
        if !(100..=3000).contains(&ms) { return Err("Длительность должна быть от 0,1 до 3 секунд.".into()); }
        let old = self.stored.transition_ms;
        self.stored.transition_ms = ms;
        if let Err(error) = self.persist() { self.stored.transition_ms = old; return Err(error); }
        Ok(())
    }
    pub(super) fn duration_display(&self) -> u32 { self.duration_draft.unwrap_or_else(|| self.transition_ms()) }
    pub(super) fn duration_dragging(&self) -> bool { self.duration_draft.is_some() }
    pub(super) fn duration_drag(&mut self, width: i32, x: i32) {
        let (left, right) = Self::fps_slider_limits(width);
        let pos = (x-left).clamp(0, right-left);
        let steps = (pos * 58 + (right-left)/2) / (right-left);
        self.duration_draft = Some(100 + steps as u32 * 50);
    }
    pub(super) fn duration_finish(&mut self) -> Option<u32> { self.duration_draft.take() }
    pub(super) fn scene_fps(&self) -> u32 {
        if (1..=120).contains(&self.stored.scene_fps) { self.stored.scene_fps } else { default_fps() }
    }
    fn fps_slider_limits(width: i32) -> (i32, i32) { (240, (width - 100).min(740).max(359)) }
    pub(super) fn fps_from_x(width: i32, x: i32) -> u32 {
        let (left, right) = Self::fps_slider_limits(width);
        let pos = (x - left).clamp(0, right - left);
        1 + ((pos * 119 + (right-left)/2) / (right-left)) as u32
    }
    pub(super) fn fps_display(&self) -> u32 { self.fps_draft.unwrap_or_else(|| self.scene_fps()) }
    pub(super) fn fps_dragging(&self) -> bool { self.fps_draft.is_some() }
    pub(super) fn fps_drag(&mut self, width: i32, x: i32) { self.fps_draft = Some(Self::fps_from_x(width, x)); }
    pub(super) fn fps_finish(&mut self) -> Option<u32> { self.fps_draft.take() }
    pub(super) fn set_scene_fps(&mut self, fps: u32) -> Result<(), String> {
        if !(1..=120).contains(&fps) { return Err("Частота должна быть от 1 до 120 FPS.".into()); }
        let old = self.stored.scene_fps;
        self.stored.scene_fps = fps;
        if let Err(error) = self.persist() { self.stored.scene_fps = old; return Err(error); }
        Ok(())
    }
    pub(super) fn set_scene_height(&mut self, height: u32) -> Result<(), String> {
        if ![480, 720, 1080, 1440, 2160].contains(&height) { return Err("Неизвестное качество.".into()); }
        let old = self.stored.scene_height;
        self.stored.scene_height = height;
        if let Err(e) = self.persist() { self.stored.scene_height = old; return Err(e); }
        Ok(())
    }
    pub(super) fn screen_mode(&self) -> u8 { self.stored.screen_mode.filter(|x| *x <= 3).unwrap_or(if self.stored.span { 1 } else { 0 }) }
    pub(super) fn set_screen_mode(&mut self, mode: u8) -> Result<(), String> {
        if mode > 3 { return Err("Неизвестный режим экранов.".into()); }
        let old = self.stored.screen_mode;
        self.stored.screen_mode = Some(mode);
        if let Err(error) = self.persist() { self.stored.screen_mode = old; return Err(error); }
        Ok(())
    }
    pub(super) fn second_wallpaper(&self) -> Option<PathBuf> { self.stored.second_wallpaper.clone() }
    pub(super) fn assign_second(&mut self) -> Result<(), String> {
        let selected = self.current().ok_or("Сначала выберите PKG.")?;
        if !selected.extension().and_then(|s|s.to_str()).map_or(false, |s|s.eq_ignore_ascii_case("pkg")) { return Err("Второй экран пока поддерживает только PKG-сцены.".into()); }
        let old = self.stored.second_wallpaper.replace(selected);
        if let Err(error) = self.persist() { self.stored.second_wallpaper = old; return Err(error); }
        Ok(())
    }
    fn read_project(path: &PathBuf) -> Result<serde_json::Value, String> {
        use std::io::Read;
        let file = std::fs::File::open(path).map_err(|e| format!("Не удалось открыть JSON: {e}"))?;
        let mut bytes = Vec::new();
        file.take(2 * 1024 * 1024 + 1).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
        if bytes.len() > 2 * 1024 * 1024 { return Err("JSON больше 2 МБ.".into()); }
        let doc: serde_json::Value = serde_json::from_slice(&bytes).map_err(|e| format!("Некорректный JSON: {e}"))?;
        if !doc.is_object() { return Err("JSON должен быть объектом проекта.".into()); }
        if let Some(kind) = doc.get("type").and_then(|v| v.as_str()) {
            if kind != "scene" { return Err("Это не проект сцены Wallpaper Engine.".into()); }
        }
        if !doc.pointer("/general/properties").map_or(false, |p| p.is_object()) {
            return Err("В JSON нет general.properties — настроек сцены не найдено.".into());
        }
        Ok(doc)
    }
    fn refresh_project(&mut self) {
        self.project = None;
        self.property_page = 0;
        let Some(wallpaper) = self.current() else { return; };
        let Some(link) = self.stored.projects.get(&wallpaper) else { return; };
        let Ok(json) = Self::read_project(&link.path) else { return; };
        let Some(defs) = json.pointer("/general/properties").and_then(|x| x.as_object()) else { return; };
        let mut properties = Vec::new();
        for (key, def) in defs.iter().take(128) {
            let kind = def.get("type").and_then(|v| v.as_str()).unwrap_or("").to_ascii_lowercase();
            if !["bool", "checkbox", "slider", "combo"].contains(&kind.as_str()) { continue; }
            let value = link.overrides.get(key).cloned().or_else(|| def.get("value").cloned()).unwrap_or(serde_json::Value::Null);
            let label = def.get("text").or_else(|| def.get("label")).and_then(|x| x.as_str()).unwrap_or(key).chars().take(70).collect();
            properties.push(SceneProperty { key: key.clone(), label, kind, value, definition: def.clone() });
        }
        self.project = Some(LoadedProject { wallpaper, json, properties });
    }
    pub(super) fn import_project(&mut self, path: PathBuf) -> Result<(), String> {
        let wallpaper = self.current().ok_or("Выберите PKG-обои.")?;
        if !wallpaper.extension().and_then(|v| v.to_str()).map_or(false, |v| v.eq_ignore_ascii_case("pkg")) { return Err("JSON можно привязать только к выбранной PKG-сцене.".into()); }
        Self::read_project(&path)?;
        let old = self.stored.projects.insert(wallpaper.clone(), ProjectLink { path, overrides: HashMap::new() });
        if let Err(e) = self.persist() {
            if let Some(old) = old { self.stored.projects.insert(wallpaper, old); } else { self.stored.projects.remove(&wallpaper); }
            return Err(e);
        }
        self.refresh_project();
        Ok(())
    }
    pub(super) fn project_payload(&self, path: &PathBuf) -> (Option<serde_json::Value>, serde_json::Value) {
        let Some(link) = self.stored.projects.get(path) else { return (None, serde_json::json!({})); };
        let project = self.project.as_ref().filter(|p| &p.wallpaper == path).map(|p| p.json.clone()).or_else(|| Self::read_project(&link.path).ok());
        (project, serde_json::json!(link.overrides))
    }
    pub(super) fn property_count(&self) -> usize { self.project.as_ref().map_or(0, |p| p.properties.len()) }
    pub(super) fn change_property(&mut self, position: usize, delta: i32) -> Result<(), String> {
        let index = self.property_page * 4 + position;
        let project = self.project.as_ref().ok_or("Сначала добавьте JSON.")?;
        let property = project.properties.get(index).ok_or("Параметр не найден.")?;
        let value = match property.kind.as_str() {
            "bool" | "checkbox" => serde_json::json!(!property.value.as_bool().unwrap_or(false)),
            "slider" => {
                let min = property.definition.get("min").and_then(|v| v.as_f64()).unwrap_or(0.0);
                let max = property.definition.get("max").and_then(|v| v.as_f64()).unwrap_or(100.0);
                if !min.is_finite() || !max.is_finite() || max <= min { return Err("Неверный диапазон параметра.".into()); }
                let step = property.definition.get("step").and_then(|v| v.as_f64()).filter(|v| *v > 0.0 && v.is_finite()).unwrap_or((max-min)/100.0);
                let current = property.value.as_f64().unwrap_or(min);
                serde_json::json!((current + step * delta as f64).clamp(min,max))
            }
            "combo" => {
                let options = property.definition.get("options").and_then(|v| v.as_array()).ok_or("У этого списка нет вариантов в JSON.")?;
                if options.is_empty() { return Err("Список вариантов пуст.".into()); }
                let vals: Vec<_> = options.iter().map(|v| v.get("value").unwrap_or(v)).filter(|v| v.is_string() || v.is_number() || v.is_boolean()).collect();
                if vals.is_empty() { return Err("Формат вариантов не поддерживается.".into()); }
                let current = vals.iter().position(|v| **v == property.value).unwrap_or(0);
                vals[(current as i32 + delta).rem_euclid(vals.len() as i32) as usize].clone()
            }
            _ => return Err("Тип параметра не поддерживается.".into()),
        };
        let wallpaper = project.wallpaper.clone(); let key = property.key.clone();
        let link = self.stored.projects.get_mut(&wallpaper).ok_or("JSON не найден.")?;
        let old = link.overrides.insert(key.clone(), value.clone());
        if let Err(e) = self.persist() {
            let link = self.stored.projects.get_mut(&wallpaper).unwrap();
            if let Some(old) = old { link.overrides.insert(key, old); } else { link.overrides.remove(&key); }
            return Err(e);
        }
        if let Some(p) = self.project.as_mut().and_then(|p| p.properties.get_mut(index)) { p.value = value; }
        Ok(())
    }
    pub(super) fn property_page(&mut self, delta: i32) {
        let last = self.property_count().saturating_sub(1) / 4;
        self.property_page = (self.property_page as i32 + delta).clamp(0, last as i32) as usize;
    }
    pub(super) fn sound(&self) -> bool { self.stored.sound }
    pub(super) fn toggle_sound(&mut self) -> Result<(), String> {
        self.stored.sound = !self.stored.sound;
        if let Err(error) = self.persist() { self.stored.sound = !self.stored.sound; return Err(error); }
        Ok(())
    }
    pub(super) fn shortcut(&self) -> &str { &self.stored.hotkey }
    pub(super) fn has_hotkey(&self) -> bool { self.hotkey_id.is_some() }
    pub(super) fn current(&self) -> Option<PathBuf> { self.stored.videos.get(self.selected).cloned() }
    pub(super) fn is_empty(&self) -> bool { self.stored.videos.is_empty() }
    pub(super) fn picker_hit(&self, width: i32, height: i32, x: i32, y: i32) -> Option<usize> {
        if y < 0 || y >= height || self.is_empty() { return None; }
        let card = (width / 5).clamp(220, 420);
        let step = card + 12;
        let skew = (card / 5).clamp(40, 84);
        let shift = skew * (height-y) / height.max(1);
        for slot in (-8i32..=8).rev() {
            let left = width/2-card/2-skew/2+slot*step+shift+(self.scroll_offset * step as f32).round() as i32;
            if x >= left && x < left+card {
                return Some((self.selected as i32 + slot).rem_euclid(self.stored.videos.len() as i32) as usize);
            }
        }
        None
    }
    fn persist(&self) -> Result<(), String> {
        if !self.writable { return Err("Библиотека не сохраняется: файл не удалось прочитать.".into()); }
        save(&self.stored)
    }
    pub(super) fn add(&mut self, path: PathBuf) -> Result<(), String> {
        if let Some(index) = self.stored.videos.iter().position(|p| p == &path) {
            self.selected = index;
        } else {
            self.stored.videos.push(path);
            if let Err(e) = self.persist() { self.stored.videos.pop(); return Err(e); }
            self.selected = self.stored.videos.len() - 1;
        }
        self.refresh_project();
        self.prepare();
        Ok(())
    }
    pub(super) fn remove_selected(&mut self) -> Result<Option<PathBuf>, String> {
        if self.stored.videos.is_empty() { return Ok(None); }
        let index = self.selected;
        let path = self.stored.videos.remove(index);
        if let Err(error) = self.persist() {
            self.stored.videos.insert(index, path);
            return Err(error);
        }
        self.thumbs.remove(&path);
        self.picker_cards.remove(&path);
        self.selected = index.min(self.stored.videos.len().saturating_sub(1));
        self.refresh_project();
        self.prepare();
        Ok(Some(path))
    }
    pub(super) unsafe fn register_initial(&mut self) -> bool {
        let Some((mods, key)) = parse_hotkey(&self.stored.hotkey) else { return false; };
        if RegisterHotKey(self.main, ID_HOTKEY_A, mods, key) == 0 { return false; }
        self.hotkey_id = Some(ID_HOTKEY_A); true
    }
    pub(super) unsafe fn change_hotkey(&mut self, input: &str) -> Result<(), String> {
        let (mods, key) = parse_hotkey(input).ok_or("Нужно сочетание Alt, Ctrl или Shift с буквой или цифрой.")?;
        if parse_hotkey(&self.stored.hotkey) == Some((mods, key)) && self.hotkey_id.is_some() { return Ok(()); }
        let next = if self.hotkey_id == Some(ID_HOTKEY_A) { ID_HOTKEY_B } else { ID_HOTKEY_A };
        if RegisterHotKey(self.main, next, mods, key) == 0 { return Err("Это сочетание уже занято другой программой.".into()); }
        let previous = self.stored.hotkey.clone();
        self.stored.hotkey = input.to_string();
        if let Err(error) = self.persist() {
            self.stored.hotkey = previous;
            UnregisterHotKey(self.main, next);
            return Err(error);
        }
        if let Some(old) = self.hotkey_id.take() { UnregisterHotKey(self.main, old); }
        self.hotkey_id = Some(next);
        Ok(())
    }
    pub(super) fn handles_hotkey(&self, id: usize) -> bool { self.hotkey_id == Some(id as i32) }
    pub(super) fn select(&mut self, i: usize) {
        if i < self.stored.videos.len() { self.selected = i; self.refresh_project(); self.prepare(); }
    }
    pub(super) fn move_by(&mut self, amount: i32) {
        if self.stored.videos.is_empty() { return; }
        self.selected = (self.selected as i64 + amount as i64).rem_euclid(self.stored.videos.len() as i64) as usize;
        self.refresh_project();
        self.prepare();
    }
    pub(super) fn is_picker_scrolling(&self) -> bool { self.scroll_time.is_some() }
    pub(super) fn scroll_picker(&mut self, amount: i32) {
        if self.is_empty() { return; }
        self.move_by(amount);
        self.scroll_offset = (self.scroll_offset + amount as f32).clamp(-2.0, 2.0);
        self.scroll_time = Some(Instant::now());
    }
    pub(super) fn tick_picker(&mut self) -> bool {
        let Some(previous) = self.scroll_time else { return false; };
        let now = Instant::now();
        let elapsed = now.duration_since(previous).as_secs_f32().min(0.25);
        self.scroll_time = Some(now);
        self.scroll_offset *= (-12.0 * elapsed).exp();
        if self.scroll_offset.abs() < 0.003 {
            self.reset_picker_scroll();
            false
        } else { true }
    }
    pub(super) fn reset_picker_scroll(&mut self) {
        self.scroll_offset = 0.0;
        self.scroll_time = None;
    }
    pub(super) fn prepare(&mut self) {
        if self.stored.videos.is_empty() { return; }
        for offset in -5i32..=5 {
            let index = (self.selected as i32 + offset).rem_euclid(self.stored.videos.len() as i32) as usize;
            let path = &self.stored.videos[index];
            if !self.thumbs.contains_key(path) && !self.pending.contains(path) && self.tx.try_send(path.clone()).is_ok() {
                self.pending.insert(path.clone());
            }
        }
    }
    pub(super) unsafe fn poll(&mut self, picker: Handle) {
        let mut changed = false;
        while let Ok(result) = self.rx.try_recv() {
            self.pending.remove(&result.path);
            if !self.stored.videos.contains(&result.path) { continue; }
            if self.thumbs.len() >= 10 {
                let visible: HashSet<_> = if self.stored.videos.is_empty() { HashSet::new() } else {
                    (-5i32..=5).map(|offset| {
                        let i = (self.selected as i32 + offset).rem_euclid(self.stored.videos.len() as i32) as usize;
                        self.stored.videos[i].clone()
                    }).collect()
                };
                if let Some(key) = self.thumbs.keys().find(|key| !visible.contains(*key)).cloned() {
                    self.thumbs.remove(&key);
                    self.picker_cards.remove(&key);
                }
            }
            self.picker_cards.remove(&result.path);
            self.thumbs.insert(result.path, result.image);
            changed = true;
        }
        if IsWindowVisible(self.main) != 0 || (!picker.is_null() && IsWindowVisible(picker) != 0) {
            self.prepare();
            if changed {
                InvalidateRect(self.main, null(), 0);
                if !picker.is_null() { InvalidateRect(picker, null(), 0); }
            }
        }
    }
    pub(super) unsafe fn close(&mut self) {
        if let Some(id) = self.hotkey_id.take() { UnregisterHotKey(self.main, id); }
    }

    pub(super) fn hit(&self, width: i32, height: i32, x: i32, y: i32) -> Action {
        if x >= 16 && x < 180 {
            if (110..154).contains(&y) { return Action::Collection; }
            if (162..206).contains(&y) { return Action::Settings; }
            if (214..258).contains(&y) { return Action::Optimization; }
            if (266..310).contains(&y) { return Action::Properties; }
            if (318..362).contains(&y) { return Action::Help; }
            if (370..414).contains(&y) { return Action::TransitionPage; }
            if (height-105..height-66).contains(&y) { return Action::Hide; }
            if (height-57..height-18).contains(&y) { return Action::Exit; }
        }
        if self.transition_page {
            if (160..205).contains(&y) {
                if (228..380).contains(&x) { return Action::Transition(0); }
                if (390..542).contains(&x) { return Action::Transition(1); }
                if (552..704).contains(&x) { return Action::Transition(2); }
            }
            let (left, right) = Self::fps_slider_limits(width);
            if (308..344).contains(&y) && (left-12..=right+12).contains(&x) { return Action::TransitionDuration; }
            return Action::None;
        }
        if self.help {
            if (350..394).contains(&y) && (228..400).contains(&x) { return Action::OpenTelegram; }
            if (350..394).contains(&y) && (412..584).contains(&x) { return Action::OpenGithub; }
            return Action::None;
        }
        if self.properties {
            if (120..162).contains(&y) && (228..510).contains(&x) { return Action::ImportJson; }
            for i in 0..4 {
                let top = 182 + i * 66;
                if (top..top+48).contains(&y) {
                    if (width-210..width-160).contains(&x) { return Action::Property(i as usize, -1); }
                    if (width-88..width-38).contains(&x) { return Action::Property(i as usize, 1); }
                }
            }
            if (462..498).contains(&y) && (228..280).contains(&x) { return Action::PropertyPage(-1); }
            if (462..498).contains(&y) && (290..342).contains(&x) { return Action::PropertyPage(1); }
            return Action::None;
        }
        if self.optimization {
            if (190..240).contains(&y) {
                for (index, quality) in [480, 720, 1080, 1440, 2160].iter().enumerate() {
                    let left = 228 + index as i32 * 108;
                    if (left..left + 96).contains(&x) { return Action::Quality(*quality); }
                }
            }
            if (308..344).contains(&y) {
                let (left, right) = Self::fps_slider_limits(width);
                if (left-12..=right+12).contains(&x) { return Action::Fps(Self::fps_from_x(width, x)); }
            }
            if (405..450).contains(&y) {
                if (228..448).contains(&x) { return Action::ScreenMode(0); }
                if (460..740).contains(&x) { return Action::ScreenMode(1); }
            }
            if (462..507).contains(&y) {
                if (228..448).contains(&x) { return Action::ScreenMode(2); }
                if (460..740).contains(&x) { return Action::ScreenMode(3); }
            }
            return Action::None;
        }
        if self.settings {
            if x >= 228 && x < (width-24).min(760) && (170..215).contains(&y) {
                return Action::Capture;
            }
            if x >= 228 && x < (width-24).min(760) && (250..290).contains(&y) { return Action::ToggleSound; }
            if x >= 228 && x < (width-24).min(760) && (300..340).contains(&y) { return Action::ToggleScripts; }
            if x >= 228 && x < (width-24).min(760) && (380..425).contains(&y) { return Action::ToggleAutostart; }
            if x >= 228 && x < (width-24).min(760) && (466..508).contains(&y) { return Action::TransitionPage; }
            return Action::None;
        }
        let side = width - 240;
        if x >= side && x < width-24 {
            if (228..270).contains(&y) { return Action::Add; }
            if (280..322).contains(&y) { return Action::Apply; }
            if (332..374).contains(&y) { return Action::Stop; }
            if (384..426).contains(&y) { return Action::Remove; }
            if (436..478).contains(&y) { return Action::AssignSecond; }
        }
        let base = height-135;
        if y >= base && y < base+36 {
            if x >= 228 && x < 268 { return Action::Previous; }
            if x >= 276 && x < 316 { return Action::Next; }
        }
        Action::None
    }
}

fn metrics(width: i32, height: i32) -> (i32, i32, i32, i32, i32) {
    let card = (width / 5).clamp(185, 310);
    (70, (height - 115).max(170), card, card + 12, (card / 5).clamp(35, 62))
}
unsafe fn fill(dc: Handle, rect: Rect, color: u32) {
    let brush = CreateSolidBrush(color);
    if !brush.is_null() { FillRect(dc, &rect, brush); DeleteObject(brush); }
}
unsafe fn label(dc: Handle, text: &str, mut rect: Rect, color: u32, flags: u32) {
    let chars = wide(text); SetTextColor(dc, color); SetBkMode(dc, 1);
    DrawTextW(dc, chars.as_ptr(), -1, &mut rect, flags | 0x20 | 0x8000);
}
unsafe fn button(dc: Handle, text: &str, rect: Rect, active: bool) {
    let brush = CreateSolidBrush(if active { ACCENT } else { SURFACE });
    if !brush.is_null() {
        let old_brush = SelectObject(dc, brush);
        let old_pen = SelectObject(dc, GetStockObject(8));
        RoundRect(dc, rect.left, rect.top, rect.right, rect.bottom, 10, 10);
        SelectObject(dc, old_pen); SelectObject(dc, old_brush); DeleteObject(brush);
    }
    label(dc, text, Rect { left:rect.left+10, top:rect.top, right:rect.right-10, bottom:rect.bottom },
        if active { BACKGROUND } else { INK }, 0x1 | 0x4);
}
unsafe fn outline(dc: Handle, points: [Point; 4], color: u32, size: i32) {
    let pen = CreatePen(0, size, color);
    if pen.is_null() { return; }
    let old = SelectObject(dc, pen);
    MoveToEx(dc, points[0].x, points[0].y, null_mut());
    for point in points.iter().skip(1).chain(std::iter::once(&points[0])) { LineTo(dc, point.x, point.y); }
    SelectObject(dc, old); DeleteObject(pen);
}
unsafe fn draw_card(dc: Handle, gallery: &Gallery, index: usize, width: i32, height: i32) {
    let (top, bottom, card, step, skew) = metrics(width,height);
    draw_card_area(dc, gallery, index, index as i32-gallery.selected as i32, width, top, bottom, card, step, skew);
}
unsafe fn draw_card_area(dc: Handle, gallery: &Gallery, index: usize, slot: i32, width: i32, top: i32, bottom: i32, card: i32, step: i32, skew: i32) {
    let x = width/2 - card/2 - skew/2 + slot*step + (gallery.scroll_offset * step as f32).round() as i32;
    if x + card + skew < 0 || x > width { return; }
    let p = [Point{x:x+skew,y:top}, Point{x:x+skew+card,y:top}, Point{x,y:bottom}, Point{x:x+card,y:bottom}];
    let frame = [p[0],p[1],p[3],p[2]];
    let brush = CreateSolidBrush(SURFACE);
    if !brush.is_null() {
        let previous_brush = SelectObject(dc, brush);
        let previous_pen = SelectObject(dc, GetStockObject(8));
        Polygon(dc, frame.as_ptr(), 4);
        SelectObject(dc, previous_pen); SelectObject(dc, previous_brush); DeleteObject(brush);
    }
    let path = &gallery.stored.videos[index];
    if let Some(thumb) = gallery.thumbs.get(path) {
        if thumb.bitmap != 0 && thumb.w > 0 && thumb.h > 0 {
            let mem = CreateCompatibleDC(dc);
            if !mem.is_null() {
                let old = SelectObject(mem, thumb.bitmap as Handle);
                let dest_ratio = card as f32 / (bottom-top) as f32;
                let src_ratio = thumb.w as f32 / thumb.h as f32;
                let (sx, sy, sw, sh) = if src_ratio > dest_ratio {
                    let sw = ((thumb.h as f32 * dest_ratio) as i32).max(1).min(thumb.w);
                    ((thumb.w-sw)/2, 0, sw, thumb.h)
                } else {
                    let sh = ((thumb.w as f32 / dest_ratio) as i32).max(1).min(thumb.h);
                    (0, (thumb.h-sh)/2, thumb.w, sh)
                };
                SetStretchBltMode(dc, 4);
                PlgBlt(dc, p.as_ptr(), mem, sx, sy, sw, sh, null_mut(), 0, 0);
                SelectObject(mem, old); DeleteDC(mem);
            }
        }
    }
    outline(dc, frame, EDGE, 1);
}

unsafe fn rounded(dc: Handle, rect: Rect, color: u32, radius: i32) {
    let brush = CreateSolidBrush(color);
    if brush.is_null() { return; }
    let old_brush = SelectObject(dc, brush);
    let old_pen = SelectObject(dc, GetStockObject(8));
    RoundRect(dc, rect.left, rect.top, rect.right, rect.bottom, radius, radius);
    SelectObject(dc, old_pen); SelectObject(dc, old_brush); DeleteObject(brush);
}

unsafe fn preview(dc: Handle, gallery: &Gallery, rect: Rect) {
    fill(dc, Rect { left:rect.left, top:rect.top, right:rect.right, bottom:rect.bottom }, 0x0029211d);
    let Some(path) = gallery.stored.videos.get(gallery.selected) else { return; };
    let Some(thumb) = gallery.thumbs.get(path) else { return; };
    if thumb.bitmap == 0 || thumb.w <= 0 || thumb.h <= 0 { return; }
    let w = rect.right - rect.left; let h = rect.bottom - rect.top;
    if w <= 0 || h <= 0 { return; }
    let dest_ratio = w as f32 / h as f32;
    let src_ratio = thumb.w as f32 / thumb.h as f32;
    let (sx, sy, sw, sh) = if src_ratio > dest_ratio {
        let sw = ((thumb.h as f32 * dest_ratio) as i32).clamp(1, thumb.w);
        ((thumb.w-sw)/2, 0, sw, thumb.h)
    } else {
        let sh = ((thumb.w as f32 / dest_ratio) as i32).clamp(1, thumb.h);
        (0, (thumb.h-sh)/2, thumb.w, sh)
    };
    let mem = CreateCompatibleDC(dc);
    if !mem.is_null() {
        let old = SelectObject(mem, thumb.bitmap as Handle);
        SetStretchBltMode(dc, 4);
        StretchBlt(dc, rect.left, rect.top, w, h, mem, sx, sy, sw, sh, 0x00cc0020);
        SelectObject(mem, old); DeleteDC(mem);
    }
}

unsafe fn font(size: i32, weight: i32) -> Handle {
    CreateFontW(-size,0,0,0,weight,0,0,0,1,0,0,5,0,wide("Segoe UI").as_ptr())
}

pub(super) unsafe fn paint(hwnd: Handle, gallery: &Gallery, status: &str) {
    let mut ps: PaintStruct = zeroed();
    let real_dc = BeginPaint(hwnd, &mut ps);
    if real_dc.is_null() { EndPaint(hwnd, &ps); return; }
    let mut client = Rect::default(); GetClientRect(hwnd, &mut client);
    let width = client.right.max(1); let height = client.bottom.max(1);
    let mem = CreateCompatibleDC(real_dc);
    let bitmap = if !mem.is_null() { CreateCompatibleBitmap(real_dc, width, height) } else { null_mut() };
    let dc = if !bitmap.is_null() { mem } else { real_dc };
    let old_bitmap = if !bitmap.is_null() { SelectObject(mem, bitmap) } else { null_mut() };
    fill(dc, Rect { left:0,top:0,right:width,bottom:height }, 0x00201a17);
    fill(dc, Rect { left:0,top:0,right:196,bottom:height }, 0x00191412);
    fill(dc, Rect { left:195,top:0,right:196,bottom:height }, 0x00372d29);
    let small = font(14, 400); let normal = font(16, 400);
    let semibold = font(16, 600); let big = font(26, 600);
    let old_font = if !normal.is_null() { SelectObject(dc, normal) } else { null_mut() };
    if !semibold.is_null() { SelectObject(dc, semibold); }
    label(dc, APP_NAME, Rect { left:22,top:25,right:180,bottom:52 }, INK, 0x4);
    if !normal.is_null() { SelectObject(dc, normal); }
    let nav_color = 0x00362a23;
    let nav_y = if gallery.transition_page { 370 } else if gallery.help { 318 } else if gallery.properties { 266 } else if gallery.optimization { 214 } else if gallery.settings { 162 } else { 110 };
    rounded(dc, Rect{left:16,top:nav_y,right:180,bottom:nav_y+44}, nav_color, 12);
    label(dc,"Коллекция",Rect{left:30,top:110,right:178,bottom:154},INK,0x4);
    label(dc,"Настройки",Rect{left:30,top:162,right:178,bottom:206},INK,0x4);
    label(dc,"Оптимизация",Rect{left:30,top:214,right:178,bottom:258},INK,0x4);
    label(dc,"Параметры PKG",Rect{left:30,top:266,right:178,bottom:310},INK,0x4);
    label(dc,"Справочник",Rect{left:30,top:318,right:178,bottom:362},INK,0x4);
    label(dc,"Переходы",Rect{left:30,top:370,right:178,bottom:414},INK,0x4);
    button(dc,"В трей",Rect{left:16,top:height-105,right:180,bottom:height-66},false);
    button(dc,"Выход",Rect{left:16,top:height-57,right:180,bottom:height-18},false);
    if !big.is_null() { SelectObject(dc, big); }
    label(dc,if gallery.transition_page {"Переходы"} else if gallery.help {"Справочник"} else if gallery.properties {"Параметры обоев"} else if gallery.optimization {"Оптимизация"} else if gallery.settings {"Настройки"} else {"Мои обои"},Rect{left:228,top:25,right:width-24,bottom:61},INK,0x4);
    if !small.is_null() { SelectObject(dc, small); }
    label(dc,if gallery.transition_page {"Настройка смены PKG-обоев"} else if gallery.help {"О проекте и авторе"} else if gallery.properties {"Настройки выбранной PKG-сцены"} else if gallery.optimization {"Качество сцен PKG"} else if gallery.settings {"Управление быстрым выбором"} else {"Ваша библиотека обоев"},Rect{left:230,top:66,right:width-24,bottom:89},MUTED,0);
    if !normal.is_null() { SelectObject(dc, normal); }
    if gallery.transition_page {
        if !semibold.is_null() { SelectObject(dc, semibold); }
        label(dc,"Эффект смены обоев",Rect{left:228,top:120,right:width-24,bottom:150},INK,0x4);
        if !normal.is_null() { SelectObject(dc, normal); }
        button(dc,"Без анимации",Rect{left:228,top:160,right:380,bottom:205},gallery.transition() == 0);
        button(dc,"Свайп",Rect{left:390,top:160,right:542,bottom:205},gallery.transition() == 1);
        button(dc,"Круг",Rect{left:552,top:160,right:704,bottom:205},gallery.transition() == 2);
        if !semibold.is_null() { SelectObject(dc, semibold); }
        label(dc,"Длительность перехода",Rect{left:228,top:250,right:width-24,bottom:280},INK,0x4);
        label(dc,&format!("{:.2} с", gallery.duration_display() as f32/1000.0),Rect{left:650,top:250,right:740,bottom:280},INK,0x2|0x4);
        let (left, right) = Gallery::fps_slider_limits(width);
        let x = left + ((right-left) * (gallery.duration_display() as i32 - 100) / 2900);
        rounded(dc, Rect{left,top:321,right,bottom:327}, SURFACE, 6);
        if x > left { rounded(dc, Rect{left,top:321,right:x,bottom:327}, ACCENT, 6); }
        rounded(dc, Rect{left:x-8,top:316,right:x+8,bottom:332}, ACCENT, 16);
        if !small.is_null() { SelectObject(dc, small); }
        label(dc,"0,1 с",Rect{left,top:338,right:left+60,bottom:358},MUTED,0);
        label(dc,"3 с",Rect{left:right-55,top:338,right,bottom:358},MUTED,0x2);
        label(dc,"Работает при смене PKG-сцен на основном экране.",Rect{left:228,top:390,right:width-24,bottom:420},MUTED,0);
    } else if gallery.help {
        if !semibold.is_null() { SelectObject(dc, semibold); }
        label(dc,"О нас",Rect{left:228,top:120,right:width-24,bottom:150},INK,0x4);
        if !normal.is_null() { SelectObject(dc, normal); }
        label(dc,ABOUT_PROJECT,Rect{left:228,top:155,right:width-24,bottom:190},MUTED,0x4);
        if !semibold.is_null() { SelectObject(dc, semibold); }
        label(dc,"Об авторе",Rect{left:228,top:220,right:width-24,bottom:250},INK,0x4);
        if !normal.is_null() { SelectObject(dc, normal); }
        label(dc,ABOUT_AUTHOR,Rect{left:228,top:255,right:width-24,bottom:290},MUTED,0x4);
        button(dc,"Мой Telegram",Rect{left:228,top:350,right:400,bottom:394},false);
        button(dc,"GitHub",Rect{left:412,top:350,right:584,bottom:394},false);
        if !small.is_null() { SelectObject(dc, small); }
        let version = format!("Версия {}", env!("CARGO_PKG_VERSION"));
        label(dc,&version,Rect{left:228,top:420,right:width-24,bottom:450},MUTED,0x4);
    } else if gallery.properties {
        button(dc,"Добавить project.json",Rect{left:228,top:120,right:510,bottom:162},false);
        if let Some(project) = gallery.project.as_ref() {
            for (row, property) in project.properties.iter().skip(gallery.property_page*4).take(4).enumerate() {
                let y = 182 + row as i32 * 66;
                if !normal.is_null() { SelectObject(dc, normal); }
                label(dc,&property.label,Rect{left:228,top:y,right:width-230,bottom:y+48},INK,0x4);
                button(dc,"−",Rect{left:width-210,top:y,right:width-160,bottom:y+48},false);
                let value = match &property.value { serde_json::Value::String(s) => s.clone(), v => v.to_string() };
                if !small.is_null() { SelectObject(dc, small); }
                label(dc,&value,Rect{left:width-154,top:y,right:width-94,bottom:y+48},INK,0x1|0x4);
                if !normal.is_null() { SelectObject(dc, normal); }
                button(dc,"+",Rect{left:width-88,top:y,right:width-38,bottom:y+48},false);
            }
            button(dc,"‹",Rect{left:228,top:462,right:280,bottom:498},false);
            button(dc,"›",Rect{left:290,top:462,right:342,bottom:498},false);
            if !small.is_null() { SelectObject(dc, small); }
            let page = format!("{} / {}", gallery.property_page+1, ((gallery.property_count()+3)/4).max(1));
            label(dc,&page,Rect{left:354,top:462,right:430,bottom:498},MUTED,0x4);
            if gallery.property_count() == 0 { label(dc,"Поддерживаемых параметров не найдено",Rect{left:228,top:200,right:width-24,bottom:235},MUTED,0); }
        } else {
            if !small.is_null() { SelectObject(dc, small); }
            label(dc,"Выберите PKG в коллекции и добавьте его project.json",Rect{left:228,top:190,right:width-24,bottom:220},MUTED,0);
        }
    } else if gallery.optimization {
        if !semibold.is_null() { SelectObject(dc, semibold); }
        label(dc,"Разрешение рендеринга",Rect{left:228,top:122,right:width-24,bottom:150},INK,0x4);
        if !small.is_null() { SelectObject(dc, small); }
        label(dc,"Целевая высота кадра для сцен PKG",Rect{left:228,top:156,right:width-24,bottom:180},MUTED,0);
        if !normal.is_null() { SelectObject(dc, normal); }
        for (i, (quality, name)) in [(480,"480p"),(720,"720p"),(1080,"1080p"),(1440,"2K"),(2160,"4K")].iter().enumerate() {
            let left = 228 + i as i32 * 108;
            button(dc,name,Rect{left,top:190,right:left+96,bottom:240},gallery.scene_height() == *quality);
        }
        if !small.is_null() { SelectObject(dc, small); }
        label(dc,"Меньше FPS и разрешение — ниже нагрузка на GPU.",Rect{left:228,top:250,right:width-24,bottom:275},MUTED,0);
        if !semibold.is_null() { SelectObject(dc, semibold); }
        label(dc,"Частота кадров PKG",Rect{left:228,top:278,right:width-24,bottom:303},INK,0x4);
        label(dc,&format!("{} FPS", gallery.fps_display()),Rect{left:650,top:278,right:740,bottom:303},INK,0x2|0x4);
        let (left, right) = Gallery::fps_slider_limits(width);
        let x = left + (((right-left) * (gallery.fps_display() as i32 - 1) + 59) / 119);
        rounded(dc, Rect{left,top:321,right,bottom:327}, SURFACE, 6);
        if x > left { rounded(dc, Rect{left,top:321,right:x,bottom:327}, ACCENT, 6); }
        rounded(dc, Rect{left:x-8,top:316,right:x+8,bottom:332}, ACCENT, 16);
        if !small.is_null() { SelectObject(dc, small); }
        label(dc,"1",Rect{left,top:336,right:left+35,bottom:355},MUTED,0);
        label(dc,"120",Rect{left:right-35,top:336,right,bottom:355},MUTED,0x2);
        if !semibold.is_null() { SelectObject(dc, semibold); }
        label(dc,"Экраны",Rect{left:228,top:365,right:width-24,bottom:395},INK,0x4);
        if !normal.is_null() { SelectObject(dc, normal); }
        button(dc,"Основной",Rect{left:228,top:405,right:448,bottom:450},gallery.screen_mode() == 0);
        button(dc,"На все экраны",Rect{left:460,top:405,right:740,bottom:450},gallery.screen_mode() == 1);
        if !small.is_null() { SelectObject(dc, small); }
        if !normal.is_null() { SelectObject(dc, normal); }
        button(dc,"Дублировать PKG",Rect{left:228,top:462,right:448,bottom:507},gallery.screen_mode() == 2);
        button(dc,"Разные PKG",Rect{left:460,top:462,right:740,bottom:507},gallery.screen_mode() == 3);
    } else if gallery.settings {
        if !semibold.is_null() { SelectObject(dc, semibold); }
        label(dc,"Быстрый выбор",Rect{left:228,top:112,right:width-24,bottom:138},INK,0x4);
        if !small.is_null() { SelectObject(dc, small); }
        label(dc,"Нажмите на поле и введите новое сочетание",Rect{left:228,top:141,right:width-24,bottom:165},MUTED,0);
        if !normal.is_null() { SelectObject(dc, normal); }
        let key = if gallery.capturing { "Нажмите сочетание…  Esc — отмена" } else { gallery.shortcut() };
        button(dc,key,Rect{left:228,top:170,right:(width-24).min(760),bottom:215},false);
        if !small.is_null() { SelectObject(dc, small); }
        label(dc,"Alt / Ctrl / Shift + буква или цифра",Rect{left:228,top:218,right:width-24,bottom:240},MUTED,0);
        if !normal.is_null() { SelectObject(dc, normal); }
        button(dc,if gallery.sound() {"Звук включён"} else {"Звук выключен"},Rect{left:228,top:250,right:(width-24).min(760),bottom:290},gallery.sound());
        button(dc,if gallery.scripts {"Скрипты: включены до выхода"} else {"Скрипты: выключены"},Rect{left:228,top:300,right:(width-24).min(760),bottom:340},gallery.scripts);
        if !small.is_null() { SelectObject(dc, small); }
        label(dc,"Скрипты запускайте только в доверенных обоях",Rect{left:228,top:345,right:width-24,bottom:368},MUTED,0);
        if !normal.is_null() { SelectObject(dc, normal); }
        button(dc,if gallery.autostart {"Автозапуск приложения: включён"} else {"Автозапуск приложения: выключен"},Rect{left:228,top:380,right:(width-24).min(760),bottom:425},gallery.autostart);
        if !small.is_null() { SelectObject(dc, small); }
        label(dc,"Приложение откроется в трее при входе в Windows",Rect{left:228,top:432,right:width-24,bottom:455},MUTED,0);
        if !normal.is_null() { SelectObject(dc, normal); }
        button(dc,"Настроить переходы",Rect{left:228,top:466,right:(width-24).min(760),bottom:508},false);
    } else {
        let preview_right = width-264;
        let bottom = height-155;
        preview(dc, gallery, Rect{left:228,top:112,right:preview_right,bottom});
        if gallery.is_empty() {
            if !normal.is_null() { SelectObject(dc, normal); }
            label(dc,"Здесь появится превью обоев",Rect{left:238,top:112,right:preview_right-10,bottom},MUTED,0x1|0x4);
        }
        let side = width-240;
        if !small.is_null() { SelectObject(dc, small); }
        label(dc,"ВЫБРАННЫЙ ФАЙЛ",Rect{left:side,top:114,right:width-24,bottom:135},MUTED,0);
        if !semibold.is_null() { SelectObject(dc, semibold); }
        let name = gallery.current().and_then(|p| p.file_name().map(|x| x.to_string_lossy().into_owned())).unwrap_or_else(|| "Нет обоев".to_string());
        label(dc,&name,Rect{left:side,top:145,right:width-24,bottom:179},INK,0x4);
        if !small.is_null() { SelectObject(dc, small); }
        let count = format!("{} в библиотеке", gallery.stored.videos.len());
        label(dc,&count,Rect{left:side,top:184,right:width-24,bottom:204},MUTED,0);
        if !normal.is_null() { SelectObject(dc, normal); }
        button(dc,"Добавить файл",Rect{left:side,top:228,right:width-24,bottom:270},false);
        button(dc,"Применить",Rect{left:side,top:280,right:width-24,bottom:322},true);
        button(dc,"Остановить",Rect{left:side,top:332,right:width-24,bottom:374},false);
        button(dc,"Удалить обои",Rect{left:side,top:384,right:width-24,bottom:426},false);
        button(dc,"Для экрана 2",Rect{left:side,top:436,right:width-24,bottom:478},false);
        let base = height-135;
        button(dc,"‹",Rect{left:228,top:base,right:268,bottom:base+36},false);
        button(dc,"›",Rect{left:276,top:base,right:316,bottom:base+36},false);
        if !small.is_null() { SelectObject(dc, small); }
        let position = if gallery.is_empty() { "0 / 0".to_string() } else { format!("{} / {}", gallery.selected+1, gallery.stored.videos.len()) };
        label(dc,&position,Rect{left:330,top:base,right:preview_right,bottom:base+36},MUTED,0x4);
    }
    fill(dc,Rect{left:228,top:height-70,right:width-24,bottom:height-69},0x00372d29);
    if !small.is_null() { SelectObject(dc, small); }
    label(dc,status,Rect{left:228,top:height-58,right:width-24,bottom:height-24},MUTED,0x4);
    if !old_font.is_null() { SelectObject(dc, old_font); }
    for f in [small, normal, semibold, big] { if !f.is_null() { DeleteObject(f); } }
    if !bitmap.is_null() { BitBlt(real_dc,0,0,width,height,mem,0,0,0x00cc0020); SelectObject(mem, old_bitmap); DeleteObject(bitmap); }
    if !mem.is_null() { DeleteDC(mem); }
    EndPaint(hwnd, &ps);
}

pub(super) unsafe fn redraw(hwnd: Handle) { InvalidateRect(hwnd, null(), 0); }
pub(super) const HOTKEY_MESSAGE: u32 = WM_HOTKEY;
pub(super) const KEYDOWN: u32 = WM_KEYDOWN;
pub(super) const LBUTTON: u32 = WM_LBUTTONDOWN;
pub(super) const WHEEL: u32 = WM_MOUSEWHEEL;

pub(super) unsafe fn foreground(hwnd: Handle) { SetForegroundWindow(hwnd); }

unsafe fn cache_picker_card(dc: Handle, gallery: &mut Gallery, index: usize, card: i32, step: i32, skew: i32, height: i32) -> bool {
    let path = gallery.stored.videos[index].clone();
    if !gallery.thumbs.contains_key(&path) { return false; }
    if gallery.picker_cards.get(&path).map_or(false, |cached| cached.card == card && cached.skew == skew && cached.height == height) { return true; }

    let mem = CreateCompatibleDC(dc);
    if mem.is_null() { return false; }
    let bitmap = CreateCompatibleBitmap(dc, card + skew, height);
    if bitmap.is_null() { DeleteDC(mem); return false; }
    let old = SelectObject(mem, bitmap);
    if old.is_null() || old as isize == -1 { DeleteObject(bitmap); DeleteDC(mem); return false; }
    fill(mem, Rect { left:0, top:0, right:card+skew, bottom:height }, 0x00ff00ff);
    let offset = gallery.scroll_offset;
    gallery.scroll_offset = 0.0;
    draw_card_area(mem, gallery, index, 0, card+skew, 0, height, card, step, skew);
    gallery.scroll_offset = offset;
    SelectObject(mem, old);
    DeleteDC(mem);
    gallery.picker_cards.insert(path, PickerCard { bitmap: bitmap as isize, card, skew, height });
    true
}

unsafe fn draw_cached_picker_card(dc: Handle, source: Handle, cached: &PickerCard, x: i32) -> bool {
    let points = [Point{x:x+cached.skew,y:0}, Point{x:x+cached.skew+cached.card,y:0},
        Point{x:x+cached.card,y:cached.height}, Point{x,y:cached.height}];
    let region = CreatePolygonRgn(points.as_ptr(), 4, 1);
    if region.is_null() { return false; }
    let saved = SaveDC(dc);
    if saved == 0 { DeleteObject(region); return false; }
    let clipped = ExtSelectClipRgn(dc, region, 1) != 0;
    DeleteObject(region);
    if !clipped { RestoreDC(dc, saved); return false; }
    let old = SelectObject(source, cached.bitmap as Handle);
    if old.is_null() || old as isize == -1 { RestoreDC(dc, saved); return false; }
    let copied = BitBlt(dc, x, 0, cached.card+cached.skew, cached.height, source, 0, 0, 0x00cc0020) != 0;
    SelectObject(source, old);
    RestoreDC(dc, saved);
    copied
}

pub(super) unsafe fn paint_picker(hwnd: Handle, gallery: &mut Gallery) {
    let mut ps: PaintStruct = zeroed();
    let real_dc = BeginPaint(hwnd, &mut ps);
    if real_dc.is_null() { EndPaint(hwnd, &ps); return; }
    let mut rect = Rect::default(); GetClientRect(hwnd, &mut rect);
    let width = rect.right.max(1); let height = rect.bottom.max(1);
    let mem = CreateCompatibleDC(real_dc);
    let bitmap = if !mem.is_null() { CreateCompatibleBitmap(real_dc, width, height) } else { null_mut() };
    let dc = if !bitmap.is_null() { mem } else { real_dc };
    let old = if !bitmap.is_null() { SelectObject(mem, bitmap) } else { null_mut() };
    fill(dc, Rect { left:0, top:0, right:width, bottom:height }, 0x00ff00ff);
    if !gallery.stored.videos.is_empty() {
        let card = (width/5).clamp(220,420);
        let step = card+12; let skew = (card/5).clamp(40,84);
        let count = gallery.stored.videos.len() as i32;
        let source = CreateCompatibleDC(dc);
        for slot in -8i32..=8 {
            let x = width/2-card/2-skew/2 + slot*step + (gallery.scroll_offset * step as f32).round() as i32;
            if x + card + skew < 0 || x > width { continue; }
            let index = (gallery.selected as i32 + slot).rem_euclid(count) as usize;
            let mut drawn = false;
            if !source.is_null() && cache_picker_card(dc, gallery, index, card, step, skew, height) {
                if let Some(cached) = gallery.picker_cards.get(&gallery.stored.videos[index]) {
                    drawn = draw_cached_picker_card(dc, source, cached, x);
                }
            }
            if !drawn { draw_card_area(dc, gallery, index, slot, width, 0, height, card, step, skew); }
        }
        if !source.is_null() { DeleteDC(source); }
    }
    if !bitmap.is_null() {
        BitBlt(real_dc,0,0,width,height,mem,0,0,0x00cc0020);
        SelectObject(mem, old); DeleteObject(bitmap);
    }
    if !mem.is_null() { DeleteDC(mem); }
    EndPaint(hwnd, &ps);
}
