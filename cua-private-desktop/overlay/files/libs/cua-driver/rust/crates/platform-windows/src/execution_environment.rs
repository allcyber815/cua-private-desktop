//! Private-desktop execution policy primitives.
//!
//! Phase 0 freezes the verified B3 provider/action/state contract in pure Rust.
//! Phase 1 adds a Windows-only ownership core under `win32_private` while keeping
//! policy and mechanism separate.

#[cfg(target_os = "windows")]
#[path = "execution_environment/win32_private.rs"]
pub mod win32_private;

#[cfg(target_os = "windows")]
#[path = "execution_environment/runtime.rs"]
pub mod runtime;

#[cfg(target_os = "windows")]
#[path = "execution_environment/winforms_native.rs"]
pub mod winforms_native;

#[cfg(target_os = "windows")]
#[path = "execution_environment/private_interference.rs"]
pub mod private_interference;

#[cfg(target_os = "windows")]
#[path = "execution_environment/private_visual.rs"]
pub mod private_visual;

#[cfg(target_os = "windows")]
#[path = "execution_environment/private_visual_worker_client.rs"]
pub mod private_visual_worker_client;

#[cfg(target_os = "windows")]
pub use runtime::{
    ExecutionEnvironmentRegistry, PrivateDesktopRuntime as PrivateEnvironmentHandle,
};

#[cfg(all(target_os = "windows", test))]
use std::sync::Arc;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IsolationMode {
    SharedUserDesktop,
    PrivateDesktop,
}

impl IsolationMode {
    pub const fn wire_value(self) -> &'static str {
        match self {
            Self::SharedUserDesktop => "shared_user_desktop",
            Self::PrivateDesktop => "private_desktop",
        }
    }
}

