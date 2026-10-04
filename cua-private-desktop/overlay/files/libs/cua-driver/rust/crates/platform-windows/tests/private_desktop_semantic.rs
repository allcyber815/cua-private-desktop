use serde_json::{json, Value};
use std::ffi::c_void;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

type Handle = *mut c_void;
const PROCESS_SYNCHRONIZE: u32 = 0x0010_0000;
const WAIT_OBJECT_0: u32 = 0;

#[link(name = "kernel32")]
unsafe extern "system" {
    fn OpenProcess(desired_access: u32, inherit: i32, process_id: u32) -> Handle;
    fn WaitForSingleObject(handle: Handle, milliseconds: u32) -> u32;
    fn CloseHandle(handle: Handle) -> i32;
}

static WPF_FIXTURE: OnceLock<PathBuf> = OnceLock::new();
static WINFORMS_KEY_FIXTURE: OnceLock<PathBuf> = OnceLock::new();
static PRIVATE_DESKTOP_INTEGRATION_LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();

async fn private_desktop_test_guard() -> tokio::sync::MutexGuard<'static, ()> {
    PRIVATE_DESKTOP_INTEGRATION_LOCK
        .get_or_init(|| tokio::sync::Mutex::new(()))
        .lock()
        .await
}

fn runtime_scoped_registry() -> cua_driver_core::tool::ToolRegistry {
    let context = cua_driver_core::session_authorization::configured_registry()
        .expect("configured authorization registry")
        .legacy_context()
        .expect("legacy authorization context");
    cua_driver_core::tool::with_runtime_scope(context.runtime_scope_key(), || {
        platform_windows::register_tools()
    })
}

fn required_conformance_path(name: &str) -> PathBuf {
    let value = std::env::var_os(name).unwrap_or_else(|| {
        panic!("{name} must be set when running ignored private-desktop provider conformance tests")
    });
    let path = PathBuf::from(value);
    assert!(path.exists(), "{name} does not exist: {}", path.display());
    path
}

fn conformance_root() -> PathBuf {
    let root = required_conformance_path("WEBGPT_CUA_CONFORMANCE_ROOT");
    assert!(
        root.is_dir(),
        "WEBGPT_CUA_CONFORMANCE_ROOT is not a directory: {}",
        root.display()
    );
    root
}

fn wpf_fixture() -> &'static Path {
    WPF_FIXTURE
        .get_or_init(|| {
            let source = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests")
                .join("fixtures")
                .join("WpfStableFixture.cs");
            assert!(
                source.is_file(),
                "committed WPF fixture source missing: {}",
                source.display()
            );

            let windir = std::env::var_os("WINDIR")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
            let candidates = [
                windir
                    .join("Microsoft.NET")
                    .join("Framework64")
                    .join("v4.0.30319"),
                windir
                    .join("Microsoft.NET")
                    .join("Framework")
                    .join("v4.0.30319"),
            ];
            let framework = candidates
                .iter()
                .find(|candidate| candidate.join("csc.exe").is_file())
                .unwrap_or_else(|| {
                    panic!(
                        "WPF fixture compiler missing; checked {} and {}",
                        candidates[0].display(),
                        candidates[1].display()
                    )
                });
            let references = [
                framework.join("WPF").join("PresentationFramework.dll"),
                framework.join("WPF").join("PresentationCore.dll"),
                framework.join("WPF").join("WindowsBase.dll"),
                framework.join("System.Xaml.dll"),
            ];
            for reference in &references {
                assert!(
                    reference.is_file(),
                    "WPF fixture compiler reference missing: {}",
                    reference.display()
                );
            }

            let output_dir = std::env::temp_dir().join("webgpt-cua-private-desktop-fixtures");
            std::fs::create_dir_all(&output_dir).unwrap_or_else(|error| {
                panic!(
                    "create WPF fixture output directory {}: {error}",
                    output_dir.display()
                )
            });
            let executable =
                output_dir.join(format!("WpfStableFixture-{}.exe", std::process::id()));

            let mut command = Command::new(framework.join("csc.exe"));
            command
                .arg("/nologo")
                .arg("/target:winexe")
                .arg(format!("/out:{}", executable.display()));
            for reference in &references {
                command.arg(format!("/reference:{}", reference.display()));
            }
            let output = command
                .arg(&source)
                .output()
                .unwrap_or_else(|error| panic!("launch WPF fixture compiler: {error}"));
            assert!(
                output.status.success(),
                "WPF fixture compile failed ({}): stdout={} stderr={}",
                output.status,
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            assert!(
                executable.is_file(),
                "WPF fixture compiler did not create {}",
                executable.display()
            );
            executable
        })
        .as_path()
}

fn winforms_key_fixture() -> &'static Path {
    WINFORMS_KEY_FIXTURE
        .get_or_init(|| {
            let source = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests")
                .join("fixtures")
                .join("WinFormsKeyFixture.cs");
            assert!(
                source.is_file(),
                "committed WinForms key fixture source missing: {}",
                source.display()
            );

            let windir = std::env::var_os("WINDIR")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
            let candidates = [
                windir
                    .join("Microsoft.NET")
                    .join("Framework64")
                    .join("v4.0.30319"),
                windir
                    .join("Microsoft.NET")
                    .join("Framework")
                    .join("v4.0.30319"),
            ];
            let framework = candidates
                .iter()
                .find(|candidate| candidate.join("csc.exe").is_file())
                .unwrap_or_else(|| {
                    panic!(
                        "WinForms fixture compiler missing; checked {} and {}",
                        candidates[0].display(),
                        candidates[1].display()
                    )
                });
            let references = [
                framework.join("System.Windows.Forms.dll"),
                framework.join("System.Drawing.dll"),
            ];
            for reference in &references {
                assert!(
                    reference.is_file(),
                    "WinForms fixture compiler reference missing: {}",
                    reference.display()
                );
            }

            let output_dir = std::env::temp_dir().join("webgpt-cua-private-desktop-fixtures");
            std::fs::create_dir_all(&output_dir).unwrap_or_else(|error| {
                panic!(
                    "create WinForms key fixture output directory {}: {error}",
                    output_dir.display()
                )
            });
            let executable =
                output_dir.join(format!("WinFormsKeyFixture-{}.exe", std::process::id()));

            let mut command = Command::new(framework.join("csc.exe"));
            command
                .arg("/nologo")
                .arg("/target:winexe")
                .arg(format!("/out:{}", executable.display()));
            for reference in &references {
                command.arg(format!("/reference:{}", reference.display()));
            }
            let output = command
                .arg(&source)
                .output()
                .unwrap_or_else(|error| panic!("launch WinForms key fixture compiler: {error}"));
            assert!(
                output.status.success(),
                "WinForms key fixture compile failed ({}): stdout={} stderr={}",
                output.status,
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            assert!(
                executable.is_file(),
                "WinForms key fixture compiler did not create {}",
                executable.display()
            );
            executable
        })
        .as_path()
}

fn structured(result: &cua_driver_core::protocol::ToolResult) -> &Value {
    result
        .structured_content
        .as_ref()
        .expect("tool result must contain structuredContent")
}

fn assert_ok(name: &str, result: &cua_driver_core::protocol::ToolResult) {
    assert_ne!(
        result.is_error,
        Some(true),
        "{name} failed: content={:?} structured={:?}",
        result.content,
        result.structured_content
    );
}

fn assert_private_interference_receipt(name: &str, receipt: &Value) {
    assert_eq!(
        receipt["isolation"]["mode"], "private_desktop",
        "{name} must publish private-desktop isolation receipt: {receipt}"
    );
    assert_eq!(
        receipt["isolation"]["process_tree_owned"], true,
        "{name} must state that the private process tree is owned: {receipt}"
    );

    let interference = &receipt["interference"];
    assert_eq!(
        interference["telemetry_complete"], true,
        "{name} interference telemetry must be complete: {receipt}"
    );
    assert_eq!(
        interference["target_user_foreground_hit"], false,
        "{name} target must never become user foreground: {receipt}"
    );
    assert_eq!(
        interference["owned_process_user_foreground_hit"], false,
        "{name} no private Job-owned process may become user foreground: {receipt}"
    );
    assert!(
        interference["input_desktop_before"].is_string()
            && interference["input_desktop_before"] == interference["input_desktop_after"],
        "{name} user input desktop must remain unchanged: {receipt}"
    );
    assert!(
        interference["sample_count"]
            .as_u64()
            .is_some_and(|count| count >= 2),
        "{name} interference receipt must include repeated sampling: {receipt}"
    );
    assert!(
        receipt["evidence"].as_array().is_some_and(|items| items
            .iter()
            .any(|item| item["kind"] == "background_isolation")),
        "{name} must publish background-isolation evidence: {receipt}"
    );
}

fn assert_private_visual_receipt(name: &str, receipt: &Value) {
    let visual = &receipt["visual"];
    assert_eq!(
        visual["observer"], "printwindow",
        "{name} strict visual receipt must name PrintWindow: {receipt}"
    );
    assert_eq!(
        visual["baseline_stable"], true,
        "{name} strict visual baseline must be settled: {receipt}"
    );
    assert!(
        visual["mutation_delta_observed"].is_boolean(),
        "{name} strict visual receipt must state whether a mutation delta was observed: {receipt}"
    );
    assert!(
        visual["baseline_attempts"]
            .as_u64()
            .is_some_and(|count| count >= 1),
        "{name} strict visual receipt must publish baseline attempts: {receipt}"
    );
    assert!(
        visual["mutation_capture_attempts"]
            .as_u64()
            .is_some_and(|count| count >= 1),
        "{name} strict visual receipt must publish post-mutation capture attempts: {receipt}"
    );
    let mutation_delta_observed = visual["mutation_delta_observed"]
        .as_bool()
        .expect("visual mutation_delta_observed bool");
    let window_change_evidence = receipt["evidence"]
        .as_array()
        .is_some_and(|items| items.iter().any(|item| item["kind"] == "window_change"));
    assert_eq!(
        window_change_evidence, mutation_delta_observed,
        "{name} must publish window-change evidence iff the noise-masked visual comparison observed a delta: {receipt}"
    );
}

fn find_token(snapshot: &Value, labels: &[&str], roles: &[&str]) -> String {
    let elements = snapshot["elements"]
        .as_array()
        .expect("snapshot elements array");
    elements
        .iter()
        .find(|element| {
            let label = element["label"].as_str().unwrap_or_default();
            let value = element["value"].as_str().unwrap_or_default();
            let role = element["role"].as_str().unwrap_or_default();
            labels
                .iter()
                .any(|expected| label == *expected || value == *expected)
                || roles.iter().any(|expected| role.eq_ignore_ascii_case(expected))
        })
        .and_then(|element| element["element_token"].as_str())
        .map(str::to_owned)
        .unwrap_or_else(|| {
            panic!(
                "no matching element token labels={labels:?} roles={roles:?} in snapshot: {elements:?}"
            )
        })
}

fn snapshot_contains(snapshot: &Value, needle: &str) -> bool {
    snapshot["elements"].as_array().is_some_and(|elements| {
        elements.iter().any(|element| {
            element["label"]
                .as_str()
                .is_some_and(|value| value.contains(needle))
                || element["value"]
                    .as_str()
                    .is_some_and(|value| value.contains(needle))
        })
    }) || snapshot["tree_markdown"]
        .as_str()
        .is_some_and(|tree| tree.contains(needle))
}

fn find_input_token(snapshot: &Value) -> String {
    let elements = snapshot["elements"]
        .as_array()
        .expect("snapshot elements array");
    elements
        .iter()
        .find(|element| {
            element["label"].as_str() == Some("FixtureInput")
                || element["role"]
                    .as_str()
                    .unwrap_or_default()
                    .eq_ignore_ascii_case("Edit")
        })
        .and_then(|element| element["element_token"].as_str())
        .map(str::to_owned)
        .unwrap_or_else(|| panic!("no Value/Edit element token in snapshot: {elements:?}"))
}

