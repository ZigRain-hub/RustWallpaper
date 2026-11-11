use std::collections::HashMap;
use std::fs::File;
use std::io::{Cursor, Read};
use std::ops::Range;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};
use serde_json::Value;
use super::gif_wallpaper::Frame;

const MAX_PACKAGE: u64 = 64 * 1024 * 1024;
const MAX_PIXELS: usize = 16_777_216;

pub(super) enum TextKind {
    Clock { use_24h: bool, seconds: bool, delimiter: String },
    Day,
    Date,
    Static(String),
}

pub(super) struct TextLayer {
    pub x: f32,
    pub y: f32,
    pub size: f32,
    pub kind: TextKind,
}

pub(super) struct Scene {
    pub frame: Frame,
    pub snow: bool,
    pub text: Vec<TextLayer>,
    pub warning: bool,
    motion: Option<Motion>,
    rendered: Option<Frame>,
}

struct Motion {
    cloud: Option<Vec<u8>>,
    flow: Option<Vec<u8>>,
}

impl Scene {
    pub(super) fn has_motion(&self) -> bool { self.motion.is_some() }

    pub(super) fn animated_frame(&mut self, seconds: f32) -> &Frame {
        let Some(motion) = self.motion.as_ref() else { return &self.frame; };
        let Some(output) = self.rendered.as_mut() else { return &self.frame; };
        let w = self.frame.width as usize;
        let h = self.frame.height as usize;
        let source = &self.frame.pixels;
        let t = seconds.rem_euclid(4096.0);
        for y in 0..h {
            let cloud_wave = ((t * 0.32 + y as f32 * 0.018).sin() * 5.0) as i32;
            let flow_wave = ((t * 0.75 + y as f32 * 0.012).sin() * 6.0) as i32;
            for x in 0..w {
                let i = y*w+x;
                let mut dx = 0;
                let mut dy = 0;
                if let Some(mask) = &motion.cloud {
                    dx += cloud_wave * mask[i] as i32 / 255;
                }
                if let Some(mask) = &motion.flow {
                    dx += flow_wave * (mask[i*2] as i32 - 127) / 127;
                    dy += flow_wave * (mask[i*2+1] as i32 - 127) / 127;
                }
                let sx = (x as i32 + dx).clamp(0, w as i32 - 1) as usize;
                let sy = (y as i32 + dy).clamp(0, h as i32 - 1) as usize;
                let src = (sy*w+sx)*4;
                let dst = i*4;
                output.pixels[dst..dst+4].copy_from_slice(&source[src..src+4]);
            }
        }
        output
    }
}


pub(super) fn start(path: PathBuf, width: i32, height: i32) -> Result<Receiver<Result<Scene, String>>, String> {
    let (tx, rx) = mpsc::sync_channel(1);
    std::thread::Builder::new().name("scene-loader".into()).spawn(move || {
        let _ = tx.send(load(path, width, height));
    }).map_err(|e| format!("Не удалось запустить загрузку сцены: {e}"))?;
    Ok(rx)
}

fn u32_at(data: &[u8], at: usize) -> Result<u32, String> {
    let end = at.checked_add(4).ok_or("Неверный размер PKG")?;
    let bytes: [u8; 4] = data.get(at..end).ok_or("Повреждённый PKG")?.try_into().map_err(|_| "Повреждённый PKG")?;
    Ok(u32::from_le_bytes(bytes))
}
fn next_u32(data: &[u8], pos: &mut usize) -> Result<usize, String> {
    let n = u32_at(data, *pos)? as usize;
    *pos += 4;
    Ok(n)
}
fn next_string(data: &[u8], pos: &mut usize) -> Result<String, String> {
    let len = next_u32(data, pos)?;
    if len > 4096 { return Err("Слишком длинное имя в PKG.".into()); }
    let end = pos.checked_add(len).ok_or("Повреждённое имя в PKG")?;
    let s = std::str::from_utf8(data.get(*pos..end).ok_or("Повреждённый PKG")?)
        .map_err(|_| "Неверное имя в PKG")?.to_owned();
    *pos = end;
    Ok(s)
}
fn entry<'a>(data: &'a [u8], entries: &HashMap<String, Range<usize>>, name: &str) -> Result<&'a [u8], String> {
    let range = entries.get(name).ok_or_else(|| format!("В PKG отсутствует {name}"))?;
    data.get(range.clone()).ok_or_else(|| "Неверные границы файла в PKG".into())
}
fn json_entry(data: &[u8], entries: &HashMap<String, Range<usize>>, name: &str) -> Result<Value, String> {
    let input = entry(data, entries, name)?;
    if input.len() > 2 * 1024 * 1024 { return Err("Описание сцены слишком большое.".into()); }
    serde_json::from_slice(input).map_err(|e| format!("Неверное описание сцены: {e}"))
}

