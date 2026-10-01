//! Outer-desktop interference telemetry for strict private-desktop actions.
//!
//! A process-lifetime WinEvent observer is deliberately born on the caller /
//! default desktop, never on the private actor. Each private action records an
//! event-log cursor plus authoritative before/after samples. Foreground and
//! desktop-switch events between those samples supplement the direct reads so
//! a short-lived focus steal cannot hide between polling intervals.
//!
//! WinEvent delivery is treated as an invalidation/evidence stream, not as the
//! sole source of truth: start/end desktop + foreground reads remain
//! authoritative, ring overflow and observer failure fail closed, and each
//! observed foreground pid is checked against a duplicated private Job handle.

use super::runtime::PrivateDesktopRuntime;
use super::win32_private::PrivateJobProbe;
use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use std::collections::VecDeque;
use std::ffi::c_void;
use std::ptr::null_mut;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex, OnceLock};
use std::thread;
use std::time::Duration;
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId;

const EVENT_LOG_CAPACITY: usize = 2048;
const OBSERVER_READY_TIMEOUT: Duration = Duration::from_secs(2);

type Dword = u32;
type Long = i32;
type Uint = u32;
type Wparam = usize;
type Lparam = isize;
type RawHandle = *mut c_void;
type WinEventHook = RawHandle;