fn has_value(snapshot: &Value, expected: &str) -> bool {
    snapshot["elements"].as_array().is_some_and(|elements| {
        elements
            .iter()
            .any(|element| element["value"].as_str() == Some(expected))
    })
}

fn has_status_event(snapshot: &Value, expected: &str) -> bool {
    let expected = format!("text={expected}");
    snapshot["elements"].as_array().is_some_and(|elements| {
        elements.iter().any(|element| {
            element["label"].as_str() == Some(expected.as_str())
                || element["value"].as_str() == Some(expected.as_str())
        })
    }) || snapshot["tree_markdown"]
        .as_str()
        .is_some_and(|tree| tree.contains(&expected))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires WEBGPT_CUA_WINFORMS_INVOKE_FIXTURE provider-conformance binary"]
async fn private_winforms_native_invoke_is_delivered_without_false_confirmation() {
    let _serial = private_desktop_test_guard().await;
    let fixture_path = required_conformance_path("WEBGPT_CUA_WINFORMS_INVOKE_FIXTURE");
    let fixture = fixture_path.as_path();
    assert!(fixture.is_file(), "fixture missing: {}", fixture.display());

    let registry = runtime_scoped_registry();
    let session = "phase2-private-winforms-invoke-e2e";

    let start = registry
        .invoke("start_session", json!({ "session": session }))
        .await;
    assert_ok("start_session", &start);

    let launch = registry
        .invoke(
            "launch_app",
            json!({
                "session": session,
                "isolation_mode": "private_desktop",
                "path": fixture.to_string_lossy(),
            }),
        )
        .await;
    assert_ok("launch_app", &launch);
    let launch_json = structured(&launch);
    let pid = launch_json["pid"].as_u64().expect("launch pid") as u32;
    let window_id = launch_json["windows"]
        .as_array()
        .expect("launch windows")
        .iter()
        .find(|window| {
            window["title"]
                .as_str()
                .is_some_and(|title| title == "WebGPT Background Fixture")
        })
        .and_then(|window| window["window_id"].as_u64())
        .expect("visible WinForms private HWND");

    let process = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, pid) };
    assert!(!process.is_null(), "cannot open launched pid {pid}");

    let before = registry
        .invoke(
            "get_window_state",
            json!({
                "session": session,
                "pid": pid,
                "window_id": window_id,
                "include_screenshot": false,
            }),
        )
        .await;
    assert_ok("WinForms invoke snapshot before", &before);
    let button_token = find_token(
        structured(&before),
        &["FixtureButton", "Background Action"],
        &["Button"],
    );

    let invoke = registry
        .invoke(
            "click",
            json!({
                "session": session,
                "pid": pid,
                "window_id": window_id,
                "element_token": button_token,
                "verify_visual": true,
            }),
        )
        .await;
    assert_ok("WinForms native invoke", &invoke);
    let invoke_receipt = structured(&invoke);
    assert_eq!(invoke_receipt["effect"], "unverifiable");
    assert_eq!(invoke_receipt["route"], "synthetic_events");
    assert_eq!(invoke_receipt["delivery"]["mode"], "background");
    assert_private_interference_receipt("WinForms native invoke", invoke_receipt);
    assert_private_visual_receipt("WinForms native invoke", invoke_receipt);
    assert!(
        invoke_receipt["evidence"]
            .as_array()
            .is_some_and(|items| items.iter().any(|item| item["kind"] == "window_change")),
        "generic Invoke strict visual verification must publish only observational window-change evidence: {invoke_receipt}"
    );
    assert!(
        invoke_receipt["evidence"]
            .as_array()
            .is_none_or(|items| !items.iter().any(|item| item["kind"] == "value_readback")),
        "generic Invoke visual evidence must not manufacture semantic value-readback evidence: {invoke_receipt}"
    );

    let after = registry
        .invoke(
            "get_window_state",
            json!({
                "session": session,
                "pid": pid,
                "window_id": window_id,
                "include_screenshot": false,
            }),
        )
        .await;
    assert_ok("WinForms invoke snapshot after", &after);
    let after_json = structured(&after);
    assert!(
        snapshot_contains(after_json, "clicked"),
        "WinForms Button.Click status was not observed by fresh state: {after_json}"
    );

    let input_token = find_token(after_json, &["FixtureInput", "Input"], &["Edit"]);
    let expected_value = "winforms-native-value";
    let set = registry
        .invoke(
            "set_value",
            json!({
                "session": session,
                "pid": pid,
                "window_id": window_id,
                "element_token": input_token,
                "value": expected_value,
            }),
        )
        .await;
    assert_ok("WinForms native set_value", &set);
    let set_receipt = structured(&set);
    assert_eq!(set_receipt["effect"], "confirmed");
    assert_eq!(set_receipt["route"], "synthetic_events");
    assert_eq!(set_receipt["delivery"]["mode"], "background");
    assert_private_interference_receipt("WinForms native set_value", set_receipt);
    assert!(
        set_receipt["evidence"]
            .as_array()
            .is_some_and(|items| items.iter().any(|item| item["kind"] == "value_readback")),
        "WinForms native set_value must publish value-readback evidence: {set_receipt}"
    );

    let after_value = registry
        .invoke(
            "get_window_state",
            json!({
                "session": session,
                "pid": pid,
                "window_id": window_id,
                "include_screenshot": false,
            }),
        )
        .await;
    assert_ok("WinForms native set_value snapshot after", &after_value);
    let after_value_json = structured(&after_value);
    assert!(
        has_value(after_value_json, expected_value),
        "WinForms TextBox value was not observed by fresh state: {after_value_json}"
    );

    let end = registry
        .invoke("end_session", json!({ "session": session }))
        .await;
    assert_ok("end_session", &end);
    assert_eq!(
        unsafe { WaitForSingleObject(process, 5_000) },
        WAIT_OBJECT_0,
        "end_session must tear down the WinForms Invoke private Job/process tree"
    );
    unsafe {
        let _ = CloseHandle(process);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires WEBGPT_CUA_WINFORMS_NATIVE_FIXTURE provider-conformance binary"]
async fn private_winforms_native_toggle_and_select_are_semantically_verified() {
    let _serial = private_desktop_test_guard().await;
    let fixture_path = required_conformance_path("WEBGPT_CUA_WINFORMS_NATIVE_FIXTURE");
    let fixture = fixture_path.as_path();
    assert!(fixture.is_file(), "fixture missing: {}", fixture.display());

    let registry = runtime_scoped_registry();
    let session = "phase2-private-winforms-native-e2e";

    let start = registry
        .invoke("start_session", json!({ "session": session }))
        .await;
    assert_ok("start_session", &start);

    let launch = registry
        .invoke(
            "launch_app",
            json!({
                "session": session,
                "isolation_mode": "private_desktop",
                "path": fixture.to_string_lossy(),
            }),
        )
        .await;
    assert_ok("launch_app", &launch);
    let launch_json = structured(&launch);
    let pid = launch_json["pid"].as_u64().expect("launch pid") as u32;
    let window_id = launch_json["windows"]
        .as_array()
        .expect("launch windows")
        .iter()
        .find(|window| {
            window["title"]
                .as_str()
                .is_some_and(|title| title.contains("WebGPT Background Fixture V3"))
        })
        .and_then(|window| window["window_id"].as_u64())
        .expect("visible WinForms private HWND");

    let process = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, pid) };
    assert!(!process.is_null(), "cannot open launched pid {pid}");

    let before = registry
        .invoke(
            "get_window_state",
            json!({
                "session": session,
                "pid": pid,
                "window_id": window_id,
                "include_screenshot": false,
            }),
        )
        .await;
    assert_ok("WinForms snapshot before", &before);
    let check_token = find_token(
        structured(&before),
        &["FixtureCheck", "Background Check"],
        &["CheckBox"],
    );

    let toggle = registry
        .invoke(
            "click",
            json!({
                "session": session,
                "pid": pid,
                "window_id": window_id,
                "element_token": check_token,
                "verify_visual": true,
            }),
        )
        .await;
    assert_ok("WinForms native toggle", &toggle);
    let toggle_receipt = structured(&toggle);
    assert_eq!(toggle_receipt["effect"], "confirmed");
    assert_eq!(toggle_receipt["route"], "synthetic_events");
    assert_eq!(toggle_receipt["delivery"]["mode"], "background");
    assert_private_interference_receipt("WinForms native toggle", toggle_receipt);
    assert_private_visual_receipt("WinForms native toggle", toggle_receipt);
    assert!(
        toggle_receipt["evidence"]
            .as_array()
            .is_some_and(|items| items.iter().any(|item| item["kind"] == "value_readback")),
        "toggle must publish semantic readback evidence: {toggle_receipt}"
    );
    assert!(
        toggle_receipt["evidence"]
            .as_array()
            .is_some_and(|items| items.iter().any(|item| item["kind"] == "window_change")),
        "WinForms native toggle strict visual verification must publish window-change evidence: {toggle_receipt}"
    );

    let after_toggle = registry
        .invoke(
            "get_window_state",
            json!({
                "session": session,
                "pid": pid,
                "window_id": window_id,
                "include_screenshot": false,
            }),
        )
        .await;
    assert_ok("WinForms snapshot after toggle", &after_toggle);
    let after_toggle_json = structured(&after_toggle);
    assert!(
        snapshot_contains(after_toggle_json, "check=True"),
        "WinForms CheckedChanged status was not observed: {after_toggle_json}"
    );
    let beta_token = find_token(after_toggle_json, &["Beta"], &[]);

    let select = registry
        .invoke(
            "click",
            json!({
                "session": session,
                "pid": pid,
                "window_id": window_id,
                "element_token": beta_token,
                "verify_visual": true,
            }),
        )
        .await;
    assert_ok("WinForms native select", &select);
    let select_receipt = structured(&select);
    assert_eq!(select_receipt["effect"], "confirmed");
    assert_eq!(select_receipt["route"], "synthetic_events");
    assert_eq!(select_receipt["delivery"]["mode"], "background");
    assert_private_interference_receipt("WinForms native select", select_receipt);
    assert_private_visual_receipt("WinForms native select", select_receipt);
    assert!(
        select_receipt["evidence"]
            .as_array()
            .is_some_and(|items| items.iter().any(|item| item["kind"] == "value_readback")),
        "select must publish semantic readback evidence: {select_receipt}"
    );
    assert!(
        select_receipt["evidence"]
            .as_array()
            .is_some_and(|items| items.iter().any(|item| item["kind"] == "window_change")),
        "WinForms native select strict visual verification must publish window-change evidence: {select_receipt}"
    );

    let after_select = registry
        .invoke(
            "get_window_state",
            json!({
                "session": session,
                "pid": pid,
                "window_id": window_id,
                "include_screenshot": false,
            }),
        )
        .await;
    assert_ok("WinForms snapshot after select", &after_select);
    let after_select_json = structured(&after_select);
    assert!(
        snapshot_contains(after_select_json, "selected=Beta"),
        "WinForms SelectedIndexChanged status was not observed: {after_select_json}"
    );

    let end = registry
        .invoke("end_session", json!({ "session": session }))
        .await;
    assert_ok("end_session", &end);
    assert_eq!(
        unsafe { WaitForSingleObject(process, 5_000) },
        WAIT_OBJECT_0,
        "end_session must tear down the WinForms private Job/process tree"
    );
    unsafe {
        let _ = CloseHandle(process);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn private_registry_launch_snapshot_set_value_readback_and_session_cleanup() {
    let _serial = private_desktop_test_guard().await;
    let fixture = wpf_fixture();
    assert!(fixture.is_file(), "fixture missing: {}", fixture.display());

    let registry = runtime_scoped_registry();
    let session = "phase2-private-semantic-e2e";

    let start = registry
        .invoke("start_session", json!({ "session": session }))
        .await;
    assert_ok("start_session", &start);

    let launch = registry
        .invoke(
            "launch_app",
            json!({
                "session": session,
                "isolation_mode": "private_desktop",
                "path": fixture.to_string_lossy(),
            }),
        )
        .await;
    assert_ok("launch_app", &launch);
    let launch_json = structured(&launch);
    let pid = launch_json["pid"].as_u64().expect("launch pid") as u32;
    assert_eq!(launch_json["isolation"]["mode"], "private_desktop");
    assert_eq!(launch_json["isolation"]["actor_affinity_verified"], true);
    assert_eq!(launch_json["isolation"]["blocking_affinity_verified"], true);

    let blocked_desktop_type = registry
        .invoke(
            "type_text",
            json!({
                "session": session,
                "scope": "desktop",
                "text": "must-not-reach-user-desktop",
            }),
        )
        .await;
    assert_eq!(
        blocked_desktop_type.is_error,
        Some(true),
        "private session scope=desktop type_text must fail closed"
    );
    assert_eq!(
        structured(&blocked_desktop_type)["code"],
        "private_global_input_unsupported"
    );

    let window_id = launch_json["windows"]
        .as_array()
        .expect("launch windows")
        .iter()
        .find(|window| {
            window["title"]
                .as_str()
                .is_some_and(|title| title.contains("WebGPT Background Fixture WPF"))
        })
        .and_then(|window| window["window_id"].as_u64())
        .expect("visible WPF private HWND");

    let process = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, pid) };
    assert!(!process.is_null(), "cannot open launched pid {pid}");

    let visual_path = std::env::temp_dir().join(format!("webgpt-private-visual-{pid}.png"));
    let _ = std::fs::remove_file(&visual_path);
    let visual = registry
        .invoke(
            "get_window_state",
            json!({
                "session": session,
                "pid": pid,
                "window_id": window_id,
                "include_accessibility_tree": false,
                "include_screenshot": true,
                "screenshot_out_file": visual_path.to_string_lossy(),
            }),
        )
        .await;
    assert_ok("private PrintWindow visual observer", &visual);
    let visual_receipt = structured(&visual);
    assert_eq!(visual_receipt["private_visual"]["observer"], "printwindow");
    assert_eq!(
        visual_receipt["private_visual"]["flags"],
        "PW_RENDERFULLCONTENT"
    );
    assert_eq!(
        visual_receipt["private_visual"]["window_state"],
        "restored_visible"
    );
    assert_eq!(visual_receipt["private_visual"]["print_succeeded"], true);
    assert_eq!(visual_receipt["private_visual"]["sentinel_count"], 0);
    assert_eq!(visual_receipt["private_visual"]["trusted"], true);
    assert_eq!(visual_receipt["private_visual"]["fallback_used"], false);
    assert!(
        visual_receipt["private_visual"]["sampled_pixels"]
            .as_u64()
            .is_some_and(|count| count > 0),
        "private visual observer must sample non-empty pixels: {visual_receipt}"
    );
    assert!(
        visual_receipt["screenshot_width"]
            .as_u64()
            .is_some_and(|width| width > 0)
            && visual_receipt["screenshot_height"]
                .as_u64()
                .is_some_and(|height| height > 0),
        "private visual observer returned invalid dimensions: {visual_receipt}"
    );
    assert!(
        visual_path.is_file(),
        "private visual observer did not write {}",
        visual_path.display()
    );
    let _ = std::fs::remove_file(&visual_path);

    let mut input_token = None;
    let mut last_snapshot = None;
    for _ in 0..60 {
        let first_snapshot = registry
            .invoke(
                "get_window_state",
                json!({
                    "session": session,
                    "pid": pid,
                    "window_id": window_id,
                    "include_screenshot": false,
                }),
            )
            .await;
        assert_ok("get_window_state before", &first_snapshot);
        let snapshot_json = structured(&first_snapshot).clone();
        if let Some(token) = try_find_input_token(&snapshot_json) {
            input_token = Some(token);
            last_snapshot = Some(snapshot_json);
            break;
        }
        last_snapshot = Some(snapshot_json);
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    let input_token = input_token.unwrap_or_else(|| {
        panic!(
            "WPF Fixture Input did not become semantically addressable: {}",
            last_snapshot.unwrap_or(Value::Null)
        )
    });

    let expected = "phase2-registry-value";
    let set = registry
        .invoke(
            "set_value",
            json!({
                "session": session,
                "pid": pid,
                "window_id": window_id,
                "element_token": input_token,
                "value": expected,
                "verify_visual": true,
            }),
        )
        .await;
    assert_ok("set_value", &set);
    let set_receipt = structured(&set);
    assert_eq!(set_receipt["effect"], "confirmed");
    assert_eq!(set_receipt["route"], "accessibility");
    assert_eq!(set_receipt["delivery"]["mode"], "background");
    assert_private_interference_receipt("WPF set_value", set_receipt);
    assert_private_visual_receipt("WPF set_value", set_receipt);
    assert!(
        set_receipt["evidence"]
            .as_array()
            .is_some_and(|items| items.iter().any(|item| item["kind"] == "value_readback")),
        "set_value must publish value-readback evidence: {set_receipt}"
    );
    assert!(
        set_receipt["evidence"]
            .as_array()
            .is_some_and(|items| items.iter().any(|item| item["kind"] == "window_change")),
        "set_value strict visual verification must publish window-change evidence: {set_receipt}"
    );

    let second_snapshot = registry
        .invoke(
            "get_window_state",
            json!({
                "session": session,
                "pid": pid,
                "window_id": window_id,
                "include_screenshot": false,
            }),
        )
        .await;
    assert_ok("get_window_state after", &second_snapshot);
    let after = structured(&second_snapshot);
    assert!(
        has_value(after, expected),
        "TextBox value did not update: {after}"
    );
    assert!(
        has_status_event(after, expected),
        "WPF TextChanged event/status was not observed: {after}"
    );

    let type_token = find_input_token(after);

    let blocked_foreground_type = registry
        .invoke(
            "type_text",
            json!({
                "session": session,
                "pid": pid,
                "window_id": window_id,
                "element_token": type_token.clone(),
                "text": "-must-not-foreground",
                "delivery_mode": "foreground",
            }),
        )
        .await;
    assert_eq!(
        blocked_foreground_type.is_error,
        Some(true),
        "private type_text must never fall through to foreground SendInput"
    );
    assert_eq!(
        structured(&blocked_foreground_type)["code"],
        "private_foreground_input_unsupported"
    );

    let blocked_pixel_type = registry
        .invoke(
            "type_text",
            json!({
                "session": session,
                "pid": pid,
                "window_id": window_id,
                "element_token": type_token.clone(),
                "text": "-must-not-pixel-focus",
                "x": 1,
                "y": 1,
                "delivery_mode": "background",
            }),
        )
        .await;
    assert_eq!(
        blocked_pixel_type.is_error,
        Some(true),
        "private type_text must never use pixel-focus/global input"
    );
    assert_eq!(
        structured(&blocked_pixel_type)["code"],
        "private_global_input_unsupported"
    );

    let typed_suffix = "-typed";
    let typed_expected = format!("{expected}{typed_suffix}");
    let typed = registry
        .invoke(
            "type_text",
            json!({
                "session": session,
                "pid": pid,
                "window_id": window_id,
                "element_token": type_token,
                "text": typed_suffix,
                "delivery_mode": "background",
            }),
        )
        .await;
    assert_ok("WPF private type_text", &typed);
    let typed_receipt = structured(&typed);
    assert_eq!(typed_receipt["effect"], "confirmed");
    assert_eq!(typed_receipt["route"], "accessibility");
    assert_eq!(typed_receipt["delivery"]["mode"], "background");
    assert_private_interference_receipt("WPF private type_text", typed_receipt);
    assert!(
        typed_receipt["evidence"]
            .as_array()
            .is_some_and(|items| items.iter().any(|item| item["kind"] == "value_readback")),
        "private type_text must publish value-readback evidence: {typed_receipt}"
    );

    let typed_snapshot = registry
        .invoke(
            "get_window_state",
            json!({
                "session": session,
                "pid": pid,
                "window_id": window_id,
                "include_screenshot": false,
            }),
        )
        .await;
    assert_ok("WPF snapshot after private type_text", &typed_snapshot);
    let after_typed = structured(&typed_snapshot);
    assert!(
        has_value(after_typed, &typed_expected),
        "private type_text did not append the requested text: {after_typed}"
    );
    assert!(
        has_status_event(after_typed, &typed_expected),
        "WPF TextChanged event/status was not observed after private type_text: {after_typed}"
    );

    let check_token = find_token(
        after_typed,
        &["FixtureCheck", "Background Check"],
        &["CheckBox"],
    );
    let toggle = registry
        .invoke(
            "click",
            json!({
                "session": session,
                "pid": pid,
                "window_id": window_id,
                "element_token": check_token,
                "verify_visual": true,
            }),
        )
        .await;
    assert_ok("WPF private UIA toggle", &toggle);
    let toggle_receipt = structured(&toggle);
    assert_eq!(toggle_receipt["effect"], "confirmed");
    assert_eq!(toggle_receipt["route"], "accessibility");
    assert_eq!(toggle_receipt["delivery"]["mode"], "background");
    assert_private_interference_receipt("WPF private UIA toggle", toggle_receipt);
    assert_private_visual_receipt("WPF private UIA toggle", toggle_receipt);
    assert!(
        toggle_receipt["evidence"]
            .as_array()
            .is_some_and(|items| items.iter().any(|item| item["kind"] == "value_readback")),
        "WPF Toggle must publish semantic readback evidence: {toggle_receipt}"
    );
    let third_snapshot = registry
        .invoke(
            "get_window_state",
            json!({
                "session": session,
                "pid": pid,
                "window_id": window_id,
                "include_screenshot": false,
            }),
        )
        .await;
    assert_ok("WPF snapshot after toggle", &third_snapshot);
    let after_toggle = structured(&third_snapshot);
    assert!(
        snapshot_contains(after_toggle, "check=true"),
        "WPF checked semantic state was not observed: {after_toggle}"
    );

    let button_token = find_token(after_toggle, &["FixtureButton", "Background Action"], &[]);
    let invoke = registry
        .invoke(
            "click",
            json!({
                "session": session,
                "pid": pid,
                "window_id": window_id,
                "element_token": button_token,
            }),
        )
        .await;
    assert_ok("WPF private UIA invoke", &invoke);
    let invoke_receipt = structured(&invoke);
    assert_eq!(invoke_receipt["effect"], "unverifiable");
    assert_eq!(invoke_receipt["route"], "accessibility");
    assert_eq!(invoke_receipt["delivery"]["mode"], "background");
    assert_private_interference_receipt("WPF private UIA invoke", invoke_receipt);
    assert!(
        invoke_receipt["evidence"]
            .as_array()
            .is_some_and(|items| items.iter().any(|item| item["kind"] == "background_isolation")),
        "generic WPF Invoke must carry independently verified background-isolation evidence: {invoke_receipt}"
    );
    assert!(
        invoke_receipt["evidence"]
            .as_array()
            .is_none_or(|items| !items.iter().any(|item| item["kind"] == "value_readback")),
        "generic WPF Invoke must not manufacture semantic value-readback evidence: {invoke_receipt}"
    );

    let fourth_snapshot = registry
        .invoke(
            "get_window_state",
            json!({
                "session": session,
                "pid": pid,
                "window_id": window_id,
                "include_screenshot": false,
            }),
        )
        .await;
    assert_ok("WPF snapshot after invoke", &fourth_snapshot);
    let after_invoke = structured(&fourth_snapshot);
    assert!(
        snapshot_contains(after_invoke, "clicked-1"),
        "WPF Button.Click status was not observed by fresh state: {after_invoke}"
    );

    // WPF drops posted modifier chords while it is not the user's foreground
    // window. The private hotkey route must therefore choose the advertised
    // UIA accelerator before dispatch rather than reporting background
    // unavailable or escalating to SendInput.
    let semantic_hotkey = registry
        .invoke(
            "hotkey",
            json!({
                "session": session,
                "pid": pid,
                "window_id": window_id,
                "keys": ["ctrl", "k"],
                "delivery_mode": "background",
            }),
        )
        .await;
    assert_ok("WPF private semantic hotkey", &semantic_hotkey);
    let semantic_hotkey_receipt = structured(&semantic_hotkey);
    assert_eq!(semantic_hotkey_receipt["effect"], "unverifiable");
    assert_eq!(semantic_hotkey_receipt["route"], "accessibility");
    assert_eq!(semantic_hotkey_receipt["delivery"]["mode"], "background");
    assert!(
        semantic_hotkey_receipt["evidence"]
            .as_array()
            .is_some_and(|items| items
                .iter()
                .any(|item| item["kind"] == "background_isolation")),
        "semantic hotkey must carry independently verified private-desktop isolation evidence: {semantic_hotkey_receipt}"
    );
    assert_private_interference_receipt("WPF private semantic hotkey", semantic_hotkey_receipt);

    let fifth_snapshot = registry
        .invoke(
            "get_window_state",
            json!({
                "session": session,
                "pid": pid,
                "window_id": window_id,
                "include_screenshot": false,
            }),
        )
        .await;
    assert_ok("WPF snapshot after semantic hotkey", &fifth_snapshot);
    let after_semantic_hotkey = structured(&fifth_snapshot);
    assert!(
        snapshot_contains(after_semantic_hotkey, "clicked-2"),
        "UIA accelerator hotkey did not invoke the WPF command: {after_semantic_hotkey}"
    );

    let semantic_press_key = registry
        .invoke(
            "press_key",
            json!({
                "session": session,
                "pid": pid,
                "window_id": window_id,
                "key": "f6",
                "delivery_mode": "background",
            }),
        )
        .await;
    assert_ok("WPF private semantic press_key", &semantic_press_key);
    let semantic_press_key_receipt = structured(&semantic_press_key);
    assert_eq!(semantic_press_key_receipt["effect"], "unverifiable");
    assert_eq!(semantic_press_key_receipt["route"], "accessibility");
    assert_eq!(semantic_press_key_receipt["delivery"]["mode"], "background");
    assert!(
        semantic_press_key_receipt["evidence"]
            .as_array()
            .is_some_and(|items| items
                .iter()
                .any(|item| item["kind"] == "background_isolation")),
        "semantic press_key must carry independently verified private-desktop isolation evidence: {semantic_press_key_receipt}"
    );
    assert_private_interference_receipt(
        "WPF private semantic press_key",
        semantic_press_key_receipt,
    );

    let sixth_snapshot = registry
        .invoke(
            "get_window_state",
            json!({
                "session": session,
                "pid": pid,
                "window_id": window_id,
                "include_screenshot": false,
            }),
        )
        .await;
    assert_ok("WPF snapshot after semantic press_key", &sixth_snapshot);
    let after_semantic_press_key = structured(&sixth_snapshot);
    assert!(
        snapshot_contains(after_semantic_press_key, "function-1"),
        "UIA accelerator press_key did not invoke the WPF F6 command: {after_semantic_press_key}"
    );

    let end = registry
        .invoke("end_session", json!({ "session": session }))
        .await;
    assert_ok("end_session", &end);

    assert_eq!(
        unsafe { WaitForSingleObject(process, 5_000) },
        WAIT_OBJECT_0,
        "end_session must tear down the private Job/process tree"
    );
    unsafe {
        let _ = CloseHandle(process);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn private_press_key_posts_inside_private_desktop_without_foreground_fallback() {
    let _serial = private_desktop_test_guard().await;
    let fixture = winforms_key_fixture();
    assert!(fixture.is_file(), "fixture missing: {}", fixture.display());

    let registry = runtime_scoped_registry();
    let session = "private-press-key-e2e";

    let start = registry
        .invoke("start_session", json!({ "session": session }))
        .await;
    assert_ok("press_key start_session", &start);

    let launch = registry
        .invoke(
            "launch_app",
            json!({
                "session": session,
                "isolation_mode": "private_desktop",
                "path": fixture.to_string_lossy(),
            }),
        )
        .await;
    assert_ok("press_key launch_app", &launch);
    let launch_json = structured(&launch);
    let pid = launch_json["pid"].as_u64().expect("launch pid") as u32;
    let window_id = launch_json["windows"]
        .as_array()
        .expect("launch windows")
        .iter()
        .find(|window| {
            window["title"]
                .as_str()
                .is_some_and(|title| title.contains("WebGPT Background Fixture WinForms Key"))
        })
        .and_then(|window| window["window_id"].as_u64())
        .unwrap_or_else(|| panic!("WinForms private key HWND not found: {launch_json}"));

    let process = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, pid) };
    assert!(!process.is_null(), "cannot open launched pid {pid}");

    let blocked_foreground = registry
        .invoke(
            "press_key",
            json!({
                "session": session,
                "pid": pid,
                "window_id": window_id,
                "key": "a",
                "delivery_mode": "foreground",
            }),
        )
        .await;
    assert_eq!(
        blocked_foreground.is_error,
        Some(true),
        "private press_key must never use SendInput"
    );
    assert_eq!(
        structured(&blocked_foreground)["code"],
        "private_foreground_input_unsupported"
    );

    let press = registry
        .invoke(
            "press_key",
            json!({
                "session": session,
                "pid": pid,
                "window_id": window_id,
                "key": "a",
                "delivery_mode": "background",
            }),
        )
        .await;
    assert_ok("private press_key", &press);
    let receipt = structured(&press);
    assert_eq!(receipt["effect"], "unverifiable");
    assert_eq!(receipt["route"], "synthetic_events");
    assert_eq!(receipt["delivery"]["mode"], "background");
    assert_private_interference_receipt("WinForms private press_key", receipt);

    let snapshot = registry
        .invoke(
            "get_window_state",
            json!({
                "session": session,
                "pid": pid,
                "window_id": window_id,
                "include_screenshot": false,
            }),
        )
        .await;
    assert_ok("WinForms snapshot after private press_key", &snapshot);
    let after = structured(&snapshot);
    assert!(
        snapshot_contains(after, "key=A"),
        "WinForms WndProc did not observe the posted key: {after}"
    );

    let blocked_hotkey_foreground = registry
        .invoke(
            "hotkey",
            json!({
                "session": session,
                "pid": pid,
                "window_id": window_id,
                "keys": ["ctrl", "s"],
                "delivery_mode": "foreground",
            }),
        )
        .await;
    assert_eq!(
        blocked_hotkey_foreground.is_error,
        Some(true),
        "private hotkey must never use foreground SendInput"
    );
    assert_eq!(
        structured(&blocked_hotkey_foreground)["code"],
        "private_foreground_input_unsupported"
    );

    let hotkey = registry
        .invoke(
            "hotkey",
            json!({
                "session": session,
                "pid": pid,
                "window_id": window_id,
                "keys": ["ctrl", "s"],
                "delivery_mode": "background",
            }),
        )
        .await;
    assert_ok("private hotkey", &hotkey);
    let hotkey_receipt = structured(&hotkey);
    assert_eq!(hotkey_receipt["effect"], "unverifiable");
    assert_eq!(hotkey_receipt["route"], "synthetic_events");
    assert_eq!(hotkey_receipt["delivery"]["mode"], "background");
    assert_private_interference_receipt("WinForms private hotkey", hotkey_receipt);

    let hotkey_snapshot = registry
        .invoke(
            "get_window_state",
            json!({
                "session": session,
                "pid": pid,
                "window_id": window_id,
                "include_screenshot": false,
            }),
        )
        .await;
    assert_ok("WinForms snapshot after private hotkey", &hotkey_snapshot);
    let after_hotkey = structured(&hotkey_snapshot);
    assert!(
        snapshot_contains(after_hotkey, "combo=Ctrl+S"),
        "WinForms WndProc did not observe the posted Ctrl+S sequence: {after_hotkey}"
    );

    let end = registry
        .invoke("end_session", json!({ "session": session }))
        .await;
    assert_ok("press_key end_session", &end);
    assert_eq!(
        unsafe { WaitForSingleObject(process, 5_000) },
        WAIT_OBJECT_0,
        "end_session must tear down the private WinForms key fixture"
    );
    unsafe {
        let _ = CloseHandle(process);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn private_move_cursor_posts_hover_without_moving_outer_pointer() {
    let _serial = private_desktop_test_guard().await;
    use windows::Win32::Foundation::POINT;
    use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;

    let fixture = winforms_key_fixture();
    assert!(fixture.is_file(), "fixture missing: {}", fixture.display());

    let registry = runtime_scoped_registry();
    let session = "phase2-private-hover-e2e";
    let start = registry
        .invoke("start_session", json!({ "session": session }))
        .await;
    assert_ok("start_session", &start);

    let launch = registry
        .invoke(
            "launch_app",
            json!({
                "session": session,
                "isolation_mode": "private_desktop",
                "path": fixture.to_string_lossy(),
            }),
        )
        .await;
    assert_ok("launch_app", &launch);
    let launch_json = structured(&launch);
    let pid = launch_json["pid"].as_u64().expect("launch pid") as u32;
    let window = launch_json["windows"]
        .as_array()
        .expect("launch windows")
        .iter()
        .find(|window| {
            window["title"]
                .as_str()
                .is_some_and(|title| title.contains("WebGPT Background Fixture WinForms Key"))
        })
        .expect("visible WinForms key fixture private HWND");
    let window_id = window["window_id"].as_u64().expect("window_id");
    let bounds = &window["bounds"];
    let left = bounds["x"].as_i64().expect("window x");
    let top = bounds["y"].as_i64().expect("window y");
    let width = bounds["width"].as_i64().expect("window width");
    let height = bounds["height"].as_i64().expect("window height");
    // Use an empty client-area point so the deepest-child hit test still
    // resolves to the Form itself, whose WndProc records WM_MOUSEMOVE.
    let hover_x = left + width - 48;
    let hover_y = top + height - 48;

    let process = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, pid) };
    assert!(!process.is_null(), "cannot open launched pid {pid}");

    let mut cursor_before = POINT::default();
    unsafe {
        GetCursorPos(&mut cursor_before).expect("GetCursorPos before hover");
    }

    let global = registry
        .invoke(
            "move_cursor",
            json!({
                "session": session,
                "target": {"kind":"desktop","display_id":"primary"},
                "x": hover_x,
                "y": hover_y,
            }),
        )
        .await;
    assert_eq!(
        global.is_error,
        Some(true),
        "private desktop pointer move must fail"
    );
    assert_eq!(
        structured(&global)["code"],
        "private_global_input_unsupported",
        "private session must not fall through to SetCursorPos: {}",
        structured(&global)
    );

    let moved = registry
        .invoke(
            "move_cursor",
            json!({
                "session": session,
                "target": {"kind":"window","pid":pid,"window_id":window_id},
                "x": hover_x,
                "y": hover_y,
            }),
        )
        .await;
    assert_ok("private background hover", &moved);
    let receipt = structured(&moved);
    assert_eq!(receipt["effect"], "unverifiable");
    assert_eq!(receipt["route"], "synthetic_events");
    assert_eq!(receipt["delivery"]["mode"], "background");
    // The standard ActionResult intentionally projects transport into route/delivery;
    // actual WM_MOUSEMOVE delivery is verified below by fresh fixture state.
    assert_private_interference_receipt("private background hover", receipt);

    let mut cursor_after = POINT::default();
    unsafe {
        GetCursorPos(&mut cursor_after).expect("GetCursorPos after hover");
    }
    assert_eq!(
        (cursor_after.x, cursor_after.y),
        (cursor_before.x, cursor_before.y),
        "background hover moved the user's real OS pointer"
    );

    let after = registry
        .invoke(
            "get_window_state",
            json!({
                "session": session,
                "pid": pid,
                "window_id": window_id,
                "include_screenshot": false,
            }),
        )
        .await;
    assert_ok("hover snapshot after", &after);
    assert!(
        snapshot_contains(structured(&after), "mouse=move"),
        "WinForms WM_MOUSEMOVE status was not observed by fresh state: {}",
        structured(&after)
    );

    let end = registry
        .invoke("end_session", json!({ "session": session }))
        .await;
    assert_ok("end_session", &end);
    assert_eq!(
        unsafe { WaitForSingleObject(process, 5_000) },
        WAIT_OBJECT_0,
        "end_session must tear down the hover fixture process tree"
    );
    unsafe {
        let _ = CloseHandle(process);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn private_double_and_right_click_post_without_foreground_fallback() {
    let _serial = private_desktop_test_guard().await;
    let fixture = winforms_key_fixture();
    assert!(fixture.is_file(), "fixture missing: {}", fixture.display());

    let registry = runtime_scoped_registry();
    let session = "private-pointer-click-e2e";

    let start = registry
        .invoke("start_session", json!({ "session": session }))
        .await;
    assert_ok("pointer start_session", &start);

    let launch = registry
        .invoke(
            "launch_app",
            json!({
                "session": session,
                "isolation_mode": "private_desktop",
                "path": fixture.to_string_lossy(),
            }),
        )
        .await;
    assert_ok("pointer launch_app", &launch);
    let launch_json = structured(&launch);
    let pid = launch_json["pid"].as_u64().expect("launch pid") as u32;
    let window_id = launch_json["windows"]
        .as_array()
        .expect("launch windows")
        .iter()
        .find(|window| {
            window["title"]
                .as_str()
                .is_some_and(|title| title.contains("WebGPT Background Fixture WinForms Key"))
        })
        .and_then(|window| window["window_id"].as_u64())
        .unwrap_or_else(|| panic!("WinForms private pointer HWND not found: {launch_json}"));

    let process = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, pid) };
    assert!(!process.is_null(), "cannot open launched pid {pid}");

    let blocked_double = registry
        .invoke(
            "double_click",
            json!({
                "session": session,
                "pid": pid,
                "window_id": window_id,
                "x": 400.0,
                "y": 120.0,
                "delivery_mode": "foreground",
            }),
        )
        .await;
    assert_eq!(
        blocked_double.is_error,
        Some(true),
        "private double_click must never use foreground SendInput"
    );
    assert_eq!(
        structured(&blocked_double)["code"],
        "private_foreground_input_unsupported"
    );

    let before_double = registry
        .invoke(
            "get_window_state",
            json!({
                "session": session,
                "pid": pid,
                "window_id": window_id,
                "include_screenshot": true,
            }),
        )
        .await;
    assert_ok("snapshot before private double_click", &before_double);

    let double_click = registry
        .invoke(
            "double_click",
            json!({
                "session": session,
                "pid": pid,
                "window_id": window_id,
                "x": 400.0,
                "y": 120.0,
                "delivery_mode": "background",
            }),
        )
        .await;
    assert_ok("private double_click", &double_click);
    let double_receipt = structured(&double_click);
    assert_eq!(double_receipt["effect"], "unverifiable");
    assert_eq!(double_receipt["route"], "synthetic_events");
    assert_eq!(double_receipt["delivery"]["mode"], "background");
    assert_private_interference_receipt("WinForms private double_click", double_receipt);

    let after_double = registry
        .invoke(
            "get_window_state",
            json!({
                "session": session,
                "pid": pid,
                "window_id": window_id,
                "include_screenshot": false,
            }),
        )
        .await;
    assert_ok("snapshot after private double_click", &after_double);
    let double_state = structured(&after_double);
    assert!(
        snapshot_contains(double_state, "mouse=double-left")
            || snapshot_contains(double_state, "mouse=left-2"),
        "WinForms WndProc did not observe the posted double-click: {double_state}"
    );

    let blocked_right = registry
        .invoke(
            "right_click",
            json!({
                "session": session,
                "pid": pid,
                "window_id": window_id,
                "x": 400.0,
                "y": 120.0,
                "delivery_mode": "foreground",
            }),
        )
        .await;
    assert_eq!(
        blocked_right.is_error,
        Some(true),
        "private right_click must never use foreground SendInput"
    );
    assert_eq!(
        structured(&blocked_right)["code"],
        "private_foreground_input_unsupported"
    );

    let before_right = registry
        .invoke(
            "get_window_state",
            json!({
                "session": session,
                "pid": pid,
                "window_id": window_id,
                "include_screenshot": true,
            }),
        )
        .await;
    assert_ok("snapshot before private right_click", &before_right);

    let right_click = registry
        .invoke(
            "right_click",
            json!({
                "session": session,
                "pid": pid,
                "window_id": window_id,
                "x": 400.0,
                "y": 120.0,
                "delivery_mode": "background",
            }),
        )
        .await;
    assert_ok("private right_click", &right_click);
    let right_receipt = structured(&right_click);
    assert_eq!(right_receipt["effect"], "unverifiable");
    assert_eq!(right_receipt["route"], "synthetic_events");
    assert_eq!(right_receipt["delivery"]["mode"], "background");
    assert_private_interference_receipt("WinForms private right_click", right_receipt);

    let after_right = registry
        .invoke(
            "get_window_state",
            json!({
                "session": session,
                "pid": pid,
                "window_id": window_id,
                "include_screenshot": false,
            }),
        )
        .await;
    assert_ok("snapshot after private right_click", &after_right);
    let right_state = structured(&after_right);
    assert!(
        snapshot_contains(right_state, "mouse=right"),
        "WinForms WndProc did not observe the posted right-click: {right_state}"
    );

    let end = registry
        .invoke("end_session", json!({ "session": session }))
        .await;
    assert_ok("pointer end_session", &end);
    assert_eq!(
        unsafe { WaitForSingleObject(process, 5_000) },
        WAIT_OBJECT_0,
        "end_session must tear down the private WinForms pointer fixture"
    );
    unsafe {
        let _ = CloseHandle(process);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn private_drag_posts_inside_private_desktop_and_refuses_nonclient_input() {
    let _serial = private_desktop_test_guard().await;
    let fixture = winforms_key_fixture();
    assert!(fixture.is_file(), "fixture missing: {}", fixture.display());

    let registry = runtime_scoped_registry();
    let session = "private-drag-e2e";

    let start = registry
        .invoke("start_session", json!({ "session": session }))
        .await;
    assert_ok("drag start_session", &start);

    let launch = registry
        .invoke(
            "launch_app",
            json!({
                "session": session,
                "isolation_mode": "private_desktop",
                "path": fixture.to_string_lossy(),
            }),
        )
        .await;
    assert_ok("drag launch_app", &launch);
    let launch_json = structured(&launch);
    let pid = launch_json["pid"].as_u64().expect("launch pid") as u32;
    let window_id = launch_json["windows"]
        .as_array()
        .expect("launch windows")
        .iter()
        .find(|window| {
            window["title"]
                .as_str()
                .is_some_and(|title| title.contains("WebGPT Background Fixture WinForms Key"))
        })
        .and_then(|window| window["window_id"].as_u64())
        .unwrap_or_else(|| panic!("WinForms private drag HWND not found: {launch_json}"));

    let process = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, pid) };
    assert!(!process.is_null(), "cannot open launched pid {pid}");

    // CUA 0.33.1 interprets window-local pointer coordinates against the
    // screenshot context published by get_window_state. Establish that context
    // explicitly before testing the private-desktop drag delivery policy.
    let snapshot = registry
        .invoke(
            "get_window_state",
            json!({
                "session": session,
                "pid": pid,
                "window_id": window_id,
            }),
        )
        .await;
    assert_ok("drag get_window_state", &snapshot);

    let blocked_foreground = registry
        .invoke(
            "drag",
            json!({
                "session": session,
                "pid": pid,
                "window_id": window_id,
                "from_x": 300.0,
                "from_y": 100.0,
                "to_x": 420.0,
                "to_y": 140.0,
                "duration_ms": 80,
                "steps": 4,
                "delivery_mode": "foreground",
            }),
        )
        .await;
    assert_eq!(
        blocked_foreground.is_error,
        Some(true),
        "private drag must never use foreground SendInput"
    );
    assert_eq!(
        structured(&blocked_foreground)["code"],
        "private_foreground_input_unsupported"
    );

    let blocked_nonclient = registry
        .invoke(
            "drag",
            json!({
                "session": session,
                "pid": pid,
                "window_id": window_id,
                "from_x": 280.0,
                "from_y": 5.0,
                "to_x": 360.0,
                "to_y": 5.0,
                "duration_ms": 40,
                "steps": 2,
                "delivery_mode": "background",
            }),
        )
        .await;
    assert_eq!(
        blocked_nonclient.is_error,
        Some(true),
        "private non-client drag must fail closed instead of entering the OS move/resize loop"
    );
    assert_eq!(
        structured(&blocked_nonclient)["code"],
        "background_unavailable"
    );

    let drag = registry
        .invoke(
            "drag",
            json!({
                "session": session,
                "pid": pid,
                "window_id": window_id,
                "from_x": 300.0,
                "from_y": 100.0,
                "to_x": 420.0,
                "to_y": 140.0,
                "duration_ms": 80,
                "steps": 4,
                "delivery_mode": "background",
            }),
        )
        .await;
    assert_ok("private drag", &drag);
    let receipt = structured(&drag);
    assert_eq!(receipt["effect"], "unverifiable");
    assert_eq!(receipt["route"], "synthetic_events");
    assert_eq!(receipt["delivery"]["mode"], "background");
    assert_private_interference_receipt("WinForms private drag", receipt);

    let snapshot = registry
        .invoke(
            "get_window_state",
            json!({
                "session": session,
                "pid": pid,
                "window_id": window_id,
                "include_screenshot": false,
            }),
        )
        .await;
    assert_ok("WinForms snapshot after private drag", &snapshot);
    let after = structured(&snapshot);
    assert!(
        snapshot_contains(after, "mouse=drag-end"),
        "WinForms WndProc did not observe the posted drag: {after}"
    );

    let end = registry
        .invoke("end_session", json!({ "session": session }))
        .await;
    assert_ok("drag end_session", &end);
    assert_eq!(
        unsafe { WaitForSingleObject(process, 5_000) },
        WAIT_OBJECT_0,
        "end_session must tear down the private WinForms drag fixture"
    );
    unsafe {
        let _ = CloseHandle(process);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn private_zoom_uses_trusted_printwindow_without_shared_desktop_capture() {
    let _serial = private_desktop_test_guard().await;
    let fixture = winforms_key_fixture();
    assert!(fixture.is_file(), "fixture missing: {}", fixture.display());

    let registry = runtime_scoped_registry();
    let session = "private-zoom-e2e";

    let start = registry
        .invoke("start_session", json!({ "session": session }))
        .await;
    assert_ok("zoom start_session", &start);

    let launch = registry
        .invoke(
            "launch_app",
            json!({
                "session": session,
                "isolation_mode": "private_desktop",
                "path": fixture.to_string_lossy(),
            }),
        )
        .await;
    assert_ok("zoom launch_app", &launch);
    let launch_json = structured(&launch);
    let pid = launch_json["pid"].as_u64().expect("launch pid") as u32;
    let window_id = launch_json["windows"]
        .as_array()
        .expect("launch windows")
        .iter()
        .find(|window| {
            window["title"]
                .as_str()
                .is_some_and(|title| title.contains("WebGPT Background Fixture WinForms Key"))
        })
        .and_then(|window| window["window_id"].as_u64())
        .unwrap_or_else(|| panic!("WinForms private zoom HWND not found: {launch_json}"));

    let process = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, pid) };
    assert!(!process.is_null(), "cannot open launched pid {pid}");

    let before_zoom = registry
        .invoke(
            "get_window_state",
            json!({
                "session": session,
                "pid": pid,
                "window_id": window_id,
                "include_screenshot": true,
            }),
        )
        .await;
    assert_ok("snapshot before private zoom", &before_zoom);

    let zoom = registry
        .invoke(
            "zoom",
            json!({
                "session": session,
                "pid": pid,
                "window_id": window_id,
                "x1": 0.0,
                "y1": 0.0,
                "x2": 220.0,
                "y2": 140.0,
            }),
        )
        .await;
    assert_ok("private zoom", &zoom);
    let receipt = structured(&zoom);
    assert_eq!(receipt["isolation_mode"], "private_desktop");
    assert_eq!(receipt["format"], "jpeg");
    assert_eq!(receipt["mime_type"], "image/jpeg");
    assert_eq!(receipt["private_visual"]["observer"], "printwindow");
    assert_eq!(receipt["private_visual"]["trusted"], true);
    assert_eq!(receipt["private_visual"]["fallback_used"], false);
    assert!(
        receipt["width"].as_u64().is_some_and(|width| width > 0)
            && receipt["height"].as_u64().is_some_and(|height| height > 0),
        "private zoom returned invalid dimensions: {receipt}"
    );

    let end = registry
        .invoke("end_session", json!({ "session": session }))
        .await;
    assert_ok("zoom end_session", &end);
    assert_eq!(
        unsafe { WaitForSingleObject(process, 5_000) },
        WAIT_OBJECT_0,
        "end_session must tear down the private WinForms zoom fixture"
    );
    unsafe {
        let _ = CloseHandle(process);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn private_invoke_menu_uses_attached_uia_without_outer_foreground_activation() {
    let _serial = private_desktop_test_guard().await;
    let fixture = winforms_key_fixture();
    assert!(fixture.is_file(), "fixture missing: {}", fixture.display());

    let registry = runtime_scoped_registry();
    let session = "private-invoke-menu-e2e";

    let start = registry
        .invoke("start_session", json!({ "session": session }))
        .await;
    assert_ok("menu start_session", &start);

    let launch = registry
        .invoke(
            "launch_app",
            json!({
                "session": session,
                "isolation_mode": "private_desktop",
                "path": fixture.to_string_lossy(),
            }),
        )
        .await;
    assert_ok("menu launch_app", &launch);
    let launch_json = structured(&launch);
    let pid = launch_json["pid"].as_u64().expect("launch pid") as u32;
    let window_id = launch_json["windows"]
        .as_array()
        .expect("launch windows")
        .iter()
        .find(|window| {
            window["title"]
                .as_str()
                .is_some_and(|title| title.contains("WebGPT Background Fixture WinForms Key"))
        })
        .and_then(|window| window["window_id"].as_u64())
        .unwrap_or_else(|| panic!("WinForms private menu HWND not found: {launch_json}"));

    let process = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, pid) };
    assert!(!process.is_null(), "cannot open launched pid {pid}");

    let invoke = registry
        .invoke(
            "invoke_menu",
            json!({
                "session": session,
                "pid": pid,
                "window_id": window_id,
                "path": ["Actions", "Mark"],
            }),
        )
        .await;
    assert_ok("private invoke_menu", &invoke);
    let receipt = structured(&invoke);
    assert_eq!(receipt["effect"], "unverifiable");
    assert_eq!(receipt["route"], "accessibility");
    assert_eq!(receipt["delivery"]["mode"], "background");
    assert_private_interference_receipt("WinForms private invoke_menu", receipt);

    let snapshot = registry
        .invoke(
            "get_window_state",
            json!({
                "session": session,
                "pid": pid,
                "window_id": window_id,
                "include_screenshot": false,
            }),
        )
        .await;
    assert_ok("WinForms snapshot after private invoke_menu", &snapshot);
    let after = structured(&snapshot);
    assert!(
        snapshot_contains(after, "menu=mark"),
        "WinForms menu Click event was not observed from fresh UIA state: {after}"
    );

    let end = registry
        .invoke("end_session", json!({ "session": session }))
        .await;
    assert_ok("menu end_session", &end);
    assert_eq!(
        unsafe { WaitForSingleObject(process, 5_000) },
        WAIT_OBJECT_0,
        "end_session must tear down the private WinForms menu fixture"
    );
    unsafe {
        let _ = CloseHandle(process);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn private_scroll_posts_inside_private_desktop_without_foreground_fallback() {
    let _serial = private_desktop_test_guard().await;
    let fixture = winforms_key_fixture();
    assert!(fixture.is_file(), "fixture missing: {}", fixture.display());

    let registry = runtime_scoped_registry();
    let session = "private-scroll-e2e";

    let start = registry
        .invoke("start_session", json!({ "session": session }))
        .await;
    assert_ok("scroll start_session", &start);

    let launch = registry
        .invoke(
            "launch_app",
            json!({
                "session": session,
                "isolation_mode": "private_desktop",
                "path": fixture.to_string_lossy(),
            }),
        )
        .await;
    assert_ok("scroll launch_app", &launch);
    let launch_json = structured(&launch);
    let pid = launch_json["pid"].as_u64().expect("launch pid") as u32;
    let window_id = launch_json["windows"]
        .as_array()
        .expect("launch windows")
        .iter()
        .find(|window| {
            window["title"]
                .as_str()
                .is_some_and(|title| title.contains("WebGPT Background Fixture WinForms Key"))
        })
        .and_then(|window| window["window_id"].as_u64())
        .unwrap_or_else(|| panic!("WinForms private scroll HWND not found: {launch_json}"));

    let process = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, pid) };
    assert!(!process.is_null(), "cannot open launched pid {pid}");

    let blocked_foreground = registry
        .invoke(
            "scroll",
            json!({
                "session": session,
                "pid": pid,
                "window_id": window_id,
                "direction": "down",
                "by": "line",
                "amount": 1,
                "delivery_mode": "foreground",
            }),
        )
        .await;
    assert_eq!(
        blocked_foreground.is_error,
        Some(true),
        "private scroll must never use foreground wheel SendInput"
    );
    assert_eq!(
        structured(&blocked_foreground)["code"],
        "private_foreground_input_unsupported"
    );

    let scroll = registry
        .invoke(
            "scroll",
            json!({
                "session": session,
                "pid": pid,
                "window_id": window_id,
                "direction": "down",
                "by": "line",
                "amount": 1,
                "delivery_mode": "background",
            }),
        )
        .await;
    assert_ok("private scroll", &scroll);
    let receipt = structured(&scroll);
    assert_eq!(receipt["effect"], "unverifiable");
    assert_eq!(receipt["route"], "synthetic_events");
    assert_eq!(receipt["delivery"]["mode"], "background");
    assert_private_interference_receipt("WinForms private scroll", receipt);

    let snapshot = registry
        .invoke(
            "get_window_state",
            json!({
                "session": session,
                "pid": pid,
                "window_id": window_id,
                "include_screenshot": false,
            }),
        )
        .await;
    assert_ok("WinForms snapshot after private scroll", &snapshot);
    let after = structured(&snapshot);
    assert!(
        snapshot_contains(after, "scroll=v-1"),
        "WinForms WndProc did not observe WM_VSCROLL/SB_LINEDOWN: {after}"
    );

    let end = registry
        .invoke("end_session", json!({ "session": session }))
        .await;
    assert_ok("scroll end_session", &end);
    assert_eq!(
        unsafe { WaitForSingleObject(process, 5_000) },
        WAIT_OBJECT_0,
        "end_session must tear down the private WinForms scroll fixture"
    );
    unsafe {
        let _ = CloseHandle(process);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn private_set_window_frame_is_confirmed_without_outer_desktop_interference() {
    let _serial = private_desktop_test_guard().await;
    let fixture = winforms_key_fixture();
    assert!(fixture.is_file(), "fixture missing: {}", fixture.display());

    let registry = runtime_scoped_registry();
    let session = "private-set-window-frame-e2e";

    let start = registry
        .invoke("start_session", json!({ "session": session }))
        .await;
    assert_ok("set_window_frame start_session", &start);

    let launch = registry
        .invoke(
            "launch_app",
            json!({
                "session": session,
                "isolation_mode": "private_desktop",
                "path": fixture.to_string_lossy(),
            }),
        )
        .await;
    assert_ok("set_window_frame launch_app", &launch);
    let launch_json = structured(&launch);
    let pid = launch_json["pid"].as_u64().expect("launch pid") as u32;
    let window_id = launch_json["windows"]
        .as_array()
        .expect("launch windows")
        .iter()
        .find(|window| {
            window["title"]
                .as_str()
                .is_some_and(|title| title.contains("WebGPT Background Fixture WinForms Key"))
        })
        .and_then(|window| window["window_id"].as_u64())
        .unwrap_or_else(|| panic!("WinForms private frame HWND not found: {launch_json}"));

    let process = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, pid) };
    assert!(!process.is_null(), "cannot open launched pid {pid}");

    let set_frame = registry
        .invoke(
            "set_window_frame",
            json!({
                "session": session,
                "pid": pid,
                "window_id": window_id,
                "x": 120.0,
                "y": 140.0,
                "width": 520.0,
                "height": 260.0,
            }),
        )
        .await;
    assert_ok("private set_window_frame", &set_frame);
    let receipt = structured(&set_frame);
    assert_eq!(receipt["effect"], "confirmed");
    assert_eq!(receipt["route"], "system_api");
    assert_eq!(receipt["delivery"]["mode"], "not_applicable");
    assert_private_interference_receipt("WinForms private set_window_frame", receipt);
    assert!(
        receipt["evidence"]
            .as_array()
            .is_some_and(|items| items.iter().any(|item| item["kind"] == "value_readback")),
        "private set_window_frame must carry geometry readback evidence: {receipt}"
    );

    let end = registry
        .invoke("end_session", json!({ "session": session }))
        .await;
    assert_ok("set_window_frame end_session", &end);
    assert_eq!(
        unsafe { WaitForSingleObject(process, 5_000) },
        WAIT_OBJECT_0,
        "end_session must tear down the private WinForms frame fixture"
    );
    unsafe {
        let _ = CloseHandle(process);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn private_verify_state_observes_session_owned_window_and_elements() {
    let _serial = private_desktop_test_guard().await;
    let fixture = wpf_fixture();
    assert!(fixture.is_file(), "fixture missing: {}", fixture.display());

    let registry = runtime_scoped_registry();
    let session = "private-verify-state-e2e";

    let start = registry
        .invoke("start_session", json!({ "session": session }))
        .await;
    assert_ok("verify_state start_session", &start);

    let launch = registry
        .invoke(
            "launch_app",
            json!({
                "session": session,
                "isolation_mode": "private_desktop",
                "path": fixture.to_string_lossy(),
            }),
        )
        .await;
    assert_ok("verify_state launch_app", &launch);
    let launch_json = structured(&launch);
    let pid = launch_json["pid"].as_u64().expect("launch pid") as u32;
    let window_id = launch_json["windows"]
        .as_array()
        .expect("launch windows")
        .iter()
        .find(|window| {
            window["title"]
                .as_str()
                .is_some_and(|title| title.contains("WebGPT Background Fixture WPF"))
        })
        .and_then(|window| window["window_id"].as_u64())
        .unwrap_or_else(|| panic!("WPF private verify HWND not found: {launch_json}"));

    let process = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, pid) };
    assert!(!process.is_null(), "cannot open launched pid {pid}");

    let without_session = registry
        .invoke(
            "verify_state",
            json!({
                "pid": pid,
                "window_id": window_id,
                "expect": [{"window": {"exists": true}}],
                "timeout_ms": 0,
                "stable_samples": 1,
            }),
        )
        .await;
    assert_ok("verify_state without private session", &without_session);
    assert_ne!(
        structured(&without_session)["status"],
        "satisfied",
        "a private HWND must not become visible through the shared observation path"
    );

    let verified = registry
        .invoke(
            "verify_state",
            json!({
                "session": session,
                "pid": pid,
                "window_id": window_id,
                "expect": [
                    {"window": {"exists": true}},
                    {"element": {
                        "selector": {"label_contains": "Fixture Input"},
                        "exists": true
                    }}
                ],
                "timeout_ms": 3000,
                "stable_samples": 1,
                "include_screenshot": true,
            }),
        )
        .await;
    assert_ok("private verify_state", &verified);
    let receipt = structured(&verified);
    assert_eq!(receipt["status"], "satisfied");
    assert_eq!(receipt["stable"], true);
    assert!(
        verified
            .content
            .iter()
            .any(|content| matches!(content, cua_driver_core::protocol::Content::Image { .. })),
        "private verify_state must return session-bound PrintWindow evidence when requested"
    );

    let end = registry
        .invoke("end_session", json!({ "session": session }))
        .await;
    assert_ok("verify_state end_session", &end);
    assert_eq!(
        unsafe { WaitForSingleObject(process, 5_000) },
        WAIT_OBJECT_0,
        "end_session must tear down the private WPF verify fixture"
    );
    unsafe {
        let _ = CloseHandle(process);
    }
}