pub(super) fn load(path: PathBuf, screen_w: i32, screen_h: i32) -> Result<Scene, String> {
    let file = File::open(path).map_err(|e| format!("Не удалось открыть PKG: {e}"))?;
    let mut data = Vec::new();
    file.take(MAX_PACKAGE + 1).read_to_end(&mut data).map_err(|e| format!("Ошибка чтения PKG: {e}"))?;
    if data.len() as u64 > MAX_PACKAGE { return Err("PKG больше 64 МБ; эта версия его не загружает.".into()); }
    let mut pos = 0;
    let version = next_string(&data, &mut pos)?;
    if version != "PKGV0018" && version != "PKGV0022" && version != "PKGV0023" {
        return Err(format!("Версия {version} пока не поддерживается."));
    }
    let count = next_u32(&data, &mut pos)?;
    if count > 4096 { return Err("Слишком много файлов внутри PKG.".into()); }
    let mut list = Vec::with_capacity(count);
    for _ in 0..count {
        let name = next_string(&data, &mut pos)?;
        let offset = next_u32(&data, &mut pos)?;
        let len = next_u32(&data, &mut pos)?;
        list.push((name, offset, len));
    }
    let mut entries = HashMap::new();
    for (name, offset, len) in list {
        let start = pos.checked_add(offset).ok_or("Неверное смещение PKG")?;
        let end = start.checked_add(len).ok_or("Неверный размер PKG")?;
        if end > data.len() || entries.insert(name, start..end).is_some() { return Err("Повреждённое содержимое PKG.".into()); }
    }
    let scene = json_entry(&data, &entries, "scene.json")?;
    let objects = scene["objects"].as_array().ok_or("В сцене нет списка объектов")?;
    let mut image = None;
    let mut primary_effects = None;
    let mut overlays = Vec::new();
    let mut snow = false;
    let mut text = Vec::new();
    let mut warning = false;
    let scene_w = scene["general"]["orthogonalprojection"]["width"].as_f64().unwrap_or(1920.0).clamp(1.0, 16384.0) as f32;
    let scene_h = scene["general"]["orthogonalprojection"]["height"].as_f64().unwrap_or(1080.0).clamp(1.0, 16384.0) as f32;
    for object in objects {
        let visibility = &object["visible"];
        if visibility == &Value::Bool(false) || visibility["value"] == Value::Bool(false) { continue; }
        if object["effects"].as_array().map_or(false, |v| !v.is_empty()) { warning = true; }
        if let Some(name) = object["image"].as_str() {
            if image.is_none() {
                image = Some(name.to_owned());
                primary_effects = object["effects"].as_array().cloned();
            }
            else if object.get("parent").is_none() && overlays.len() < 8 {
                overlays.push((name.to_owned(), object.clone()));
                warning = true;
            } else { warning = true; }
        } else if let Some(name) = object["particle"].as_str() {
            if name == "particles/presets/snowperspective.json" { snow = true; }
            else { warning = true; }
        } else if object.get("text").is_some() {
            if object.get("parent").is_some() { warning = true; continue; }
            if let Some(layer) = text_layer(object, scene_w, scene_h) { text.push(layer); }
            else { warning = true; }
            if object["text"].get("script").is_some() { warning = true; }
        } else { warning = true; }
    }
    let model_name = image.ok_or("В сцене нет поддерживаемого изображения")?;
    let mut mask_specs: Vec<(String, String)> = Vec::new();
    if let Some(effects) = primary_effects {
        for effect in effects.iter().take(32) {
            if effect["visible"] == Value::Bool(false) || effect["visible"]["value"] == Value::Bool(false) { continue; }
            let path = effect["file"].as_str().unwrap_or("");
            let kind = if path == "effects/cloudmotion/effect.json" { "cloud" }
                else if path == "effects/waterflow/effect.json" { "flow" } else { continue; };
            if let Some(mask) = effect["passes"][0]["textures"][1].as_str() {
                mask_specs.push((kind.to_owned(), format!("materials/{mask}.tex")));
            }
        }
    }
    let (decode_w, decode_h) = if mask_specs.is_empty() { (screen_w, screen_h) } else {
        let ratio = (960.0 / screen_w.max(1) as f64).min(540.0 / screen_h.max(1) as f64).min(1.0);
        (((screen_w.max(1) as f64 * ratio) as i32).max(1), ((screen_h.max(1) as f64 * ratio) as i32).max(1))
    };
    let mut frame = decode_model(&data, &entries, &model_name, decode_w, decode_h)?;
    for px in frame.pixels.chunks_exact_mut(4) {
        let a = px[3] as u16;
        for c in 0..3 { px[c] = (px[c] as u16 * a / 255) as u8; }
        px[3] = 255;
    }
    for (name, object) in overlays {
        let Some((ox, oy)) = pair(&object["origin"]) else { continue; };
        if ox.abs() > scene_w * 8.0 || oy.abs() > scene_h * 8.0 { continue; }
        let Some((width, height)) = pair(&object["size"]) else { continue; };
        let (scale_x, scale_y) = pair(&object["scale"]).unwrap_or((1.0, 1.0));
        let angles = object["angles"].as_str().unwrap_or("0 0 0");
        if angles.split_whitespace().any(|v| v.parse::<f32>().map_or(true, |n| !n.is_finite() || n.abs() > 0.001)) { continue; }
        let width = width * scale_x;
        let height = height * scale_y;
        if !width.is_finite() || !height.is_finite() || width <= 0.0 || height <= 0.0 || width > scene_w * 4.0 || height > scene_h * 4.0 { continue; }
        let dw = (width / scene_w * frame.width as f32).round().clamp(1.0, 8192.0) as i32;
        let dh = (height / scene_h * frame.height as f32).round().clamp(1.0, 8192.0) as i32;
        if (dw as usize).checked_mul(dh as usize).map_or(true, |v| v > MAX_PIXELS) { continue; }
        let layer = match decode_model(&data, &entries, &name, dw, dh) { Ok(v) => v, Err(_) => continue };
        let left = ((ox - width / 2.0) / scene_w * frame.width as f32).round() as i32;
        let top = ((scene_h - oy - height / 2.0) / scene_h * frame.height as f32).round() as i32;
        let opacity = object["alpha"].as_f64().unwrap_or(1.0).clamp(0.0, 1.0) as f32;
        composite(&mut frame, &layer, left, top, dw, dh, opacity);
    }
    let mut motion = Motion { cloud: None, flow: None };
    for (kind, name) in mask_specs {
        if let Ok(mask) = decode_motion_mask(&data, &entries, &name, frame.width as usize, frame.height as usize, &kind) {
            if kind == "cloud" { motion.cloud = Some(mask); } else { motion.flow = Some(mask); }
        }
    }
    let motion = if motion.cloud.is_some() || motion.flow.is_some() { Some(motion) } else { None };
    let rendered = motion.as_ref().map(|_| Frame { width: frame.width, height: frame.height, pixels: frame.pixels.clone() });
    Ok(Scene { frame, snow, text, warning, motion, rendered })
}