const EVENT_SYSTEM_FOREGROUND: Dword = 0x0003;
const EVENT_SYSTEM_DESKTOPSWITCH: Dword = 0x0020;
const WINEVENT_OUTOFCONTEXT: Dword = 0x0000;

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
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum OuterEventKind {
    Foreground(Option<u32>),
    DesktopSwitch,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct OuterEvent {
    seq: u64,
    kind: OuterEventKind,
}

#[derive(Debug)]
struct ObserverLog {
    next_seq: u64,
    dropped_through_seq: u64,
    events: VecDeque<OuterEvent>,
}

impl ObserverLog {
    fn new() -> Self {
        Self {
            next_seq: 0,
            dropped_through_seq: 0,
            events: VecDeque::with_capacity(EVENT_LOG_CAPACITY),
        }
    }

    fn push(&mut self, kind: OuterEventKind) {
        self.next_seq = self.next_seq.wrapping_add(1);
        // A wrapped sequence cannot preserve cursor ordering. Fail-closed by
        // invalidating every older cursor rather than pretending continuity.
        if self.next_seq == 0 {
            self.next_seq = 1;
            self.events.clear();
            self.dropped_through_seq = u64::MAX;
        }

        let event = OuterEvent {
            seq: self.next_seq,
            kind,
        };
        self.events.push_back(event);
        while self.events.len() > EVENT_LOG_CAPACITY {
            if let Some(dropped) = self.events.pop_front() {
                self.dropped_through_seq = dropped.seq;
            }
        }
    }

    fn cursor(&self) -> u64 {
        self.next_seq
    }

    fn events_between(
        &self,
        start_cursor: u64,
        end_cursor: u64,
    ) -> Result<Vec<OuterEvent>, String> {
        if end_cursor < start_cursor {
            return Err(format!(
                "persistent interference event cursor regressed: start={start_cursor}, end={end_cursor}"
            ));
        }
        if self.dropped_through_seq == u64::MAX || start_cursor < self.dropped_through_seq {
            return Err(format!(
                "persistent interference event log overflowed after cursor {start_cursor}"
            ));
        }
        if end_cursor > self.next_seq {
            return Err(format!(
                "persistent interference end cursor {end_cursor} exceeds current cursor {}",
                self.next_seq
            ));
        }
        Ok(self
            .events
            .iter()
            .copied()
            .filter(|event| event.seq > start_cursor && event.seq <= end_cursor)
            .collect())
    }
}

struct ObserverShared {
    log: Mutex<ObserverLog>,
    alive: AtomicBool,
    log_poisoned: AtomicBool,
}

impl ObserverShared {
    fn new() -> Self {
        Self {
            log: Mutex::new(ObserverLog::new()),
            alive: AtomicBool::new(false),
            log_poisoned: AtomicBool::new(false),
        }
    }

    fn push(&self, kind: OuterEventKind) {
        match self.log.lock() {
            Ok(mut log) => log.push(kind),
            Err(_) => self.log_poisoned.store(true, Ordering::Release),
        }
    }

    fn cursor(&self) -> Result<u64, String> {
        if self.log_poisoned.load(Ordering::Acquire) {
            return Err("persistent interference event log mutex poisoned".into());
        }
        self.log
            .lock()
            .map(|log| log.cursor())
            .map_err(|_| "persistent interference event log mutex poisoned".into())
    }

    fn events_between(
        &self,
        start_cursor: u64,
        end_cursor: u64,
    ) -> Result<Vec<OuterEvent>, String> {
        if !self.alive.load(Ordering::Acquire) {
            return Err("persistent interference WinEvent observer is not alive".into());
        }
        if self.log_poisoned.load(Ordering::Acquire) {
            return Err("persistent interference event log mutex poisoned".into());
        }
        self.log
            .lock()
            .map_err(|_| "persistent interference event log mutex poisoned".to_string())?
            .events_between(start_cursor, end_cursor)
    }
}

struct PersistentObserver {
    shared: Arc<ObserverShared>,
    thread_desktop_name: Option<String>,
    input_desktop_name_at_start: Option<String>,
}

static CALLBACK_SHARED: OnceLock<Arc<ObserverShared>> = OnceLock::new();
static PERSISTENT_OBSERVER: OnceLock<Result<PersistentObserver, String>> = OnceLock::new();

unsafe extern "system" fn win_event_callback(
    _hook: WinEventHook,
    event: Dword,
    hwnd: RawHandle,
    _id_object: Long,
    _id_child: Long,
    _event_thread: Dword,
    _event_time: Dword,
) {
    let Some(shared) = CALLBACK_SHARED.get() else {
        return;
    };

    match event {
        EVENT_SYSTEM_FOREGROUND => {
            let pid = if hwnd.is_null() {
                None
            } else {
                let mut pid = 0u32;
                let thread_id = GetWindowThreadProcessId(HWND(hwnd), Some(&mut pid));
                (thread_id != 0 && pid != 0).then_some(pid)
            };
            shared.push(OuterEventKind::Foreground(pid));
        }
        EVENT_SYSTEM_DESKTOPSWITCH => shared.push(OuterEventKind::DesktopSwitch),
        _ => {}
    }
}

impl PersistentObserver {
    fn start() -> Result<Self, String> {
        let shared = Arc::new(ObserverShared::new());
        CALLBACK_SHARED.set(shared.clone()).map_err(|_| {
            "persistent interference callback state was already initialized".to_string()
        })?;

        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let shared_for_thread = shared.clone();
        let join = thread::Builder::new()
            .name("cua-private-interference-events".into())
            .spawn(move || {
                let desktop = crate::diagnostics::desktop_state();
                let thread_desktop_name = desktop.thread_desktop_name.clone();
                let input_desktop_name = desktop.input_desktop_name.clone();

                let identity_ok = match (
                    thread_desktop_name.as_deref(),
                    input_desktop_name.as_deref(),
                ) {
                    (Some(thread_name), Some(input_name))
                        if thread_name.eq_ignore_ascii_case(input_name) =>
                    {
                        Ok(())
                    }
                    (Some(thread_name), Some(input_name)) => Err(format!(
                        "persistent interference observer is not attached to the input desktop: thread={thread_name:?}, input={input_name:?}"
                    )),
                    _ => Err(format!(
                        "persistent interference observer desktop authority unavailable: thread={:?}, input={:?}, input_error={:?}",
                        thread_desktop_name,
                        input_desktop_name,
                        desktop.input_desktop_error
                    )),
                };

                if let Err(error) = identity_ok {
                    let _ = ready_tx.send(Err(error));
                    return;
                }

                let foreground_hook = unsafe {
                    SetWinEventHook(
                        EVENT_SYSTEM_FOREGROUND,
                        EVENT_SYSTEM_FOREGROUND,
                        null_mut(),
                        Some(win_event_callback),
                        0,
                        0,
                        WINEVENT_OUTOFCONTEXT,
                    )
                };
                if foreground_hook.is_null() {
                    let _ = ready_tx.send(Err(format!(
                        "SetWinEventHook(EVENT_SYSTEM_FOREGROUND) failed: {}",
                        std::io::Error::last_os_error()
                    )));
                    return;
                }

                let desktop_hook = unsafe {
                    SetWinEventHook(
                        EVENT_SYSTEM_DESKTOPSWITCH,
                        EVENT_SYSTEM_DESKTOPSWITCH,
                        null_mut(),
                        Some(win_event_callback),
                        0,
                        0,
                        WINEVENT_OUTOFCONTEXT,
                    )
                };
                if desktop_hook.is_null() {
                    unsafe {
                        let _ = UnhookWinEvent(foreground_hook);
                    }
                    let _ = ready_tx.send(Err(format!(
                        "SetWinEventHook(EVENT_SYSTEM_DESKTOPSWITCH) failed: {}",
                        std::io::Error::last_os_error()
                    )));
                    return;
                }

                shared_for_thread.alive.store(true, Ordering::Release);
                let _ = ready_tx.send(Ok((
                    thread_desktop_name,
                    input_desktop_name,
                )));

                let mut msg = Msg::default();
                loop {
                    let status = unsafe { GetMessageW(&mut msg, null_mut(), 0, 0) };
                    if status <= 0 {
                        break;
                    }
                }

                shared_for_thread.alive.store(false, Ordering::Release);
                unsafe {
                    let _ = UnhookWinEvent(foreground_hook);
                    let _ = UnhookWinEvent(desktop_hook);
                }
            })
            .map_err(|error| format!("failed to spawn persistent interference observer: {error}"))?;

        let identity = ready_rx
            .recv_timeout(OBSERVER_READY_TIMEOUT)
            .map_err(|error| {
                format!("persistent interference observer startup timed out: {error}")
            })??;

        // The observer intentionally lives for the Driver process lifetime.
        // Dropping JoinHandle detaches the thread; the thread owns no private
        // Job/Desktop handles and therefore cannot extend a session lifetime.
        drop(join);

        Ok(Self {
            shared,
            thread_desktop_name: identity.0,
            input_desktop_name_at_start: identity.1,
        })
    }

    fn get() -> Result<&'static Self> {
        match PERSISTENT_OBSERVER.get_or_init(Self::start) {
            Ok(observer) => Ok(observer),
            Err(error) => Err(anyhow::anyhow!(error.clone())),
        }
    }

    fn cursor(&self) -> Result<u64, String> {
        self.shared.cursor()
    }

    fn events_between(
        &self,
        start_cursor: u64,
        end_cursor: u64,
    ) -> Result<Vec<OuterEvent>, String> {
        self.shared.events_between(start_cursor, end_cursor)
    }
}