fn try_find_input_token(snapshot: &Value) -> Option<String> {
    snapshot["elements"]
        .as_array()?
        .iter()
        .find(|element| {
            matches!(element["label"].as_str(), Some("Fixture Input" | "Input"))
                && element["role"]
                    .as_str()
                    .is_some_and(|role| role.eq_ignore_ascii_case("Edit"))
                && element["actions"].as_array().is_some_and(|actions| {
                    actions
                        .iter()
                        .any(|action| action.as_str() == Some("set_value"))
                })
        })
        .and_then(|element| element["element_token"].as_str())
        .map(str::to_owned)
}

async fn exercise_private_value_fixture(
    case_name: &str,
    session: &str,
    fixture: &Path,
    additional_arguments: &[String],
    expected_window_title: &str,
    expected_value: &str,
) {
    assert!(
        fixture.is_file(),
        "{case_name} fixture missing: {}",
        fixture.display()
    );

    let registry = runtime_scoped_registry();
    let start = registry
        .invoke("start_session", json!({ "session": session }))
        .await;
    assert_ok(&format!("{case_name} start_session"), &start);

    let launch = registry
        .invoke(
            "launch_app",
            json!({
                "session": session,
                "isolation_mode": "private_desktop",
                "path": fixture.to_string_lossy(),
                "additional_arguments": additional_arguments,
            }),
        )
        .await;
    assert_ok(&format!("{case_name} launch_app"), &launch);
    let launch_json = structured(&launch);
    let pid = launch_json["pid"].as_u64().expect("launch pid") as u32;
    assert_eq!(launch_json["isolation"]["mode"], "private_desktop");
    assert_eq!(launch_json["isolation"]["actor_affinity_verified"], true);
    assert_eq!(launch_json["isolation"]["blocking_affinity_verified"], true);

    let windows = launch_json["windows"].as_array().expect("launch windows");
    let window_id = windows
        .iter()
        .find(|window| {
            window["title"]
                .as_str()
                .is_some_and(|title| title.contains(expected_window_title))
        })
        .or_else(|| {
            windows
                .iter()
                .filter(|window| window["private_visible"] == true)
                .max_by_key(|window| {
                    let bounds = &window["bounds"];
                    bounds["width"].as_u64().unwrap_or_default()
                        * bounds["height"].as_u64().unwrap_or_default()
                })
        })
        .and_then(|window| window["window_id"].as_u64())
        .unwrap_or_else(|| {
            panic!(
                "{case_name} visible private HWND not found for title {expected_window_title:?}: {launch_json}"
            )
        });

    let process = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, pid) };
    assert!(
        !process.is_null(),
        "{case_name}: cannot open launched pid {pid}"
    );

    let mut input_token = None;
    let mut last_snapshot = None;
    for _ in 0..60 {
        let snapshot = registry
            .invoke(
                "get_window_state",
                json!({
                    "session": session,
                    "pid": pid,
                    "window_id": window_id,
                    "include_screenshot": false,
                }),
            )
            .await;
        if snapshot.is_error != Some(true) {
            let json = structured(&snapshot).clone();
            if let Some(token) = try_find_input_token(&json) {
                input_token = Some(token);
                last_snapshot = Some(json);
                break;
            }
            last_snapshot = Some(json);
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    let input_token = input_token.unwrap_or_else(|| {
        panic!(
            "{case_name}: Fixture Input did not become UIA-visible; launch={launch_json}; last_snapshot={:?}",
            last_snapshot
        )
    });

    let set = registry
        .invoke(
            "set_value",
            json!({
                "session": session,
                "pid": pid,
                "window_id": window_id,
                "element_token": input_token,
                "value": expected_value,
                "verify_visual": true,
            }),
        )
        .await;
    assert_ok(&format!("{case_name} set_value"), &set);
    let set_receipt = structured(&set);
    assert_eq!(
        set_receipt["effect"], "confirmed",
        "{case_name}: {set_receipt}"
    );
    assert_eq!(
        set_receipt["route"], "accessibility",
        "{case_name}: {set_receipt}"
    );
    assert_eq!(
        set_receipt["delivery"]["mode"], "background",
        "{case_name}: {set_receipt}"
    );
    assert_private_interference_receipt(case_name, set_receipt);
    assert_private_visual_receipt(case_name, set_receipt);
    assert!(
        set_receipt["evidence"]
            .as_array()
            .is_some_and(|items| items.iter().any(|item| item["kind"] == "value_readback")),
        "{case_name} must publish semantic value-readback evidence: {set_receipt}"
    );

    let mut observed = None;
    for _ in 0..40 {
        let after = registry
            .invoke(
                "get_window_state",
                json!({
                    "session": session,
                    "pid": pid,
                    "window_id": window_id,
                    "include_screenshot": false,
                }),
            )
            .await;
        assert_ok(&format!("{case_name} get_window_state after"), &after);
        let after_json = structured(&after).clone();
        if has_value(&after_json, expected_value) && has_status_event(&after_json, expected_value) {
            observed = Some(after_json);
            break;
        }
        observed = Some(after_json);
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    let observed = observed.expect("post-action snapshot");
    assert!(
        has_value(&observed, expected_value),
        "{case_name}: semantic value readback missing after action: {observed}"
    );
    assert!(
        has_status_event(&observed, expected_value),
        "{case_name}: app-side TextChanged/input event missing after action: {observed}"
    );

    let end = registry
        .invoke("end_session", json!({ "session": session }))
        .await;
    assert_ok(&format!("{case_name} end_session"), &end);
    assert_eq!(
        unsafe { WaitForSingleObject(process, 5_000) },
        WAIT_OBJECT_0,
        "{case_name}: end_session must tear down the private Job/process tree"
    );
    unsafe {
        let _ = CloseHandle(process);
    }
}

#[test]
#[ignore = "requires WEBGPT_CUA_CONFORMANCE_ROOT provider-conformance assets"]
fn private_core_managed_uia_worker_sees_winui_without_actor_attachment() {
    use platform_windows::execution_environment::win32_private::PrivateDesktopCore;
    use std::ffi::OsString;
    use std::time::Duration;

    let root = conformance_root();
    let fixture = root
        .join("winui-private-fixture")
        .join("bin")
        .join("x64")
        .join("Debug")
        .join("net10.0-windows10.0.26100.0")
        .join("winui-private-fixture.exe");
    let helper = root.join("PrivateDesktopUiaDumpWorker.exe");
    assert!(
        fixture.is_file(),
        "WinUI fixture missing: {}",
        fixture.display()
    );
    assert!(
        helper.is_file(),
        "managed UIA helper missing: {}",
        helper.display()
    );

    let core = PrivateDesktopCore::create("CuaPrivateWinUiDiag")
        .expect("private desktop core create without actor attachment");
    let target = core
        .spawn_direct(&fixture, &[], fixture.parent())
        .expect("launch WinUI fixture through bare private core");
    let pid = target.id();
    let windows = core
        .wait_for_windows(pid, Duration::from_secs(6))
        .expect("discover WinUI HWND through bare private core");
    let window = windows
        .iter()
        .find(|window| window.title.contains("WebGPT Background Fixture WinUI"))
        .unwrap_or_else(|| panic!("WinUI HWND missing from bare core launch: {windows:?}"));

    let output = std::env::temp_dir().join(format!(
        "cua-private-winui-managed-uia-{}-{}.txt",
        std::process::id(),
        pid
    ));
    let _ = std::fs::remove_file(&output);
    let args = vec![
        OsString::from(pid.to_string()),
        OsString::from(window.hwnd.to_string()),
        output.as_os_str().to_owned(),
    ];
    let child = core
        .spawn_direct(&helper, &args, Some(root.as_path()))
        .expect("spawn managed UIA helper through bare private core");
    assert!(
        child
            .wait_timeout(Duration::from_secs(8))
            .expect("wait managed UIA helper"),
        "managed UIA helper timed out"
    );
    let report = std::fs::read_to_string(&output)
        .unwrap_or_else(|error| panic!("read managed UIA report {}: {error}", output.display()));
    let _ = std::fs::remove_file(&output);

    assert!(
        report.contains("ok=true"),
        "managed UIA helper failed in CUA private runtime: {report}"
    );
    assert!(
        report.contains("name=Fixture Input") && report.contains("aid=fixture-input"),
        "managed UIA helper did not expose the expected WinUI input: {report}"
    );
    assert!(
        report.contains("descendants=") && !report.contains("descendants=0"),
        "managed UIA helper returned an empty WinUI tree: {report}"
    );

    drop(target);
    drop(core);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires WEBGPT_CUA_CONFORMANCE_ROOT provider-conformance assets"]
async fn phase_f_replays_winui_webview2_and_xaml_island_value_through_real_cua() {
    let _serial = private_desktop_test_guard().await;
    let root = conformance_root();

    exercise_private_value_fixture(
        "WinUI 3 Value",
        "phase-f-winui-value",
        &root
            .join("winui-private-fixture")
            .join("bin")
            .join("x64")
            .join("Debug")
            .join("net10.0-windows10.0.26100.0")
            .join("winui-private-fixture.exe"),
        &[],
        "WebGPT Background Fixture WinUI",
        "phase-f-winui-value",
    )
    .await;

    exercise_private_value_fixture(
        "WebView2 Value",
        "phase-f-webview2-value",
        &root
            .join("webview2-private-fixture")
            .join("WebView2PrivateFixture.exe"),
        &[],
        "WebGPT WebView2 Private Fixture",
        "phase-f-webview2-value",
    )
    .await;

    exercise_private_value_fixture(
        "XAML Island Value",
        "phase-f-xaml-island-value",
        &root
            .join("xaml-island-private-fixture")
            .join("bin")
            .join("x64")
            .join("Debug")
            .join("net10.0-windows10.0.17763.0")
            .join("win-x64")
            .join("XamlIslandPrivateFixture.exe"),
        &[],
        "WebGPT Xaml Island Private Fixture",
        "phase-f-xaml-island-value",
    )
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires WEBGPT_CUA_CONFORMANCE_ROOT and WEBGPT_CUA_PYTHONW_EXE"]
async fn phase_f_replays_qt_value_through_real_cua() {
    let _serial = private_desktop_test_guard().await;
    let root = conformance_root();
    let pythonw_path = required_conformance_path("WEBGPT_CUA_PYTHONW_EXE");
    let pythonw = pythonw_path.as_path();
    let qt_runtime = root.join("qt-python-runtime");
    let code = format!(
        "import sys; sys.path.insert(0, r'{}'); from PySide6.QtCore import Qt; from PySide6.QtWidgets import QApplication,QWidget,QVBoxLayout,QLabel,QLineEdit,QPushButton; app=QApplication(sys.argv); win=QWidget(); win.setWindowTitle('WebGPT Qt Private Fixture'); layout=QVBoxLayout(win); entry=QLineEdit(); entry.setAccessibleName('Fixture Input'); entry.setObjectName('fixture-input'); entry.setFocusPolicy(Qt.FocusPolicy.NoFocus); layout.addWidget(entry); button=QPushButton('Stable Focus'); layout.addWidget(button); status=QLabel('idle'); status.setAccessibleName('idle'); status.setObjectName('fixture-status'); layout.addWidget(status); entry.textChanged.connect(lambda v: (status.setText('text='+v), status.setAccessibleName('text='+v))); win.resize(945,760); win.show(); button.setFocus(); sys.exit(app.exec())",
        qt_runtime.to_string_lossy()
    );
    let additional_arguments = vec!["-c".to_string(), code];

    exercise_private_value_fixture(
        "Qt 6.11 Value",
        "phase-f-qt-value",
        pythonw,
        &additional_arguments,
        "WebGPT Qt Private Fixture",
        "phase-f-qt-value",
    )
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires WEBGPT_CUA_CONFORMANCE_ROOT and WEBGPT_CUA_ELECTRON_EXE"]
async fn phase_f_replays_electron_value_through_real_cua() {
    let _serial = private_desktop_test_guard().await;
    let root = conformance_root();
    let electron_path = required_conformance_path("WEBGPT_CUA_ELECTRON_EXE");
    let electron = electron_path.as_path();
    let app = root.join("electron-private-app");
    let profile = std::env::temp_dir().join(format!(
        "webgpt-cua-electron-profile-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&profile);
    let additional_arguments = vec![
        app.to_string_lossy().into_owned(),
        format!("--webgpt-user-data-dir={}", profile.to_string_lossy()),
    ];

    exercise_private_value_fixture(
        "Electron Value",
        "phase-f-electron-value",
        electron,
        &additional_arguments,
        "WebGPT Background Fixture Electron",
        "phase-f-electron-value",
    )
    .await;
    let _ = std::fs::remove_dir_all(&profile);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires WEBGPT_CUA_CONFORMANCE_ROOT and WEBGPT_CUA_CHROME_EXE"]
async fn phase_f_replays_chromium_value_through_real_cua() {
    let _serial = private_desktop_test_guard().await;
    let root = conformance_root();
    let chrome_path = required_conformance_path("WEBGPT_CUA_CHROME_EXE");
    let chrome = chrome_path.as_path();
    let profile = std::env::temp_dir().join(format!(
        "webgpt-cua-chromium-profile-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&profile);
    let fixture = root.join("chromium-private-fixture.html");
    let file_url = format!("file:///{}", fixture.to_string_lossy().replace('\\', "/"));
    let additional_arguments = vec![
        format!("--user-data-dir={}", profile.to_string_lossy()),
        "--no-first-run".to_string(),
        "--no-default-browser-check".to_string(),
        "--disable-background-mode".to_string(),
        "--force-renderer-accessibility".to_string(),
        "--new-window".to_string(),
        file_url,
    ];

    exercise_private_value_fixture(
        "Chromium Value",
        "phase-f-chromium-value",
        chrome,
        &additional_arguments,
        "WebGPT Background Fixture Chromium",
        "phase-f-chromium-value",
    )
    .await;
    let _ = std::fs::remove_dir_all(&profile);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires WEBGPT_CUA_CONFORMANCE_ROOT and WEBGPT_CUA_PYTHONW_EXE"]
async fn phase_f_tk_invoke_is_blocked_by_real_cua_policy() {
    let _serial = private_desktop_test_guard().await;
    let root = conformance_root();
    let pythonw_path = required_conformance_path("WEBGPT_CUA_PYTHONW_EXE");
    let pythonw = pythonw_path.as_path();
    let script = root.join("tk-private-fixture.py");
    let session = "phase-f-tk-negative";
    let registry = runtime_scoped_registry();

    let start = registry
        .invoke("start_session", json!({ "session": session }))
        .await;
    assert_ok("Tk negative start_session", &start);

    let launch = registry
        .invoke(
            "launch_app",
            json!({
                "session": session,
                "isolation_mode": "private_desktop",
                "path": pythonw.to_string_lossy(),
                "additional_arguments": [script.to_string_lossy()],
            }),
        )
        .await;
    assert_ok("Tk negative launch_app", &launch);
    let launch_json = structured(&launch);
    let pid = launch_json["pid"].as_u64().expect("Tk launch pid") as u32;
    let window_id = launch_json["windows"]
        .as_array()
        .expect("Tk launch windows")
        .iter()
        .find(|window| {
            window["title"]
                .as_str()
                .is_some_and(|title| title.contains("WebGPT Tk Private Fixture"))
        })
        .and_then(|window| window["window_id"].as_u64())
        .expect("visible Tk private HWND");
    let process = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, pid) };
    assert!(!process.is_null(), "cannot open Tk pid {pid}");

    let snapshot = registry
        .invoke(
            "get_window_state",
            json!({
                "session": session,
                "pid": pid,
                "window_id": window_id,
                "include_screenshot": false,
            }),
        )
        .await;
    assert_ok("Tk negative snapshot", &snapshot);
    let invoke_token = find_token(structured(&snapshot), &["최소화", "Minimize"], &[]);

    let blocked = registry
        .invoke(
            "click",
            json!({
                "session": session,
                "pid": pid,
                "window_id": window_id,
                "element_token": invoke_token,
                "verify_visual": false,
            }),
        )
        .await;
    assert_eq!(blocked.is_error, Some(true), "Tk Invoke must fail closed");
    let blocked_json = structured(&blocked);
    assert_eq!(blocked_json["code"], "private_action_unsupported");
    assert_eq!(blocked_json["provider"], "Tk");
    assert_eq!(blocked_json["action"], "Invoke");
    assert_eq!(blocked_json["target_launched"], true);

    let end = registry
        .invoke("end_session", json!({ "session": session }))
        .await;
    assert_ok("Tk negative end_session", &end);
    assert_eq!(
        unsafe { WaitForSingleObject(process, 5_000) },
        WAIT_OBJECT_0,
        "Tk negative end_session must tear down the private Job/process tree"
    );
    unsafe {
        let _ = CloseHandle(process);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires WEBGPT_CUA_CONFORMANCE_ROOT and WEBGPT_CUA_PYTHONW_EXE"]
async fn phase_f_qt_select_is_blocked_by_real_cua_policy() {
    let _serial = private_desktop_test_guard().await;
    let root = conformance_root();
    let pythonw_path = required_conformance_path("WEBGPT_CUA_PYTHONW_EXE");
    let pythonw = pythonw_path.as_path();
    let session = "phase-f-qt-select-negative";
    let qt_runtime = root.join("qt-python-runtime");
    let code = format!(
        "import sys; sys.path.insert(0, r'{}'); from PySide6.QtWidgets import QApplication,QWidget,QVBoxLayout,QComboBox,QLabel; from PySide6.QtCore import QTimer; app=QApplication(sys.argv); win=QWidget(); win.setWindowTitle('WebGPT Qt Select Negative Fixture'); layout=QVBoxLayout(win); combo=QComboBox(); combo.setAccessibleName('Fixture Select'); combo.setObjectName('fixture-select'); combo.addItems(['Red','Green','Blue']); layout.addWidget(combo); status=QLabel('idle'); status.setAccessibleName('idle'); layout.addWidget(status); combo.currentTextChanged.connect(lambda v: (status.setText('selected='+v), status.setAccessibleName('selected='+v))); win.resize(640,360); win.show(); QTimer.singleShot(250, combo.showPopup); sys.exit(app.exec())",
        qt_runtime.to_string_lossy()
    );
    let registry = runtime_scoped_registry();

    let start = registry
        .invoke("start_session", json!({ "session": session }))
        .await;
    assert_ok("Qt Select negative start_session", &start);

    let launch = registry
        .invoke(
            "launch_app",
            json!({
                "session": session,
                "isolation_mode": "private_desktop",
                "path": pythonw.to_string_lossy(),
                "additional_arguments": ["-c", code],
            }),
        )
        .await;
    assert_ok("Qt Select negative launch_app", &launch);
    let launch_json = structured(&launch);
    let pid = launch_json["pid"].as_u64().expect("Qt Select launch pid") as u32;
    let window_id = launch_json["windows"]
        .as_array()
        .expect("Qt Select launch windows")
        .iter()
        .find(|window| {
            window["title"]
                .as_str()
                .is_some_and(|title| title.contains("WebGPT Qt Select Negative Fixture"))
        })
        .and_then(|window| window["window_id"].as_u64())
        .expect("visible Qt Select private HWND");
    let process = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, pid) };
    assert!(!process.is_null(), "cannot open Qt Select pid {pid}");

    let mut blue_token = None;
    let mut last_snapshot = None;
    for _ in 0..40 {
        let snapshot = registry
            .invoke(
                "get_window_state",
                json!({
                    "session": session,
                    "pid": pid,
                    "window_id": window_id,
                    "include_screenshot": false,
                }),
            )
            .await;
        assert_ok("Qt Select negative snapshot", &snapshot);
        let snapshot_json = structured(&snapshot).clone();
        if let Some(token) = snapshot_json["elements"].as_array().and_then(|elements| {
            elements.iter().find_map(|element| {
                if element["label"].as_str() == Some("Blue") {
                    element["element_token"].as_str().map(str::to_owned)
                } else {
                    None
                }
            })
        }) {
            blue_token = Some(token);
            last_snapshot = Some(snapshot_json);
            break;
        }
        last_snapshot = Some(snapshot_json);
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    let blue_token = blue_token.unwrap_or_else(|| {
        panic!("Qt Select list item did not become UIA-visible: {last_snapshot:?}")
    });

    let blocked = registry
        .invoke(
            "click",
            json!({
                "session": session,
                "pid": pid,
                "window_id": window_id,
                "element_token": blue_token,
                "verify_visual": false,
            }),
        )
        .await;
    assert_eq!(blocked.is_error, Some(true), "Qt Select must fail closed");
    let blocked_json = structured(&blocked);
    assert_eq!(blocked_json["code"], "private_action_unsupported");
    assert_eq!(blocked_json["provider"], "Qt611Widgets");
    assert_eq!(blocked_json["action"], "Select");
    assert_eq!(blocked_json["target_launched"], true);

    let end = registry
        .invoke("end_session", json!({ "session": session }))
        .await;
    assert_ok("Qt Select negative end_session", &end);
    assert_eq!(
        unsafe { WaitForSingleObject(process, 5_000) },
        WAIT_OBJECT_0,
        "Qt Select negative end_session must tear down the private Job/process tree"
    );
    unsafe {
        let _ = CloseHandle(process);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn phase_f_packaged_launch_is_rejected_before_process_creation() {
    let _serial = private_desktop_test_guard().await;
    let session = "phase-f-packaged-negative";
    let registry = runtime_scoped_registry();

    let start = registry
        .invoke("start_session", json!({ "session": session }))
        .await;
    assert_ok("packaged negative start_session", &start);

    let blocked = registry
        .invoke(
            "launch_app",
            json!({
                "session": session,
                "isolation_mode": "private_desktop",
                "aumid": "Microsoft.WindowsNotepad_8wekyb3d8bbwe!App",
            }),
        )
        .await;
    assert_eq!(
        blocked.is_error,
        Some(true),
        "packaged private launch must fail closed"
    );
    let blocked_json = structured(&blocked);
    assert_eq!(blocked_json["code"], "private_launch_unsupported");
    assert_eq!(blocked_json["isolation_mode"], "private_desktop");
    assert_eq!(blocked_json["target_launched"], false);

    let end = registry
        .invoke("end_session", json!({ "session": session }))
        .await;
    assert_ok("packaged negative end_session", &end);
}
