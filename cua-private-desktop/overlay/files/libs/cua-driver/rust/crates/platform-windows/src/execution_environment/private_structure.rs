//! Lazy private-desktop structure invalidation for retained UIA verification.
//!
//! WinForms does not reliably emit UIA StructureChanged events on a private
//! HDESK, but process-scoped WinEvent object notifications do. The observer is
//! intentionally session-local, lazy, and bound to one target PID. It owns no
//! desktop or process handles; the actor must stop it before its HDESK closes.

use super::win32_private::attach_current_thread_to;
use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::c_void;
use std::io;
use std::ptr::null_mut;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

type Dword = u32;
type Long = i32;
type Uint = u32;
type Wparam = usize;
type Lparam = isize;
type RawHandle = *mut c_void;
type WinEventHook = RawHandle;

const EVENT_OBJECT_CREATE: Dword = 0x8000;
const EVENT_OBJECT_END: Dword = 0x80ff;
const WINEVENT_OUTOFCONTEXT: Dword = 0x0000;
const WM_QUIT: Uint = 0x0012;
const WM_PRIVATE_STRUCTURE_SYNC: Uint = 0x8321;
const PM_NOREMOVE: Uint = 0x0000;
const OBSERVER_READY_TIMEOUT: Duration = Duration::from_secs(2);
const OBSERVER_SYNC_TIMEOUT: Duration = Duration::from_millis(500);
const OBSERVER_STACK_SIZE: usize = 512 * 1024;

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Point {
    x: Long,
    y: Long,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Msg {
    hwnd: RawHandle,
    message: Uint,
    wparam: Wparam,
    lparam: Lparam,
    time: Dword,
    pt: Point,
    l_private: Dword,
}

impl Default for Msg {
    fn default() -> Self {
        unsafe { std::mem::zeroed() }
    }
}

#[link(name = "user32")]
unsafe extern "system" {
    fn SetWinEventHook(
        event_min: Dword,
        event_max: Dword,
        hmod_win_event_proc: RawHandle,
        win_event_proc: Option<
            unsafe extern "system" fn(WinEventHook, Dword, RawHandle, Long, Long, Dword, Dword),
        >,
        process_id: Dword,
        thread_id: Dword,
        flags: Dword,
    ) -> WinEventHook;
    fn UnhookWinEvent(hook: WinEventHook) -> i32;
    fn GetMessageW(msg: *mut Msg, hwnd: RawHandle, min: Uint, max: Uint) -> i32;
    fn PeekMessageW(msg: *mut Msg, hwnd: RawHandle, min: Uint, max: Uint, remove: Uint) -> i32;
    fn PostThreadMessageW(thread_id: Dword, msg: Uint, wparam: Wparam, lparam: Lparam) -> i32;
}

#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetCurrentThreadId() -> Dword;
}

#[derive(Clone, Debug)]
pub(crate) struct PrivateStructureEpoch {
    pid: u32,
    epoch: Arc<AtomicU64>,
    alive: Arc<AtomicBool>,
    thread_id: u32,
    next_sync_id: Arc<AtomicUsize>,
    sync_waiters: Arc<Mutex<HashMap<usize, tokio::sync::oneshot::Sender<u64>>>>,
}

impl PrivateStructureEpoch {
    pub(crate) fn pid(&self) -> u32 {
        self.pid
    }

    pub(crate) fn current(&self) -> u64 {
        self.epoch.load(Ordering::Acquire)
    }

    pub(crate) fn is_alive(&self) -> bool {
        self.alive.load(Ordering::Acquire)
    }

    pub(crate) async fn synchronized_current(&self) -> io::Result<u64> {
        if !self.is_alive() {
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "private structure observer is not alive",
            ));
        }

        let request_id = self.next_sync_id.fetch_add(1, Ordering::Relaxed);
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        self.sync_waiters
            .lock()
            .map_err(|_| io::Error::other("private structure sync waiter lock poisoned"))?
            .insert(request_id, reply_tx);

        let posted =
            unsafe { PostThreadMessageW(self.thread_id, WM_PRIVATE_STRUCTURE_SYNC, request_id, 0) };
        if posted == 0 {
            if let Ok(mut waiters) = self.sync_waiters.lock() {
                waiters.remove(&request_id);
            }
            return Err(io::Error::last_os_error());
        }

        match tokio::time::timeout(OBSERVER_SYNC_TIMEOUT, reply_rx).await {
            Ok(Ok(epoch)) => Ok(epoch),
            Ok(Err(_)) => Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "private structure observer stopped before sync reply",
            )),
            Err(_) => {
                if let Ok(mut waiters) = self.sync_waiters.lock() {
                    waiters.remove(&request_id);
                }
                Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "private structure observer sync timed out",
                ))
            }
        }
    }
}

thread_local! {
    static CALLBACK_EPOCH: RefCell<Option<Arc<AtomicU64>>> = const { RefCell::new(None) };
}

unsafe extern "system" fn structure_event_callback(
    _hook: WinEventHook,
    _event: Dword,
    _hwnd: RawHandle,
    _id_object: Long,
    _id_child: Long,
    _event_thread: Dword,
    _event_time: Dword,
) {
    CALLBACK_EPOCH.with(|slot| {
        if let Some(epoch) = slot.borrow().as_ref() {
            epoch.fetch_add(1, Ordering::AcqRel);
        }
    });
}