#[derive(Clone, Debug)]
struct MonitorState {
    target_pid: u32,
    input_desktop_before: Option<String>,
    input_desktop_after: Option<String>,
    monitor_thread_desktop: Option<String>,
    outer_foreground_before_pid: u32,
    outer_foreground_after_pid: u32,
    input_desktop_changed: bool,
    target_observed_on_outer_foreground: bool,
    owned_job_process_observed_on_outer_foreground: bool,
    telemetry_complete: bool,
    sample_count: u64,
    errors: Vec<String>,
}

impl MonitorState {
    fn new(target_pid: u32, monitor_thread_desktop: Option<String>) -> Self {
        Self {
            target_pid,
            input_desktop_before: None,
            input_desktop_after: None,
            monitor_thread_desktop,
            outer_foreground_before_pid: 0,
            outer_foreground_after_pid: 0,
            input_desktop_changed: false,
            target_observed_on_outer_foreground: false,
            owned_job_process_observed_on_outer_foreground: false,
            telemetry_complete: true,
            sample_count: 0,
            errors: Vec::new(),
        }
    }

    fn fail(&mut self, detail: impl Into<String>) {
        self.telemetry_complete = false;
        if self.errors.len() < 8 {
            self.errors.push(detail.into());
        }
    }
}

#[derive(Clone, Debug)]
pub struct PrivateInterferenceReceipt {
    pub input_desktop_before: Option<String>,
    pub input_desktop_after: Option<String>,
    pub monitor_thread_desktop: Option<String>,
    pub outer_foreground_before_pid: u32,
    pub outer_foreground_after_pid: u32,
    pub input_desktop_changed: bool,
    pub target_observed_on_outer_foreground: bool,
    pub owned_job_process_observed_on_outer_foreground: bool,
    pub interference_telemetry_complete: bool,
    pub sample_count: u64,
    pub errors: Vec<String>,
}

