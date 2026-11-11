use super::{Handle, null_mut, wide};
use std::os::windows::ffi::OsStrExt;

const RUN_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
const VALUE: &str = "RustWallpaper";
const KEY_QUERY_VALUE: u32 = 0x0001;
const KEY_SET_VALUE: u32 = 0x0002;
const REG_SZ: u32 = 1;
const NOT_FOUND: i32 = 2;

#[link(name = "advapi32")]
extern "system" {
    fn RegOpenKeyExW(root: Handle, subkey: *const u16, options: u32, access: u32, result: *mut Handle) -> i32;
    fn RegCreateKeyExW(root: Handle, subkey: *const u16, reserved: u32, class: *mut u16, options: u32, access: u32, security: Handle, result: *mut Handle, disposition: *mut u32) -> i32;
    fn RegQueryValueExW(key: Handle, name: *const u16, reserved: *mut u32, kind: *mut u32, data: *mut u8, size: *mut u32) -> i32;
    fn RegSetValueExW(key: Handle, name: *const u16, reserved: u32, kind: u32, data: *const u8, size: u32) -> i32;
    fn RegDeleteValueW(key: Handle, name: *const u16) -> i32;
    fn RegCloseKey(key: Handle) -> i32;
}
fn root() -> Handle { (0x80000001u32 as i32 as isize) as Handle }
fn error(code: i32) -> String { format!("Не удалось изменить автозапуск Windows (код {code}).") }
fn command() -> Result<Vec<u16>, String> {
    let path = std::env::current_exe().map_err(|e| format!("Не удалось определить путь приложения: {e}"))?;
    let mut result = vec![b'"' as u16];
    result.extend(path.as_os_str().encode_wide());
    result.extend("\" --startup".encode_utf16());
    result.push(0);
    Ok(result)
}
pub(super) fn is_enabled() -> Result<bool, String> {
    unsafe {
        let mut key: Handle = null_mut();
        let status = RegOpenKeyExW(root(), wide(RUN_KEY).as_ptr(), 0, KEY_QUERY_VALUE, &mut key);
        if status == NOT_FOUND { return Ok(false); }
        if status != 0 { return Err(error(status)); }
        let mut size = 0u32;
        let mut kind = 0u32;
        let result = RegQueryValueExW(key, wide(VALUE).as_ptr(), null_mut(), &mut kind, null_mut(), &mut size);
        if result == NOT_FOUND { RegCloseKey(key); return Ok(false); }
        if result != 0 || kind != REG_SZ || size > 65536 || size % 2 != 0 {
            RegCloseKey(key);
            return if result != 0 { Err(error(result)) } else { Ok(false) };
        }
        let mut value = vec![0u16; size as usize / 2];
        let result = RegQueryValueExW(key, wide(VALUE).as_ptr(), null_mut(), &mut kind, value.as_mut_ptr() as *mut u8, &mut size);
        RegCloseKey(key);
        if result != 0 { return Err(error(result)); }
        while value.last() == Some(&0) { value.pop(); }
        let mut expected = command()?;
        expected.pop();
        Ok(value == expected)
    }
}
pub(super) fn set_enabled(enabled: bool) -> Result<(), String> {
    unsafe {
        let mut key: Handle = null_mut();
        let status = if enabled {
            RegCreateKeyExW(root(), wide(RUN_KEY).as_ptr(), 0, null_mut(), 0, KEY_SET_VALUE, null_mut(), &mut key, null_mut())
        } else {
            RegOpenKeyExW(root(), wide(RUN_KEY).as_ptr(), 0, KEY_SET_VALUE, &mut key)
        };
        if !enabled && status == NOT_FOUND { return Ok(()); }
        if status != 0 { return Err(error(status)); }
        let result = if enabled {
            let data = match command() { Ok(v) => v, Err(e) => { RegCloseKey(key); return Err(e); } };
            RegSetValueExW(key, wide(VALUE).as_ptr(), 0, REG_SZ, data.as_ptr() as *const u8, (data.len()*2) as u32)
        } else { RegDeleteValueW(key, wide(VALUE).as_ptr()) };
        RegCloseKey(key);
        if result == 0 || (!enabled && result == NOT_FOUND) { Ok(()) } else { Err(error(result)) }
    }
}
