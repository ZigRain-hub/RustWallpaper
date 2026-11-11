#![cfg_attr(windows, windows_subsystem = "windows")]
#![allow(unsafe_op_in_unsafe_fn)]

#[cfg(not(windows))]
fn main() {
    eprintln!("RustWallpaper работает только в Windows.");
}

#[cfg(windows)]
mod app {
    use std::ffi::{c_void, OsStr, OsString};
    use std::mem::{size_of, zeroed};
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    use std::path::PathBuf;
    use std::ptr::{null, null_mut};
    use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
    use std::time::{Duration, Instant};

    mod startup { include!("startup.rs"); }
    mod gallery { include!("gallery.rs"); }
    mod gif_wallpaper { include!("gif_wallpaper.rs"); }
    mod scene_wallpaper { include!("scene_wallpaper.rs"); }
    mod web_scene { include!("web_scene.rs"); }

    type Handle = *mut c_void;
    type Lresult = isize;
    type Wparam = usize;
    type Lparam = isize;
    type WndProc = Option<unsafe extern "system" fn(Handle, u32, Wparam, Lparam) -> Lresult>;

    const WS_OVERLAPPED: u32 = 0;
    const WS_POPUP: u32 = 0x80000000;
    const WS_EX_TOPMOST: u32 = 0x00000008;
    const WS_CAPTION: u32 = 0x00C00000;
    const WS_SYSMENU: u32 = 0x00080000;
    const WS_MINIMIZEBOX: u32 = 0x00020000;
    const WS_CHILD: u32 = 0x40000000;
    const WS_VISIBLE: u32 = 0x10000000;
    const WS_CLIPCHILDREN: u32 = 0x02000000;
    const WS_EX_NOACTIVATE: u32 = 0x08000000;
    const WS_EX_TOOLWINDOW: u32 = 0x00000080;
    const WS_EX_LAYERED: u32 = 0x00080000;
    const WS_EX_NOREDIRECTIONBITMAP: isize = 0x00200000;
    const SS_PATHELLIPSIS: u32 = 0x00004000;
    const CW_USEDEFAULT: i32 = 0x80000000u32 as i32;
    const SW_SHOW: i32 = 5;
    const WM_DESTROY: u32 = 0x0002;
    const WM_SIZE: u32 = 0x0005;
    const WM_PAINT: u32 = 0x000f;
    const WM_ERASEBKGND: u32 = 0x0014;
    const WM_PLAYER_STATUS: u32 = 0x8001;
    const WM_APP_EXIT: u32 = 0x8004;
    const WM_PICKER_TOGGLE: u32 = 0x8005;
    const WM_PICKER_APPLY: u32 = 0x8006;
    const WM_PICKER_HIDE: u32 = 0x8007;
    const WM_TRAY: u32 = 0x801e;
    const WM_SCENE_WEB_STATUS: u32 = 0x8020;
    const WM_TRANSITION_TICK: u32 = 0x8021;
    const WM_CLOSE: u32 = 0x0010;
    const WM_COMMAND: u32 = 0x0111;
    const WM_TIMER: u32 = 0x0113;
    const WM_SETFONT: u32 = 0x0030;
    const GWLP_USERDATA: i32 = -21;
    const COLOR_WINDOW: usize = 5;
    const DEFAULT_GUI_FONT: i32 = 17;
    const ID_VIDEO: usize = 101;
    const ID_APPLY: usize = 103;
    const ID_STOP: usize = 104;
    const ID_GALLERY: usize = 105;
    const ID_SAVE_HOTKEY: usize = 106;
    const TIMER: usize = 1;
    const ANIM_TIMER: usize = 2;
    const GIF_TIMER: usize = 3;
    const SCENE_TIMER: usize = 4;
    const SCROLL_TIMER: usize = 5;
    const TRANSITION_TIMER: usize = 6;
    const ANIM_MS: u64 = 220;

    #[repr(C)]
    struct WndClass {
        style: u32,
        wnd_proc: WndProc,
        cls_extra: i32,
        wnd_extra: i32,
        instance: Handle,
        icon: Handle,
        cursor: Handle,
        background: Handle,
        menu_name: *const u16,
        class_name: *const u16,
    }

    #[repr(C)]
    struct PaintStruct { dc: Handle, erase: i32, rect: Rect, restore: i32, inc_update: i32, reserved: [u8; 32] }

    #[repr(C)]
    #[derive(Default, Clone, Copy)]
    struct Point { x: i32, y: i32 }

    #[repr(C)]
    #[derive(Default)]
    struct SystemTime { year: u16, month: u16, day_of_week: u16, day: u16,
        hour: u16, minute: u16, second: u16, milliseconds: u16 }

    #[repr(C)]
    #[derive(Default, Clone, Copy)]
    struct Rect { left: i32, top: i32, right: i32, bottom: i32 }

    #[repr(C)]
    struct Msg {
        hwnd: Handle,
        message: u32,
        wparam: Wparam,
        lparam: Lparam,
        time: u32,
        point: Point,
        private: u32,
    }

    #[repr(C)]
    struct OpenFileName {
        size: u32,
        owner: Handle,
        instance: Handle,
        filter: *const u16,
        custom_filter: *mut u16,
        max_custom_filter: u32,
        filter_index: u32,
        file: *mut u16,
        max_file: u32,
        file_title: *mut u16,
        max_file_title: u32,
        initial_dir: *const u16,
        title: *const u16,
        flags: u32,
        file_offset: u16,
        file_extension: u16,
        default_extension: *const u16,
        custom_data: isize,
        hook: *mut c_void,
        template: *const u16,
        reserved: *mut c_void,
        reserved2: u32,
        flags_ex: u32,
    }

    #[link(name = "user32")]
    extern "system" {
        fn RegisterClassW(class: *const WndClass) -> u16;
        fn RegisterWindowMessageW(name: *const u16) -> u32;
        fn LoadIconW(instance: Handle, name: *const u16) -> Handle;
        fn GetCursorPos(point: *mut Point) -> i32;
        fn CreatePopupMenu() -> Handle;
        fn AppendMenuW(menu: Handle, flags: u32, id: usize, text: *const u16) -> i32;
        fn TrackPopupMenu(menu: Handle, flags: u32, x: i32, y: i32, reserved: i32, owner: Handle, rect: *const Rect) -> u32;
        fn DestroyMenu(menu: Handle) -> i32;
        fn SetForegroundWindow(hwnd: Handle) -> i32;
        fn SetCapture(hwnd: Handle) -> Handle;
        fn GetCapture() -> Handle;
        fn ReleaseCapture() -> i32;
        fn CreateWindowExW(ex_style: u32, class: *const u16, title: *const u16,
            style: u32, x: i32, y: i32, width: i32, height: i32, parent: Handle,
            menu: Handle, instance: Handle, param: *mut c_void) -> Handle;
        fn DefWindowProcW(hwnd: Handle, msg: u32, wparam: Wparam, lparam: Lparam) -> Lresult;
        fn DestroyWindow(hwnd: Handle) -> i32;
        fn IsWindow(hwnd: Handle) -> i32;
        fn IsWindowVisible(hwnd: Handle) -> i32;
        fn ShowWindow(hwnd: Handle, command: i32) -> i32;
        fn UpdateWindow(hwnd: Handle) -> i32;
        fn GetMessageW(msg: *mut Msg, hwnd: Handle, min: u32, max: u32) -> i32;
        fn TranslateMessage(msg: *const Msg) -> i32;
        fn DispatchMessageW(msg: *const Msg) -> Lresult;
        fn PostQuitMessage(code: i32);
        fn GetWindowLongPtrW(hwnd: Handle, index: i32) -> isize;
        fn SetWindowLongPtrW(hwnd: Handle, index: i32, value: isize) -> isize;
        fn SendMessageW(hwnd: Handle, msg: u32, wparam: Wparam, lparam: Lparam) -> Lresult;
        fn SendMessageTimeoutW(hwnd: Handle, msg: u32, wparam: Wparam, lparam: Lparam,
            flags: u32, timeout: u32, result: *mut usize) -> Lresult;
        fn SetWindowTextW(hwnd: Handle, text: *const u16) -> i32;
        fn LoadCursorW(instance: Handle, name: *const u16) -> Handle;
        fn MessageBoxW(owner: Handle, text: *const u16, title: *const u16, flags: u32) -> i32;
        fn FindWindowW(class: *const u16, title: *const u16) -> Handle;
        fn FindWindowExW(parent: Handle, after: Handle, class: *const u16, title: *const u16) -> Handle;
        fn EnumWindows(callback: Option<unsafe extern "system" fn(Handle, isize) -> i32>, data: isize) -> i32;
        fn GetClientRect(hwnd: Handle, rect: *mut Rect) -> i32;
        fn GetParent(hwnd: Handle) -> Handle;
        fn SetWindowRgn(hwnd: Handle, region: Handle, redraw: i32) -> i32;
        fn ClientToScreen(hwnd: Handle, point: *mut Point) -> i32;
        fn GetSystemMetrics(index: i32) -> i32;
        fn EnumDisplayMonitors(dc: Handle, clip: *const Rect, callback: Option<unsafe extern "system" fn(Handle, Handle, *const Rect, isize) -> i32>, data: isize) -> i32;
        fn SetTimer(hwnd: Handle, id: usize, milliseconds: u32, callback: *mut c_void) -> usize;
        fn KillTimer(hwnd: Handle, id: usize) -> i32;
        fn SetProcessDPIAware() -> i32;
        fn GetKeyState(key: i32) -> i16;
        fn SetLayeredWindowAttributes(hwnd: Handle, color: u32, alpha: u8, flags: u32) -> i32;
        fn SetWindowPos(hwnd: Handle, after: Handle, x: i32, y: i32, w: i32, h: i32, flags: u32) -> i32;
        fn PostMessageW(hwnd: Handle, msg: u32, wparam: Wparam, lparam: Lparam) -> i32;
        fn BeginPaint(hwnd: Handle, paint: *mut PaintStruct) -> Handle;
        fn EndPaint(hwnd: Handle, paint: *const PaintStruct) -> i32;
        fn InvalidateRect(hwnd: Handle, rect: *const Rect, erase: i32) -> i32;
        fn DrawTextW(dc: Handle, text: *const u16, count: i32, rect: *mut Rect, format: u32) -> i32;
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn GetModuleHandleW(name: *const u16) -> Handle;
        fn GetLocalTime(time: *mut SystemTime);
    }