impl PrivateInterferenceReceipt {
    pub fn clean(&self) -> bool {
        self.interference_telemetry_complete
            && !self.input_desktop_changed
            && !self.target_observed_on_outer_foreground
            && !self.owned_job_process_observed_on_outer_foreground
    }

    pub fn as_json(&self) -> Value {
        json!({
            "input_desktop_before": self.input_desktop_before,
            "input_desktop_after": self.input_desktop_after,
            "monitor_thread_desktop": self.monitor_thread_desktop,
            "outer_foreground_before_pid": self.outer_foreground_before_pid,
            "outer_foreground_after_pid": self.outer_foreground_after_pid,
            "input_desktop_changed": self.input_desktop_changed,
            "target_observed_on_outer_foreground": self.target_observed_on_outer_foreground,
            "owned_job_process_observed_on_outer_foreground": self.owned_job_process_observed_on_outer_foreground,
            "interference_telemetry_complete": self.interference_telemetry_complete,
            "sample_count": self.sample_count,
            "errors": self.errors,
            "clean": self.clean(),
            "monitor_mode": "persistent_win_event",
            "sample_interval_ms": 0,
            "event_log_capacity": EVENT_LOG_CAPACITY as u64,
        })
    }
}

impl From<MonitorState> for PrivateInterferenceReceipt {
    fn from(state: MonitorState) -> Self {
        Self {
            input_desktop_before: state.input_desktop_before,
            input_desktop_after: state.input_desktop_after,
            monitor_thread_desktop: state.monitor_thread_desktop,
            outer_foreground_before_pid: state.outer_foreground_before_pid,
            outer_foreground_after_pid: state.outer_foreground_after_pid,
            input_desktop_changed: state.input_desktop_changed,
            target_observed_on_outer_foreground: state.target_observed_on_outer_foreground,
            owned_job_process_observed_on_outer_foreground: state
                .owned_job_process_observed_on_outer_foreground,
            interference_telemetry_complete: state.telemetry_complete,
            sample_count: state.sample_count,
            errors: state.errors,
        }
    }
}

fn foreground_pid(hwnd: Option<usize>) -> Result<u32, String> {
    let Some(hwnd) = hwnd else {
        return Ok(0);
    };
    let mut pid = 0u32;
    let thread_id = unsafe { GetWindowThreadProcessId(HWND(hwnd as *mut _), Some(&mut pid)) };
    if thread_id == 0 || pid == 0 {
        return Err(format!(
            "GetWindowThreadProcessId failed for outer foreground HWND 0x{hwnd:x}"
        ));
    }
    Ok(pid)
}

