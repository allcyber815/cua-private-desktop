//! Client for the process-born private visual observer.
//!
//! The accepted B3 contract relies on the visual worker process itself being
//! born on the owned HDESK. Attaching a daemon thread to that desktop is not
//! equivalent for WPF PrintWindow freshness, so strict visual verification
//! uses this one-shot helper boundary.

use super::runtime::PrivateDesktopRuntime;
use super::win32_private::PrivateChild;
use anyhow::{anyhow, bail, Context, Result};
use serde_json::Value;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant};

const READY_TIMEOUT: Duration = Duration::from_secs(10);
const RESULT_TIMEOUT: Duration = Duration::from_secs(10);
const WORKER_EXIT_TIMEOUT: Duration = Duration::from_secs(2);
const POLL_INTERVAL: Duration = Duration::from_millis(20);

static SESSION_COUNTER: AtomicU64 = AtomicU64::new(1);

pub struct PrivateVisualWorkerSession {
    child: PrivateChild,
    control_dir: PathBuf,
    ready: Value,
}

impl PrivateVisualWorkerSession {
    pub fn ready_receipt(&self) -> &Value {
        &self.ready
    }

    pub fn worker_pid(&self) -> u32 {
        self.child.id()
    }
}

impl Drop for PrivateVisualWorkerSession {
    fn drop(&mut self) {
        // If the semantic action fails before visual completion, release a
        // waiting helper rather than leaving it parked until its 15 s timeout.
        // Successful finish clears control_dir before Drop.
        if !self.control_dir.as_os_str().is_empty() {
            let _ = std::fs::write(self.control_dir.join("continue"), b"cancel");
        }
    }
}

fn worker_executable() -> Result<PathBuf> {
    if let Some(override_path) = std::env::var_os("CUA_PRIVATE_VISUAL_WORKER") {
        let path = PathBuf::from(override_path);
        if path.is_file() {
            return Ok(path);
        }
        bail!(
            "CUA_PRIVATE_VISUAL_WORKER points to a missing file: {}",
            path.display()
        );
    }

    let current = std::env::current_exe().context("resolve current executable")?;
    let directory = current
        .parent()
        .ok_or_else(|| anyhow!("current executable has no parent: {}", current.display()))?;

    let worker_name = "private_visual_worker.exe";
    let mut candidates = vec![directory.join(worker_name)];
    if directory
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.eq_ignore_ascii_case("deps"))
    {
        if let Some(parent) = directory.parent() {
            candidates.push(parent.join(worker_name));
        }
    }

    for candidate in &candidates {
        if candidate.is_file() {
            return Ok(candidate.clone());
        }
    }

    bail!(
        "private visual worker executable was not found; checked {}",
        candidates
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>()
            .join(", ")
    )
}

fn fresh_control_dir() -> Result<PathBuf> {
    let sequence = SESSION_COUNTER.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "webgpt-private-visual-{}-{sequence}",
        std::process::id()
    ));
    if path.exists() {
        std::fs::remove_dir_all(&path)
            .with_context(|| format!("remove stale {}", path.display()))?;
    }
    std::fs::create_dir_all(&path).with_context(|| format!("create {}", path.display()))?;
    Ok(path)
}

fn wait_json(path: &Path, timeout: Duration) -> Result<Value> {
    let deadline = Instant::now() + timeout;
    loop {
        if path.is_file() {
            let bytes = std::fs::read(path).with_context(|| format!("read {}", path.display()))?;
            return serde_json::from_slice(&bytes)
                .with_context(|| format!("parse {}", path.display()));
        }
        if Instant::now() >= deadline {
            bail!("timed out waiting for {}", path.display());
        }
        thread::sleep(POLL_INTERVAL);
    }
}

fn validate_ready(receipt: &Value) -> Result<()> {
    if receipt["status"].as_str() != Some("ready") {
        bail!(
            "private visual worker baseline failed: {}",
            receipt["error"]
                .as_str()
                .unwrap_or("worker did not report ready")
        );
    }
    let sampled_pixels = receipt["baseline_sampled_pixels"].as_u64().unwrap_or(0);
    let masked_samples = receipt["baseline_noise_masked_samples"]
        .as_u64()
        .unwrap_or(u64::MAX);
    let computed_limit = super::private_visual::baseline_noise_limit(sampled_pixels);
    if receipt["baseline_stable"].as_bool() != Some(true)
        || masked_samples > computed_limit
        || receipt["baseline_noise_limit"].as_u64() != Some(computed_limit)
        || receipt["baseline_sentinel_count"].as_u64() != Some(0)
        || receipt["baseline_trusted"].as_bool() != Some(true)
    {
        bail!("private visual worker returned an untrusted baseline: {receipt}");
    }
    Ok(())
}

pub fn start(environment: &PrivateDesktopRuntime, hwnd: u64) -> Result<PrivateVisualWorkerSession> {
    let executable = worker_executable()?;
    let control_dir = fresh_control_dir()?;
    let args = [
        OsString::from(hwnd.to_string()),
        control_dir.as_os_str().to_os_string(),
    ];
    let child = environment
        .spawn_direct_helper(&executable, &args, executable.parent())
        .with_context(|| {
            format!(
                "spawn private visual worker {} on owned desktop",
                executable.display()
            )
        })?;

    let ready_path = control_dir.join("ready.json");
    let ready = match wait_json(&ready_path, READY_TIMEOUT) {
        Ok(receipt) => receipt,
        Err(error) => {
            let _ = std::fs::write(control_dir.join("continue"), b"cancel");
            return Err(error);
        }
    };
    validate_ready(&ready)?;

    Ok(PrivateVisualWorkerSession {
        child,
        control_dir,
        ready,
    })
}

pub fn finish(mut session: PrivateVisualWorkerSession) -> Result<Value> {
    let worker_pid = session.child.id();
    std::fs::write(session.control_dir.join("continue"), b"go")
        .with_context(|| format!("signal private visual worker {worker_pid}"))?;

    let mut receipt = wait_json(&session.control_dir.join("result.json"), RESULT_TIMEOUT)?;
    if receipt["status"].as_str() != Some("ok") {
        bail!(
            "private visual worker post-capture failed: {}",
            receipt["error"]
                .as_str()
                .unwrap_or("worker returned non-ok status")
        );
    }
    if receipt["trusted"].as_bool() != Some(true)
        || receipt["post_sentinel_count"].as_u64() != Some(0)
        || receipt["capture_success"].as_bool() != Some(true)
    {
        bail!("private visual worker returned an untrusted post frame: {receipt}");
    }

    let exited = session
        .child
        .wait_timeout(WORKER_EXIT_TIMEOUT)
        .context("wait for private visual worker exit")?;
    if !exited {
        bail!("private visual worker {worker_pid} did not exit after result receipt");
    }

    if let Some(object) = receipt.as_object_mut() {
        object.insert(
            "worker_process_born_on_private_desktop".into(),
            Value::Bool(true),
        );
        object.insert("worker_pid".into(), Value::from(worker_pid));
        object.insert(
            "baseline_worker_pid".into(),
            session
                .ready
                .get("pid")
                .cloned()
                .unwrap_or_else(|| Value::from(worker_pid)),
        );
    }

    let control_dir = session.control_dir.clone();
    // Suppress Drop's cancellation signal after successful completion.
    session.control_dir = PathBuf::new();
    drop(session);
    let _ = std::fs::remove_dir_all(control_dir);

    Ok(receipt)
}