pub(super) struct PrivateStructureObserver {
    handle: PrivateStructureEpoch,
    thread_id: u32,
    join: Option<JoinHandle<io::Result<()>>>,
}

impl PrivateStructureObserver {
    pub(super) fn start(desktop_addr: usize, pid: u32) -> io::Result<Self> {
        if pid == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "private structure observer requires pid > 0",
            ));
        }

        let epoch = Arc::new(AtomicU64::new(1));
        let alive = Arc::new(AtomicBool::new(false));
        let next_sync_id = Arc::new(AtomicUsize::new(1));
        let sync_waiters = Arc::new(Mutex::new(HashMap::new()));
        let epoch_for_thread = epoch.clone();
        let alive_for_thread = alive.clone();
        let sync_waiters_for_thread = sync_waiters.clone();
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);

        let join = thread::Builder::new()
            .name("cua-private-structure-events".into())
            .stack_size(OBSERVER_STACK_SIZE)
            .spawn(move || -> io::Result<()> {
                if let Err(error) = attach_current_thread_to(desktop_addr) {
                    let _ = ready_tx.send(Err(io::Error::new(
                        error.kind(),
                        format!("private structure observer SetThreadDesktop failed: {error}"),
                    )));
                    return Err(error);
                }

                let mut message = Msg::default();
                unsafe {
                    let _ = PeekMessageW(&mut message, null_mut(), 0, 0, PM_NOREMOVE);
                }
                let thread_id = unsafe { GetCurrentThreadId() };

                CALLBACK_EPOCH.with(|slot| {
                    *slot.borrow_mut() = Some(epoch_for_thread.clone());
                });

                let hook = unsafe {
                    SetWinEventHook(
                        EVENT_OBJECT_CREATE,
                        EVENT_OBJECT_END,
                        null_mut(),
                        Some(structure_event_callback),
                        pid,
                        0,
                        WINEVENT_OUTOFCONTEXT,
                    )
                };
                if hook.is_null() {
                    CALLBACK_EPOCH.with(|slot| {
                        *slot.borrow_mut() = None;
                    });
                    let error = io::Error::last_os_error();
                    let _ = ready_tx.send(Err(io::Error::new(
                        error.kind(),
                        format!("private structure SetWinEventHook failed: {error}"),
                    )));
                    return Err(error);
                }

                alive_for_thread.store(true, Ordering::Release);
                if ready_tx.send(Ok(thread_id)).is_err() {
                    alive_for_thread.store(false, Ordering::Release);
                    unsafe {
                        let _ = UnhookWinEvent(hook);
                    }
                    CALLBACK_EPOCH.with(|slot| {
                        *slot.borrow_mut() = None;
                    });
                    return Ok(());
                }

                loop {
                    let status = unsafe { GetMessageW(&mut message, null_mut(), 0, 0) };
                    if status <= 0 {
                        break;
                    }
                    if message.message == WM_PRIVATE_STRUCTURE_SYNC {
                        let request_id = message.wparam;
                        let reply = sync_waiters_for_thread
                            .lock()
                            .ok()
                            .and_then(|mut waiters| waiters.remove(&request_id));
                        if let Some(reply) = reply {
                            let _ = reply.send(epoch_for_thread.load(Ordering::Acquire));
                        }
                    }
                }

                alive_for_thread.store(false, Ordering::Release);
                unsafe {
                    let _ = UnhookWinEvent(hook);
                }
                CALLBACK_EPOCH.with(|slot| {
                    *slot.borrow_mut() = None;
                });
                Ok(())
            })?;

        let thread_id = match ready_rx.recv_timeout(OBSERVER_READY_TIMEOUT) {
            Ok(Ok(thread_id)) => thread_id,
            Ok(Err(error)) => {
                let _ = join.join();
                return Err(error);
            }
            Err(error) => {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    format!("private structure observer startup timed out: {error}"),
                ));
            }
        };

        Ok(Self {
            handle: PrivateStructureEpoch {
                pid,
                epoch,
                alive,
                thread_id,
                next_sync_id,
                sync_waiters,
            },
            thread_id,
            join: Some(join),
        })
    }

    pub(super) fn handle(&self) -> PrivateStructureEpoch {
        self.handle.clone()
    }

    pub(super) fn pid(&self) -> u32 {
        self.handle.pid()
    }

    pub(super) fn stop(mut self) -> io::Result<()> {
        if self.handle.is_alive() {
            let posted = unsafe { PostThreadMessageW(self.thread_id, WM_QUIT, 0, 0) };
            if posted == 0 {
                return Err(io::Error::last_os_error());
            }
        }

        let Some(join) = self.join.take() else {
            return Ok(());
        };
        match join.join() {
            Ok(result) => result,
            Err(_) => Err(io::Error::other(
                "private structure observer thread panicked during shutdown",
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn callback_advances_epoch() {
        let epoch = Arc::new(AtomicU64::new(7));
        CALLBACK_EPOCH.with(|slot| {
            *slot.borrow_mut() = Some(epoch.clone());
        });
        unsafe {
            structure_event_callback(null_mut(), EVENT_OBJECT_CREATE, null_mut(), 0, 0, 0, 0);
        }
        CALLBACK_EPOCH.with(|slot| {
            *slot.borrow_mut() = None;
        });
        assert_eq!(epoch.load(Ordering::Acquire), 8);
    }
}