    #[link(name = "gdi32")]
    extern "system" {
        fn CreateRectRgn(left: i32, top: i32, right: i32, bottom: i32) -> Handle;
        fn CreateEllipticRgn(left: i32, top: i32, right: i32, bottom: i32) -> Handle;
        fn CreatePolygonRgn(points: *const Point, count: i32, mode: i32) -> Handle;
        fn CombineRgn(dest: Handle, first: Handle, second: Handle, mode: i32) -> i32;
        fn CreateCompatibleDC(dc: Handle) -> Handle;
        fn CreateCompatibleBitmap(dc: Handle, width: i32, height: i32) -> Handle;
        fn DeleteDC(dc: Handle) -> i32;
        fn BitBlt(dest: Handle, x: i32, y: i32, width: i32, height: i32, src: Handle, sx: i32, sy: i32, rop: u32) -> i32;
        fn GetStockObject(index: i32) -> Handle;
        fn CreateSolidBrush(color: u32) -> Handle;
        fn SelectObject(dc: Handle, object: Handle) -> Handle;
        fn DeleteObject(object: Handle) -> i32;
        fn Ellipse(dc: Handle, left: i32, top: i32, right: i32, bottom: i32) -> i32;
        fn SetBkMode(dc: Handle, mode: i32) -> i32;
        fn SetTextColor(dc: Handle, color: u32) -> u32;
        fn CreateFontW(height: i32, width: i32, escapement: i32, orientation: i32, weight: i32,
            italic: u32, underline: u32, strike: u32, charset: u32, output_precision: u32,
            clip_precision: u32, quality: u32, pitch: u32, face: *const u16) -> Handle;
        fn StretchDIBits(dc: Handle, x: i32, y: i32, width: i32, height: i32,
            sx: i32, sy: i32, sw: i32, sh: i32, bits: *const c_void,
            info: *const BitmapInfo, usage: u32, rop: u32) -> i32;
    }

    #[link(name = "comdlg32")]
    extern "system" {
        fn GetOpenFileNameW(dialog: *mut OpenFileName) -> i32;
        fn CommDlgExtendedError() -> u32;
    }

    #[link(name = "ole32")]
    extern "system" {
        fn CoInitializeEx(reserved: *mut c_void, flags: u32) -> i32;
        fn CoUninitialize();
    }

    #[link(name = "mfplat")]
    extern "system" {
        fn MFStartup(version: u32, flags: u32) -> i32;
        fn MFShutdown() -> i32;
    }

    #[link(name = "mfplay")]
    extern "system" {
        fn MFPCreateMediaPlayer(url: *const u16, autoplay: i32, options: u32,
            callback: *mut PlayerCallback, video_window: Handle, player: *mut *mut MediaPlayer) -> i32;
    }

    #[repr(C)]
    #[derive(Clone, Copy, PartialEq, Eq)]
    struct Guid { a: u32, b: u16, c: u16, d: [u8; 8] }
    #[repr(C)]
    struct NotifyIconData {
        size: u32, hwnd: Handle, id: u32, flags: u32, callback: u32,
        icon: Handle, tip: [u16; 128], state: u32, state_mask: u32,
        info: [u16; 256], version: u32, title: [u16; 64],
        info_flags: u32, guid: Guid, balloon_icon: Handle,
    }

    #[link(name = "shell32")]
    extern "system" {
        fn Shell_NotifyIconW(message: u32, data: *mut NotifyIconData) -> i32;
        fn ShellExecuteW(hwnd: Handle, operation: *const u16, file: *const u16, parameters: *const u16, directory: *const u16, show: i32) -> Handle;
    }

    unsafe fn tray_data(hwnd: Handle) -> NotifyIconData {
        let mut data: NotifyIconData = zeroed();
        data.size = size_of::<NotifyIconData>() as u32;
        data.hwnd = hwnd;
        data.id = 1;
        data.flags = 0x1 | 0x2 | 0x4;
        data.callback = WM_TRAY;
        data.icon = LoadIconW(null_mut(), 32512usize as *const u16);
        let tip = wide(gallery::APP_NAME);
        data.tip[..tip.len()].copy_from_slice(&tip);
        data
    }
    unsafe fn tray_add(hwnd: Handle) -> bool {
        let mut data = tray_data(hwnd);
        !data.icon.is_null() && Shell_NotifyIconW(0, &mut data) != 0
    }
    unsafe fn tray_remove(hwnd: Handle) {
        let mut data = tray_data(hwnd);
        Shell_NotifyIconW(2, &mut data);
    }

    const IID_UNKNOWN: Guid = Guid { a: 0, b: 0, c: 0, d: [0xc0, 0, 0, 0, 0, 0, 0, 0x46] };
    const IID_CALLBACK: Guid = Guid { a: 0x766c8ffb, b: 0x5fdb, c: 0x4fea, d: [0xa2,0x8d,0xb9,0x12,0x99,0x6f,0x51,0xbd] };

    #[repr(C)]
    struct MediaPlayer { vtable: *const PlayerVtable }
    #[repr(C)]
    struct PlayerVtable {
        query: usize, add_ref: usize,
        release: unsafe extern "system" fn(*mut MediaPlayer) -> u32,
        play: unsafe extern "system" fn(*mut MediaPlayer) -> i32,
        pause: unsafe extern "system" fn(*mut MediaPlayer) -> i32,
        stop: unsafe extern "system" fn(*mut MediaPlayer) -> i32,
        before_create: [usize; 8],
        create_url: unsafe extern "system" fn(*mut MediaPlayer, *const u16, i32, usize, *mut *mut c_void) -> i32,
        create_object: usize,
        set_item: unsafe extern "system" fn(*mut MediaPlayer, *mut c_void) -> i32,
        before_mute: [usize; 7],
        set_mute: unsafe extern "system" fn(*mut MediaPlayer, i32) -> i32,
        before_update: [usize; 7],
        update_video: unsafe extern "system" fn(*mut MediaPlayer) -> i32,
        before_shutdown: [usize; 5],
        shutdown: unsafe extern "system" fn(*mut MediaPlayer) -> i32,
    }

    #[repr(C)]
    struct EventHeader { kind: u32, result: i32, player: *mut MediaPlayer, state: u32, property_store: Handle }
    #[repr(C)]
    struct ItemCreated { header: EventHeader, item: *mut c_void, data: usize }

    #[repr(C)]
    struct PlayerCallback {
        vtable: *const CallbackVtable,
        refs: AtomicU32,
        active: AtomicBool,
        window: Handle,
        generation: usize,
    }
    #[repr(C)]
    struct CallbackVtable {
        query: unsafe extern "system" fn(*mut PlayerCallback, *const Guid, *mut *mut c_void) -> i32,
        add_ref: unsafe extern "system" fn(*mut PlayerCallback) -> u32,
        release: unsafe extern "system" fn(*mut PlayerCallback) -> u32,
        event: unsafe extern "system" fn(*mut PlayerCallback, *const EventHeader),
    }

    unsafe extern "system" fn callback_query(this: *mut PlayerCallback, iid: *const Guid, out: *mut *mut c_void) -> i32 {
        if out.is_null() { return 0x80004003u32 as i32; }
        *out = null_mut();
        if iid.is_null() { return 0x80004003u32 as i32; }
        if *iid == IID_UNKNOWN || *iid == IID_CALLBACK {
            *out = this as *mut c_void;
            callback_add_ref(this);
            0
        } else { 0x80004002u32 as i32 }
    }
    unsafe extern "system" fn callback_add_ref(this: *mut PlayerCallback) -> u32 {
        (&*this).refs.fetch_add(1, Ordering::Relaxed) + 1
    }
    unsafe extern "system" fn callback_release(this: *mut PlayerCallback) -> u32 {
        let previous = (&*this).refs.fetch_sub(1, Ordering::Release);
        if previous == 1 {
            std::sync::atomic::fence(Ordering::Acquire);
            drop(Box::from_raw(this));
            0
        } else { previous - 1 }
    }
    unsafe extern "system" fn callback_event(this: *mut PlayerCallback, event: *const EventHeader) {
        if event.is_null() || !(&*this).active.load(Ordering::Acquire) { return; }
        let h = &*event;
        let mut result = h.result;
        if result >= 0 && !h.player.is_null() {
            let table = &*(*h.player).vtable;
            result = match h.kind {
                5 => {
                    let item = (*(event as *const ItemCreated)).item;
                    if item.is_null() { 0x80004005u32 as i32 } else { (table.set_item)(h.player, item) }
                }
                6 | 2 => (table.play)(h.player),
                11 => (table.stop)(h.player),
                10 => 0x80004005u32 as i32,
                _ => 0,
            };
        }
        if result < 0 || h.kind == 0 {
            PostMessageW((*this).window, WM_PLAYER_STATUS, (*this).generation, result as isize);
        }
    }
    static CALLBACK_VTABLE: CallbackVtable = CallbackVtable {
        query: callback_query, add_ref: callback_add_ref, release: callback_release, event: callback_event,
    };

    fn wide(value: &str) -> Vec<u16> { OsStr::new(value).encode_wide().chain(Some(0)).collect() }

    unsafe fn set_text(hwnd: Handle, text: &str) {
        SetWindowTextW(hwnd, wide(text).as_ptr());
    }

    unsafe fn show_error(hwnd: Handle, text: &str) {
        MessageBoxW(hwnd, wide(text).as_ptr(), wide(gallery::APP_NAME).as_ptr(), 0x10);
    }