fn pair(value: &Value) -> Option<(f32, f32)> {
    let mut it = value.as_str()?.split_whitespace();
    let x: f32 = it.next()?.parse().ok()?;
    let y: f32 = it.next()?.parse().ok()?;
    if x.is_finite() && y.is_finite() { Some((x, y)) } else { None }
}

fn composite(dst: &mut Frame, src: &Frame, left: i32, top: i32, width: i32, height: i32, opacity: f32) {
    let x0 = left.max(0); let y0 = top.max(0);
    let x1 = left.saturating_add(width).min(dst.width);
    let y1 = top.saturating_add(height).min(dst.height);
    for y in y0..y1 { for x in x0..x1 {
        let sx = ((x-left) as i64 * src.width as i64 / width as i64) as usize;
        let sy = ((y-top) as i64 * src.height as i64 / height as i64) as usize;
        let si = (sy * src.width as usize + sx) * 4;
        let di = (y as usize * dst.width as usize + x as usize) * 4;
        let a = (src.pixels[si+3] as f32 * opacity).round() as u32;
        for c in 0..3 { dst.pixels[di+c] = ((src.pixels[si+c] as u32 * a + dst.pixels[di+c] as u32 * (255-a) + 127) / 255) as u8; }
    }}
}

fn decode_motion_mask(data: &[u8], entries: &HashMap<String, Range<usize>>, name: &str, out_w: usize, out_h: usize, kind: &str) -> Result<Vec<u8>, String> {
    let tex = entry(data, entries, name)?;
    if tex.get(0..9) != Some(b"TEXV0005\0".as_slice()) || tex.get(9..18) != Some(b"TEXI0001\0".as_slice()) || tex.get(46..55) != Some(b"TEXB0004\0".as_slice()) { return Err("Неподдерживаемая маска".into()); }
    let channels = if kind == "cloud" && u32_at(tex, 18)? == 9 { 1 }
        else if kind == "flow" && u32_at(tex, 18)? == 8 { 2 } else { return Err("Неподдерживаемый формат маски".into()); };
    if u32_at(tex, 55)? != 1 || u32_at(tex, 63)? != 0 { return Err("Неподдерживаемая маска".into()); }
    let w = u32_at(tex, 71)? as usize; let h = u32_at(tex, 75)? as usize;
    let expected = w.checked_mul(h).and_then(|n| n.checked_mul(channels)).ok_or("Размер маски превышен")?;
    if expected == 0 || expected > 32 * 1024 * 1024 || out_w.checked_mul(out_h).map_or(true, |n| n > MAX_PIXELS) { return Err("Размер маски превышен".into()); }
    let compression = u32_at(tex, 79)?;
    let raw_size = u32_at(tex, 83)? as usize;
    let compressed_size = u32_at(tex, 87)? as usize;
    let end = 91usize.checked_add(compressed_size).ok_or("Неверный размер маски")?;
    let bytes = tex.get(91..end).ok_or("Повреждённая маска")?;
    let raw = if compression == 1 && raw_size == expected { lz4_block(bytes, expected)? }
        else if compression == 0 && bytes.len() == expected { bytes.to_vec() }
        else { return Err("Сжатие маски не поддерживается".into()); };
    let mut output = vec![0u8; out_w*out_h*channels];
    for y in 0..out_h { for x in 0..out_w {
        let from = ((y*h/out_h)*w + x*w/out_w)*channels;
        let to = (y*out_w+x)*channels;
        output[to..to+channels].copy_from_slice(&raw[from..from+channels]);
    }}
    Ok(output)
}

