//! PrintWindow-only visual observer for the private-desktop lane.
//!
//! This intentionally does not fall back to WGC, desktop-region BitBlt, or
//! foreground capture. The private v1 contract is narrower: restored-visible
//! targets, same-desktop PrintWindow(PW_RENDERFULLCONTENT), a sentinel-filled
//! backing bitmap, and explicit trust/diff receipts.

use anyhow::{bail, Context, Result};
use std::thread;
use std::time::Duration;
use windows::Win32::Foundation::{HANDLE, HWND, RECT};
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GetWindowDC, ReleaseDC,
    SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, RGBQUAD,
};
use windows::Win32::Storage::Xps::{PrintWindow, PRINT_WINDOW_FLAGS};
use windows::Win32::UI::WindowsAndMessaging::{GetWindowRect, IsIconic, IsWindowVisible};

const PW_RENDERFULLCONTENT: PRINT_WINDOW_FLAGS = PRINT_WINDOW_FLAGS(2);
const SENTINEL_BGRA: [u8; 4] = [255, 0, 255, 255];
const BASELINE_SETTLE_DELAY: Duration = Duration::from_millis(500);
const BASELINE_PAIR_DELAY: Duration = Duration::from_millis(250);
const BASELINE_RETRY_DELAY: Duration = Duration::from_millis(120);
const POST_MUTATION_DELAY: Duration = Duration::from_millis(350);
const POST_MUTATION_RETRY_DELAY: Duration = Duration::from_millis(250);
const MAX_BASELINE_ATTEMPTS: usize = 6;
const MAX_POST_MUTATION_ATTEMPTS: usize = 8;
const BASELINE_NOISE_DIVISOR: u64 = 200; // 0.5% of sampled pixels.
const BASELINE_NOISE_MIN_SAMPLES: u64 = 16;
const BASELINE_NOISE_MAX_SAMPLES: u64 = 1_024;

#[derive(Clone, Debug)]
pub struct PrivateVisualFrame {
    pub png: Vec<u8>,
    pub pixels_bgra: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub print_succeeded: bool,
    pub sentinel_count: u64,
    pub sampled_pixels: u64,
}