    unsafe fn pick_file_with(owner: Handle, json: bool) -> Result<Option<PathBuf>, String> {
        let filter: Vec<u16> = if json { "Проект Wallpaper Engine\0*.json\0Все файлы\0*.*\0\0" } else { "Обои: видео, GIF, PKG\0*.mp4;*.m4v;*.wmv;*.mov;*.mkv;*.avi;*.webm;*.gif;*.pkg\0Все файлы\0*.*\0\0" }.encode_utf16().collect();
        let title = wide(if json { "Выберите project.json для выбранной сцены" } else { "Выберите обои" });
        let mut buffer = vec![0u16; 32768];
        let mut dialog: OpenFileName = zeroed();
        dialog.size = size_of::<OpenFileName>() as u32;
        dialog.owner = owner;
        dialog.filter = filter.as_ptr();
        dialog.filter_index = 1;
        dialog.file = buffer.as_mut_ptr();
        dialog.max_file = buffer.len() as u32;
        dialog.title = title.as_ptr();
        dialog.flags = 0x00080000 | 0x00001000 | 0x00000800 | 0x00000008;
        if GetOpenFileNameW(&mut dialog) == 0 {
            let error = CommDlgExtendedError();
            return if error == 0 { Ok(None) } else { Err(format!("Не удалось открыть диалог выбора файла: {error:#x}")) };
        }
        let end = buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len());
        Ok(Some(PathBuf::from(OsString::from_wide(&buffer[..end]))))
    }

    unsafe fn pick_file(owner: Handle) -> Result<Option<PathBuf>, String> { pick_file_with(owner, false) }

    unsafe extern "system" fn find_wallpaper_layer(hwnd: Handle, data: isize) -> i32 {
        let def_view = FindWindowExW(hwnd, null_mut(), wide("SHELLDLL_DefView").as_ptr(), null());
        if def_view.is_null() { return 1; }
        let mut candidate = FindWindowExW(null_mut(), hwnd, wide("WorkerW").as_ptr(), null());
        while !candidate.is_null() {
            let icons = FindWindowExW(candidate, null_mut(), wide("SHELLDLL_DefView").as_ptr(), null());
            if icons.is_null() && IsWindowVisible(candidate) != 0 {
                *(data as *mut Handle) = candidate;
                return 0;
            }
            candidate = FindWindowExW(null_mut(), candidate, wide("WorkerW").as_ptr(), null());
        }
        1
    }

    unsafe extern "system" fn collect_monitor(_: Handle, _: Handle, rect: *const Rect, data: isize) -> i32 {
        if !rect.is_null() && data != 0 { (&mut *(data as *mut Vec<Rect>)).push(*rect); }
        1
    }
    unsafe fn displays() -> Result<Vec<Rect>, String> {
        let mut monitors: Vec<Rect> = Vec::new();
        if EnumDisplayMonitors(null_mut(), null(), Some(collect_monitor), &mut monitors as *mut _ as isize) == 0 { return Err("Не удалось получить список мониторов.".into()); }
        monitors.sort_by_key(|r| if r.left == 0 && r.top == 0 { 0 } else { 1 });
        Ok(monitors)
    }

    struct DesktopLayer { parent: Handle, icons: Handle, layered: bool }

    unsafe fn desktop_layer() -> Result<DesktopLayer, String> {
        let progman = FindWindowW(wide("Progman").as_ptr(), null());
        if progman.is_null() { return Err("Не найден рабочий стол Explorer.".into()); }
        let mut result = 0usize;
        SendMessageTimeoutW(progman, 0x052C, 0, 0, 0x0002, 1000, &mut result);
        if (GetWindowLongPtrW(progman, -20) & WS_EX_NOREDIRECTIONBITMAP) != 0 {
            let icons = FindWindowExW(progman, null_mut(), wide("SHELLDLL_DefView").as_ptr(), null());
            let background = FindWindowExW(progman, null_mut(), wide("WorkerW").as_ptr(), null());
            if icons.is_null() || background.is_null() {
                return Err("Не удалось найти слои рабочего стола Windows 11.".into());
            }
            return Ok(DesktopLayer { parent: progman, icons, layered: true });
        }
        let mut worker: Handle = null_mut();
        EnumWindows(Some(find_wallpaper_layer), &mut worker as *mut _ as isize);
        if worker.is_null() {
            return Err("Не удалось найти подходящий слой рабочего стола. Эта версия Windows или Explorer может использовать другую структуру окон.".into());
        }
        Ok(DesktopLayer { parent: worker, icons: null_mut(), layered: false })
    }

    #[repr(C)]
    struct BitmapInfoHeader {
        size: u32, width: i32, height: i32, planes: u16, bit_count: u16,
        compression: u32, size_image: u32, x_pixels: i32, y_pixels: i32,
        colors_used: u32, colors_important: u32,
    }
    #[repr(C)]
    struct BitmapInfo { header: BitmapInfoHeader, colors: [u32; 1] }

    enum HostContent {
        Video(*mut MediaPlayer),
        Gif(Option<gif_wallpaper::Frame>),
        Scene(Option<scene_wallpaper::Scene>, Instant),
    }
    struct HostState { content: HostContent }

    struct SecondaryScene { host: Handle, path: PathBuf, view: wry::WebView }
    mod transitions { include!("transitions.rs"); }
    mod transition_clock { include!("transition_clock.rs"); }
    struct PreviousScene {
        host: Handle,
        view: Option<wry::WebView>,
        path: Option<PathBuf>,
        generation: usize,
        effect: u8,
        waiting_since: Instant,
        animation_since: Option<Instant>,
        exposed_since: Option<Instant>,
        duration_ms: u32,
    }
    struct App {
        instance: Handle,
        video: Option<PathBuf>,
        active_video: Option<PathBuf>,
        previous: Option<PreviousScene>,
        transition_clock: Option<transition_clock::Clock>,
        status_line: String,
        host: Handle,
        player: *mut MediaPlayer,
        gif_player: Option<gif_wallpaper::Player>,
        scene_rx: Option<std::sync::mpsc::Receiver<Result<scene_wallpaper::Scene, String>>>,
        scene_active: bool,
        scene_webview: Option<wry::WebView>,
        secondary: Option<SecondaryScene>,
        host_state: Option<Box<HostState>>,
        callback: *mut PlayerCallback,
        generation: usize,
        main_window: Handle,
        picker: Handle,
        picker_alpha: f32,
        picker_start: f32,
        picker_open: bool,
        picker_since: Instant,
        tray_active: bool,
        taskbar_created: u32,
        gallery: gallery::Gallery,
    }

    impl App {
        unsafe fn status(&mut self, text: &str) {
            self.status_line = text.to_string();
            gallery::redraw(self.main_window);
        }

        unsafe fn discard_previous(&mut self) {
            KillTimer(self.main_window, TRANSITION_TIMER);
            self.transition_clock = None;
            if let Some(mut old) = self.previous.take() {
                drop(old.view.take());
                if !old.host.is_null() && IsWindow(old.host) != 0 { DestroyWindow(old.host); }
            }
        }

        unsafe fn stop(&mut self) {
            self.discard_previous();
            self.stop_current();
            self.active_video = None;
        }

        unsafe fn stop_current(&mut self) {
            self.generation = self.generation.wrapping_add(1);
            KillTimer(self.main_window, GIF_TIMER);
            KillTimer(self.main_window, SCENE_TIMER);
            self.gif_player = None;
            self.scene_rx = None;
            self.scene_active = false;
            self.scene_webview = None;
            if let Some(secondary) = self.secondary.take() {
                let SecondaryScene { host, view, .. } = secondary;
                drop(view);
                if !host.is_null() && IsWindow(host) != 0 { DestroyWindow(host); }
            }
            if !self.callback.is_null() { (&*self.callback).active.store(false, Ordering::Release); }
            if !self.host.is_null() && IsWindow(self.host) != 0 { SetWindowLongPtrW(self.host, GWLP_USERDATA, 0); }
            if !self.player.is_null() {
                let player = self.player;
                self.player = null_mut();
                ((*(*player).vtable).shutdown)(player);
                ((*(*player).vtable).release)(player);
            }
            if !self.callback.is_null() {
                callback_release(self.callback);
                self.callback = null_mut();
            }
            if !self.host.is_null() && IsWindow(self.host) != 0 { DestroyWindow(self.host); }
            self.host = null_mut();
            self.host_state = None;
        }

        unsafe fn restore_previous(&mut self) {
            let Some(mut old) = self.previous.take() else { self.stop_current(); return; };
            KillTimer(self.main_window, TRANSITION_TIMER);
            self.transition_clock = None;
            self.stop_current();
            if !old.host.is_null() && IsWindow(old.host) != 0 {
                SetWindowRgn(old.host, null_mut(), 1);
                self.host = old.host;
                self.scene_webview = old.view.take();
                self.scene_active = self.scene_webview.is_some();
                if let Some(view) = self.scene_webview.as_ref() {
                    let volume = if self.gallery.sound() { "0.5" } else { "0" };
                    let _ = view.evaluate_script(&format!("if(window.scene) window.scene.setVolume({volume});"));
                }
                self.generation = old.generation;
                self.video = old.path.clone();
                self.active_video = old.path;
            } else {
                drop(old.view.take());
                self.active_video = None;
            }
        }

        unsafe fn apply(&mut self) -> Result<(), String> {
            let desired = self.video.clone();
            if self.previous.is_some() { self.restore_previous(); self.video = desired; }
            let result = self.apply_inner();
            if result.is_err() && self.previous.is_some() { self.restore_previous(); }
            if result.is_ok() { self.active_video = self.video.clone(); }
            result
        }

        unsafe fn apply_inner(&mut self) -> Result<(), String> {
            let video = self.video.clone().ok_or("Сначала выберите обои.")?;
            if !video.is_file() { return Err("Файл не найден. Выберите его снова.".into()); }
            let extension = video.extension().and_then(|x| x.to_str()).unwrap_or("");
            let is_scene = extension.eq_ignore_ascii_case("pkg");
            let is_gif = extension.eq_ignore_ascii_case("gif");
            let mode = self.gallery.screen_mode();
            let (second_path, second_rect) = if mode >= 2 {
                if !is_scene { return Err("Два отдельных экрана пока доступны только для PKG-сцен.".into()); }
                let second = if mode == 2 { video.clone() } else { self.gallery.second_wallpaper().ok_or("Выберите обои и нажмите «Для экрана 2» в коллекции.")? };
                if !second.is_file() || !second.extension().and_then(|x|x.to_str()).map_or(false, |x|x.eq_ignore_ascii_case("pkg")) { return Err("Файл PKG для второго экрана не найден.")?; }
                let monitors = displays()?;
                if monitors.len() < 2 { return Err("Второй монитор не найден. Включите расширение экранов в Windows.".into()); }
                (Some(second), Some(monitors[1]))
            } else { (None, None) };
            let should_transition = is_scene && mode == 0
                && self.scene_active && self.secondary.is_none() && self.scene_webview.is_some()
                && !self.host.is_null() && IsWindow(self.host) != 0;
            let previous = if should_transition {
                let old = PreviousScene { host: self.host, view: self.scene_webview.take(), path: self.active_video.clone(),
                    generation: self.generation, effect: self.gallery.transition(), waiting_since: Instant::now(), animation_since: None, exposed_since: None, duration_ms: self.gallery.transition_ms() };
                self.host = null_mut();
                self.scene_active = false;
                Some(old)
            } else { None };
            self.stop_current();
            self.previous = previous;
            if self.previous.is_some() && SetTimer(self.main_window, TRANSITION_TIMER, 100, null_mut()) == 0 {
                return Err("Не удалось запустить анимацию перехода.".into());
            }
            let layer = desktop_layer()?;
            if let Some(old) = self.previous.as_ref() {
                if GetParent(old.host) != layer.parent { return Err("Структура рабочего стола изменилась; прежние обои сохранены.".into()); }
            }
            let worker = layer.parent;
            let mut rect = Rect::default();
            let mut origin = Point::default();
            if GetClientRect(worker, &mut rect) == 0 || ClientToScreen(worker, &mut origin) == 0 {
                return Err("Не удалось определить размер рабочего стола.".into());
            }
            let (screen_x, screen_y, width, height) = if mode == 1 {
                (GetSystemMetrics(76), GetSystemMetrics(77), GetSystemMetrics(78), GetSystemMetrics(79))
            } else { (0, 0, GetSystemMetrics(0), GetSystemMetrics(1)) };
            if width <= 0 || height <= 0 || rect.right <= rect.left || rect.bottom <= rect.top {
                return Err("Некорректный размер рабочего стола.".into());
            }
            let extended = WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW | if layer.layered { WS_EX_LAYERED } else { 0 };
            self.host = CreateWindowExW(extended,
                wide("RustWallpaperHost").as_ptr(), wide("").as_ptr(),
                WS_CHILD | WS_CLIPCHILDREN | if self.previous.is_none() { WS_VISIBLE } else { 0 }, screen_x-origin.x, screen_y-origin.y, width, height,
                worker, null_mut(), self.instance, null_mut());
            if self.host.is_null() { return Err("Не удалось создать окно обоев на рабочем столе.".into()); }
            if layer.layered && (SetLayeredWindowAttributes(self.host, 0, 255, 2) == 0 ||
                SetWindowPos(self.host, layer.icons, 0, 0, 0, 0, 0x0001 | 0x0002 | 0x0010) == 0) {
                self.stop_current();
                return Err("Не удалось разместить окно под значками рабочего стола.".into());
            }
            if let Some(old) = self.previous.as_ref() {
                if SetWindowPos(self.host, old.host, 0, 0, 0, 0, 0x0001 | 0x0002 | 0x0010) == 0 {
                    return Err("Не удалось разместить новые обои под прежними.".into());
                }
                ShowWindow(self.host, 4);
            }
            if is_scene {
                let window = self.main_window as isize;
                let generation = self.generation;
                let (project, overrides) = self.gallery.project_payload(&video);
                let view = match web_scene::open(self.host as isize, video, width as u32, height as u32, self.gallery.scene_height(), self.gallery.scene_fps(), self.gallery.scripts, self.gallery.sound(), project, overrides, false, move |ok| {
                    unsafe { PostMessageW(window as Handle, WM_SCENE_WEB_STATUS, generation, if ok { 1 } else { 0 }); }
                }) {
                    Ok(view) => view,
                    Err(error) => { self.stop_current(); return Err(error); }
                };
                self.scene_webview = Some(view);
                self.scene_active = true;
                if let (Some(path), Some(screen)) = (second_path, second_rect) {
                    let w = screen.right-screen.left; let h = screen.bottom-screen.top;
                    if w <= 0 || h <= 0 { self.stop_current(); return Err("Неверный размер второго экрана.".into()); }
                    let host = CreateWindowExW(extended, wide("RustWallpaperHost").as_ptr(), wide("").as_ptr(),
                        WS_CHILD | WS_VISIBLE | WS_CLIPCHILDREN, screen.left-origin.x, screen.top-origin.y, w, h,
                        worker, null_mut(), self.instance, null_mut());
                    if host.is_null() { self.stop_current(); return Err("Не удалось создать обои для второго экрана.".into()); }
                    if layer.layered && (SetLayeredWindowAttributes(host, 0, 255, 2) == 0 || SetWindowPos(host, layer.icons, 0, 0, 0, 0, 0x0001 | 0x0002 | 0x0010) == 0) {
                        DestroyWindow(host); self.stop_current(); return Err("Не удалось разместить обои второго экрана.".into());
                    }
                    let (project, overrides) = self.gallery.project_payload(&path);
                    let view = match web_scene::open(host as isize, path.clone(), w as u32, h as u32, self.gallery.scene_height(), self.gallery.scene_fps(), self.gallery.scripts, false, project, overrides, true, move |ok| {
                        PostMessageW(window as Handle, WM_SCENE_WEB_STATUS, generation, if ok { 1 } else { 0 });
                    }) {
                        Ok(view) => view,
                        Err(error) => { DestroyWindow(host); self.stop_current(); return Err(error); }
                    };
                    self.secondary = Some(SecondaryScene { host, path, view });
                }
                return Ok(());
            }
            if is_gif {
                let player = match gif_wallpaper::Player::start(video.clone()) {
                    Ok(player) => player,
                    Err(error) => { self.stop_current(); return Err(error); }
                };
                self.host_state = Some(Box::new(HostState { content: HostContent::Gif(None) }));
                if let Some(state) = self.host_state.as_mut() {
                    SetWindowLongPtrW(self.host, GWLP_USERDATA, (&mut **state as *mut HostState) as isize);
                }
                self.gif_player = Some(player);
                if SetTimer(self.main_window, GIF_TIMER, 16, null_mut()) == 0 {
                    self.stop_current();
                    return Err("Не удалось запустить таймер GIF.".into());
                }
                return Ok(());
            }
            self.callback = Box::into_raw(Box::new(PlayerCallback {
                vtable: &CALLBACK_VTABLE, refs: AtomicU32::new(1), active: AtomicBool::new(true),
                window: self.main_window, generation: self.generation,
            }));
            let hr = MFPCreateMediaPlayer(null(), 0, 0, self.callback, self.host, &mut self.player);
            if hr < 0 || self.player.is_null() {
                self.stop_current();
                return Err(format!("Не удалось создать видеоплеер Windows: 0x{:08X}", hr as u32));
            }
            self.host_state = Some(Box::new(HostState { content: HostContent::Video(self.player) }));
            if let Some(state) = self.host_state.as_mut() {
                SetWindowLongPtrW(self.host, GWLP_USERDATA, (&mut **state as *mut HostState) as isize);
            }
            let table = &*(*self.player).vtable;
            let hr = (table.set_mute)(self.player, if self.gallery.sound() { 0 } else { 1 });
            if hr < 0 {
                self.stop_current();
                return Err(format!("Не удалось настроить звук видео: 0x{:08X}", hr as u32));
            }
            let path: Vec<u16> = video.as_os_str().encode_wide().chain(Some(0)).collect();
            let hr = (table.create_url)(self.player, path.as_ptr(), 0, 0, null_mut());
            if hr < 0 {
                self.stop_current();
                return Err(format!("Не удалось загрузить видео: 0x{:08X}", hr as u32));
            }
            Ok(())
        }

        unsafe fn start_transition_clock(&mut self) -> bool {
            KillTimer(self.main_window, TRANSITION_TIMER);
            self.transition_clock = transition_clock::Clock::start(self.main_window, self.generation).ok();
            if self.transition_clock.is_none() && SetTimer(self.main_window, TRANSITION_TIMER, 10, null_mut()) == 0 {
                self.restore_previous();
                self.status("Не удалось запустить таймер перехода. Прежние обои сохранены.");
                return false;
            }
            true
        }

        unsafe fn tick_transition(&mut self) {
            let Some(old) = self.previous.as_ref() else { KillTimer(self.main_window, TRANSITION_TIMER); return; };
            if old.host.is_null() || IsWindow(old.host) == 0 { self.discard_previous(); return; }
            let Some(start) = old.animation_since else {
                if old.waiting_since.elapsed() > Duration::from_secs(65) {
                    self.restore_previous();
                    self.status("Новая сцена слишком долго загружается. Прежние обои сохранены.");
                }
                return;
            };
            if let Some(exposed) = old.exposed_since {
                if exposed.elapsed() >= Duration::from_millis(250) { self.discard_previous(); }
                return;
            }
            let progress = if old.effect == 0 { 1.0 } else {
                (start.elapsed().as_secs_f32() / (old.duration_ms as f32 / 1000.0)).clamp(0.0, 1.0)
            };
            let smooth = progress * progress * (3.0 - 2.0 * progress);
            if !transitions::show(old.host, old.effect, smooth) {
                self.restore_previous();
                self.status("Переход недоступен. Прежние обои сохранены.");
                return;
            }
            if progress >= 1.0 {
                if let Some(old) = self.previous.as_mut() { old.exposed_since = Some(Instant::now()); }
                if !self.host.is_null() { UpdateWindow(self.host); }
            }
        }

        unsafe fn poll_gif(&mut self) {
            let result = match self.gif_player.as_ref() { Some(player) => player.try_frame(), None => return };
            match result {
                Ok(Some(frame)) => {
                    let first = match self.host_state.as_mut() {
                        Some(state) => match &mut state.content {
                            HostContent::Gif(current) => { let first = current.is_none(); *current = Some(frame); first },
                            _ => false,
                        },
                        None => false,
                    };
                    if !self.host.is_null() && IsWindow(self.host) != 0 { InvalidateRect(self.host, null(), 0); }
                    if first { self.status("GIF воспроизводится."); }
                }
                Ok(None) => {}
                Err(error) => {
                    self.stop();
                    self.status(&error);
                    if IsWindowVisible(self.main_window) == 0 { show_main(self.main_window); }
                }
            }
        }

        unsafe fn poll_scene(&mut self) {
            if !self.scene_active { return; }
            let result = self.scene_rx.as_ref().map(|rx| rx.try_recv());
            match result {
                Some(Ok(Ok(scene))) => {
                    self.scene_rx = None;
                    let snow = scene.snow;
                    let motion = scene.has_motion();
                    let has_text = !scene.text.is_empty();
                    let warning = scene.warning;
                    if let Some(state) = self.host_state.as_mut() {
                        if let HostContent::Scene(current, started) = &mut state.content {
                            *current = Some(scene);
                            *started = Instant::now();
                        }
                    }
                    if !snow && motion { SetTimer(self.main_window, SCENE_TIMER, 80, null_mut()); }
                    else if !snow && has_text { SetTimer(self.main_window, SCENE_TIMER, 1000, null_mut()); }
                    else if !snow { KillTimer(self.main_window, SCENE_TIMER); }
                    self.status(if motion { "PKG: движение приблизительное; не все эффекты поддерживаются." }
                        else if warning { "PKG: часть эффектов, слоёв или звук не поддерживается." }
                        else if snow { "Сцена загружена · снег воспроизводится приближённо." }
                        else { "Сцена загружена." });
                }
                Some(Ok(Err(error))) => {
                    self.stop(); self.status(&error);
                    if IsWindowVisible(self.main_window) == 0 { show_main(self.main_window); }
                    return;
                }
                Some(Err(std::sync::mpsc::TryRecvError::Disconnected)) => {
                    self.stop(); self.status("Загрузчик сцены остановился с ошибкой.");
                    if IsWindowVisible(self.main_window) == 0 { show_main(self.main_window); }
                    return;
                }
                _ => {}
            }
            if self.scene_rx.is_none() && !self.host.is_null() && IsWindow(self.host) != 0 {
                InvalidateRect(self.host, null(), 0);
            }
        }

        unsafe fn check_desktop(&mut self) {
            if (!self.player.is_null() || self.gif_player.is_some() || self.scene_active) && (self.host.is_null() || IsWindow(self.host) == 0) {
                self.stop();
                self.status("Рабочий стол изменился. Нажмите «Применить» снова.");
            }
        }
    }

    unsafe fn paint_gif(dc: Handle, hwnd: Handle, frame: &gif_wallpaper::Frame) {
        let mut rect = Rect::default();
        if GetClientRect(hwnd, &mut rect) == 0 { return; }
        let width = rect.right - rect.left; let height = rect.bottom - rect.top;
        if width <= 0 || height <= 0 || frame.width <= 0 || frame.height <= 0 { return; }
        let dest_ratio = width as f64 / height as f64;
        let src_ratio = frame.width as f64 / frame.height as f64;
        let (sx, sy, sw, sh) = if src_ratio > dest_ratio {
            let sw = ((frame.height as f64 * dest_ratio) as i32).clamp(1, frame.width);
            ((frame.width-sw)/2, 0, sw, frame.height)
        } else {
            let sh = ((frame.width as f64 / dest_ratio) as i32).clamp(1, frame.height);
            (0, (frame.height-sh)/2, frame.width, sh)
        };
        let info = BitmapInfo { header: BitmapInfoHeader {
            size: size_of::<BitmapInfoHeader>() as u32, width: frame.width, height: -frame.height,
            planes: 1, bit_count: 32, compression: 0, size_image: 0,
            x_pixels: 0, y_pixels: 0, colors_used: 0, colors_important: 0,
        }, colors: [0] };
        StretchDIBits(dc, 0, 0, width, height, sx, sy, sw, sh,
            frame.pixels.as_ptr() as *const c_void, &info, 0, 0x00cc0020);
    }

    unsafe fn paint_snow(dc: Handle, hwnd: Handle, time: f32) {
        let mut rect = Rect::default();
        if GetClientRect(hwnd, &mut rect) == 0 { return; }
        let w = (rect.right - rect.left).max(1) as f32;
        let h = (rect.bottom - rect.top).max(1) as f32;
        let brush = CreateSolidBrush(0x00e4e9ee);
        if brush.is_null() { return; }
        let old_brush = SelectObject(dc, brush);
        let old_pen = SelectObject(dc, GetStockObject(8));
        for i in 0..110u32 {
            let seed = i.wrapping_mul(2_654_435_761).wrapping_add(91_721);
            let x0 = (seed % 10000) as f32 / 10000.0;
            let y0 = ((seed >> 8) % 10000) as f32 / 10000.0;
            let speed = 0.025 + ((seed >> 16) % 80) as f32 / 1000.0;
            let y = (y0 + time * speed).fract() * h;
            let x = (x0 + (time * 0.35 + y0 * 9.0).sin() * 0.012 - time * speed * 0.06).rem_euclid(1.0) * w;
            let radius = 1 + ((seed >> 24) % 3) as i32;
            Ellipse(dc, x as i32-radius, y as i32-radius, x as i32+radius+1, y as i32+radius+1);
        }
        SelectObject(dc, old_pen); SelectObject(dc, old_brush); DeleteObject(brush);
    }

    unsafe fn paint_scene_text(dc: Handle, hwnd: Handle, scene: &scene_wallpaper::Scene) {
        if scene.text.is_empty() { return; }
        let mut rect = Rect::default();
        if GetClientRect(hwnd, &mut rect) == 0 { return; }
        let w = rect.right - rect.left; let h = rect.bottom - rect.top;
        let frame = &scene.frame;
        if w <= 0 || h <= 0 || frame.width <= 0 || frame.height <= 0 { return; }
        let dest_ratio = w as f64 / h as f64;
        let src_ratio = frame.width as f64 / frame.height as f64;
        let (sx, sy, sw, sh) = if src_ratio > dest_ratio {
            let sw = ((frame.height as f64 * dest_ratio) as i32).clamp(1, frame.width);
            ((frame.width-sw)/2, 0, sw, frame.height)
        } else {
            let sh = ((frame.width as f64 / dest_ratio) as i32).clamp(1, frame.height);
            (0, (frame.height-sh)/2, frame.width, sh)
        };
        let mut time = SystemTime::default();
        GetLocalTime(&mut time);
        let days = ["SUNDAY", "MONDAY", "TUESDAY", "WEDNESDAY", "THURSDAY", "FRIDAY", "SATURDAY"];
        let months = ["JAN", "FEB", "MAR", "APR", "MAY", "JUN", "JUL", "AUG", "SEP", "OCT", "NOV", "DEC"];
        SetBkMode(dc, 1);
        SetTextColor(dc, 0x00f2f2f2);
        for layer in &scene.text {
            let value = match &layer.kind {
                scene_wallpaper::TextKind::Clock { use_24h, seconds, delimiter } => {
                    let hour = if *use_24h { time.hour } else { let h = time.hour % 12; if h == 0 { 12 } else { h } };
                    if *seconds { format!("{hour:02}{delimiter}{:02}{delimiter}{:02}", time.minute, time.second) }
                    else { format!("{hour:02}{delimiter}{:02}", time.minute) }
                }
                scene_wallpaper::TextKind::Day => days.get(time.day_of_week as usize).unwrap_or(&"").chars().map(|c| c.to_string()).collect::<Vec<_>>().join(" "),
                scene_wallpaper::TextKind::Date => format!("{} {} {}", time.day, months.get(time.month.saturating_sub(1) as usize).unwrap_or(&""), time.year),
                scene_wallpaper::TextKind::Static(value) => value.clone(),
            };
            let x = ((layer.x * frame.width as f32 - sx as f32) * w as f32 / sw as f32) as i32;
            let y = ((layer.y * frame.height as f32 - sy as f32) * h as f32 / sh as f32) as i32;
            let font_size = (layer.size * frame.height as f32 * h as f32 / sh as f32).round().clamp(8.0, 128.0) as i32;
            let font = CreateFontW(-font_size,0,0,0,600,0,0,0,1,0,0,5,0,wide("Segoe UI").as_ptr());
            if font.is_null() { continue; }
            let old = SelectObject(dc, font);
            let mut bounds = Rect { left:x-w/3, top:y-font_size, right:x+w/3, bottom:y+font_size };
            let message = wide(&value);
            DrawTextW(dc, message.as_ptr(), -1, &mut bounds, 0x1 | 0x4 | 0x20 | 0x8000);
            SelectObject(dc, old); DeleteObject(font);
        }
    }

    unsafe extern "system" fn host_proc(hwnd: Handle, msg: u32, wp: Wparam, lp: Lparam) -> Lresult {
        let state = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut HostState;
        match msg {
            WM_ERASEBKGND => 1,
            WM_PAINT => {
                let mut paint: PaintStruct = zeroed();
                let dc = BeginPaint(hwnd, &mut paint);
                if !state.is_null() {
                    match &mut (*state).content {
                        HostContent::Video(player) => {
                            let player = *player;
                            if !player.is_null() { ((*(*player).vtable).update_video)(player); }
                        }
                        HostContent::Gif(Some(frame)) if !dc.is_null() => { paint_gif(dc, hwnd, frame); }
                        HostContent::Scene(Some(scene), started) if !dc.is_null() => {
                            let mut rect = Rect::default();
                            GetClientRect(hwnd, &mut rect);
                            let width = rect.right.max(1);
                            let height = rect.bottom.max(1);
                            let mem = CreateCompatibleDC(dc);
                            let bitmap = if !mem.is_null() { CreateCompatibleBitmap(dc, width, height) } else { null_mut() };
                            if !bitmap.is_null() {
                                let old = SelectObject(mem, bitmap);
                                if !old.is_null() && old as isize != -1 {
                                    paint_gif(mem, hwnd, scene.animated_frame(started.elapsed().as_secs_f32()));
                                    if scene.snow { paint_snow(mem, hwnd, started.elapsed().as_secs_f32()); }
                                    paint_scene_text(mem, hwnd, scene);
                                    BitBlt(dc, 0, 0, width, height, mem, 0, 0, 0x00cc0020);
                                    SelectObject(mem, old);
                                } else {
                                    paint_gif(dc, hwnd, scene.animated_frame(started.elapsed().as_secs_f32()));
                                    if scene.snow { paint_snow(dc, hwnd, started.elapsed().as_secs_f32()); }
                                    paint_scene_text(dc, hwnd, scene);
                                }
                                DeleteObject(bitmap);
                            } else {
                                paint_gif(dc, hwnd, scene.animated_frame(started.elapsed().as_secs_f32()));
                                if scene.snow { paint_snow(dc, hwnd, started.elapsed().as_secs_f32()); }
                                paint_scene_text(dc, hwnd, scene);
                            }
                            if !mem.is_null() { DeleteDC(mem); }
                        }
                        _ => {}
                    }
                }
                EndPaint(hwnd, &paint);
                0
            }
            WM_SIZE => {
                if !state.is_null() {
                    if let HostContent::Video(player) = &(*state).content {
                        let player = *player;
                        if !player.is_null() { ((*(*player).vtable).update_video)(player); }
                    } else { InvalidateRect(hwnd, null(), 0); }
                }
                0
            }
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }

    unsafe fn show_main(hwnd: Handle) {
        ShowWindow(hwnd, 9);
        SetForegroundWindow(hwnd);
        gallery::redraw(hwnd);
    }

    unsafe fn hide_ui(app: &mut App) {
        if app.tray_active {
            app.gallery.capturing = false;
            ShowWindow(app.main_window, 0);
        } else {
            app.status("Не удалось добавить значок в трей. Окно оставлено открытым.");
        }
    }

    unsafe fn animate_picker(app: &mut App) {
        let elapsed = app.picker_since.elapsed().as_secs_f32();
        let progress = (elapsed / Duration::from_millis(ANIM_MS).as_secs_f32()).clamp(0.0, 1.0);
        let eased = 1.0 - (1.0 - progress).powi(3);
        let target = if app.picker_open { 1.0 } else { 0.0 };
        app.picker_alpha = app.picker_start + (target - app.picker_start) * eased;
        let alpha = (app.picker_alpha * 255.0).round().clamp(0.0, 255.0) as u8;
        SetLayeredWindowAttributes(app.picker, 0x00ff00ff, alpha, 0x1 | 0x2);
        if progress >= 1.0 {
            KillTimer(app.main_window, ANIM_TIMER);
            app.picker_alpha = target;
            if !app.picker_open {
                KillTimer(app.main_window, SCROLL_TIMER);
                app.gallery.reset_picker_scroll();
                ShowWindow(app.picker, 0);
            }
        }
    }

    unsafe fn set_picker_open(app: &mut App, open: bool) {
        if app.picker.is_null() || IsWindow(app.picker) == 0 {
            if open { app.status("Не удалось создать быстрый выбор."); show_main(app.main_window); }
            return;
        }
        if open && app.gallery.is_empty() {
            app.status("Сначала добавьте файл в библиотеку.");
            show_main(app.main_window);
            return;
        }
        if app.picker_open == open && (open || IsWindowVisible(app.picker) == 0) { return; }
        if IsWindowVisible(app.picker) != 0 { animate_picker(app); }
        app.picker_start = app.picker_alpha;
        app.picker_open = open;
        app.picker_since = Instant::now();
        if open {
            app.gallery.prepare();
            if app.tray_active && IsWindowVisible(app.main_window) != 0 { ShowWindow(app.main_window, 0); }
            SetLayeredWindowAttributes(app.picker, 0x00ff00ff, (app.picker_alpha * 255.0) as u8, 0x1 | 0x2);
            gallery::redraw(app.picker);
            ShowWindow(app.picker, SW_SHOW);
            SetForegroundWindow(app.picker);
        }
        if SetTimer(app.main_window, ANIM_TIMER, 16, null_mut()) == 0 {
            app.picker_since = Instant::now() - Duration::from_millis(ANIM_MS);
            animate_picker(app);
        }
    }

    unsafe fn toggle_picker(app: &mut App) {
        set_picker_open(app, !app.picker_open);
    }

    unsafe fn tray_menu(hwnd: Handle) {
        let menu = CreatePopupMenu();
        if menu.is_null() { return; }
        AppendMenuW(menu, 0, 1, wide("Открыть").as_ptr());
        AppendMenuW(menu, 0, 2, wide("Быстрый выбор").as_ptr());
        AppendMenuW(menu, 0x800, 0, null());
        AppendMenuW(menu, 0, 3, wide("Выход").as_ptr());
        let mut point = Point::default();
        GetCursorPos(&mut point);
        SetForegroundWindow(hwnd);
        let choice = TrackPopupMenu(menu, 0x100 | 0x80 | 0x2, point.x, point.y, 0, hwnd, null());
        DestroyMenu(menu);
        match choice {
            1 => show_main(hwnd),
            2 => { PostMessageW(hwnd, WM_PICKER_TOGGLE, 0, 0); },
            3 => { PostMessageW(hwnd, WM_APP_EXIT, 0, 0); },
            _ => {}
        }
    }

    unsafe fn scroll_picker(app: &mut App, amount: i32) {
        let active = app.gallery.is_picker_scrolling();
        app.gallery.scroll_picker(amount);
        if !active && SetTimer(app.main_window, SCROLL_TIMER, 16, null_mut()) == 0 {
            app.gallery.reset_picker_scroll();
        }
        gallery::redraw(app.picker);
        gallery::redraw(app.main_window);
    }

    unsafe extern "system" fn picker_proc(hwnd: Handle, msg: u32, wp: Wparam, lp: Lparam) -> Lresult {
        let main = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as Handle;
        let ptr = if main.is_null() { null_mut() } else { GetWindowLongPtrW(main, GWLP_USERDATA) as *mut App };
        match msg {
            WM_ERASEBKGND => 1,
            WM_PAINT if !ptr.is_null() => { gallery::paint_picker(hwnd, &mut (*ptr).gallery); 0 }
            gallery::WHEEL if !ptr.is_null() => {
                let delta = ((wp >> 16) as u16) as i16;
                if delta != 0 {
                    scroll_picker(&mut *ptr, if delta > 0 { -1 } else { 1 });
                }
                0
            }
            gallery::KEYDOWN if !ptr.is_null() => {
                match wp {
                    0x1b => { PostMessageW(main, WM_PICKER_HIDE, 0, 0); },
                    0x25 | 0x27 => {
                        scroll_picker(&mut *ptr, if wp == 0x25 { -1 } else { 1 });
                    }
                    0x0d => { PostMessageW(main, WM_PICKER_HIDE, 0, 0); PostMessageW(main, WM_PICKER_APPLY, 0, 0); },
                    _ => {}
                }
                0
            }
            gallery::LBUTTON if !ptr.is_null() => {
                let x = (lp as u16) as i16 as i32;
                let y = ((lp >> 16) as u16) as i16 as i32;
                let mut rect = Rect::default();
                GetClientRect(hwnd, &mut rect);
                if let Some(index) = (*ptr).gallery.picker_hit(rect.right, rect.bottom, x, y) {
                    KillTimer(main, SCROLL_TIMER);
                    (*ptr).gallery.reset_picker_scroll();
                    (*ptr).gallery.select(index);
                    gallery::redraw(main);
                    PostMessageW(main, WM_PICKER_HIDE, 0, 0);
                    PostMessageW(main, WM_PICKER_APPLY, 0, 0);
                }
                0
            }
            WM_CLOSE if !main.is_null() => { PostMessageW(main, WM_PICKER_HIDE, 0, 0); 0 }
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }

    unsafe fn apply_selection(app: &mut App) {
        app.video = app.gallery.current();
        match app.apply() {
            Ok(()) => app.status("Загрузка обоев…"),
            Err(error) => { app.status(&error); if IsWindowVisible(app.main_window) == 0 { show_main(app.main_window); } },
        }
    }

    unsafe fn commit_scene_fps(app: &mut App, fps: u32) {
        if fps == app.gallery.scene_fps() { gallery::redraw(app.main_window); return; }
        match app.gallery.set_scene_fps(fps) {
            Ok(()) => {
                let code = format!("window.__targetFps={fps}; if(window.scene) window.scene.setFps({fps});");
                let mut failed = false;
                if let Some(view) = app.scene_webview.as_ref() { failed |= view.evaluate_script(&code).is_err(); }
                if let Some(second) = app.secondary.as_ref() { failed |= second.view.evaluate_script(&code).is_err(); }
                if failed { app.status("FPS сохранён, но обновить сцену не удалось. Примените обои снова."); }
                else if app.scene_active { app.status("FPS сохранён и отправлен сцене без перезапуска."); }
                else { app.status("Частота кадров PKG сохранена."); }
            }
            Err(error) => app.status(&error),
        }
        gallery::redraw(app.main_window);
    }

    unsafe fn set_shortcut_from_key(app: &mut App, key: usize) {
        if key == 0x1b { app.gallery.capturing = false; app.status("Изменение клавиши отменено."); return; }
        if !(0x30..=0x39).contains(&key) && !(0x41..=0x5a).contains(&key) { return; }
        let mut parts = Vec::new();
        if GetKeyState(0x11) < 0 { parts.push("Ctrl".to_string()); }
        if GetKeyState(0x12) < 0 { parts.push("Alt".to_string()); }
        if GetKeyState(0x10) < 0 { parts.push("Shift".to_string()); }
        if parts.is_empty() { app.status("Добавьте Alt, Ctrl или Shift."); return; }
        parts.push((key as u8 as char).to_string());
        let text = parts.join("+");
        app.gallery.capturing = false;
        match app.gallery.change_hotkey(&text) {
            Ok(()) => app.status("Горячая клавиша сохранена."),
            Err(error) => app.status(&error),
        }
    }

    unsafe extern "system" fn window_proc(hwnd: Handle, msg: u32, wp: Wparam, lp: Lparam) -> Lresult {
        let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut App;
        if !ptr.is_null() && (*ptr).taskbar_created != 0 && msg == (*ptr).taskbar_created {
            let app = &mut *ptr;
            app.tray_active = tray_add(hwnd);
            if !app.tray_active { app.status("Не удалось восстановить значок в трее."); show_main(hwnd); }
            return 0;
        }
        match msg {
            WM_ERASEBKGND => 1,
            WM_PAINT if !ptr.is_null() => { let app = &*ptr; gallery::paint(hwnd, &app.gallery, &app.status_line); 0 }
            0x007e if !ptr.is_null() => {
                let app = &mut *ptr;
                if !app.host.is_null() { app.stop(); app.status("Экраны изменились. Выберите «Применить» снова."); }
                0
            }
            WM_TIMER if !ptr.is_null() && wp == TIMER => { let app = &mut *ptr; app.check_desktop(); app.gallery.poll(app.picker); 0 }
            WM_TIMER if !ptr.is_null() && wp == ANIM_TIMER => { animate_picker(&mut *ptr); 0 }
            WM_TIMER if !ptr.is_null() && wp == SCROLL_TIMER => {
                let app = &mut *ptr;
                let moving = app.gallery.tick_picker();
                if !app.picker.is_null() { gallery::redraw(app.picker); }
                if !moving { KillTimer(hwnd, SCROLL_TIMER); }
                0
            }
            WM_TIMER if !ptr.is_null() && wp == TRANSITION_TIMER => { (&mut *ptr).tick_transition(); 0 }
            WM_TRANSITION_TICK if !ptr.is_null() => {
                let app = &mut *ptr;
                if wp == app.generation {
                    if let Some(clock) = app.transition_clock.as_ref() { clock.acknowledge(); }
                    app.tick_transition();
                }
                0
            }
            WM_TIMER if !ptr.is_null() && wp == GIF_TIMER => { (&mut *ptr).poll_gif(); 0 }
            WM_TIMER if !ptr.is_null() && wp == SCENE_TIMER => { (&mut *ptr).poll_scene(); 0 }
            WM_SCENE_WEB_STATUS if !ptr.is_null() => {
                let app = &mut *ptr;
                if wp == app.generation && app.scene_webview.is_some() {
                    if lp == 1 {
                        let mut start_clock = false;
                        if let Some(old) = app.previous.as_mut() {
                            if old.animation_since.is_none() {
                                if let Some(view) = old.view.as_ref() { let _ = view.evaluate_script("if(window.scene) window.scene.setVolume(0);"); }
                                old.animation_since = Some(Instant::now());
                                start_clock = true;
                            }
                        }
                        if start_clock && !app.start_transition_clock() { return 0; }
                        let code = format!("window.__targetFps={0}; if(window.scene) window.scene.setFps({0});", app.gallery.scene_fps());
                        if let Some(view) = app.scene_webview.as_ref() { let _ = view.evaluate_script(&code); }
                        if let Some(second) = app.secondary.as_ref() { let _ = second.view.evaluate_script(&code); }
                        app.status(if app.gallery.scripts { "Сцена работает · скрипты включены" } else { "Сцена работает · скрипты выключены" });
                    }
                    else { if app.previous.is_some() { app.restore_previous(); } else { app.stop(); } app.status("Не удалось воспроизвести PKG. Прежние обои сохранены, если были доступны."); if IsWindowVisible(hwnd) == 0 { show_main(hwnd); } }
                }
                0
            }
            gallery::HOTKEY_MESSAGE if !ptr.is_null() => {
                let app = &mut *ptr;
                if app.gallery.handles_hotkey(wp) {
                    if app.gallery.capturing { app.gallery.capturing = false; app.status("Сочетание не изменилось."); }
                    else { toggle_picker(app); }
                }
                0
            }
            gallery::WHEEL if !ptr.is_null() => {
                let app = &mut *ptr;
                if !app.gallery.settings && !app.gallery.optimization && !app.gallery.properties && !app.gallery.help && !app.gallery.transition_page {
                    let delta = ((wp >> 16) as u16) as i16;
                    if delta != 0 { app.gallery.move_by(if delta > 0 { -1 } else { 1 }); gallery::redraw(hwnd); }
                }
                0
            }
            gallery::KEYDOWN | 0x0104 if !ptr.is_null() => {
                let app = &mut *ptr;
                if app.gallery.capturing { set_shortcut_from_key(app, wp); }
                else if wp == 0x1b {
                    if app.gallery.settings || app.gallery.optimization || app.gallery.properties || app.gallery.help || app.gallery.transition_page { app.gallery.settings = false; app.gallery.optimization = false; app.gallery.properties = false; app.gallery.help = false; app.gallery.transition_page = false; gallery::redraw(hwnd); }
                    else { hide_ui(app); }
                } else if !app.gallery.settings && !app.gallery.optimization && !app.gallery.properties && !app.gallery.help && !app.gallery.transition_page && (wp == 0x25 || wp == 0x27) {
                    app.gallery.move_by(if wp == 0x25 { -1 } else { 1 }); gallery::redraw(hwnd);
                } else if !app.gallery.settings && !app.gallery.optimization && !app.gallery.properties && !app.gallery.help && !app.gallery.transition_page && wp == 0x0d { apply_selection(app); }
                else if msg == 0x0104 { return DefWindowProcW(hwnd, msg, wp, lp); }
                0
            }
            0x0200 if !ptr.is_null() && (&*ptr).gallery.duration_dragging() => {
                let x = (lp as u16) as i16 as i32;
                let mut rect = Rect::default(); GetClientRect(hwnd, &mut rect);
                (&mut *ptr).gallery.duration_drag(rect.right, x);
                gallery::redraw(hwnd); 0
            }
            0x0202 if !ptr.is_null() && (&*ptr).gallery.duration_dragging() => {
                let x = (lp as u16) as i16 as i32;
                let mut rect = Rect::default(); GetClientRect(hwnd, &mut rect);
                let app = &mut *ptr;
                app.gallery.duration_drag(rect.right, x);
                let value = app.gallery.duration_finish();
                if GetCapture() == hwnd { ReleaseCapture(); }
                if let Some(ms) = value { match app.gallery.set_transition_ms(ms) { Ok(()) => app.status("Скорость перехода сохранена."), Err(e) => app.status(&e) } }
                gallery::redraw(hwnd); 0
            }
            0x0215 if !ptr.is_null() && (&*ptr).gallery.duration_dragging() => {
                (&mut *ptr).gallery.duration_finish(); gallery::redraw(hwnd); 0
            }
            0x0200 if !ptr.is_null() && (&*ptr).gallery.fps_dragging() => {
                let x = (lp as u16) as i16 as i32;
                let mut rect = Rect::default(); GetClientRect(hwnd, &mut rect);
                (&mut *ptr).gallery.fps_drag(rect.right, x);
                gallery::redraw(hwnd);
                0
            }
            0x0202 if !ptr.is_null() && (&*ptr).gallery.fps_dragging() => {
                let x = (lp as u16) as i16 as i32;
                let mut rect = Rect::default(); GetClientRect(hwnd, &mut rect);
                let app = &mut *ptr;
                app.gallery.fps_drag(rect.right, x);
                let fps = app.gallery.fps_finish();
                if GetCapture() == hwnd { ReleaseCapture(); }
                if let Some(fps) = fps { commit_scene_fps(app, fps); }
                0
            }
            0x0215 if !ptr.is_null() && (&*ptr).gallery.fps_dragging() => {
                (&mut *ptr).gallery.fps_finish();
                gallery::redraw(hwnd);
                0
            }
            gallery::LBUTTON if !ptr.is_null() => {
                let x = (lp as u16) as i16 as i32;
                let y = ((lp >> 16) as u16) as i16 as i32;
                let mut rect = Rect::default(); GetClientRect(hwnd, &mut rect);
                let action = (&*ptr).gallery.hit(rect.right, rect.bottom, x, y);
                if let gallery::Action::TransitionDuration = action {
                    let app = &mut *ptr;
                    app.gallery.duration_drag(rect.right, x);
                    SetCapture(hwnd);
                    if GetCapture() != hwnd {
                        if let Some(ms) = app.gallery.duration_finish() { match app.gallery.set_transition_ms(ms) { Ok(()) => app.status("Скорость перехода сохранена."), Err(e) => app.status(&e) } }
                    }
                    gallery::redraw(hwnd); return 0;
                }
                if let gallery::Action::Fps(_) = action {
                    let app = &mut *ptr;
                    app.gallery.fps_drag(rect.right, x);
                    SetCapture(hwnd);
                    if GetCapture() != hwnd {
                        if let Some(fps) = app.gallery.fps_finish() { commit_scene_fps(app, fps); }
                    } else { gallery::redraw(hwnd); }
                    return 0;
                }
                if let gallery::Action::Add = action {
                    let picked = pick_file(hwnd);
                    let app = &mut *ptr;
                    match picked {
                        Ok(Some(path)) => match app.gallery.add(path) {
                            Ok(()) => app.status("Файл добавлен. Выберите «Применить»."),
                            Err(error) => app.status(&error),
                        },
                        Ok(None) => {},
                        Err(error) => app.status(&error),
                    }
                    gallery::redraw(hwnd);
                    return 0;
                }
                if let gallery::Action::ImportJson = action {
                    let picked = pick_file_with(hwnd, true);
                    let app = &mut *ptr;
                    match picked {
                        Ok(Some(path)) => match app.gallery.import_project(path) {
                            Ok(()) if app.scene_active && (app.video == app.gallery.current() || app.secondary.as_ref().map(|s| &s.path) == app.gallery.current().as_ref()) => match app.apply() {
                                Ok(()) => app.status("JSON добавлен. Сцена перезапускается."),
                                Err(error) => app.status(&error),
                            },
                            Ok(()) => app.status("JSON привязан к выбранной сцене."),
                            Err(error) => app.status(&error),
                        },
                        Ok(None) => {},
                        Err(error) => app.status(&error),
                    }
                    gallery::redraw(hwnd);
                    return 0;
                }
                let app = &mut *ptr;
                match action {
                    gallery::Action::Apply => apply_selection(app),
                    gallery::Action::Stop => { app.stop(); app.status("Обои остановлены."); }
                    gallery::Action::Remove => {
                        match app.gallery.remove_selected() {
                            Ok(Some(path)) => {
                                if app.video.as_ref() == Some(&path) {
                                    app.stop();
                                    app.video = None;
                                }
                                app.status("Удалено из библиотеки. Сам файл сохранён.");
                            }
                            Ok(None) => app.status("Библиотека пуста."),
                            Err(error) => app.status(&error),
                        }
                        if !app.picker.is_null() { gallery::redraw(app.picker); }
                    }
                    gallery::Action::Collection => { app.gallery.settings = false; app.gallery.optimization = false; app.gallery.properties = false; app.gallery.help = false; app.gallery.transition_page = false; app.gallery.capturing = false; gallery::redraw(hwnd); }
                    gallery::Action::Settings => { app.gallery.settings = true; app.gallery.optimization = false; app.gallery.properties = false; app.gallery.help = false; app.gallery.transition_page = false; app.gallery.capturing = false; gallery::redraw(hwnd); }
                    gallery::Action::Optimization => { app.gallery.settings = false; app.gallery.optimization = true; app.gallery.properties = false; app.gallery.help = false; app.gallery.transition_page = false; app.gallery.capturing = false; gallery::redraw(hwnd); }
                    gallery::Action::Properties => { app.gallery.settings = false; app.gallery.optimization = false; app.gallery.properties = true; app.gallery.help = false; app.gallery.transition_page = false; app.gallery.capturing = false; gallery::redraw(hwnd); }
                    gallery::Action::Help => { app.gallery.settings = false; app.gallery.optimization = false; app.gallery.properties = false; app.gallery.help = true; app.gallery.transition_page = false; app.gallery.capturing = false; gallery::redraw(hwnd); }
                    gallery::Action::TransitionPage => { app.gallery.settings = false; app.gallery.optimization = false; app.gallery.properties = false; app.gallery.help = false; app.gallery.transition_page = true; app.gallery.capturing = false; gallery::redraw(hwnd); }
                    gallery::Action::OpenTelegram | gallery::Action::OpenGithub => {
                        let url = if matches!(action, gallery::Action::OpenTelegram) { gallery::TELEGRAM_URL } else { gallery::GITHUB_URL };
                        let result = ShellExecuteW(hwnd, wide("open").as_ptr(), wide(url).as_ptr(), null(), null(), SW_SHOW);
                        if result as isize <= 32 { app.status("Не удалось открыть ссылку в браузере."); }
                    }
                    gallery::Action::PropertyPage(delta) => { app.gallery.property_page(delta); gallery::redraw(hwnd); }
                    gallery::Action::Property(row, delta) => {
                        match app.gallery.change_property(row, delta) {
                            Ok(()) => {
                                let selected = app.gallery.current();
                                let mut sent = false;
                                let mut failed = false;
                                if let Some(path) = selected.as_ref() {
                                    let (_, overrides) = app.gallery.project_payload(path);
                                    let code = format!("if(window.scene) window.scene.setProperties({});", overrides);
                                    if app.video.as_ref() == Some(path) {
                                        if let Some(view) = app.scene_webview.as_ref() { sent = true; failed |= view.evaluate_script(&code).is_err(); }
                                    }
                                    if let Some(other) = app.secondary.as_ref().filter(|other| &other.path == path) {
                                        sent = true; failed |= other.view.evaluate_script(&code).is_err();
                                    }
                                }
                                if failed { app.status("Параметр сохранён, но его не удалось отправить сцене."); }
                                else if sent { app.status("Параметр сохранён и отправлен сцене."); }
                                else { app.status("Параметр сохранён для выбранной сцены."); }
                            }
                            Err(error) => app.status(&error),
                        }
                        gallery::redraw(hwnd);
                    }
                    gallery::Action::TransitionDuration => {},
                    gallery::Action::Transition(mode) => {
                        match app.gallery.set_transition(mode) {
                            Ok(()) => app.status("Эффект перехода сохранён для PKG на основном экране."),
                            Err(error) => app.status(&error),
                        }
                        gallery::redraw(hwnd);
                    }
                    gallery::Action::ToggleAutostart => {
                        let enable = !app.gallery.autostart;
                        match startup::set_enabled(enable) {
                            Ok(()) => { app.gallery.autostart = enable; app.status(if enable { "Автозапуск включён для текущего пользователя." } else { "Автозапуск выключен." }); }
                            Err(error) => app.status(&error),
                        }
                        gallery::redraw(hwnd);
                    }
                    gallery::Action::ToggleSound => {
                        match app.gallery.toggle_sound() {
                            Ok(()) if !app.player.is_null() || app.scene_active => match app.apply() {
                                Ok(()) => app.status("Настройка звука сохранена. Обои перезапускаются."),
                                Err(error) => app.status(&error),
                            },
                            Ok(()) => app.status("Настройка звука сохранена."),
                            Err(error) => app.status(&error),
                        }
                        gallery::redraw(hwnd);
                    }
                    gallery::Action::ToggleScripts => {
                        if !app.gallery.scripts {
                            let text = "Скрипты из PKG выполняют программный код. Защита движка не является надёжной песочницей. Включайте только для файлов, которым полностью доверяете. После выхода настройка отключится.\n\nВключить скрипты до закрытия программы?";
                            if MessageBoxW(hwnd, wide(text).as_ptr(), wide("Скрипты обоев").as_ptr(), 0x00000004 | 0x00000030 | 0x00000100) != 6 { return 0; }
                        }
                        app.gallery.scripts = !app.gallery.scripts;
                        if app.scene_active {
                            match app.apply() {
                                Ok(()) => app.status("Настройка изменена. Сцена перезапускается."),
                                Err(error) => app.status(&error),
                            }
                        } else { app.status(if app.gallery.scripts { "Скрипты включены до выхода из программы." } else { "Скрипты выключены." }); }
                        gallery::redraw(hwnd);
                    }
                    gallery::Action::ScreenMode(mode) => {
                        let previous = app.gallery.screen_mode();
                        match app.gallery.set_screen_mode(mode) {
                            Ok(()) if !app.player.is_null() || app.gif_player.is_some() || app.scene_active => match app.apply() {
                                Ok(()) => app.status("Режим экранов сохранён. Обои перезапускаются."),
                                Err(error) => {
                                    if let Err(restore) = app.gallery.set_screen_mode(previous) {
                                        app.status(&format!("{error} Не удалось вернуть прежний режим: {restore}"));
                                    } else { app.status(&error); }
                                }
                            },
                            Ok(()) => app.status("Режим экранов сохранён."),
                            Err(error) => app.status(&error),
                        }
                        gallery::redraw(hwnd);
                    }
                    gallery::Action::AssignSecond => {
                        match app.gallery.assign_second() {
                            Ok(()) if app.gallery.screen_mode() == 3 && app.scene_active => match app.apply() {
                                Ok(()) => app.status("Обои второго экрана сохранены и загружаются."),
                                Err(error) => app.status(&error),
                            },
                            Ok(()) => app.status("Выбраны обои для экрана 2. Включите режим «Разные PKG»."),
                            Err(error) => app.status(&error),
                        }
                        gallery::redraw(hwnd);
                    }
                    gallery::Action::Fps(fps) => commit_scene_fps(app, fps),
                    gallery::Action::Quality(height) => {
                        match app.gallery.set_scene_height(height) {
                            Ok(()) if app.scene_active => match app.apply() {
                                Ok(()) => app.status("Качество сохранено. Сцена перезапускается."),
                                Err(error) => app.status(&error),
                            },
                            Ok(()) => app.status("Качество сцен PKG сохранено."),
                            Err(error) => app.status(&error),
                        }
                        gallery::redraw(hwnd);
                    }
                    gallery::Action::Hide => hide_ui(app),
                    gallery::Action::Exit => { PostMessageW(hwnd, WM_APP_EXIT, 0, 0); },
                    gallery::Action::Capture => { app.gallery.capturing = true; app.status("Нажмите сочетание клавиш."); }
                    gallery::Action::Select(index) => { app.gallery.select(index); gallery::redraw(hwnd); }
                    gallery::Action::Previous => { app.gallery.move_by(-1); gallery::redraw(hwnd); }
                    gallery::Action::Next => { app.gallery.move_by(1); gallery::redraw(hwnd); }
                    gallery::Action::Add | gallery::Action::ImportJson | gallery::Action::None => {}
                }
                0
            }
            WM_TRAY if !ptr.is_null() => {
                if wp == 1 {
                    match lp as u32 {
                        0x0202 => show_main(hwnd),
                        0x0205 => tray_menu(hwnd),
                        _ => {}
                    }
                }
                0
            }
            WM_PICKER_TOGGLE if !ptr.is_null() => { toggle_picker(&mut *ptr); 0 }
            WM_PICKER_HIDE if !ptr.is_null() => { set_picker_open(&mut *ptr, false); 0 }
            WM_PICKER_APPLY if !ptr.is_null() => {
                let app = &mut *ptr;
                apply_selection(app);
                if app.player.is_null() && app.gif_player.is_none() && !app.scene_active { show_main(hwnd); }
                0
            }
            WM_PLAYER_STATUS if !ptr.is_null() => {
                let app = &mut *ptr;
                if wp == app.generation && !app.player.is_null() {
                    let hr = lp as i32;
                    if hr < 0 {
                        app.stop();
                        app.status(&format!("Ошибка видео: 0x{:08X}. Проверьте формат и кодек.", hr as u32));
                        if IsWindowVisible(hwnd) == 0 { show_main(hwnd); }
                    } else { app.status(if app.gallery.sound() { "Видео воспроизводится · звук включён" } else { "Видео воспроизводится · звук выключен" }); }
                }
                0
            }
            WM_CLOSE if !ptr.is_null() => { hide_ui(&mut *ptr); 0 }
            WM_APP_EXIT => { DestroyWindow(hwnd); 0 }
            WM_DESTROY => {
                KillTimer(hwnd, TIMER);
                KillTimer(hwnd, ANIM_TIMER);
                KillTimer(hwnd, SCROLL_TIMER);
                if !ptr.is_null() {
                    let mut app = Box::from_raw(ptr);
                    if app.tray_active { tray_remove(hwnd); }
                    if !app.picker.is_null() && IsWindow(app.picker) != 0 {
                        SetWindowLongPtrW(app.picker, GWLP_USERDATA, 0);
                        DestroyWindow(app.picker);
                    }
                    SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                    app.gallery.close();
                    app.stop();
                }
                PostQuitMessage(0);
                0
            }
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }

    unsafe fn register(instance: Handle, name: &str, proc: WndProc) -> bool {
        let class = wide(name);
        let wc = WndClass {
            style: 0, wnd_proc: proc, cls_extra: 0, wnd_extra: 0, instance,
            icon: null_mut(), cursor: LoadCursorW(null_mut(), 32512usize as *const u16),
            background: (COLOR_WINDOW + 1) as Handle, menu_name: null(), class_name: class.as_ptr(),
        };
        RegisterClassW(&wc) != 0
    }

    pub fn run() {
        unsafe {
            SetProcessDPIAware();
            let com = CoInitializeEx(null_mut(), 2);
            if com < 0 { show_error(null_mut(), &format!("Не удалось запустить COM: 0x{:08X}", com as u32)); return; }
            let mf = MFStartup(0x00020070, 0);
            if mf < 0 {
                show_error(null_mut(), &format!("Не удалось запустить Media Foundation: 0x{:08X}", mf as u32));
                CoUninitialize();
                return;
            }
            let instance = GetModuleHandleW(null());
            if instance.is_null() || !register(instance, "RustWallpaperMain", Some(window_proc))
                || !register(instance, "RustWallpaperHost", Some(host_proc))
                || !register(instance, "RustWallpaperPicker", Some(picker_proc)) {
                show_error(null_mut(), "Не удалось инициализировать интерфейс.");
                MFShutdown(); CoUninitialize();
                return;
            }
            let screen_w = GetSystemMetrics(0).max(1);
            let screen_h = GetSystemMetrics(1).max(1);
            let hwnd = CreateWindowExW(0,
                wide("RustWallpaperMain").as_ptr(), wide(gallery::APP_NAME).as_ptr(),
                WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX,
                CW_USEDEFAULT, CW_USEDEFAULT, 1000, 610, null_mut(), null_mut(), instance, null_mut());
            if hwnd.is_null() { show_error(null_mut(), "Не удалось создать окно программы."); MFShutdown(); CoUninitialize(); return; }
            let picker_h = (screen_h / 2).clamp(220, 700).min(screen_h);
            let mut picker = CreateWindowExW(WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_LAYERED,
                wide("RustWallpaperPicker").as_ptr(), wide("").as_ptr(), WS_POPUP,
                0, (screen_h-picker_h)/2, screen_w, picker_h, null_mut(), null_mut(), instance, null_mut());
            if !picker.is_null() && SetLayeredWindowAttributes(picker, 0x00ff00ff, 0, 1) == 0 {
                DestroyWindow(picker);
                picker = null_mut();
            }
            let mut app = Box::new(App { instance, video: None, active_video: None, previous: None, transition_clock: None, status_line: "Колесо мыши — выбор · Enter — применить".into(),
                host: null_mut(), player: null_mut(), gif_player: None, scene_rx: None, scene_active: false, scene_webview: None, secondary: None, host_state: None,
                callback: null_mut(), generation: 0, main_window: hwnd,
                picker, picker_alpha: 0.0, picker_start: 0.0, picker_open: false,
                picker_since: Instant::now(), tray_active: false, taskbar_created: RegisterWindowMessageW(wide("TaskbarCreated").as_ptr()),
                gallery: gallery::Gallery::new(hwnd) });
            if let Some(error) = app.gallery.load_error().map(str::to_string) { app.status(&error); }
            if !app.gallery.register_initial() { app.status("Горячая клавиша занята. Измените её в настройках."); }
            if picker.is_null() { app.status("Не удалось создать быстрый выбор."); }
            app.tray_active = tray_add(hwnd);
            if !app.tray_active { app.status("Значок в трее недоступен. Окно нельзя скрыть."); }
            app.gallery.prepare();
            let start_hidden = std::env::args_os().skip(1).any(|arg| arg == "--startup") && app.tray_active;
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, Box::into_raw(app) as isize);
            if !picker.is_null() { SetWindowLongPtrW(picker, GWLP_USERDATA, hwnd as isize); }
            SetTimer(hwnd, TIMER, 350, null_mut());
            if !start_hidden { ShowWindow(hwnd, SW_SHOW); UpdateWindow(hwnd); }
            let mut msg: Msg = zeroed();
            loop {
                let result = GetMessageW(&mut msg, null_mut(), 0, 0);
                if result <= 0 { break; }
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            MFShutdown();
            CoUninitialize();
        }
    }
}

#[cfg(windows)]
fn main() { app::run(); }
