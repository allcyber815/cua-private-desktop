//! Windows private-desktop process ownership core.
//!
//! Each process is created suspended on the private desktop, assigned to the
//! environment Job Object, verified as a Job member, and only then resumed.
//! KILL_ON_JOB_CLOSE owns the process tree and lpDesktop selects the target
//! desktop without switching the user's input desktop.
//!
//! This file intentionally depends only on std + Win32 FFI so the ownership
//! primitive can be tested independently from the wider CUA dependency graph.

#![allow(non_snake_case)]

use std::ffi::{c_void, OsStr, OsString};
use std::io;
use std::mem::{size_of, size_of_val};
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::path::Path;
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

type Bool = i32;
type Dword = u32;
type Handle = *mut c_void;
type Hdesk = Handle;

// Match the verified B3 canary desktop authority. Holding this access mask
// does not switch the user's input desktop; the private lane still never calls
// SwitchDesktop. The broader handle rights are required by some UIA provider
// paths (notably WinUI 3) even when enumeration/window creation alone works.
const PRIVATE_DESKTOP_ACCESS: Dword = 0x01ff; // DESKTOP_ALL_ACCESS

const JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE: Dword = 0x0000_2000;
const JOB_OBJECT_BASIC_PROCESS_ID_LIST_CLASS: i32 = 3;
const JOB_OBJECT_EXTENDED_LIMIT_INFORMATION_CLASS: i32 = 9;
const DUPLICATE_SAME_ACCESS: Dword = 0x0000_0002;
const JOB_PID_QUERY_CAPACITY: usize = 1024;

const CREATE_SUSPENDED: Dword = 0x0000_0004;
const CREATE_UNICODE_ENVIRONMENT: Dword = 0x0000_0400;

const WAIT_OBJECT_0: Dword = 0;
const WAIT_TIMEOUT: Dword = 258;
const INFINITE: Dword = 0xffff_ffff;
const UOI_NAME: i32 = 2;

static NEXT_DESKTOP_ID: AtomicU64 = AtomicU64::new(1);

