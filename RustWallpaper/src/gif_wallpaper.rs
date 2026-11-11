use std::fs::File;
use std::io::BufReader;
use std::num::NonZeroU64;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender, SyncSender, TryRecvError};
use std::thread;
use std::time::Duration;

const MAX_PIXELS: usize = 8_388_608;

pub(super) struct Frame {
    pub width: i32,
    pub height: i32,
    pub pixels: Vec<u8>,
}

pub(super) struct Player {
    frames: Receiver<Result<Frame, String>>,
    stop: Sender<()>,
}

impl Player {
    pub fn start(path: PathBuf) -> Result<Self, String> {
        let (frames_tx, frames) = mpsc::sync_channel(1);
        let (stop, stop_rx) = mpsc::channel();
        thread::Builder::new().name("gif-wallpaper".into()).spawn(move || {
            if let Err(error) = decode(&path, &frames_tx, &stop_rx) {
                let _ = frames_tx.send(Err(error));
            }
        }).map_err(|e| format!("Не удалось запустить GIF: {e}"))?;
        Ok(Self { frames, stop })
    }

    pub fn try_frame(&self) -> Result<Option<Frame>, String> {
        match self.frames.try_recv() {
            Ok(Ok(frame)) => Ok(Some(frame)),
            Ok(Err(error)) => Err(error),
            Err(TryRecvError::Empty) => Ok(None),
            Err(TryRecvError::Disconnected) => Err("Воспроизведение GIF остановилось.".into()),
        }
    }
}

impl Drop for Player {
    fn drop(&mut self) { let _ = self.stop.send(()); }
}

fn decode(path: &Path, tx: &SyncSender<Result<Frame, String>>, stop: &Receiver<()>) -> Result<(), String> {
    loop {
        let file = File::open(path).map_err(|e| format!("Не удалось открыть GIF: {e}"))?;
        let mut options = gif::DecodeOptions::new();
        options.set_color_output(gif::ColorOutput::RGBA);
        options.check_frame_consistency(true);
        options.set_memory_limit(gif::MemoryLimit::Bytes(NonZeroU64::new((MAX_PIXELS * 4) as u64).unwrap()));
        let mut decoder = options.read_info(BufReader::new(file)).map_err(|e| format!("Неверный GIF: {e}"))?;
        let width = decoder.width() as usize;
        let height = decoder.height() as usize;
        let pixels = width.checked_mul(height).filter(|&n| n > 0 && n <= MAX_PIXELS)
            .ok_or("GIF слишком большой: предел — 8 388 608 пикселей.")?;
        let mut canvas = vec![0u8; pixels * 4];
        let mut count = 0usize;
        loop {
            let frame = decoder.read_next_frame().map_err(|e| format!("Ошибка декодирования GIF: {e}"))?;
            let Some(frame) = frame else { break; };
            count += 1;
            let left = frame.left as usize;
            let top = frame.top as usize;
            let fw = frame.width as usize;
            let fh = frame.height as usize;
            if left + fw > width || top + fh > height || frame.buffer.len() != fw * fh * 4 {
                return Err("Неверные размеры кадра GIF.".into());
            }
            let previous = if frame.dispose == gif::DisposalMethod::Previous { Some(canvas.clone()) } else { None };
            for y in 0..fh {
                for x in 0..fw {
                    let src = (y * fw + x) * 4;
                    let dst = ((top + y) * width + left + x) * 4;
                    if frame.buffer[src + 3] != 0 {
                        canvas[dst] = frame.buffer[src + 2];
                        canvas[dst + 1] = frame.buffer[src + 1];
                        canvas[dst + 2] = frame.buffer[src];
                        canvas[dst + 3] = 255;
                    }
                }
            }
            let delay = Duration::from_millis((frame.delay as u64 * 10).max(20));
            if tx.send(Ok(Frame { width: width as i32, height: height as i32, pixels: canvas.clone() })).is_err() { return Ok(()); }
            if stop.recv_timeout(delay).is_ok() { return Ok(()); }
            match frame.dispose {
                gif::DisposalMethod::Background => {
                    for y in top..top + fh {
                        let start = (y * width + left) * 4;
                        canvas[start..start + fw * 4].fill(0);
                    }
                }
                gif::DisposalMethod::Previous => { if let Some(saved) = previous { canvas = saved; } }
                _ => {}
            }
        }
        if count == 0 { return Err("В GIF нет кадров.".into()); }
        if count == 1 {
            let _ = stop.recv();
            return Ok(());
        }
    }
}
