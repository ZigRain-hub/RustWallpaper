use std::borrow::Cow;
use std::io::Read;
use std::num::NonZeroIsize;
use std::path::PathBuf;
use wry::http::{Request, Response};
use wry::raw_window_handle::{HandleError, HasWindowHandle, RawWindowHandle, Win32WindowHandle, WindowHandle};
use wry::{NewWindowResponse, Rect, WebContext, WebView, WebViewBuilder};

const MAX_PKG: u64 = 128 * 1024 * 1024;
static ENGINE: &[u8] = include_bytes!("webwallgl.global.min.js");
static INDEX: &[u8] = br##"<!doctype html><html><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><meta http-equiv="Content-Security-Policy" content="default-src 'none'; script-src 'self' 'unsafe-eval' blob:; connect-src 'self' blob:; worker-src blob:; img-src 'self' data: blob:; media-src 'self' blob:; font-src 'self' data: blob:; style-src 'self' 'unsafe-inline'"><style>html,body,#view{width:100%;height:100%;margin:0;overflow:hidden;background:#000}</style></head><body><div id="view"></div><script src="/engine.js"></script><script src="/player.js"></script></body></html>"##;
static PLAYER: &[u8] = br##"(async () => {
  try {
    const response = await fetch('/scene.pkg', {cache:'no-store'});
    if (!response.ok) throw new Error('Could not read scene.pkg');
    const bytes = await response.arrayBuffer();
    const settings = await (await fetch('/settings.json')).json();
    const source = WebWallGL.bytesSource(bytes, settings.project || undefined);
    const dpr = Math.max(0.1, Math.min(settings.height / Math.max(1, innerHeight), 4096 / Math.max(1, innerWidth, innerHeight)));
    window.scene = await WebWallGL.mount(document.getElementById('view'), {
      source, properties:settings.properties || {}, scripts:settings.scripts, fps:settings.fps, renderDpr:dpr, volume:settings.sound ? 0.5 : 0, audio:null, media:null,
      bake:false, mountTimeoutMs:60000,
      onError: () => window.ipc.postMessage('error')
    });
    if (Number.isFinite(window.__targetFps)) window.scene.setFps(window.__targetFps);
    // Only this known clock asset is handled; SceneScript is never executed.
    const clocks = (settings.scripts ? [] : (window.__textWidgets || [])).filter(item => {
      const layer = item.layer;
      return typeof layer.textScript === 'string' &&
        /__workshopId\s*=\s*['"]3550561961['"]/.test(layer.textScript) &&
        ((layer.name === 'DAY DATE TIME (EN)' && layer.text === 'DAY') ||
         (layer.name === 'Date' && layer.text === '<Date>') ||
         (layer.name === 'Clock' && layer.text === '12:34'));
    });
    const updateClock = () => {
      const now = new Date();
      for (const item of clocks) {
        const l = item.layer;
        if (l.name === 'DAY DATE TIME (EN)') l.text = new Intl.DateTimeFormat('en-US', {weekday:'long'}).format(now).toUpperCase();
        else if (l.name === 'Date') l.text = new Intl.DateTimeFormat('en-GB', {day:'numeric', month:'short', year:'numeric'}).format(now).toUpperCase();
        else if (l.name === 'Clock') l.text = new Intl.DateTimeFormat('en-GB', {hour:'2-digit', minute:'2-digit', hour12:false}).format(now);
      }
    };
    updateClock();
    setInterval(updateClock, 1000);
    window.ipc.postMessage('ready');
  } catch (err) {
    console.error('Scene failed:', err);
    window.ipc.postMessage('error');
  }
})();"##;

struct Parent(NonZeroIsize);
impl HasWindowHandle for Parent {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        let handle = Win32WindowHandle::new(self.0);
        // The parent window outlives this WebView and stays on the same UI thread.
        Ok(unsafe { WindowHandle::borrow_raw(RawWindowHandle::Win32(handle)) })
    }
}

fn response(code: u16, kind: &'static str, body: Cow<'static, [u8]>) -> Response<Cow<'static, [u8]>> {
    Response::builder().status(code)
        .header("Content-Type", kind)
        .header("Cache-Control", "no-store")
        .header("X-Content-Type-Options", "nosniff")
        .body(body).expect("static HTTP response")
}

pub(super) fn open(hwnd: isize, path: PathBuf, width: u32, height: u32, quality_height: u32, fps: u32, scripts: bool, sound: bool, project: Option<serde_json::Value>, properties: serde_json::Value, second: bool, notify: impl Fn(bool) + 'static) -> Result<WebView, String> {
    let handle = NonZeroIsize::new(hwnd).ok_or("Нет окна для сцены")?;
    let file = std::fs::File::open(path).map_err(|e| format!("Не удалось открыть PKG: {e}"))?;
    if file.metadata().map_err(|e| e.to_string())?.len() > MAX_PKG { return Err("Размер PKG превышает 128 МБ.".into()); }
    let mut bytes = Vec::new();
    file.take(MAX_PKG + 1).read_to_end(&mut bytes).map_err(|e| format!("Ошибка чтения PKG: {e}"))?;
    if bytes.len() as u64 > MAX_PKG { return Err("Размер PKG превышает 128 МБ.".into()); }
    let parent = Parent(handle);
    let settings = serde_json::to_vec(&serde_json::json!({ "height": quality_height, "fps": fps, "scripts": scripts, "sound": sound, "project": project, "properties": properties })).map_err(|e| format!("Не удалось загрузить настройки: {e}"))?;
    let scheme = if second { "rustwallsecond" } else { "rustwall" };
    let suffix = if quality_height >= 2160 { "?resources=native" } else { "" };
    let page_url = format!("{scheme}://localhost/index.html{suffix}");
    let allow_http = format!("http://{scheme}.localhost/index.html{suffix}");
    let allow_custom = page_url.clone();
    let data_dir = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from).ok_or("Не найдена папка LOCALAPPDATA")?
        .join("rustwallpapr").join(if second { "webview2-second" } else { "webview2" });
    let mut context = WebContext::new(Some(data_dir));
    WebViewBuilder::new_with_web_context(&mut context)
        .with_bounds(Rect { position: wry::dpi::PhysicalPosition::new(0, 0).into(), size: wry::dpi::PhysicalSize::new(width, height).into() })
        .with_custom_protocol(scheme.into(), move |_, req: Request<Vec<u8>>| {
            if req.method() != wry::http::Method::GET { return response(405, "text/plain", Cow::Borrowed(b"Method not allowed")); }
            match req.uri().path() {
                "/" | "/index.html" => response(200, "text/html; charset=utf-8", Cow::Borrowed(INDEX)),
                "/engine.js" => response(200, "text/javascript; charset=utf-8", Cow::Borrowed(ENGINE)),
                "/player.js" => response(200, "text/javascript; charset=utf-8", Cow::Borrowed(PLAYER)),
                "/settings.json" => response(200, "application/json", Cow::Owned(settings.clone())),
                "/scene.pkg" => response(200, "application/octet-stream", Cow::Owned(bytes.clone())),
                _ => response(404, "text/plain; charset=utf-8", Cow::Borrowed(b"Not found")),
            }
        })
        .with_navigation_handler(move |url| url == allow_custom || url == allow_http)
        .with_new_window_req_handler(|_, _| NewWindowResponse::Deny)
        .with_ipc_handler(move |req| match req.body().as_str() { "ready" => notify(true), "error" => notify(false), _ => {} })
        .with_devtools(false)
        .with_url(&page_url)
        .build_as_child(&parent)
        .map_err(|e| format!("Не удалось запустить WebView2: {e}"))
}