impl Default for IsolationMode {
    fn default() -> Self {
        Self::SharedUserDesktop
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PrivateFramework {
    Wpf,
    WinUi3Unpackaged,
    Chromium,
    Electron,
    WebView2,
    XamlIsland,
    Qt611Widgets,
    WinForms,
    Tk,
    PackagedWinUiUwp,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PrivateSemanticAction {
    Value,
    Invoke,
    Toggle,
    Select,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PrivateActionTransport {
    UiaPattern,
    Win32Message,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PrivateWindowState {
    RestoredVisible,
    Minimized,
    Hidden,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PrivateStateRule {
    RestoredVisible,
    VisibleOrMinimized,
    AnyVisibility,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PrivateLaunchClass {
    DirectExecutable,
    Shell,
    Url,
    PackagedActivation,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PrivatePolicyDecision {
    pub transport: Option<PrivateActionTransport>,
    pub state_rule: Option<PrivateStateRule>,
    pub reason: &'static str,
}

impl PrivatePolicyDecision {
    pub const fn supported(
        transport: PrivateActionTransport,
        state_rule: PrivateStateRule,
        reason: &'static str,
    ) -> Self {
        Self {
            transport: Some(transport),
            state_rule: Some(state_rule),
            reason,
        }
    }

    pub const fn blocked(reason: &'static str) -> Self {
        Self {
            transport: None,
            state_rule: None,
            reason,
        }
    }

    pub const fn is_supported(self) -> bool {
        self.transport.is_some()
    }
}

pub const fn private_launch_allowed(class: PrivateLaunchClass) -> bool {
    matches!(class, PrivateLaunchClass::DirectExecutable)
}

pub const fn private_window_state_allowed(
    rule: PrivateStateRule,
    state: PrivateWindowState,
) -> bool {
    match rule {
        PrivateStateRule::RestoredVisible => {
            matches!(state, PrivateWindowState::RestoredVisible)
        }
        PrivateStateRule::VisibleOrMinimized => matches!(
            state,
            PrivateWindowState::RestoredVisible | PrivateWindowState::Minimized
        ),
        PrivateStateRule::AnyVisibility => true,
    }
}

/// Return the strict B3 allowlist decision for a framework/action pair.
///
/// This is deliberately exhaustive and fail-closed. A future provider or action
/// must be added here together with evidence; callers must not infer support
/// merely because UIA exposes a pattern.
pub const fn resolve_private_policy(
    framework: PrivateFramework,
    action: PrivateSemanticAction,
) -> PrivatePolicyDecision {
    use PrivateActionTransport::{UiaPattern, Win32Message};
    use PrivateFramework::*;
    use PrivateSemanticAction::*;
    use PrivateStateRule::{AnyVisibility, RestoredVisible, VisibleOrMinimized};

    match framework {
        Wpf => PrivatePolicyDecision::supported(
            UiaPattern,
            VisibleOrMinimized,
            "verified WPF private-desktop semantic cell",
        ),
        WinUi3Unpackaged => PrivatePolicyDecision::supported(
            UiaPattern,
            AnyVisibility,
            "verified unpackaged WinUI 3 private-desktop semantic cell",
        ),
        Chromium | Electron | WebView2 | XamlIsland => PrivatePolicyDecision::supported(
            UiaPattern,
            RestoredVisible,
            "verified provider/action cell; untested states fail closed",
        ),
        Qt611Widgets => match action {
            Select => PrivatePolicyDecision::blocked(
                "Qt QComboBox selection event fidelity is a verified semantic gap",
            ),
            Value | Invoke | Toggle => PrivatePolicyDecision::supported(
                UiaPattern,
                RestoredVisible,
                "verified Qt 6.11 Widgets private-desktop semantic cell",
            ),
        },
        WinForms => match action {
            Value => PrivatePolicyDecision::supported(
                Win32Message,
                VisibleOrMinimized,
                "verified WinForms strict-background native Value adapter cell",
            ),
            Invoke | Toggle | Select => PrivatePolicyDecision::supported(
                Win32Message,
                VisibleOrMinimized,
                "verified WinForms strict-background native adapter cell",
            ),
        },
        Tk => PrivatePolicyDecision::blocked(
            "Tk representative has no verified generic semantic actuator",
        ),
        PackagedWinUiUwp => PrivatePolicyDecision::blocked(
            "packaged WinUI/UWP activation remains unverified for the private lane",
        ),
    }
}

pub fn classify_private_framework(
    exe_basename: &str,
    class_name: &str,
    has_chromium_descendant: bool,
    has_xaml_island_descendant: bool,
) -> Option<PrivateFramework> {
    let exe = exe_basename.trim().to_ascii_lowercase();
    let class = class_name.trim();

    if matches!(
        exe.as_str(),
        "applicationframehost.exe"
            | "calculatorapp.exe"
            | "calc.exe"
            | "photos.exe"
            | "systemsettings.exe"
    ) || matches!(
        class,
        "ApplicationFrameWindow" | "Windows.UI.Core.CoreWindow"
    ) {
        return Some(PrivateFramework::PackagedWinUiUwp);
    }

    if class.starts_with("HwndWrapper") {
        return Some(PrivateFramework::Wpf);
    }
    if class == "WinUIDesktopWin32WindowClass" {
        return Some(PrivateFramework::WinUi3Unpackaged);
    }
    if class.starts_with("Chrome_WidgetWin_") || class.starts_with("CefBrowser") {
        return Some(if exe == "electron.exe" {
            PrivateFramework::Electron
        } else {
            PrivateFramework::Chromium
        });
    }
    if class.starts_with("Qt") {
        return Some(PrivateFramework::Qt611Widgets);
    }
    if class == "TkTopLevel" || class.starts_with("TkTopLevel.") {
        return Some(PrivateFramework::Tk);
    }
    if class.starts_with("WindowsForms10.") {
        if has_xaml_island_descendant || exe.contains("xamlisland") || exe.contains("xaml-island") {
            return Some(PrivateFramework::XamlIsland);
        }
        if has_chromium_descendant || exe.contains("webview2") {
            return Some(PrivateFramework::WebView2);
        }
        return Some(PrivateFramework::WinForms);
    }

    None
}

pub const fn observed_private_window_state(
    is_on_screen: bool,
    minimized: bool,
) -> PrivateWindowState {
    if minimized {
        PrivateWindowState::Minimized
    } else if is_on_screen {
        PrivateWindowState::RestoredVisible
    } else {
        PrivateWindowState::Hidden
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ACTIONS: [PrivateSemanticAction; 4] = [
        PrivateSemanticAction::Value,
        PrivateSemanticAction::Invoke,
        PrivateSemanticAction::Toggle,
        PrivateSemanticAction::Select,
    ];

    #[cfg(target_os = "windows")]
    #[test]
    fn private_registry_reuses_session_and_removes_it() {
        let registry = ExecutionEnvironmentRegistry::new();
        let first = registry.get_or_create_private("session-a").unwrap();
        let second = registry.get_or_create_private("session-a").unwrap();
        assert!(Arc::ptr_eq(&first, &second));
        assert_eq!(registry.private_count().unwrap(), 1);
        assert!(registry.remove_private("session-a").unwrap());
        assert_eq!(registry.private_count().unwrap(), 0);
        assert!(!registry.remove_private("session-a").unwrap());
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn shared_session_never_allocates_a_private_runtime() {
        let registry = ExecutionEnvironmentRegistry::new();
        registry.claim_shared("session-shared").unwrap();

        assert_eq!(
            registry.isolation_for_session("session-shared"),
            Some(IsolationMode::SharedUserDesktop)
        );
        assert!(registry.private_for_session("session-shared").is_none());
        assert_eq!(registry.private_count().unwrap(), 0);

        let error = match registry.get_or_create_private("session-shared") {
            Ok(_) => panic!("shared session must not switch to private_desktop"),
            Err(error) => error,
        };
        assert!(error.contains("already bound to shared_user_desktop"));
        assert_eq!(registry.private_count().unwrap(), 0);
    }

    #[test]
    fn private_framework_classification_is_fail_closed_and_specific() {
        assert_eq!(
            classify_private_framework("fixture.exe", "HwndWrapper[fixture;;abc]", false, false),
            Some(PrivateFramework::Wpf)
        );
        assert_eq!(
            classify_private_framework(
                "winui-private-fixture.exe",
                "WinUIDesktopWin32WindowClass",
                false,
                false,
            ),
            Some(PrivateFramework::WinUi3Unpackaged)
        );
        assert_eq!(
            classify_private_framework("electron.exe", "Chrome_WidgetWin_1", false, false),
            Some(PrivateFramework::Electron)
        );
        assert_eq!(
            classify_private_framework("chrome.exe", "Chrome_WidgetWin_1", false, false),
            Some(PrivateFramework::Chromium)
        );
        assert_eq!(
            classify_private_framework(
                "WebView2PrivateFixture.exe",
                "WindowsForms10.Window.8.app.0.2bf8098_r6_ad1",
                true,
                false,
            ),
            Some(PrivateFramework::WebView2)
        );
        assert_eq!(
            classify_private_framework(
                "XamlIslandPrivateFixture.exe",
                "WindowsForms10.Window.8.app.0.2bf8098_r6_ad1",
                false,
                true,
            ),
            Some(PrivateFramework::XamlIsland)
        );
        assert_eq!(
            classify_private_framework("qt-private.exe", "Qt663QWindowIcon", false, false),
            Some(PrivateFramework::Qt611Widgets)
        );
        assert_eq!(
            classify_private_framework("pythonw.exe", "TkTopLevel", false, false),
            Some(PrivateFramework::Tk)
        );
        assert_eq!(
            classify_private_framework(
                "BgFixtureV6.exe",
                "WindowsForms10.Window.8.app.0.2bf8098_r6_ad1",
                false,
                false,
            ),
            Some(PrivateFramework::WinForms)
        );
        assert_eq!(
            classify_private_framework("unknown.exe", "UnknownWindowClass", false, false),
            None
        );
    }

    #[test]
    fn observed_window_state_maps_without_implicit_restoration() {
        assert_eq!(
            observed_private_window_state(true, false),
            PrivateWindowState::RestoredVisible
        );
        assert_eq!(
            observed_private_window_state(false, true),
            PrivateWindowState::Minimized
        );
        assert_eq!(
            observed_private_window_state(false, false),
            PrivateWindowState::Hidden
        );
    }

    #[test]
    fn isolation_mode_defaults_to_shared_user_desktop() {
        assert_eq!(IsolationMode::default(), IsolationMode::SharedUserDesktop);
        assert_eq!(
            IsolationMode::SharedUserDesktop.wire_value(),
            "shared_user_desktop"
        );
        assert_eq!(
            IsolationMode::PrivateDesktop.wire_value(),
            "private_desktop"
        );
    }

    #[test]
    fn direct_executable_is_the_only_initial_private_launch_class() {
        assert!(private_launch_allowed(PrivateLaunchClass::DirectExecutable));
        assert!(!private_launch_allowed(PrivateLaunchClass::Shell));
        assert!(!private_launch_allowed(PrivateLaunchClass::Url));
        assert!(!private_launch_allowed(
            PrivateLaunchClass::PackagedActivation
        ));
    }

    #[test]
    fn wpf_and_unpackaged_winui_allow_all_semantic_actions() {
        for action in ACTIONS {
            let wpf = resolve_private_policy(PrivateFramework::Wpf, action);
            assert_eq!(wpf.transport, Some(PrivateActionTransport::UiaPattern));
            assert_eq!(wpf.state_rule, Some(PrivateStateRule::VisibleOrMinimized));

            let winui = resolve_private_policy(PrivateFramework::WinUi3Unpackaged, action);
            assert_eq!(winui.transport, Some(PrivateActionTransport::UiaPattern));
            assert_eq!(winui.state_rule, Some(PrivateStateRule::AnyVisibility));
        }
    }

    #[test]
    fn browser_like_providers_are_restored_visible_only() {
        for framework in [
            PrivateFramework::Chromium,
            PrivateFramework::Electron,
            PrivateFramework::WebView2,
            PrivateFramework::XamlIsland,
        ] {
            for action in ACTIONS {
                let decision = resolve_private_policy(framework, action);
                assert_eq!(decision.transport, Some(PrivateActionTransport::UiaPattern));
                assert_eq!(decision.state_rule, Some(PrivateStateRule::RestoredVisible));
            }
        }
    }

    #[test]
    fn qt_select_is_explicitly_blocked() {
        for action in [
            PrivateSemanticAction::Value,
            PrivateSemanticAction::Invoke,
            PrivateSemanticAction::Toggle,
        ] {
            assert!(resolve_private_policy(PrivateFramework::Qt611Widgets, action).is_supported());
        }

        let select = resolve_private_policy(
            PrivateFramework::Qt611Widgets,
            PrivateSemanticAction::Select,
        );
        assert!(!select.is_supported());
        assert!(select.reason.contains("semantic gap"));
    }

    #[test]
    fn winforms_uses_native_messages_for_verified_semantic_actions() {
        for action in [
            PrivateSemanticAction::Value,
            PrivateSemanticAction::Invoke,
            PrivateSemanticAction::Toggle,
            PrivateSemanticAction::Select,
        ] {
            let decision = resolve_private_policy(PrivateFramework::WinForms, action);
            assert_eq!(
                decision.transport,
                Some(PrivateActionTransport::Win32Message)
            );
            assert_eq!(
                decision.state_rule,
                Some(PrivateStateRule::VisibleOrMinimized)
            );
        }
    }

    #[test]
    fn tk_and_packaged_windows_apps_fail_closed() {
        for framework in [PrivateFramework::Tk, PrivateFramework::PackagedWinUiUwp] {
            for action in ACTIONS {
                let decision = resolve_private_policy(framework, action);
                assert!(!decision.is_supported());
                assert_eq!(decision.transport, None);
                assert_eq!(decision.state_rule, None);
            }
        }
    }

    #[test]
    fn state_rules_do_not_silently_restore_or_expand_support() {
        assert!(private_window_state_allowed(
            PrivateStateRule::RestoredVisible,
            PrivateWindowState::RestoredVisible
        ));
        assert!(!private_window_state_allowed(
            PrivateStateRule::RestoredVisible,
            PrivateWindowState::Minimized
        ));
        assert!(!private_window_state_allowed(
            PrivateStateRule::RestoredVisible,
            PrivateWindowState::Hidden
        ));

        assert!(private_window_state_allowed(
            PrivateStateRule::VisibleOrMinimized,
            PrivateWindowState::RestoredVisible
        ));
        assert!(private_window_state_allowed(
            PrivateStateRule::VisibleOrMinimized,
            PrivateWindowState::Minimized
        ));
        assert!(!private_window_state_allowed(
            PrivateStateRule::VisibleOrMinimized,
            PrivateWindowState::Hidden
        ));

        for state in [
            PrivateWindowState::RestoredVisible,
            PrivateWindowState::Minimized,
            PrivateWindowState::Hidden,
        ] {
            assert!(private_window_state_allowed(
                PrivateStateRule::AnyVisibility,
                state
            ));
        }
    }
}