fn observe_foreground_pid(probe: &PrivateJobProbe, state: &mut MonitorState, pid: u32) {
    if pid == 0 {
        return;
    }
    if pid == state.target_pid {
        state.target_observed_on_outer_foreground = true;
    }
    match probe.contains_pid(pid) {
        Ok(true) => state.owned_job_process_observed_on_outer_foreground = true,
        Ok(false) => {}
        Err(error) => state.fail(format!(
            "private Job membership probe failed for outer foreground pid {pid}: {error}"
        )),
    }
}

fn sample_current(probe: &PrivateJobProbe, state: &mut MonitorState, first: bool) {
    let desktop = crate::diagnostics::desktop_state();
    state.sample_count += 1;

    let input_name = match desktop.input_desktop_name.clone() {
        Some(name) => Some(name),
        None => {
            state.fail(format!(
                "input desktop unavailable: {}",
                desktop
                    .input_desktop_error
                    .as_deref()
                    .unwrap_or("unknown error")
            ));
            None
        }
    };

    if first {
        state.input_desktop_before = input_name.clone();
        match (
            state.monitor_thread_desktop.as_deref(),
            state.input_desktop_before.as_deref(),
        ) {
            (Some(thread_name), Some(input_name)) if thread_name.eq_ignore_ascii_case(input_name) => {}
            (Some(thread_name), Some(input_name)) => state.fail(format!(
                "persistent interference observer is not attached to the input desktop: thread={thread_name:?}, input={input_name:?}"
            )),
            _ => state.fail("persistent interference observer desktop authority could not be established"),
        }
    } else if let (Some(before), Some(current)) =
        (state.input_desktop_before.as_deref(), input_name.as_deref())
    {
        if !before.eq_ignore_ascii_case(current) {
            state.input_desktop_changed = true;
        }
    }
    state.input_desktop_after = input_name;

    let pid = match foreground_pid(desktop.foreground_hwnd) {
        Ok(pid) => pid,
        Err(error) => {
            state.fail(error);
            0
        }
    };
    if first {
        state.outer_foreground_before_pid = pid;
    }
    state.outer_foreground_after_pid = pid;
    observe_foreground_pid(probe, state, pid);
}

pub struct PrivateInterferenceMonitor {
    observer: &'static PersistentObserver,
    cursor: u64,
    probe: PrivateJobProbe,
    state: Option<MonitorState>,
}

impl PrivateInterferenceMonitor {
    pub fn start(environment: &PrivateDesktopRuntime, target_pid: u32) -> Result<Self> {
        let observer = PersistentObserver::get()
            .context("persistent outer-desktop WinEvent observer is unavailable")?;
        let probe = environment
            .duplicate_job_probe()
            .context("could not duplicate private Job handle for interference telemetry")?;

        if observer.input_desktop_name_at_start.is_none() {
            bail!("persistent interference observer input-desktop identity is unavailable");
        }

        // Cursor first: any foreground/desktop-switch event racing with the
        // authoritative baseline is then included in finish()'s event slice.
        let cursor = observer
            .cursor()
            .map_err(anyhow::Error::msg)
            .context("could not capture persistent interference event cursor")?;

        let mut state = MonitorState::new(target_pid, observer.thread_desktop_name.clone());
        sample_current(&probe, &mut state, true);
        if !state.telemetry_complete {
            bail!(
                "interference telemetry baseline is not authoritative: {}",
                state.errors.join("; ")
            );
        }
        if state.target_observed_on_outer_foreground
            || state.owned_job_process_observed_on_outer_foreground
        {
            bail!("private Job already owns the user's outer foreground before action dispatch");
        }

        Ok(Self {
            observer,
            cursor,
            probe,
            state: Some(state),
        })
    }