impl PrivateVisualFrame {
    pub fn trusted(&self) -> bool {
        self.print_succeeded && self.sentinel_count == 0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PrivateVisualDiff {
    pub changed_samples: u64,
    pub diff_sum: u64,
}

#[derive(Clone, Debug)]
pub struct PrivateVisualBaseline {
    pub frame: PrivateVisualFrame,
    pub attempts: usize,
    pub baseline_diff: PrivateVisualDiff,
    noise_mask: Vec<bool>,
}

impl PrivateVisualBaseline {
    pub fn noise_masked_samples(&self) -> u64 {
        self.noise_mask.iter().filter(|masked| **masked).count() as u64
    }
}

#[derive(Clone, Debug)]
pub struct PrivateVisualMutation {
    pub frame: PrivateVisualFrame,
    pub mutation_diff: PrivateVisualDiff,
    pub attempts: usize,
}

fn restored_visible(hwnd: HWND) -> bool {
    unsafe { IsWindowVisible(hwnd).as_bool() && !IsIconic(hwnd).as_bool() }
}

fn sampled_sentinel_count(pixels: &[u8], width: u32, height: u32) -> (u64, u64) {
    let stride = width as usize * 4;
    let mut sentinel = 0u64;
    let mut sampled = 0u64;
    for y in (0..height as usize).step_by(2) {
        for x in (0..width as usize).step_by(2) {
            let offset = y * stride + x * 4;
            if offset + 2 >= pixels.len() {
                continue;
            }
            sampled += 1;
            let b = pixels[offset];
            let g = pixels[offset + 1];
            let r = pixels[offset + 2];
            if r == 255 && g == 0 && b == 255 {
                sentinel += 1;
            }
        }
    }
    (sentinel, sampled)
}

fn sampled_pixel_capacity(width: u32, height: u32) -> usize {
    (width as usize).div_ceil(2) * (height as usize).div_ceil(2)
}

pub fn baseline_noise_limit(sampled_pixels: u64) -> u64 {
    if sampled_pixels == 0 {
        return 0;
    }
    let proportional = sampled_pixels.div_ceil(BASELINE_NOISE_DIVISOR);
    proportional
        .max(BASELINE_NOISE_MIN_SAMPLES)
        .min(BASELINE_NOISE_MAX_SAMPLES)
        .min(sampled_pixels)
}

fn diff_frames_with_mask(
    a: &PrivateVisualFrame,
    b: &PrivateVisualFrame,
    ignored_samples: Option<&[bool]>,
    mut observed_changes: Option<&mut [bool]>,
) -> Result<PrivateVisualDiff> {
    if a.width != b.width || a.height != b.height {
        bail!(
            "private visual frame size changed: {}x{} -> {}x{}",
            a.width,
            a.height,
            b.width,
            b.height
        );
    }

    let capacity = sampled_pixel_capacity(a.width, a.height);
    if ignored_samples.is_some_and(|mask| mask.len() != capacity)
        || observed_changes
            .as_ref()
            .is_some_and(|mask| mask.len() != capacity)
    {
        bail!("private visual sample mask does not match frame dimensions");
    }

    let stride = a.width as usize * 4;
    let mut changed = 0u64;
    let mut sum = 0u64;
    let mut sample_index = 0usize;
    for y in (0..a.height as usize).step_by(2) {
        for x in (0..a.width as usize).step_by(2) {
            let offset = y * stride + x * 4;
            if offset + 3 >= a.pixels_bgra.len() || offset + 3 >= b.pixels_bgra.len() {
                sample_index += 1;
                continue;
            }
            let mut pixel_diff = 0u64;
            for channel in 0..4 {
                pixel_diff += a.pixels_bgra[offset + channel]
                    .abs_diff(b.pixels_bgra[offset + channel]) as u64;
            }
            if pixel_diff != 0 {
                if let Some(mask) = observed_changes.as_deref_mut() {
                    mask[sample_index] = true;
                }
                if !ignored_samples.is_some_and(|mask| mask[sample_index]) {
                    changed += 1;
                    sum += pixel_diff;
                }
            }
            sample_index += 1;
        }
    }
    Ok(PrivateVisualDiff {
        changed_samples: changed,
        diff_sum: sum,
    })
}

pub fn diff_frames(a: &PrivateVisualFrame, b: &PrivateVisualFrame) -> Result<PrivateVisualDiff> {
    diff_frames_with_mask(a, b, None, None)
}

pub fn capture(hwnd_raw: u64) -> Result<PrivateVisualFrame> {
    let hwnd = HWND(hwnd_raw as *mut _);
    if !restored_visible(hwnd) {
        bail!("private visual observer requires a restored-visible target; hwnd=0x{hwnd_raw:x}");
    }

    let mut rect = RECT::default();
    unsafe { GetWindowRect(hwnd, &mut rect) }
        .context("GetWindowRect failed for private visual observer")?;
    let width_i = rect.right - rect.left;
    let height_i = rect.bottom - rect.top;
    if width_i <= 0 || height_i <= 0 {
        bail!(
            "private visual observer got invalid window bounds {}x{}",
            width_i,
            height_i
        );
    }

    unsafe {
        let screen_dc = GetWindowDC(hwnd);
        if screen_dc.0.is_null() {
            bail!("GetWindowDC returned null for private visual observer");
        }
        let mem_dc = CreateCompatibleDC(screen_dc);
        if mem_dc.0.is_null() {
            let _ = ReleaseDC(hwnd, screen_dc);
            bail!("CreateCompatibleDC returned null for private visual observer");
        }

        // System.Drawing's Format32bppArgb Bitmap (used by the accepted B3
        // canary) is DIB-backed. Use the same kind of backing store rather
        // than a device-dependent compatible bitmap so PrintWindow writes
        // into directly readable 32-bit BGRA memory.
        let bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width_i,
                biHeight: -height_i,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                biSizeImage: (width_i * height_i * 4) as u32,
                ..Default::default()
            },
            bmiColors: [RGBQUAD::default(); 1],
        };
        let mut bits = std::ptr::null_mut();
        let bitmap = match CreateDIBSection(
            screen_dc,
            &bmi,
            DIB_RGB_COLORS,
            &mut bits,
            HANDLE::default(),
            0,
        ) {
            Ok(bitmap) => bitmap,
            Err(error) => {
                let _ = DeleteDC(mem_dc);
                let _ = ReleaseDC(hwnd, screen_dc);
                return Err(error).context("CreateDIBSection failed for private visual observer");
            }
        };
        if bits.is_null() {
            let _ = DeleteObject(bitmap);
            let _ = DeleteDC(mem_dc);
            let _ = ReleaseDC(hwnd, screen_dc);
            bail!("CreateDIBSection returned a null pixel pointer");
        }
        let old_bitmap = SelectObject(mem_dc, bitmap);

        let byte_len = (width_i * height_i * 4) as usize;
        let backing = std::slice::from_raw_parts_mut(bits as *mut u8, byte_len);
        for pixel in backing.chunks_exact_mut(4) {
            pixel.copy_from_slice(&SENTINEL_BGRA);
        }

        let print_succeeded = PrintWindow(hwnd, mem_dc, PW_RENDERFULLCONTENT).as_bool();
        // Copy while the DIB section is still alive; deleting the HBITMAP
        // invalidates the backing pointer.
        let pixels = backing.to_vec();

        let _ = SelectObject(mem_dc, old_bitmap);
        let _ = DeleteObject(bitmap);
        let _ = DeleteDC(mem_dc);
        let _ = ReleaseDC(hwnd, screen_dc);

        let width = width_i as u32;
        let height = height_i as u32;
        let (sentinel_count, sampled_pixels) = sampled_sentinel_count(&pixels, width, height);
        let png = cua_driver_core::image_utils::encode_bgra_to_png(&pixels, width, height)?;

        Ok(PrivateVisualFrame {
            png,
            pixels_bgra: pixels,
            width,
            height,
            print_succeeded,
            sentinel_count,
            sampled_pixels,
        })
    }
}