fn decode_model(data: &[u8], entries: &HashMap<String, Range<usize>>, name: &str, screen_w: i32, screen_h: i32) -> Result<Frame, String> {
    let model = json_entry(data, entries, name)?;
    let material_name = model["material"].as_str().ok_or("Не найден материал сцены")?;
    let material = json_entry(data, entries, material_name)?;
    let passes = material["passes"].as_array().ok_or("Не найдены слои материала")?;
    if passes.len() != 1 || !matches!(passes[0]["shader"].as_str(), Some("genericimage2") | Some("genericimage4")) {
        return Err("Фоновый материал сцены пока не поддерживается.".into());
    }
    let texture_name = passes[0]["textures"][0].as_str().ok_or("Не найдена текстура сцены")?;
    let tex_path = format!("materials/{texture_name}.tex");
    let tex = entry(data, entries, &tex_path)?;
    if tex.get(0..9) != Some(b"TEXV0005\0".as_slice()) || tex.get(9..18) != Some(b"TEXI0001\0".as_slice()) {
        return Err("Формат текстуры пока не поддерживается.".into());
    }
    let texture_type = u32_at(tex, 18)?;
    let flags = u32_at(tex, 22)?;
    if flags & (0x4 | 0x40) != 0 { return Err("Анимированная или объёмная текстура пока не поддерживается.".into()); }
    let version = tex.get(46..55).ok_or("Повреждённая текстура")?;
    let extra = if version == b"TEXB0003\0" { 0 } else if version == b"TEXB0004\0" { 4 } else {
        return Err("Версия TEX пока не поддерживается.".into());
    };
    if extra == 4 && u32_at(tex, 63)? != 0 { return Err("Видеотекстуры пока не поддерживаются.".into()); }
    if u32_at(tex, 55)? != 1 { return Err("Многослойные TEX пока не поддерживаются.".into()); }
    let kind = u32_at(tex, 59)?;
    let compressed = u32_at(tex, 75+extra)?;
    let raw_size = u32_at(tex, 79+extra)? as usize;
    let size = u32_at(tex, 83+extra)? as usize;
    let start = 87usize+extra;
    let end = start.checked_add(size).ok_or("Неверный размер текстуры")?;
    let source = tex.get(start..end).ok_or("Повреждённая текстура")?;
    if compressed == 0 && kind == 2 && source.starts_with(&[0xff, 0xd8, 0xff]) {
        decode_jpeg(source, screen_w, screen_h)
    } else if compressed == 0 && kind == 13 && source.starts_with(b"\x89PNG\r\n\x1a\n") {
        decode_png(source, screen_w, screen_h)
    } else if matches!(texture_type, 4 | 6 | 7) && kind == u32::MAX && (compressed == 0 || compressed == 1) {
        let tw = u32_at(tex, 67+extra)? as usize;
        let th = u32_at(tex, 71+extra)? as usize;
        let w = u32_at(tex, 34)? as usize;
        let h = u32_at(tex, 38)? as usize;
        if tw == 0 || th == 0 || w == 0 || h == 0 || w > tw || h > th || tw.checked_mul(th).map_or(true, |v| v > MAX_PIXELS + 262_144) { return Err("Недопустимый размер TEX.".into()); }
        let block_size = if texture_type == 7 { 8 } else { 16 };
        let expected = ((tw + 3) / 4).checked_mul((th + 3) / 4).and_then(|n| n.checked_mul(block_size)).ok_or("Размер TEX превышен")?;
        if expected > 32 * 1024 * 1024 { return Err("Неверный размер DXT.".into()); }
        let blocks = if compressed == 1 && raw_size == expected { lz4_block(source, expected)? }
            else if compressed == 0 && source.len() == expected { source.to_vec() }
            else { return Err("Неверный размер DXT.".into()); };
        decode_bc(&blocks, texture_type, tw, w, h, screen_w, screen_h)
    } else { Err("Формат текстуры TEX пока не поддерживается.".into()) }
}