#[repr(C)]
#[derive(Clone, Copy)]
struct StartupInfoW {
    cb: Dword,
    lpReserved: *mut u16,
    lpDesktop: *mut u16,
    lpTitle: *mut u16,
    dwX: Dword,
    dwY: Dword,
    dwXSize: Dword,
    dwYSize: Dword,
    dwXCountChars: Dword,
    dwYCountChars: Dword,
    dwFillAttribute: Dword,
    dwFlags: Dword,
    wShowWindow: u16,
    cbReserved2: u16,
    lpReserved2: *mut u8,
    hStdInput: Handle,
    hStdOutput: Handle,
    hStdError: Handle,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct ProcessInformation {
    process: Handle,
    thread: Handle,
    process_id: Dword,
    thread_id: Dword,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct JobObjectBasicLimitInformation {
    per_process_user_time_limit: i64,
    per_job_user_time_limit: i64,
    limit_flags: Dword,
    minimum_working_set_size: usize,
    maximum_working_set_size: usize,
    active_process_limit: Dword,
    affinity: usize,
    priority_class: Dword,
    scheduling_class: Dword,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct IoCounters {
    read_operation_count: u64,
    write_operation_count: u64,
    other_operation_count: u64,
    read_transfer_count: u64,
    write_transfer_count: u64,
    other_transfer_count: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct JobObjectExtendedLimitInformation {
    basic_limit_information: JobObjectBasicLimitInformation,
    io_info: IoCounters,
    process_memory_limit: usize,
    job_memory_limit: usize,
    peak_process_memory_used: usize,
    peak_job_memory_used: usize,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Rect {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PrivateWindowInfo {
    pub hwnd: usize,
    pub pid: u32,
    pub title: String,
    pub class_name: String,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub is_on_screen: bool,
    pub minimized: bool,
}

#[link(name = "user32")]
unsafe extern "system" {
    fn CreateDesktopW(
        desktop: *const u16,
        device: *const u16,
        devmode: *mut c_void,
        flags: Dword,
        desired_access: Dword,
        security_attributes: *const c_void,
    ) -> Hdesk;
    fn CloseDesktop(desktop: Hdesk) -> Bool;
    fn EnumDesktopWindows(
        desktop: Hdesk,
        callback: Option<unsafe extern "system" fn(Handle, isize) -> Bool>,
        lparam: isize,
    ) -> Bool;
    fn GetWindowThreadProcessId(hwnd: Handle, pid: *mut Dword) -> Dword;
    fn GetWindowTextW(hwnd: Handle, text: *mut u16, max_count: i32) -> i32;
    fn GetClassNameW(hwnd: Handle, class_name: *mut u16, max_count: i32) -> i32;
    fn GetWindowRect(hwnd: Handle, rect: *mut Rect) -> Bool;
    fn IsWindowVisible(hwnd: Handle) -> Bool;
    fn IsIconic(hwnd: Handle) -> Bool;
    fn SetThreadDesktop(desktop: Hdesk) -> Bool;
    fn GetThreadDesktop(thread_id: Dword) -> Hdesk;
    fn GetUserObjectInformationW(
        object: Handle,
        index: i32,
        information: *mut c_void,
        length: Dword,
        needed: *mut Dword,
    ) -> Bool;
}

#[link(name = "kernel32")]
unsafe extern "system" {
    fn CreateJobObjectW(attributes: *const c_void, name: *const u16) -> Handle;
    fn SetInformationJobObject(
        job: Handle,
        information_class: i32,
        information: *const c_void,
        information_length: Dword,
    ) -> Bool;
    fn TerminateJobObject(job: Handle, exit_code: Dword) -> Bool;
    fn IsProcessInJob(process: Handle, job: Handle, result: *mut Bool) -> Bool;
    fn QueryInformationJobObject(
        job: Handle,
        information_class: i32,
        information: *mut c_void,
        information_length: Dword,
        return_length: *mut Dword,
    ) -> Bool;
    fn GetCurrentProcess() -> Handle;
    fn DuplicateHandle(
        source_process: Handle,
        source_handle: Handle,
        target_process: Handle,
        target_handle: *mut Handle,
        desired_access: Dword,
        inherit_handle: Bool,
        options: Dword,
    ) -> Bool;

    fn CreateProcessW(
        application_name: *const u16,
        command_line: *mut u16,
        process_attributes: *const c_void,
        thread_attributes: *const c_void,
        inherit_handles: Bool,
        creation_flags: Dword,
        environment: *const c_void,
        current_directory: *const u16,
        startup_info: *const StartupInfoW,
        process_information: *mut ProcessInformation,
    ) -> Bool;

    fn AssignProcessToJobObject(job: Handle, process: Handle) -> Bool;
    fn ResumeThread(thread: Handle) -> Dword;
    fn TerminateProcess(process: Handle, exit_code: Dword) -> Bool;
    fn WaitForSingleObject(handle: Handle, milliseconds: Dword) -> Dword;
    fn GetCurrentThreadId() -> Dword;
    fn SetLastError(error: Dword);
}

struct DesktopHandle(Hdesk);

// A desktop handle is a kernel object handle. Moving the owner between threads
// does not attach those threads to the desktop; SetThreadDesktop remains an
// explicit later responsibility of the environment actor.
unsafe impl Send for DesktopHandle {}

impl Drop for DesktopHandle {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseDesktop(self.0);
        }
    }
}

pub(super) fn attach_current_thread_to(desktop_addr: usize) -> io::Result<()> {
    check(unsafe { SetThreadDesktop(desktop_addr as Hdesk) })
}

pub(super) fn current_thread_desktop_name() -> io::Result<String> {
    let desktop = unsafe { GetThreadDesktop(GetCurrentThreadId()) };
    if desktop.is_null() {
        return Err(io::Error::last_os_error());
    }
    let mut needed = 0u32;
    unsafe {
        let _ = GetUserObjectInformationW(desktop, UOI_NAME, null_mut(), 0, &mut needed);
    }
    if needed == 0 {
        return Err(io::Error::last_os_error());
    }
    let mut buffer = vec![0u16; ((needed as usize) / 2).max(64) + 2];
    check(unsafe {
        GetUserObjectInformationW(
            desktop,
            UOI_NAME,
            buffer.as_mut_ptr().cast(),
            (buffer.len() * 2) as Dword,
            &mut needed,
        )
    })?;
    let end = buffer
        .iter()
        .position(|&value| value == 0)
        .unwrap_or(buffer.len());
    Ok(String::from_utf16_lossy(&buffer[..end]))
}

/// Session-scoped ownership primitive for one private Win32 desktop + Job.
pub struct PrivateDesktopCore {
    job: OwnedHandle,
    _desktop: DesktopHandle,
    name: String,
}

impl PrivateDesktopCore {
    pub fn create(prefix: &str) -> io::Result<Self> {
        let unique = NEXT_DESKTOP_ID.fetch_add(1, Ordering::Relaxed);
        let name = format!("{prefix}-{}-{unique}", std::process::id());
        let desktop_name = wide(OsStr::new(&name))?;

        let desktop = unsafe {
            CreateDesktopW(
                desktop_name.as_ptr(),
                null(),
                null_mut(),
                0,
                PRIVATE_DESKTOP_ACCESS,
                null(),
            )
        };
        if desktop.is_null() {
            return Err(io::Error::last_os_error());
        }
        let desktop = DesktopHandle(desktop);

        let job = owned(unsafe { CreateJobObjectW(null(), null()) })?;
        let mut limits = JobObjectExtendedLimitInformation::default();
        limits.basic_limit_information.limit_flags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        check(unsafe {
            SetInformationJobObject(
                raw(&job),
                JOB_OBJECT_EXTENDED_LIMIT_INFORMATION_CLASS,
                &limits as *const _ as *const c_void,
                size_of_val(&limits) as Dword,
            )
        })?;

        Ok(Self {
            job,
            _desktop: desktop,
            name,
        })
    }

    pub fn desktop_name(&self) -> &str {
        &self.name
    }

    pub(super) fn desktop_handle_addr(&self) -> usize {
        self._desktop.0 as usize
    }

    pub(super) fn attach_current_thread(&self) -> io::Result<()> {
        attach_current_thread_to(self.desktop_handle_addr())
    }

    pub fn duplicate_job_probe(&self) -> io::Result<PrivateJobProbe> {
        let process = unsafe { GetCurrentProcess() };
        let mut duplicate = null_mut();
        check(unsafe {
            DuplicateHandle(
                process,
                raw(&self.job),
                process,
                &mut duplicate,
                0,
                0,
                DUPLICATE_SAME_ACCESS,
            )
        })?;
        Ok(PrivateJobProbe {
            job: owned(duplicate)?,
        })
    }

    pub fn windows_for_pid(&self, pid: u32) -> io::Result<Vec<PrivateWindowInfo>> {
        struct EnumContext {
            pid: u32,
            windows: Vec<PrivateWindowInfo>,
        }

        unsafe extern "system" fn collect(hwnd: Handle, lparam: isize) -> Bool {
            let context = &mut *(lparam as *mut EnumContext);
            let mut owner_pid = 0u32;
            let _ = GetWindowThreadProcessId(hwnd, &mut owner_pid);
            if owner_pid != context.pid {
                return 1;
            }

            fn read_text(hwnd: Handle) -> String {
                let mut buffer = [0u16; 512];
                let len = unsafe { GetWindowTextW(hwnd, buffer.as_mut_ptr(), buffer.len() as i32) };
                if len <= 0 {
                    String::new()
                } else {
                    String::from_utf16_lossy(&buffer[..len as usize])
                }
            }

            fn read_class(hwnd: Handle) -> String {
                let mut buffer = [0u16; 256];
                let len = unsafe { GetClassNameW(hwnd, buffer.as_mut_ptr(), buffer.len() as i32) };
                if len <= 0 {
                    String::new()
                } else {
                    String::from_utf16_lossy(&buffer[..len as usize])
                }
            }

            let mut rect = Rect::default();
            let rect_ok = unsafe { GetWindowRect(hwnd, &mut rect) } != 0;
            let minimized = unsafe { IsIconic(hwnd) } != 0;
            let visible = unsafe { IsWindowVisible(hwnd) } != 0;
            let width = if rect_ok { rect.right - rect.left } else { 0 };
            let height = if rect_ok { rect.bottom - rect.top } else { 0 };

            context.windows.push(PrivateWindowInfo {
                hwnd: hwnd as usize,
                pid: owner_pid,
                title: read_text(hwnd),
                class_name: read_class(hwnd),
                x: if rect_ok { rect.left } else { 0 },
                y: if rect_ok { rect.top } else { 0 },
                width,
                height,
                is_on_screen: visible && !minimized && width > 0 && height > 0,
                minimized,
            });
            1
        }

        let mut context = EnumContext {
            pid,
            windows: Vec::new(),
        };
        unsafe { SetLastError(0) };
        let ok = unsafe {
            EnumDesktopWindows(
                self._desktop.0,
                Some(collect),
                &mut context as *mut EnumContext as isize,
            )
        };
        if ok == 0 {
            let error = io::Error::last_os_error();
            if error.raw_os_error().unwrap_or(0) != 0 {
                return Err(error);
            }
        }
        Ok(context.windows)
    }

    pub fn wait_for_windows(
        &self,
        pid: u32,
        timeout: Duration,
    ) -> io::Result<Vec<PrivateWindowInfo>> {
        let deadline = std::time::Instant::now() + timeout;
        loop {
            let windows = self.windows_for_pid(pid)?;
            // Auxiliary IME/input-indicator HWNDs can become visible before
            // the application's real top-level window (notably XAML Islands).
            // Do not let the first arbitrary visible HWND satisfy launch
            // readiness. A titled restored-visible root-PID window is the
            // minimum generic top-level identity signal; if an application
            // intentionally has no titled window, the bounded timeout still
            // returns the enumerated set for explicit caller handling.
            if windows
                .iter()
                .any(|window| window.is_on_screen && !window.title.trim().is_empty())
                || std::time::Instant::now() >= deadline
            {
                return Ok(windows);
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    pub fn spawn_direct(
        &self,
        program: &Path,
        args: &[OsString],
        current_directory: Option<&Path>,
    ) -> io::Result<PrivateChild> {
        // Preserve a normal DOS path for CreateProcessW. Rust's Windows
        // canonicalize() commonly returns a verbatim \\?\ path; unpackaged
        // WinUI can launch through that spelling yet expose a broken cross-
        // process UIA provider. B3 uses the ordinary C:\... spelling.
        let program = if program.is_absolute() {
            program.to_path_buf()
        } else {
            std::env::current_dir()?.join(program)
        };
        let application = wide(program.as_os_str())?;
        let mut command_line = quoted(program.as_os_str())?;
        for arg in args {
            command_line.push(b' ' as u16);
            command_line.extend(quoted(arg.as_os_str())?);
        }
        command_line.push(0);

        let mut desktop_spec = wide(OsStr::new(&format!("winsta0\\{}", self.name)))?;
        let cwd = current_directory
            .map(|path| wide(path.as_os_str()))
            .transpose()?;

        // Match the verified B3 launch invariant exactly:
        // create suspended, assign to the owned Job, prove membership, then
        // resume the primary thread. No user code runs before Job ownership.
        let mut startup: StartupInfoW = unsafe { std::mem::zeroed() };
        startup.cb = size_of_val(&startup) as Dword;
        startup.lpDesktop = desktop_spec.as_mut_ptr();

        let mut info: ProcessInformation = unsafe { std::mem::zeroed() };
        check(unsafe {
            CreateProcessW(
                application.as_ptr(),
                command_line.as_mut_ptr(),
                null(),
                null(),
                0,
                CREATE_SUSPENDED | CREATE_UNICODE_ENVIRONMENT,
                null(),
                cwd.as_ref().map_or(null(), |value| value.as_ptr()),
                &startup,
                &mut info,
            )
        })?;

        let process = owned(info.process)?;
        let thread = owned(info.thread)?;

        if let Err(error) =
            check(unsafe { AssignProcessToJobObject(raw(&self.job), raw(&process)) })
        {
            unsafe {
                let _ = TerminateProcess(raw(&process), 1);
            }
            return Err(error);
        }

        let mut in_job = 0;
        if let Err(error) =
            check(unsafe { IsProcessInJob(raw(&process), raw(&self.job), &mut in_job) })
        {
            unsafe {
                let _ = TerminateProcess(raw(&process), 1);
            }
            return Err(error);
        }
        if in_job == 0 {
            unsafe {
                let _ = TerminateProcess(raw(&process), 1);
            }
            return Err(io::Error::other(
                "private process was not a Job member before primary-thread resume",
            ));
        }

        let prior_suspend_count = unsafe { ResumeThread(raw(&thread)) };
        if prior_suspend_count == Dword::MAX {
            unsafe {
                let _ = TerminateProcess(raw(&process), 1);
            }
            return Err(io::Error::last_os_error());
        }
        drop(thread);

        Ok(PrivateChild {
            process,
            pid: info.process_id,
        })
    }
}

impl Drop for PrivateDesktopCore {
    fn drop(&mut self) {
        // Terminate the owned process tree before DesktopHandle is closed.
        unsafe {
            let _ = TerminateJobObject(raw(&self.job), 1);
        }
    }
}

pub struct PrivateJobProbe {
    job: OwnedHandle,
}

impl PrivateJobProbe {
    pub fn contains_pid(&self, pid: u32) -> io::Result<bool> {
        if pid == 0 {
            return Ok(false);
        }
        let bytes = 8 + JOB_PID_QUERY_CAPACITY * size_of::<usize>();
        let mut buffer = vec![0u8; bytes];
        let mut returned = 0u32;
        check(unsafe {
            QueryInformationJobObject(
                raw(&self.job),
                JOB_OBJECT_BASIC_PROCESS_ID_LIST_CLASS,
                buffer.as_mut_ptr().cast(),
                bytes as Dword,
                &mut returned,
            )
        })?;
        if returned < 8 {
            return Err(io::Error::other(
                "JobObjectBasicProcessIdList returned a truncated header",
            ));
        }
        let count = u32::from_ne_bytes(buffer[4..8].try_into().unwrap()) as usize;
        if count > JOB_PID_QUERY_CAPACITY {
            return Err(io::Error::other(format!(
                "JobObjectBasicProcessIdList exceeded bounded capacity: {count} > {JOB_PID_QUERY_CAPACITY}"
            )));
        }
        for index in 0..count {
            let offset = 8 + index * size_of::<usize>();
            let value = if size_of::<usize>() == 8 {
                u64::from_ne_bytes(buffer[offset..offset + 8].try_into().unwrap())
            } else {
                u32::from_ne_bytes(buffer[offset..offset + 4].try_into().unwrap()) as u64
            };
            if value == pid as u64 {
                return Ok(true);
            }
        }
        Ok(false)
    }
}

pub struct PrivateChild {
    process: OwnedHandle,
    pid: Dword,
}

impl PrivateChild {
    pub fn id(&self) -> u32 {
        self.pid
    }

    pub fn wait_timeout(&self, timeout: Duration) -> io::Result<bool> {
        let millis = timeout.as_millis().min(Dword::MAX as u128) as Dword;
        match unsafe { WaitForSingleObject(raw(&self.process), millis) } {
            WAIT_OBJECT_0 => Ok(true),
            WAIT_TIMEOUT => Ok(false),
            _ => Err(io::Error::last_os_error()),
        }
    }

    pub fn wait(&self) -> io::Result<()> {
        match unsafe { WaitForSingleObject(raw(&self.process), INFINITE) } {
            WAIT_OBJECT_0 => Ok(()),
            _ => Err(io::Error::last_os_error()),
        }
    }
}

fn check(result: Bool) -> io::Result<()> {
    if result == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

fn raw(handle: &OwnedHandle) -> Handle {
    handle.as_raw_handle() as Handle
}

fn owned(handle: Handle) -> io::Result<OwnedHandle> {
    if handle.is_null() {
        Err(io::Error::last_os_error())
    } else {
        Ok(unsafe { OwnedHandle::from_raw_handle(handle) })
    }
}

fn wide(value: &OsStr) -> io::Result<Vec<u16>> {
    let mut value: Vec<_> = value.encode_wide().collect();
    if value.contains(&0) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "NUL in Windows process argument",
        ));
    }
    value.push(0);
    Ok(value)
}

/// Quote one Windows C-runtime command-line argument.
fn quoted(value: &OsStr) -> io::Result<Vec<u16>> {
    let value = wide(value)?;
    let mut output = vec![b'"' as u16];
    let mut slashes = 0usize;

    for &ch in &value[..value.len() - 1] {
        if ch == b'\\' as u16 {
            slashes += 1;
            continue;
        }

        output.extend(std::iter::repeat_n(b'\\' as u16, slashes));
        if ch == b'"' as u16 {
            output.extend(std::iter::repeat_n(b'\\' as u16, slashes + 1));
        }
        output.push(ch);
        slashes = 0;
    }

    output.extend(std::iter::repeat_n(b'\\' as u16, slashes * 2));
    output.push(b'"' as u16);
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;
    use std::thread;
    use std::time::{Instant, SystemTime, UNIX_EPOCH};

    const TEST_DIR_ENV: &str = "CUA_PRIVATE_DESKTOP_CORE_TEST_DIR";
    const PROCESS_SYNCHRONIZE: Dword = 0x0010_0000;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn OpenProcess(desired_access: Dword, inherit: Bool, pid: Dword) -> Handle;
    }

    fn current_desktop_name() -> String {
        current_thread_desktop_name().unwrap()
    }

    fn test_dir() -> PathBuf {
        std::env::var_os(TEST_DIR_ENV)
            .expect("test helper requires test dir")
            .into()
    }

    fn wait_for_text(path: &Path) -> String {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Ok(text) = fs::read_to_string(path) {
                if !text.is_empty() {
                    return text;
                }
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for {}",
                path.display()
            );
            thread::sleep(Duration::from_millis(20));
        }
    }

    fn helper_args(test_name: &str) -> Vec<OsString> {
        vec![
            test_name.into(),
            "--ignored".into(),
            "--nocapture".into(),
            "--test-threads=1".into(),
        ]
    }

    #[test]
    #[ignore = "internal subprocess helper"]
    fn private_leaf_helper() {
        fs::write(test_dir().join("leaf.pid"), std::process::id().to_string()).unwrap();
        thread::sleep(Duration::from_secs(30));
    }

    #[test]
    #[ignore = "internal subprocess helper"]
    fn private_tree_helper() {
        let dir = test_dir();
        fs::write(dir.join("desktop.txt"), current_desktop_name()).unwrap();

        let mut leaf = std::process::Command::new(std::env::current_exe().unwrap())
            .args(helper_args("tests::private_leaf_helper"))
            .spawn()
            .unwrap();
        let _ = wait_for_text(&dir.join("leaf.pid"));
        let _ = leaf.wait();
    }

    #[test]
    fn private_environment_owns_desktop_and_process_tree() {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir =
            std::env::temp_dir().join(format!("cua-private-core-{}-{suffix}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        std::env::set_var(TEST_DIR_ENV, &dir);

        let environment = PrivateDesktopCore::create("CuaPrivateCoreTest").unwrap();
        let child = environment
            .spawn_direct(
                &std::env::current_exe().unwrap(),
                &helper_args("tests::private_tree_helper"),
                None,
            )
            .unwrap();

        let desktop = wait_for_text(&dir.join("desktop.txt"));
        assert_eq!(desktop, environment.desktop_name());

        let leaf_pid: u32 = wait_for_text(&dir.join("leaf.pid")).trim().parse().unwrap();
        let leaf = owned(unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, leaf_pid) }).unwrap();

        drop(environment);

        assert!(child.wait_timeout(Duration::from_secs(5)).unwrap());
        assert_eq!(
            unsafe { WaitForSingleObject(raw(&leaf), 5_000) },
            WAIT_OBJECT_0,
            "Job close must terminate descendants, not only the root process"
        );

        std::env::remove_var(TEST_DIR_ENV);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn quoted_arguments_preserve_backslashes_and_quotes() {
        let q = quoted(OsStr::new(r#"C:\path with space\a\"b"#)).unwrap();
        let text = String::from_utf16_lossy(&q);
        assert!(text.starts_with('"'));
        assert!(text.ends_with('"'));
        assert!(text.contains(r#"path with space"#));
        assert!(text.contains(r#"\\\""#));
    }
}
