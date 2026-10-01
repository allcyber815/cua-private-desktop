//! Outer-desktop interference telemetry for strict private-desktop actions.
//!
//! The monitor is deliberately born on the caller/default desktop, not on the
//! private actor. It samples the user's input desktop and foreground owner while
//! the private action runs, and uses a duplicated Job handle to prove that no
//! process owned by the private environment became the outer foreground.

use super::runtime::PrivateDesktopRuntime;
use super::win32_private::PrivateJobProbe;
use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId;

const SAMPLE_INTERVAL: Duration = Duration::from_millis(20);

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
    fn new(target_pid: u32) -> Self {
        Self {
            target_pid,
            input_desktop_before: None,
            input_desktop_after: None,
            monitor_thread_desktop: None,
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
            "sample_interval_ms": SAMPLE_INTERVAL.as_millis() as u64,
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

fn sample(probe: &PrivateJobProbe, state: &mut MonitorState) {
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

    if state.sample_count == 1 {
        state.input_desktop_before = input_name.clone();
        state.monitor_thread_desktop = desktop.thread_desktop_name.clone();
        match (
            state.monitor_thread_desktop.as_deref(),
            state.input_desktop_before.as_deref(),
        ) {
            (Some(thread_name), Some(input_name)) if thread_name.eq_ignore_ascii_case(input_name) => {
            }
            (Some(thread_name), Some(input_name)) => state.fail(format!(
                "interference monitor is not attached to the input desktop: thread={thread_name:?}, input={input_name:?}"
            )),
            _ => state.fail("interference monitor desktop authority could not be established"),
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
    if state.sample_count == 1 {
        state.outer_foreground_before_pid = pid;
    }
    state.outer_foreground_after_pid = pid;

    if pid != 0 {
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
}

pub struct PrivateInterferenceMonitor {
    stop: Arc<AtomicBool>,
    join: Option<JoinHandle<MonitorState>>,
}

impl PrivateInterferenceMonitor {
    pub fn start(environment: &PrivateDesktopRuntime, target_pid: u32) -> Result<Self> {
        let probe = environment
            .duplicate_job_probe()
            .context("could not duplicate private Job handle for interference telemetry")?;
        let stop = Arc::new(AtomicBool::new(false));
        let stop_for_thread = stop.clone();
        let (ready_tx, ready_rx) = std::sync::mpsc::sync_channel(1);

        let join = thread::Builder::new()
            .name("cua-private-interference-monitor".into())
            .spawn(move || {
                let mut state = MonitorState::new(target_pid);
                sample(&probe, &mut state);
                let baseline = if !state.telemetry_complete {
                    Err(format!(
                        "interference telemetry baseline is not authoritative: {}",
                        state.errors.join("; ")
                    ))
                } else if state.target_observed_on_outer_foreground
                    || state.owned_job_process_observed_on_outer_foreground
                {
                    Err("private Job already owns the user's outer foreground before action dispatch".to_string())
                } else {
                    Ok(())
                };
                let baseline_ok = baseline.is_ok();
                let _ = ready_tx.send(baseline);
                if !baseline_ok {
                    return state;
                }

                loop {
                    thread::sleep(SAMPLE_INTERVAL);
                    sample(&probe, &mut state);
                    if stop_for_thread.load(Ordering::Acquire) {
                        break;
                    }
                }
                state
            })
            .context("failed to spawn private interference monitor")?;

        match ready_rx.recv_timeout(Duration::from_secs(2)) {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                stop.store(true, Ordering::Release);
                let _ = join.join();
                bail!("{error}");
            }
            Err(error) => {
                stop.store(true, Ordering::Release);
                let _ = join.join();
                bail!("interference monitor baseline handshake failed: {error}");
            }
        }

        Ok(Self {
            stop,
            join: Some(join),
        })
    }

    pub fn finish(mut self) -> PrivateInterferenceReceipt {
        self.stop.store(true, Ordering::Release);
        let Some(join) = self.join.take() else {
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
                errors: vec!["interference monitor join handle missing".into()],
            };
        };
        match join.join() {
            Ok(state) => state.into(),
            Err(_) => PrivateInterferenceReceipt {
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
                errors: vec!["interference monitor thread panicked".into()],
            },
        }
    }
}

impl Drop for PrivateInterferenceMonitor {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