fn lz4_block(input: &[u8], expected: usize) -> Result<Vec<u8>, String> {
    let mut out = Vec::with_capacity(expected);
    let mut pos = 0usize;
    while pos < input.len() {
        let token = input[pos]; pos += 1;
        let mut literal = (token >> 4) as usize;
        if literal == 15 { loop {
            let v = *input.get(pos).ok_or("Повреждённый LZ4")? as usize; pos += 1;
            literal = literal.checked_add(v).ok_or("Размер LZ4 превышен")?;
            if v != 255 { break; }
        }}
        let end = pos.checked_add(literal).ok_or("Размер LZ4 превышен")?;
        if end > input.len() || out.len().checked_add(literal).map_or(true, |n| n > expected) { return Err("Повреждённый LZ4".into()); }
        out.extend_from_slice(&input[pos..end]); pos = end;
        if pos == input.len() { break; }
        let distance = u16::from_le_bytes([*input.get(pos).ok_or("Повреждённый LZ4")?, *input.get(pos+1).ok_or("Повреждённый LZ4")?]) as usize;
        pos += 2;
        if distance == 0 || distance > out.len() { return Err("Неверное смещение LZ4".into()); }
        let mut len = (token & 15) as usize + 4;
        if token & 15 == 15 { loop {
            let v = *input.get(pos).ok_or("Повреждённый LZ4")? as usize; pos += 1;
            len = len.checked_add(v).ok_or("Размер LZ4 превышен")?;
            if v != 255 { break; }
        }}
        if out.len().checked_add(len).map_or(true, |n| n > expected) { return Err("Размер LZ4 превышен".into()); }
        for _ in 0..len { let v = out[out.len()-distance]; out.push(v); }
    }
    if out.len() != expected { return Err("Неверная длина LZ4".into()); }
    Ok(out)
}

