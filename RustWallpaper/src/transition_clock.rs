use super::{Handle, PostMessageW, WM_TRANSITION_TICK};
use std::ffi::c_void;
use std::ptr::null_mut;
use std::sync::{Arc, atomic::{AtomicBool, Ordering}};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

#[link(name = "kernel32")]
extern "system" {
    fn CreateWaitableTimerExW(security: *const c_void, name: *const u16, flags: u32, access: u32) -> Handle;
    fn SetWaitableTimer(timer: Handle, due: *const i64, period: i32, callback: *mut c_void, arg: *mut c_void, resume: i32) -> i32;
    fn WaitForSingleObject(handle: Handle, milliseconds: u32) -> u32;
    fn CloseHandle(handle: Handle) -> i32;
}
#[link(name = "winmm")]
extern "system" {
    fn timeBeginPeriod(ms: u32) -> u32;
    fn timeEndPeriod(ms: u32) -> u32;
}

pub(super) struct Clock {
    running: Arc<AtomicBool>,
    pending: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}
impl Clock {
    pub(super) fn start(hwnd: Handle, generation: usize) -> Result<Self, String> {
        let running = Arc::new(AtomicBool::new(true));
        let pending = Arc::new(AtomicBool::new(false));
        let run = Arc::clone(&running);
        let posted = Arc::clone(&pending);
        let window = hwnd as usize;
        let worker = thread::Builder::new().name("wallpaper-transition".into()).spawn(move || unsafe {
            let mut timer = CreateWaitableTimerExW(null_mut(), null_mut(), 2, 0x00100002);
            let mut fallback_period = false;
            if timer.is_null() {
                fallback_period = timeBeginPeriod(1) == 0;
                timer = CreateWaitableTimerExW(null_mut(), null_mut(), 0, 0x00100002);
            }
            let frame = Duration::from_nanos(8_333_333);
            let mut next = Instant::now() + frame;
            while run.load(Ordering::Acquire) {
                let now = Instant::now();
                if now < next {
                    let delay = next.duration_since(now);
                    let due = -((delay.as_nanos() / 100).max(1).min(i64::MAX as u128) as i64);
                    if !timer.is_null() && SetWaitableTimer(timer, &due, 0, null_mut(), null_mut(), 0) != 0 {
                        WaitForSingleObject(timer, 100);
                    } else { thread::sleep(delay); }
                }
                if !run.load(Ordering::Acquire) { break; }
                if !posted.swap(true, Ordering::AcqRel) && PostMessageW(window as Handle, WM_TRANSITION_TICK, generation, 0) == 0 {
                    posted.store(false, Ordering::Release);
                    break;
                }
                next += frame;
                if Instant::now().saturating_duration_since(next) > frame * 3 { next = Instant::now() + frame; }
            }
            if !timer.is_null() { CloseHandle(timer); }
            if fallback_period { timeEndPeriod(1); }
        }).map_err(|e| format!("Не удалось запустить таймер: {e}"))?;
        Ok(Self { running, pending, worker: Some(worker) })
    }
    pub(super) fn acknowledge(&self) { self.pending.store(false, Ordering::Release); }
}
impl Drop for Clock {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Release);
        if let Some(worker) = self.worker.take() { let _ = worker.join(); }
    }
}
