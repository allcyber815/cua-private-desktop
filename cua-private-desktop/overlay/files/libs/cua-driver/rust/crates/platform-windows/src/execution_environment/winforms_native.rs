//! Strict WinForms native semantic adapters for the private-desktop lane.
//!
//! This module intentionally exposes only the B3-verified semantic operations.
//! It is not a generic arbitrary-window-message surface.

use crate::execution_environment::PrivateSemanticAction;
use anyhow::{anyhow, bail, Context};
use std::thread;
use std::time::Duration;
use windows::core::Interface;
use windows::Win32::Foundation::{BOOL, HWND, LPARAM, WPARAM};
use windows::Win32::UI::Accessibility::{
    IUIAutomationElement, IUIAutomationSelectionItemPattern, IUIAutomationTogglePattern,
    UIA_SelectionItemPatternId, UIA_TogglePatternId,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumChildWindows, GetClassNameW, GetDlgCtrlID, GetParent, SendMessageW,
};

const BM_CLICK: u32 = 0x00F5;
const WM_COMMAND: u32 = 0x0111;
const WM_SETTEXT: u32 = 0x000C;
const WM_GETTEXT: u32 = 0x000D;
const WM_GETTEXTLENGTH: u32 = 0x000E;
const LB_GETCURSEL: u32 = 0x0188;
const LB_SETCURSEL: u32 = 0x0186;
const LB_FINDSTRINGEXACT: u32 = 0x01A2;
const CB_GETCURSEL: u32 = 0x0147;
const CB_SETCURSEL: u32 = 0x014E;
const CB_FINDSTRINGEXACT: u32 = 0x0158;
const LBN_SELCHANGE: usize = 1;
const CBN_SELCHANGE: usize = 1;
const POLL_ATTEMPTS: usize = 20;
const POLL_INTERVAL: Duration = Duration::from_millis(20);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WinFormsNativeReceipt {
    pub actuator: &'static str,
    pub semantic_verified: Option<bool>,
    pub postcondition_readback: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum NativeControlClass {
    Button,
    Edit,
    ListBox,
    ComboBox,
}

fn classify_native_control_class(class_name: &str) -> Option<NativeControlClass> {
    let upper = class_name.to_ascii_uppercase();
    if upper.contains("BUTTON") {
        Some(NativeControlClass::Button)
    } else if upper.contains("EDIT") {
        Some(NativeControlClass::Edit)
    } else if upper.contains("LISTBOX") {
        Some(NativeControlClass::ListBox)
    } else if upper.contains("COMBOBOX") {
        Some(NativeControlClass::ComboBox)
    } else {
        None
    }
}

fn window_class(hwnd: HWND) -> anyhow::Result<String> {
    let mut buf = [0u16; 256];
    let len = unsafe { GetClassNameW(hwnd, &mut buf) };
    if len <= 0 {
        bail!("GetClassNameW failed for native control HWND");
    }
    Ok(String::from_utf16_lossy(&buf[..len as usize]))
}

fn native_hwnd(element: &IUIAutomationElement) -> anyhow::Result<HWND> {
    let hwnd = unsafe { element.CurrentNativeWindowHandle() }
        .context("UIA element did not expose CurrentNativeWindowHandle")?;
    if hwnd.0.is_null() {
        bail!("UIA element exposed a null NativeWindowHandle");
    }
    Ok(hwnd)
}

fn selection_messages(class: NativeControlClass) -> Option<(u32, u32, u32, usize, &'static str)> {
    match class {
        NativeControlClass::ListBox => Some((
            LB_FINDSTRINGEXACT,
            LB_SETCURSEL,
            LB_GETCURSEL,
            LBN_SELCHANGE,
            "lb_setcursel+wm_command",
        )),
        NativeControlClass::ComboBox => Some((
            CB_FINDSTRINGEXACT,
            CB_SETCURSEL,
            CB_GETCURSEL,
            CBN_SELCHANGE,
            "cb_setcursel+wm_command",
        )),
        NativeControlClass::Button | NativeControlClass::Edit => None,
    }
}

unsafe extern "system" fn collect_child_window(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let children = unsafe { &mut *(lparam.0 as *mut Vec<HWND>) };
    children.push(hwnd);
    BOOL(1)
}

fn exact_selection_match(hwnd: HWND, wide: &[u16]) -> Option<(NativeControlClass, isize)> {
    let class_name = window_class(hwnd).ok()?;
    let class = classify_native_control_class(&class_name)?;
    let (find_msg, _, _, _, _) = selection_messages(class)?;
    let idx = send(hwnd, find_msg, usize::MAX, wide.as_ptr() as isize);
    (idx >= 0).then_some((class, idx))
}

fn resolve_selection_target(
    pattern: &IUIAutomationSelectionItemPattern,
    top_hwnd: HWND,
    item_name: &str,
    wide: &[u16],
) -> anyhow::Result<(HWND, NativeControlClass, isize)> {
    if let Ok(container) = unsafe { pattern.CurrentSelectionContainer() } {
        if let Ok(hwnd) = unsafe { container.CurrentNativeWindowHandle() } {
            if !hwnd.0.is_null() {
                if let Some((class, idx)) = exact_selection_match(hwnd, wide) {
                    return Ok((hwnd, class, idx));
                }
            }
        }
    }

    let mut children = Vec::<HWND>::new();
    unsafe {
        let _ = EnumChildWindows(
            top_hwnd,
            Some(collect_child_window),
            LPARAM((&mut children as *mut Vec<HWND>) as isize),
        );
    }

    let mut matches = children.into_iter().filter_map(|hwnd| {
        exact_selection_match(hwnd, wide).map(|(class, idx)| (hwnd, class, idx))
    });

    let first = matches.next().ok_or_else(|| {
        anyhow!(
            "WinForms native Select found no LISTBOX/COMBOBOX under the exact target window containing item {item_name:?}"
        )
    })?;
    if matches.next().is_some() {
        bail!(
            "WinForms native Select found multiple native selection controls containing exact item {item_name:?}; refusing ambiguous mutation"
        );
    }
    Ok(first)
}

fn send(hwnd: HWND, msg: u32, wparam: usize, lparam: isize) -> isize {
    unsafe { SendMessageW(hwnd, msg, WPARAM(wparam), LPARAM(lparam)).0 }
}

fn read_window_text(hwnd: HWND) -> anyhow::Result<String> {
    // GetWindowTextW does not read another process's Edit control text; system
    // WM_GETTEXT messages are marshalled across the process boundary.
    let len = send(hwnd, WM_GETTEXTLENGTH, 0, 0);
    if len < 0 {
        bail!("WM_GETTEXTLENGTH failed for WinForms Edit control");
    }
    let mut buf = vec![0u16; len as usize + 1];
    let copied = send(hwnd, WM_GETTEXT, buf.len(), buf.as_mut_ptr() as isize);
    if copied < 0 {
        bail!("WM_GETTEXT failed for WinForms Edit control");
    }
    Ok(String::from_utf16_lossy(&buf[..copied as usize]))
}

pub fn set_value(
    element: &IUIAutomationElement,
    value: &str,
) -> anyhow::Result<WinFormsNativeReceipt> {
    let hwnd = native_hwnd(element)?;
    let class_name = window_class(hwnd)?;
    if classify_native_control_class(&class_name) != Some(NativeControlClass::Edit) {
        bail!("WinForms native Value requires an EDIT control; got {class_name:?}");
    }

    let wide: Vec<u16> = value.encode_utf16().chain(std::iter::once(0)).collect();
    let accepted = send(hwnd, WM_SETTEXT, 0, wide.as_ptr() as isize);
    if accepted == 0 {
        bail!("WinForms native Value WM_SETTEXT was rejected");
    }
    let readback = read_window_text(hwnd)?;
    Ok(WinFormsNativeReceipt {
        actuator: "wm_settext",
        semantic_verified: Some(readback == value),
        postcondition_readback: readback,
    })
}

fn toggle(element: &IUIAutomationElement) -> anyhow::Result<WinFormsNativeReceipt> {
    let hwnd = native_hwnd(element)?;
    let class_name = window_class(hwnd)?;
    if classify_native_control_class(&class_name) != Some(NativeControlClass::Button) {
        bail!("WinForms native Toggle requires a BUTTON control; got {class_name:?}");
    }

    let pattern = unsafe { element.GetCurrentPattern(UIA_TogglePatternId) }
        .and_then(|value| value.cast::<IUIAutomationTogglePattern>())
        .context("TogglePattern unavailable for WinForms native Toggle")?;
    let before = unsafe { pattern.CurrentToggleState() }
        .context("could not read pre-toggle UIA ToggleState")?;

    let _ = send(hwnd, BM_CLICK, 0, 0);

    for _ in 0..POLL_ATTEMPTS {
        if let Ok(after) = unsafe { pattern.CurrentToggleState() } {
            if after != before {
                return Ok(WinFormsNativeReceipt {
                    actuator: "bm_click",
                    semantic_verified: Some(true),
                    postcondition_readback: format!("UIA_ToggleState:{before:?}->{after:?}"),
                });
            }
        }
        thread::sleep(POLL_INTERVAL);
    }

    let after = unsafe { pattern.CurrentToggleState() }
        .map(|value| format!("{value:?}"))
        .unwrap_or_else(|_| "<unreadable>".to_string());
    Ok(WinFormsNativeReceipt {
        actuator: "bm_click",
        semantic_verified: Some(false),
        postcondition_readback: format!("UIA_ToggleState remained {after}"),
    })
}

fn invoke(element: &IUIAutomationElement) -> anyhow::Result<WinFormsNativeReceipt> {
    let hwnd = native_hwnd(element)?;
    let class_name = window_class(hwnd)?;
    if classify_native_control_class(&class_name) != Some(NativeControlClass::Button) {
        bail!("WinForms native Invoke requires a BUTTON control; got {class_name:?}");
    }

    let _ = send(hwnd, BM_CLICK, 0, 0);
    Ok(WinFormsNativeReceipt {
        actuator: "bm_click",
        semantic_verified: None,
        postcondition_readback:
            "generic WinForms Invoke has no action-local semantic readback contract".into(),
    })
}

fn select(element: &IUIAutomationElement, top_hwnd: HWND) -> anyhow::Result<WinFormsNativeReceipt> {
    let pattern = unsafe { element.GetCurrentPattern(UIA_SelectionItemPatternId) }
        .and_then(|value| value.cast::<IUIAutomationSelectionItemPattern>())
        .context("SelectionItemPattern unavailable for WinForms native Select")?;
    let item_name = unsafe { element.CurrentName() }
        .context("selected UIA item has no readable Name")?
        .to_string();
    if item_name.is_empty() {
        bail!("WinForms native Select requires a non-empty UIA item name");
    }

    let wide: Vec<u16> = item_name.encode_utf16().chain(std::iter::once(0)).collect();
    let (hwnd, class, idx) = resolve_selection_target(&pattern, top_hwnd, &item_name, &wide)?;
    let (_, set_msg, get_msg, notify, actuator) =
        selection_messages(class).expect("selection target class must be selectable");

    let selected = send(hwnd, set_msg, idx as usize, 0);
    if selected < 0 {
        bail!("WinForms native Select rejected index {idx}");
    }

    let parent = unsafe { GetParent(hwnd) }.context("WinForms selection control has no parent")?;
    let ctrl_id = unsafe { GetDlgCtrlID(hwnd) };
    let packed = (notify << 16) | (ctrl_id as usize & 0xffff);
    let _ = send(parent, WM_COMMAND, packed, hwnd.0 as isize);

    for _ in 0..POLL_ATTEMPTS {
        let native_idx = send(hwnd, get_msg, 0, 0);
        let uia_selected = unsafe { pattern.CurrentIsSelected() }
            .ok()
            .map(|value| value.as_bool())
            .unwrap_or(false);
        if native_idx == idx && uia_selected {
            return Ok(WinFormsNativeReceipt {
                actuator,
                semantic_verified: Some(true),
                postcondition_readback: format!(
                    "selected_index={native_idx};uia_selected=true;item={item_name}"
                ),
            });
        }
        thread::sleep(POLL_INTERVAL);
    }

    let native_idx = send(hwnd, get_msg, 0, 0);
    let uia_selected = unsafe { pattern.CurrentIsSelected() }
        .ok()
        .map(|value| value.as_bool())
        .unwrap_or(false);
    Ok(WinFormsNativeReceipt {
        actuator,
        semantic_verified: Some(false),
        postcondition_readback: format!(
            "selected_index={native_idx};expected_index={idx};uia_selected={uia_selected};item={item_name}"
        ),
    })
}

/// Execute only a verified, class-constrained WinForms native semantic action.
///
/// Toggle/Select return an action-local semantic readback. Invoke uses the
/// B3-verified BM_CLICK transport but has no generic postcondition, so callers
/// must publish it as delivered-but-unverifiable rather than false confirmation.
pub fn execute(
    element: &IUIAutomationElement,
    action: PrivateSemanticAction,
    top_hwnd: u64,
) -> anyhow::Result<WinFormsNativeReceipt> {
    let top_hwnd = HWND(top_hwnd as *mut _);
    match action {
        PrivateSemanticAction::Toggle => toggle(element),
        PrivateSemanticAction::Select => select(element, top_hwnd),
        PrivateSemanticAction::Invoke => invoke(element),
        PrivateSemanticAction::Value => {
            bail!("WinForms Value requires execute_value with an explicit value payload")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_control_class_is_constrained() {
        assert_eq!(
            classify_native_control_class("WindowsForms10.BUTTON.app.0.141b42a_r8_ad1"),
            Some(NativeControlClass::Button)
        );
        assert_eq!(
            classify_native_control_class("WindowsForms10.LISTBOX.app.0.141b42a_r8_ad1"),
            Some(NativeControlClass::ListBox)
        );
        assert_eq!(
            classify_native_control_class("WindowsForms10.COMBOBOX.app.0.141b42a_r8_ad1"),
            Some(NativeControlClass::ComboBox)
        );
        assert_eq!(
            classify_native_control_class("WindowsForms10.EDIT.app.0.141b42a_r8_ad1"),
            Some(NativeControlClass::Edit)
        );
        assert_eq!(
            classify_native_control_class("Edit"),
            Some(NativeControlClass::Edit)
        );
    }
}