    pub fn finish(mut self) -> PrivateInterferenceReceipt {
        let Some(mut state) = self.state.take() else {
            return PrivateInterferenceReceipt {
                input_desktop_before: None,
                input_desktop_after: None,
                monitor_thread_desktop: None,
                outer_foreground_before_pid: 0,
                outer_foreground_after_pid: 0,
                input_desktop_changed: false,
                target_observed_on_outer_foreground: false,
                owned_job_process_observed_on_outer_foreground: false,
                interference_telemetry_complete: false,
                sample_count: 0,
                errors: vec!["persistent interference monitor state missing".into()],
            };
        };

        // Establish the terminal state first, then freeze the event-log
        // boundary. Any foreground/desktop-switch event that races with the
        // final direct read is therefore included in the bounded event slice.
        // Events after end_cursor are outside this action's acceptance window.
        sample_current(&self.probe, &mut state, false);
        let end_cursor = match self.observer.cursor() {
            Ok(cursor) => cursor,
            Err(error) => {
                state.fail(error);
                return state.into();
            }
        };

        match self.observer.events_between(self.cursor, end_cursor) {
            Ok(events) => {
                for event in events {
                    state.sample_count += 1;
                    match event.kind {
                        OuterEventKind::Foreground(Some(pid)) => {
                            observe_foreground_pid(&self.probe, &mut state, pid);
                        }
                        OuterEventKind::Foreground(None) => state.fail(
                            "persistent interference observer saw foreground change but could not resolve pid",
                        ),
                        OuterEventKind::DesktopSwitch => {
                            state.input_desktop_changed = true;
                        }
                    }
                }
            }
            Err(error) => state.fail(error),
        }

        state.into()
    }
}

impl Drop for PrivateInterferenceMonitor {
    fn drop(&mut self) {
        // No per-action worker exists in v2. The process-lifetime observer owns
        // no private session handles; dropping an unfinished monitor therefore
        // only discards its cursor/state.
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fg(seq: u64, pid: u32) -> OuterEvent {
        OuterEvent {
            seq,
            kind: OuterEventKind::Foreground(Some(pid)),
        }
    }

    #[test]
    fn observer_log_returns_only_events_after_cursor() {
        let mut log = ObserverLog::new();
        log.push(OuterEventKind::Foreground(Some(10)));
        let cursor = log.cursor();
        log.push(OuterEventKind::Foreground(Some(20)));
        log.push(OuterEventKind::DesktopSwitch);

        let end_cursor = log.cursor();
        let events = log.events_between(cursor, end_cursor).unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].kind, OuterEventKind::Foreground(Some(20)));
        assert_eq!(events[1].kind, OuterEventKind::DesktopSwitch);
    }

    #[test]
    fn observer_log_overflow_fails_old_cursor_closed() {
        let mut log = ObserverLog::new();
        let cursor = log.cursor();
        for pid in 1..=(EVENT_LOG_CAPACITY as u32 + 1) {
            log.push(OuterEventKind::Foreground(Some(pid)));
        }
        let current = log.cursor();
        assert!(log.events_between(cursor, current).is_err());
        assert!(log.events_between(current, current).unwrap().is_empty());
    }

    #[test]
    fn outer_event_shape_is_copyable() {
        let event = fg(7, 42);
        let copy = event;
        assert_eq!(copy.seq, 7);
        assert_eq!(copy.kind, OuterEventKind::Foreground(Some(42)));
    }

    #[test]
    fn clean_requires_complete_and_no_interference() {
        let mut receipt = PrivateInterferenceReceipt {
            input_desktop_before: Some("Default".into()),
            input_desktop_after: Some("Default".into()),
            monitor_thread_desktop: Some("Default".into()),
            outer_foreground_before_pid: 1,
            outer_foreground_after_pid: 1,
            input_desktop_changed: false,
            target_observed_on_outer_foreground: false,
            owned_job_process_observed_on_outer_foreground: false,
            interference_telemetry_complete: true,
            sample_count: 2,
            errors: Vec::new(),
        };
        assert!(receipt.clean());
        receipt.owned_job_process_observed_on_outer_foreground = true;
        assert!(!receipt.clean());
    }
}