fn decode_bc(blocks: &[u8], texture_type: u32, tw: usize, w: usize, h: usize, screen_w: i32, screen_h: i32) -> Result<Frame, String> {
    let ratio = (screen_w.max(1) as f64 / w as f64).max(screen_h.max(1) as f64 / h as f64).min(1.0);
    let dw = ((w as f64 * ratio).ceil() as usize).max(1);
    let dh = ((h as f64 * ratio).ceil() as usize).max(1);
    if dw.checked_mul(dh).map_or(true, |v| v > MAX_PIXELS) { return Err("Размер кадра слишком большой".into()); }
    let mut pixels = vec![0; dw * dh * 4];
    let blocks_w = (tw + 3) / 4;
    for y in 0..dh { for x in 0..dw {
        let sx = x*w/dw; let sy = y*h/dh;
        let block_size = if texture_type == 7 { 8 } else { 16 };
        let offset = ((sy/4)*blocks_w + sx/4)*block_size;
        let b = blocks.get(offset..offset+block_size).ok_or("Повреждённый блок DXT")?;
        let i = (sy%4)*4 + sx%4;
        let colors = if texture_type == 7 { 0 } else { 8 };
        let c0 = u16::from_le_bytes([b[colors],b[colors+1]]) as u32;
        let c1 = u16::from_le_bytes([b[colors+2],b[colors+3]]) as u32;
        let color = |v:u32| -> [u32;3] { [((v & 31)*255+15)/31, (((v >> 5)&63)*255+31)/63, (((v >> 11)&31)*255+15)/31] };
        let p = color(c0); let q = color(c1);
        let indices = u32::from_le_bytes([b[colors+4],b[colors+5],b[colors+6],b[colors+7]]);
        let ci = (indices >> (2*i)) & 3;
        let alpha = if texture_type == 7 {
            if c0 <= c1 && ci == 3 { 0 } else { 255 }
        } else if texture_type == 6 {
            let packed = u64::from_le_bytes(b[0..8].try_into().map_err(|_| "Повреждённый DXT3")?);
            (((packed >> (4*i)) & 15) * 17) as u32
        } else {
            let a0 = b[0] as u32; let a1 = b[1] as u32;
            let mut ai = 0u64;
            for k in 0..6 { ai |= (b[2+k] as u64) << (8*k); }
            let index = ((ai >> (3*i)) & 7) as u32;
            if index == 0 { a0 } else if index == 1 { a1 }
            else if a0 > a1 { ((8-index)*a0 + (index-1)*a1)/7 }
            else if index <= 5 { ((6-index)*a0 + (index-1)*a1)/5 }
            else if index == 6 { 0 } else { 255 }
        };
        let at = (y*dw+x)*4;
        for k in 0..3 {
            let component = if texture_type == 7 && c0 <= c1 {
                match ci { 0 => p[k], 1 => q[k], 2 => (p[k]+q[k])/2, _ => 0 }
            } else { match ci { 0 => p[k], 1 => q[k], 2 => (2*p[k]+q[k])/3, _ => (p[k]+2*q[k])/3 } };
            pixels[at+k] = component as u8;
        }
        pixels[at+3] = alpha as u8;
    }}
    Ok(Frame { width: dw as i32, height: dh as i32, pixels })
}

fn text_layer(object: &Value, scene_w: f32, scene_h: f32) -> Option<TextLayer> {
    let origin = object["origin"].as_str()?;
    let mut parts = origin.split_whitespace();
    let x: f32 = parts.next()?.parse().ok()?;
    let y: f32 = parts.next()?.parse().ok()?;
    if !x.is_finite() || !y.is_finite() { return None; }
    let point_size = object["pointsize"].as_f64().unwrap_or(32.0).clamp(6.0, 256.0) as f32;
    let scale = object["scale"].as_str().and_then(|s| s.split_whitespace().nth(1)).and_then(|s| s.parse::<f32>().ok()).unwrap_or(1.0).abs().clamp(0.1, 4.0);
    let raw = &object["text"];
    let name: String = object["name"].as_str().unwrap_or("").chars().filter(|c| !c.is_whitespace()).flat_map(|c| c.to_lowercase()).collect();
    let props = &raw["scriptproperties"];
    let kind = match name.as_str() {
        "clock" => TextKind::Clock {
            use_24h: props["use24hFormat"].as_bool().unwrap_or(true),
            seconds: props["showSeconds"].as_bool().unwrap_or(false),
            delimiter: props["delimiter"].as_str().unwrap_or(":").chars().take(4).collect(),
        },
        "day" => TextKind::Day,
        "date" => TextKind::Date,
        _ => {
            if raw.get("script").is_some() { return None; }
            let value = raw.as_str().or_else(|| raw["value"].as_str())?;
            TextKind::Static(value.chars().take(100).collect())
        }
    };
    Some(TextLayer { x: x / scene_w, y: 1.0 - y / scene_h, size: point_size * scale / scene_h, kind })
}

