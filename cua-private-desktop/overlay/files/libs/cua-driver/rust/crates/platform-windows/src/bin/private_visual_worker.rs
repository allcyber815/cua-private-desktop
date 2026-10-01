#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

#[cfg(target_os = "windows")]
use anyhow::{bail, Context, Result};
#[cfg(target_os = "windows")]
use serde_json::json;
#[cfg(target_os = "windows")]
use std::path::{Path, PathBuf};
#[cfg(target_os = "windows")]
use std::thread;
#[cfg(target_os = "windows")]
use std::time::{Duration, Instant};
#[cfg(target_os = "windows")]
use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED};

#[cfg(target_os = "windows")]
struct ComGuard;

#[cfg(target_os = "windows")]
impl Drop for ComGuard {
    fn drop(&mut self) {
        unsafe { CoUninitialize() };
    }
}

#[cfg(target_os = "windows")]
fn parse_args() -> Result<(u64, PathBuf)> {
    let mut args = std::env::args_os().skip(1);
    let hwnd = args
        .next()
        .context("private_visual_worker requires HWND argument")?
        .to_string_lossy()
        .parse::<u64>()
        .context("invalid HWND argument")?;
    let control_dir = PathBuf::from(
        args.next()
            .context("private_visual_worker requires control directory argument")?,
    );
    if args.next().is_some() {
        bail!("private_visual_worker received unexpected extra arguments");
    }
    Ok((hwnd, control_dir))
}

#[cfg(target_os = "windows")]
fn write_json(path: &Path, value: &serde_json::Value) -> Result<()> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, serde_json::to_vec(value)?)
        .with_context(|| format!("write {}", tmp.display()))?;
    std::fs::rename(&tmp, path).with_context(|| {
        format!(
            "rename worker receipt {} -> {}",
            tmp.display(),
            path.display()
        )
    })?;
    Ok(())
}

#[cfg(target_os = "windows")]
fn worker(hwnd: u64, control_dir: &Path) -> Result<()> {
    let hr = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
    if hr.is_err() {
        bail!("CoInitializeEx(STA) failed: {hr:?}");
    }
    let _com = ComGuard;

    std::fs::create_dir_all(control_dir)
        .with_context(|| format!("create {}", control_dir.display()))?;

    let baseline = platform_windows::execution_environment::private_visual::settle_baseline(hwnd)?;
    let baseline_noise_limit =
        platform_windows::execution_environment::private_visual::baseline_noise_limit(
            baseline.frame.sampled_pixels,
        );
    write_json(
        &control_dir.join("ready.json"),
        &json!({
            "status": "ready",
            "pid": std::process::id(),
            "observer": "printwindow",
            "flags": "PW_RENDERFULLCONTENT",
            "visual_baseline_attempts": baseline.attempts,
            "baseline_changed": baseline.baseline_diff.changed_samples,
            "baseline_diff_sum": baseline.baseline_diff.diff_sum,
            "baseline_noise_masked_samples": baseline.noise_masked_samples(),
            "baseline_noise_limit": baseline_noise_limit,
            "baseline_exact": baseline.noise_masked_samples() == 0,
            "baseline_stable": baseline.noise_masked_samples() <= baseline_noise_limit,
            "baseline_sentinel_count": baseline.frame.sentinel_count,
            "baseline_sampled_pixels": baseline.frame.sampled_pixels,
            "baseline_trusted": baseline.frame.trusted(),
        }),
    )?;

    let trigger = control_dir.join("continue");
    let deadline = Instant::now() + Duration::from_secs(15);
    while !trigger.is_file() {
        if Instant::now() >= deadline {
            bail!("timed out waiting for visual continuation trigger");
        }
        thread::sleep(Duration::from_millis(20));
    }

    let mutation = platform_windows::execution_environment::private_visual::capture_post_mutation(
        hwnd, &baseline,
    )?;
    write_json(
        &control_dir.join("result.json"),
        &json!({
            "status": "ok",
            "pid": std::process::id(),
            "observer": "printwindow",
            "flags": "PW_RENDERFULLCONTENT",
            "window_state": "restored_visible",
            "visual_baseline_attempts": baseline.attempts,
            "baseline_changed": baseline.baseline_diff.changed_samples,
            "baseline_diff_sum": baseline.baseline_diff.diff_sum,
            "baseline_noise_masked_samples": baseline.noise_masked_samples(),
            "baseline_noise_limit": baseline_noise_limit,
            "baseline_exact": baseline.noise_masked_samples() == 0,
            "baseline_stable": baseline.noise_masked_samples() <= baseline_noise_limit,
            "baseline_sentinel_count": baseline.frame.sentinel_count,
            "baseline_sampled_pixels": baseline.frame.sampled_pixels,
            "mutation_changed": mutation.mutation_diff.changed_samples,
            "mutation_diff_sum": mutation.mutation_diff.diff_sum,
            "mutation_capture_attempts": mutation.attempts,
            "post_sentinel_count": mutation.frame.sentinel_count,
            "post_sampled_pixels": mutation.frame.sampled_pixels,
            "capture_success": mutation.frame.print_succeeded,
            "trusted": mutation.frame.trusted(),
            "fallback_used": false,
        }),
    )?;
    Ok(())
}

#[cfg(target_os = "windows")]
fn main() {
    let parsed = parse_args();
    let (hwnd, control_dir) = match parsed {
        Ok(value) => value,
        Err(_) => return,
    };
    if let Err(error) = worker(hwnd, &control_dir) {
        let receipt = json!({
            "status": "error",
            "pid": std::process::id(),
            "observer": "printwindow",
            "error": error.to_string(),
        });
        let ready = control_dir.join("ready.json");
        if !ready.exists() {
            let _ = write_json(&ready, &receipt);
        }
        let _ = write_json(&control_dir.join("result.json"), &receipt);
    }
}

#[cfg(not(target_os = "windows"))]
fn main() {}
