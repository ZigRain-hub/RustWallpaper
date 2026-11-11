use super::*;
use windows::core::{Interface, PCWSTR};
use windows::Win32::Media::MediaFoundation::*;

pub(super) unsafe fn frame(path: &PathBuf) -> Option<Thumb> {
    let name: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut attributes = None;
    MFCreateAttributes(&mut attributes, 1).ok()?;
    let attributes = attributes?;
    attributes.SetUINT32(&MF_SOURCE_READER_ENABLE_VIDEO_PROCESSING, 1).ok()?;
    let reader = MFCreateSourceReaderFromURL(PCWSTR(name.as_ptr()), &attributes).ok()?;
    let stream = MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32;
    let _ = reader.SetStreamSelection(MF_SOURCE_READER_ALL_STREAMS.0 as u32, false);
    reader.SetStreamSelection(stream, true).ok()?;
    let media_type = MFCreateMediaType().ok()?;
    media_type.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video).ok()?;
    media_type.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_RGB32).ok()?;
    reader.SetCurrentMediaType(stream, None, &media_type).ok()?;
    let current = reader.GetCurrentMediaType(stream).ok()?;
    let size = current.GetUINT64(&MF_MT_FRAME_SIZE).ok()?;
    let (width, height) = ((size >> 32) as usize, (size as u32) as usize);
    if width == 0 || height == 0 || width > 7680 || height > 4320 { return None; }
    let row = width.checked_mul(4)?;
    let total = row.checked_mul(height)?;
    let mut sample = None;
    for _ in 0..60 {
        let mut flags = 0u32;
        let mut timestamp = 0i64;
        let mut next = None;
        if reader.ReadSample(stream, 0, None, Some(&mut flags), Some(&mut timestamp), Some(&mut next)).is_err() { break; }
        if let Some(value) = next { sample = Some(value); if timestamp >= 5_000_000 { break; } }
        if flags & (MF_SOURCE_READERF_ENDOFSTREAM.0 as u32) != 0 { break; }
    }
    let buffer = sample?.ConvertToContiguousBuffer().ok()?;
    let mut pixels = vec![0u8; total];
    if let Ok(two_d) = buffer.cast::<IMF2DBuffer>() {
        let mut ptr = null_mut();
        let mut pitch = 0i32;
        two_d.Lock2D(&mut ptr, &mut pitch).ok()?;
        let valid = !ptr.is_null() && (pitch as i64).abs() >= row as i64;
        if valid {
            for y in 0..height {
                std::ptr::copy_nonoverlapping(ptr.offset(y as isize * pitch as isize), pixels.as_mut_ptr().add(y * row), row);
            }
        }
        let _ = two_d.Unlock2D();
        if !valid { return None; }
    } else {
        let mut ptr = null_mut(); let mut length = 0u32;
        buffer.Lock(&mut ptr, None, Some(&mut length)).ok()?;
        let stride = current.GetUINT32(&MF_MT_DEFAULT_STRIDE).map(|v| v as i32).unwrap_or(-(row as i32));
        let pitch = (stride as i64).unsigned_abs() as usize;
        let valid = !ptr.is_null() && pitch >= row && pitch.checked_mul(height).map_or(false, |n| n <= length as usize);
        if valid {
            for y in 0..height {
                let source_y = if stride < 0 { height - 1 - y } else { y };
                std::ptr::copy_nonoverlapping(ptr.add(source_y * pitch), pixels.as_mut_ptr().add(y * row), row);
            }
        }
        let _ = buffer.Unlock();
        if !valid { return None; }
    }
    let info = BitmapInfo { header: BitmapInfoHeader {
        size: size_of::<BitmapInfoHeader>() as u32, width: width as i32, height: -(height as i32),
        planes: 1, bit_count: 32, compression: 0, size_image: 0,
        x_pixels: 0, y_pixels: 0, colors_used: 0, colors_important: 0,
    }, colors: [0] };
    let mut bits = null_mut();
    let bitmap = CreateDIBSection(null_mut(), &info, 0, &mut bits, null_mut(), 0);
    if bitmap.is_null() { return None; }
    if bits.is_null() { DeleteObject(bitmap); return None; }
    std::ptr::copy_nonoverlapping(pixels.as_ptr(), bits as *mut u8, total);
    let scale = (1920.0 / width as f64).min(1080.0 / height as f64).min(1.0);
    if scale < 1.0 {
        let w = ((width as f64 * scale).round() as i32).max(1);
        let h = ((height as f64 * scale).round() as i32).max(1);
        let smaller_info = BitmapInfo { header: BitmapInfoHeader {
            size: size_of::<BitmapInfoHeader>() as u32, width: w, height: -h,
            planes: 1, bit_count: 32, compression: 0, size_image: 0,
            x_pixels: 0, y_pixels: 0, colors_used: 0, colors_important: 0,
        }, colors: [0] };
        let mut smaller_bits = null_mut();
        let smaller = CreateDIBSection(null_mut(), &smaller_info, 0, &mut smaller_bits, null_mut(), 0);
        let src = CreateCompatibleDC(null_mut());
        let dst = CreateCompatibleDC(null_mut());
        let mut success = false;
        if !smaller.is_null() && !src.is_null() && !dst.is_null() {
            let old_src = SelectObject(src, bitmap);
            let old_dst = SelectObject(dst, smaller);
            if !old_src.is_null() && old_src as isize != -1 && !old_dst.is_null() && old_dst as isize != -1 {
                SetStretchBltMode(dst, 4);
                success = StretchBlt(dst, 0, 0, w, h, src, 0, 0, width as i32, height as i32, 0x00cc0020) != 0;
            }
            if !old_src.is_null() && old_src as isize != -1 { SelectObject(src, old_src); }
            if !old_dst.is_null() && old_dst as isize != -1 { SelectObject(dst, old_dst); }
        }
        if !src.is_null() { DeleteDC(src); }
        if !dst.is_null() { DeleteDC(dst); }
        if success { DeleteObject(bitmap); return Some(Thumb { bitmap: smaller as isize, w, h }); }
        if !smaller.is_null() { DeleteObject(smaller); }
    }
    Some(Thumb { bitmap: bitmap as isize, w: width as i32, h: height as i32 })
}