fn decode_jpeg(jpeg: &[u8], screen_w: i32, screen_h: i32) -> Result<Frame, String> {
    let mut decoder = jpeg_decoder::Decoder::new(Cursor::new(jpeg));
    decoder.set_max_decoding_buffer_size(64 * 1024 * 1024);
    decoder.read_info().map_err(|e| format!("Ошибка JPEG: {e}"))?;
    let info = decoder.info().ok_or("Не удалось прочитать JPEG")?;
    if usize::from(info.width) * usize::from(info.height) > MAX_PIXELS { return Err("Изображение слишком большое.".into()); }
    decoder.scale(screen_w.clamp(1, 65535) as u16, screen_h.clamp(1, 65535) as u16)
        .map_err(|e| format!("Ошибка масштабирования JPEG: {e}"))?;
    let rgb = decoder.decode().map_err(|e| format!("Ошибка JPEG: {e}"))?;
    let info = decoder.info().ok_or("Не удалось прочитать размеры JPEG")?;
    let count = usize::from(info.width) * usize::from(info.height);
    let mut pixels = vec![0u8; count.checked_mul(4).ok_or("Слишком большое изображение")?];
    match info.pixel_format {
        jpeg_decoder::PixelFormat::RGB24 if rgb.len() == count * 3 => {
            for (src, dst) in rgb.chunks_exact(3).zip(pixels.chunks_exact_mut(4)) {
                dst.copy_from_slice(&[src[2], src[1], src[0], 255]);
            }
        }
        jpeg_decoder::PixelFormat::L8 if rgb.len() == count => {
            for (gray, dst) in rgb.iter().zip(pixels.chunks_exact_mut(4)) { dst.copy_from_slice(&[*gray, *gray, *gray, 255]); }
        }
        _ => return Err("Не поддерживается цветовой формат JPEG.".into()),
    }
    Ok(Frame { width: info.width as i32, height: info.height as i32, pixels })
}

fn decode_png(input: &[u8], screen_w: i32, screen_h: i32) -> Result<Frame, String> {
    let mut decoder = png::Decoder::new(Cursor::new(input));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut limits = png::Limits::default();
    limits.bytes = 64 * 1024 * 1024;
    decoder.set_limits(limits);
    let mut reader = decoder.read_info().map_err(|e| format!("Ошибка PNG: {e}"))?;
    let w = reader.info().width as usize;
    let h = reader.info().height as usize;
    if w == 0 || h == 0 || w.checked_mul(h).map_or(true, |n| n > MAX_PIXELS) { return Err("Размер PNG превышает допустимый предел.".into()); }
    if reader.info().animation_control.is_some() { return Err("Анимированный PNG в TEX пока не поддерживается.".into()); }
    let size = reader.output_buffer_size();
    if size > 64 * 1024 * 1024 { return Err("Размер кадра PNG превышает допустимый предел.".into()); }
    let mut source = vec![0u8; size];
    let info = reader.next_frame(&mut source).map_err(|e| format!("Ошибка PNG: {e}"))?;
    source.truncate(info.buffer_size());
    let channels = match info.color_type {
        png::ColorType::Rgb => 3,
        png::ColorType::Rgba => 4,
        png::ColorType::Grayscale => 1,
        png::ColorType::GrayscaleAlpha => 2,
        _ => return Err("Формат цвета PNG не поддерживается.".into()),
    };
    if source.len() != w * h * channels { return Err("Неверный размер данных PNG.".into()); }
    let ratio = (screen_w.max(1) as f64 / w as f64).max(screen_h.max(1) as f64 / h as f64).min(1.0);
    let dw = ((w as f64 * ratio).ceil() as usize).max(1);
    let dh = ((h as f64 * ratio).ceil() as usize).max(1);
    let mut pixels = vec![0u8; dw * dh * 4];
    for y in 0..dh {
        for x in 0..dw {
            let src = ((y * h / dh) * w + x * w / dw) * channels;
            let dst = (y * dw + x) * 4;
            let (r, g, b, a) = match info.color_type {
                png::ColorType::Rgb => (source[src], source[src+1], source[src+2], 255),
                png::ColorType::Rgba => (source[src], source[src+1], source[src+2], source[src+3]),
                png::ColorType::Grayscale => (source[src], source[src], source[src], 255),
                png::ColorType::GrayscaleAlpha => (source[src], source[src], source[src], source[src+1]),
                _ => unreachable!(),
            };
            pixels[dst] = b;
            pixels[dst+1] = g;
            pixels[dst+2] = r;
            pixels[dst+3] = a;
        }
    }
    Ok(Frame { width: dw as i32, height: dh as i32, pixels })
}