pub fn capture_trusted(hwnd: u64) -> Result<PrivateVisualFrame> {
    let frame = capture(hwnd)?;
    if !frame.print_succeeded {
        bail!("PrintWindow(PW_RENDERFULLCONTENT) returned false for private visual observer");
    }
    if frame.sentinel_count != 0 {
        bail!(
            "private visual observer rejected sentinel-contaminated frame: {}/{} sampled pixels remain sentinel",
            frame.sentinel_count,
            frame.sampled_pixels
        );
    }
    Ok(frame)
}

pub fn settle_baseline(hwnd: u64) -> Result<PrivateVisualBaseline> {
    thread::sleep(BASELINE_SETTLE_DELAY);
    let first = capture_trusted(hwnd)?;
    let mut noise_mask = vec![false; sampled_pixel_capacity(first.width, first.height)];
    let mut reference = first;
    let mut max_diff_sum = 0u64;

    for attempt in 1..=MAX_BASELINE_ATTEMPTS {
        thread::sleep(BASELINE_PAIR_DELAY);
        let next = capture_trusted(hwnd)?;
        let pair_diff =
            diff_frames_with_mask(&reference, &next, None, Some(noise_mask.as_mut_slice()))?;
        max_diff_sum = max_diff_sum.max(pair_diff.diff_sum);
        if pair_diff.changed_samples == 0 {
            return Ok(PrivateVisualBaseline {
                frame: next,
                attempts: attempt,
                baseline_diff: PrivateVisualDiff {
                    changed_samples: noise_mask.iter().filter(|masked| **masked).count() as u64,
                    diff_sum: max_diff_sum,
                },
                noise_mask,
            });
        }
        reference = next;
        if attempt != MAX_BASELINE_ATTEMPTS {
            thread::sleep(BASELINE_RETRY_DELAY);
        }
    }

    let noise_samples = noise_mask.iter().filter(|masked| **masked).count() as u64;
    let noise_limit = baseline_noise_limit(reference.sampled_pixels);
    if noise_samples <= noise_limit {
        return Ok(PrivateVisualBaseline {
            frame: reference,
            attempts: MAX_BASELINE_ATTEMPTS,
            baseline_diff: PrivateVisualDiff {
                changed_samples: noise_samples,
                diff_sum: max_diff_sum,
            },
            noise_mask,
        });
    }

    bail!(
        "private visual baseline exceeded bounded noise envelope after {MAX_BASELINE_ATTEMPTS} attempts; masked_samples={noise_samples}, noise_limit={noise_limit}, max_pair_diff_sum={max_diff_sum}"
    )
}

pub fn capture_post_mutation(
    hwnd: u64,
    baseline: &PrivateVisualBaseline,
) -> Result<PrivateVisualMutation> {
    thread::sleep(POST_MUTATION_DELAY);
    for attempt in 1..=MAX_POST_MUTATION_ATTEMPTS {
        let frame = capture_trusted(hwnd)?;
        let mutation_diff =
            diff_frames_with_mask(&baseline.frame, &frame, Some(&baseline.noise_mask), None)?;
        if mutation_diff.changed_samples != 0 || attempt == MAX_POST_MUTATION_ATTEMPTS {
            return Ok(PrivateVisualMutation {
                frame,
                mutation_diff,
                attempts: attempt,
            });
        }
        thread::sleep(POST_MUTATION_RETRY_DELAY);
    }
    unreachable!("post-mutation capture loop must return")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sampled_sentinel_detects_magenta() {
        let pixels = vec![255, 0, 255, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        let (sentinel, sampled) = sampled_sentinel_count(&pixels, 2, 2);
        assert_eq!(sampled, 1);
        assert_eq!(sentinel, 1);
    }

    #[test]
    fn diff_counts_changed_sampled_pixels() {
        let base = PrivateVisualFrame {
            png: Vec::new(),
            pixels_bgra: vec![0; 4 * 4 * 4],
            width: 4,
            height: 4,
            print_succeeded: true,
            sentinel_count: 0,
            sampled_pixels: 4,
        };
        let mut changed = base.clone();
        changed.pixels_bgra[0] = 10;
        let diff = diff_frames(&base, &changed).unwrap();
        assert_eq!(diff.changed_samples, 1);
        assert_eq!(diff.diff_sum, 10);
    }

    #[test]
    fn bounded_baseline_noise_is_small_and_capped() {
        assert_eq!(baseline_noise_limit(0), 0);
        assert_eq!(baseline_noise_limit(4), 4);
        assert_eq!(baseline_noise_limit(57_600), 288);
        assert_eq!(baseline_noise_limit(1_000_000), 1_024);
    }

    #[test]
    fn masked_baseline_noise_does_not_become_mutation_evidence() {
        let base = PrivateVisualFrame {
            png: Vec::new(),
            pixels_bgra: vec![0; 4 * 4 * 4],
            width: 4,
            height: 4,
            print_succeeded: true,
            sentinel_count: 0,
            sampled_pixels: 4,
        };
        let mut changed = base.clone();
        changed.pixels_bgra[0] = 10;
        changed.pixels_bgra[8] = 20;
        let mask = [true, false, false, false];
        let diff = diff_frames_with_mask(&base, &changed, Some(&mask), None).unwrap();
        assert_eq!(diff.changed_samples, 1);
        assert_eq!(diff.diff_sum, 20);
    }
}
