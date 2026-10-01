//! Session-owned private-desktop runtime.
//!
//! One OS actor thread owns each private execution environment. The actor is
//! attached to the private Win32 desktop before it creates its current-thread
//! Tokio runtime. Tokio blocking workers are attached to the same desktop at
//! thread start. Commands are serialized so private GUI mutations cannot race.

use super::win32_private::{
    attach_current_thread_to, current_thread_desktop_name, PrivateChild, PrivateDesktopCore,
    PrivateJobProbe, PrivateWindowInfo,
};
use super::{IsolationMode, PrivateFramework};
use std::cell::Cell;
use std::collections::HashMap;
use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;
use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED};

thread_local! {
    static PRIVATE_COM_STA_INITIALIZED: Cell<bool> = const { Cell::new(false) };
}

const ACTOR_REPLY_TIMEOUT: Duration = Duration::from_secs(20);
// Unpackaged WinUI/XAML Island providers can take longer than five seconds
// to create their first titled HWND on a cold Windows App SDK start. Keep the
// readiness wait bounded below the actor reply timeout, but long enough for a
// real cold-start window rather than returning an empty successful launch.
const WINDOW_DISCOVERY_TIMEOUT: Duration = Duration::from_secs(12);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PrivateLaunchResult {
    pub pid: u32,
    pub windows: Vec<PrivateWindowInfo>,
    pub desktop_name: String,
    pub actor_desktop_name: String,
    pub blocking_desktop_name: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PrivateEnvironmentReceipt {
    pub isolation_mode: &'static str,
    pub desktop_name: String,
    pub actor_affinity_verified: bool,
    pub blocking_affinity_verified: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PrivateTargetProfile {
    pub exe_basename: String,
    pub class_name: String,
    pub framework: PrivateFramework,
    pub worker_integrity_rid: u32,
    pub target_integrity_rid: u32,
}

struct StartupReceipt {
    desktop_name: String,
    actor_desktop_name: String,
    blocking_desktop_name: String,
}

enum ActorCommand {
    LaunchDirect {
        program: PathBuf,
        args: Vec<OsString>,
        current_directory: Option<PathBuf>,
        reply: mpsc::SyncSender<io::Result<PrivateLaunchResult>>,
    },
    WindowsForPid {
        pid: u32,
        reply: mpsc::SyncSender<io::Result<Vec<PrivateWindowInfo>>>,
    },
    WindowForPidExact {
        pid: u32,
        hwnd: u64,
        reply: mpsc::SyncSender<io::Result<Option<PrivateWindowInfo>>>,
    },
    SpawnDirectHelper {
        program: PathBuf,
        args: Vec<OsString>,
        current_directory: Option<PathBuf>,
        reply: mpsc::SyncSender<io::Result<PrivateChild>>,
    },
    DuplicateJobProbe {
        reply: mpsc::SyncSender<io::Result<PrivateJobProbe>>,
    },
    RunAttachedBlocking {
        task: Box<dyn FnOnce(&tokio::runtime::Runtime) + Send + 'static>,
    },
    Shutdown {
        reply: mpsc::SyncSender<()>,
    },
}

pub struct PrivateDesktopRuntime {
    sender: mpsc::Sender<ActorCommand>,
    join: Mutex<Option<JoinHandle<()>>>,
    target_profiles: Mutex<HashMap<(u32, u64), PrivateTargetProfile>>,
    desktop_name: String,
    actor_desktop_name: String,
    blocking_desktop_name: String,
}

impl PrivateDesktopRuntime {
    pub fn start() -> io::Result<Arc<Self>> {
        let (sender, receiver) = mpsc::channel();
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);

        let join = thread::Builder::new()
            .name("cua-private-desktop-actor".into())
            .spawn(move || actor_main(receiver, ready_tx))?;

        let startup = match ready_rx.recv_timeout(ACTOR_REPLY_TIMEOUT) {
            Ok(Ok(startup)) => startup,
            Ok(Err(error)) => {
                let _ = join.join();
                return Err(io::Error::other(error));
            }
            Err(error) => {
                let _ = join.join();
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    format!("private desktop actor startup did not complete: {error}"),
                ));
            }
        };

        Ok(Arc::new(Self {
            sender,
            join: Mutex::new(Some(join)),
            target_profiles: Mutex::new(HashMap::new()),
            desktop_name: startup.desktop_name,
            actor_desktop_name: startup.actor_desktop_name,
            blocking_desktop_name: startup.blocking_desktop_name,
        }))
    }

    pub fn receipt(&self) -> PrivateEnvironmentReceipt {
        PrivateEnvironmentReceipt {
            isolation_mode: IsolationMode::PrivateDesktop.wire_value(),
            desktop_name: self.desktop_name.clone(),
            actor_affinity_verified: self.actor_desktop_name == self.desktop_name,
            blocking_affinity_verified: self.blocking_desktop_name == self.desktop_name,
        }
    }

    pub fn cached_target_profile(
        &self,
        pid: u32,
        hwnd: u64,
        class_name: &str,
    ) -> Option<PrivateTargetProfile> {
        let key = (pid, hwnd);
        let mut profiles = self.target_profiles.lock().ok()?;
        match profiles.get(&key) {
            Some(profile) if profile.class_name == class_name => Some(profile.clone()),
            Some(_) => {
                profiles.remove(&key);
                None
            }
            None => None,
        }
    }

    pub fn cache_target_profile(&self, pid: u32, hwnd: u64, profile: PrivateTargetProfile) {
        if let Ok(mut profiles) = self.target_profiles.lock() {
            profiles.insert((pid, hwnd), profile);
        }
    }

    pub fn launch_direct(
        &self,
        program: &Path,
        args: &[OsString],
        current_directory: Option<&Path>,
    ) -> io::Result<PrivateLaunchResult> {
        // A new process can eventually reuse an old numeric PID/HWND pair.
        // Drop all cached provider identities at this lifecycle boundary so a
        // later action can never inherit classification from a prior launch.
        if let Ok(mut profiles) = self.target_profiles.lock() {
            profiles.clear();
        }
        let (reply_tx, reply_rx) = mpsc::sync_channel(1);
        self.sender
            .send(ActorCommand::LaunchDirect {
                program: program.to_path_buf(),
                args: args.to_vec(),
                current_directory: current_directory.map(Path::to_path_buf),
                reply: reply_tx,
            })
            .map_err(|_| {
                io::Error::new(io::ErrorKind::BrokenPipe, "private desktop actor stopped")
            })?;
        reply_rx
            .recv_timeout(ACTOR_REPLY_TIMEOUT)
            .map_err(|error| {
                io::Error::new(
                    io::ErrorKind::TimedOut,
                    format!("private launch actor reply timed out: {error}"),
                )
            })?
    }

    pub fn windows_for_pid(&self, pid: u32) -> io::Result<Vec<PrivateWindowInfo>> {
        let (reply_tx, reply_rx) = mpsc::sync_channel(1);
        self.sender
            .send(ActorCommand::WindowsForPid {
                pid,
                reply: reply_tx,
            })
            .map_err(|_| {
                io::Error::new(io::ErrorKind::BrokenPipe, "private desktop actor stopped")
            })?;
        reply_rx
            .recv_timeout(ACTOR_REPLY_TIMEOUT)
            .map_err(|error| {
                io::Error::new(
                    io::ErrorKind::TimedOut,
                    format!("private window actor reply timed out: {error}"),
                )
            })?
    }

    pub fn window_for_pid_exact(
        &self,
        pid: u32,
        hwnd: u64,
    ) -> io::Result<Option<PrivateWindowInfo>> {
        let (reply_tx, reply_rx) = mpsc::sync_channel(1);
        self.sender
            .send(ActorCommand::WindowForPidExact {
                pid,
                hwnd,
                reply: reply_tx,
            })
            .map_err(|_| {
                io::Error::new(io::ErrorKind::BrokenPipe, "private desktop actor stopped")
            })?;
        reply_rx
            .recv_timeout(ACTOR_REPLY_TIMEOUT)
            .map_err(|error| {
                io::Error::new(
                    io::ErrorKind::TimedOut,
                    format!("private exact-window actor reply timed out: {error}"),
                )
            })?
    }

    pub fn spawn_direct_helper(
        &self,
        program: &Path,
        args: &[OsString],
        current_directory: Option<&Path>,
    ) -> io::Result<PrivateChild> {
        let (reply_tx, reply_rx) = mpsc::sync_channel(1);
        self.sender
            .send(ActorCommand::SpawnDirectHelper {
                program: program.to_path_buf(),
                args: args.to_vec(),
                current_directory: current_directory.map(Path::to_path_buf),
                reply: reply_tx,
            })
            .map_err(|_| {
                io::Error::new(io::ErrorKind::BrokenPipe, "private desktop actor stopped")
            })?;
        reply_rx
            .recv_timeout(ACTOR_REPLY_TIMEOUT)
            .map_err(|error| {
                io::Error::new(
                    io::ErrorKind::TimedOut,
                    format!("private helper actor reply timed out: {error}"),
                )
            })?
    }

    pub fn duplicate_job_probe(&self) -> io::Result<PrivateJobProbe> {
        let (reply_tx, reply_rx) = mpsc::sync_channel(1);
        self.sender
            .send(ActorCommand::DuplicateJobProbe { reply: reply_tx })
            .map_err(|_| {
                io::Error::new(io::ErrorKind::BrokenPipe, "private desktop actor stopped")
            })?;
        reply_rx
            .recv_timeout(ACTOR_REPLY_TIMEOUT)
            .map_err(|error| {
                io::Error::new(
                    io::ErrorKind::TimedOut,
                    format!("private Job probe actor reply timed out: {error}"),
                )
            })?
    }

    pub fn run_attached_blocking<T, F>(&self, task: F) -> anyhow::Result<T>
    where
        T: Send + 'static,
        F: FnOnce() -> anyhow::Result<T> + Send + 'static,
    {
        let (reply_tx, reply_rx) = mpsc::sync_channel(1);
        let actor_task = Box::new(move |runtime: &tokio::runtime::Runtime| {
            let result = runtime.block_on(async {
                tokio::task::spawn_blocking(task)
                    .await
                    .map_err(|error| anyhow::anyhow!("private blocking task panicked: {error}"))?
            });
            let _ = reply_tx.send(result);
        });
        self.sender
            .send(ActorCommand::RunAttachedBlocking { task: actor_task })
            .map_err(|_| anyhow::anyhow!("private desktop actor stopped"))?;
        reply_rx
            .recv_timeout(ACTOR_REPLY_TIMEOUT)
            .map_err(|error| anyhow::anyhow!("private blocking actor reply timed out: {error}"))?
    }

    pub fn shutdown(&self) -> Result<(), String> {
        let join = self.join.lock().unwrap().take();
        let Some(join) = join else {
            return Ok(());
        };

        let (reply_tx, reply_rx) = mpsc::sync_channel(1);
        let send_result = self.sender.send(ActorCommand::Shutdown { reply: reply_tx });
        if send_result.is_ok() {
            let _ = reply_rx.recv_timeout(Duration::from_secs(5));
        }

        join.join()
            .map_err(|_| "private desktop actor panicked during shutdown".to_string())
    }
}

impl Drop for PrivateDesktopRuntime {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

fn actor_main(
    receiver: mpsc::Receiver<ActorCommand>,
    ready: mpsc::SyncSender<Result<StartupReceipt, String>>,
) {
    let result = actor_initialize();
    let (core, runtime, startup) = match result {
        Ok(value) => value,
        Err(error) => {
            let _ = ready.send(Err(error.to_string()));
            return;
        }
    };

    if ready.send(Ok(startup)).is_err() {
        return;
    }

    while let Ok(command) = receiver.recv() {
        match command {
            ActorCommand::LaunchDirect {
                program,
                args,
                current_directory,
                reply,
            } => {
                let result = (|| {
                    let child = core
                        .spawn_direct(&program, &args, current_directory.as_deref())
                        .map_err(|error| {
                            io::Error::new(error.kind(), format!("spawn_direct failed: {error}"))
                        })?;
                    let pid = child.id();
                    let windows = core
                        .wait_for_windows(pid, WINDOW_DISCOVERY_TIMEOUT)
                        .map_err(|error| {
                            io::Error::new(
                                error.kind(),
                                format!("private HWND discovery failed for pid {pid}: {error}"),
                            )
                        })?;

                    // Re-run blocking affinity after a real target exists. The
                    // semantic/UIA route will later add exact HWND resolution on
                    // this worker; for Phase 1 this proves the blocking pool did
                    // not drift back to Default.
                    let blocking_desktop_name = runtime
                        .block_on(async {
                            tokio::task::spawn_blocking(current_thread_desktop_name)
                                .await
                                .map_err(|error| io::Error::other(error.to_string()))?
                        })
                        .map_err(|error| {
                            io::Error::new(
                                error.kind(),
                                format!("blocking desktop affinity probe failed: {error}"),
                            )
                        })?;
                    if blocking_desktop_name != core.desktop_name() {
                        return Err(io::Error::other(format!(
                            "blocking worker desktop drifted: expected {:?}, got {:?}",
                            core.desktop_name(),
                            blocking_desktop_name
                        )));
                    }

                    Ok(PrivateLaunchResult {
                        pid,
                        windows,
                        desktop_name: core.desktop_name().to_owned(),
                        actor_desktop_name: current_thread_desktop_name().map_err(|error| {
                            io::Error::new(
                                error.kind(),
                                format!("actor desktop affinity probe failed: {error}"),
                            )
                        })?,
                        blocking_desktop_name,
                    })
                })();
                let _ = reply.send(result);
            }
            ActorCommand::WindowsForPid { pid, reply } => {
                let _ = reply.send(core.windows_for_pid(pid));
            }
            ActorCommand::WindowForPidExact { pid, hwnd, reply } => {
                let _ = reply.send(core.window_for_pid_exact(pid, hwnd));
            }
            ActorCommand::SpawnDirectHelper {
                program,
                args,
                current_directory,
                reply,
            } => {
                let _ =
                    reply.send(core.spawn_direct(&program, &args, current_directory.as_deref()));
            }
            ActorCommand::DuplicateJobProbe { reply } => {
                let _ = reply.send(core.duplicate_job_probe());
            }
            ActorCommand::RunAttachedBlocking { task } => {
                task(&runtime);
            }
            ActorCommand::Shutdown { reply } => {
                let _ = reply.send(());
                break;
            }
        }
    }

    // Drop the runtime before the core so blocking workers are gone before the
    // Job and desktop handles are torn down.
    drop(runtime);
    drop(core);
}

fn actor_initialize() -> io::Result<(PrivateDesktopCore, tokio::runtime::Runtime, StartupReceipt)> {
    let core = PrivateDesktopCore::create("CuaPrivate")?;
    core.attach_current_thread()?;

    let actor_desktop_name = current_thread_desktop_name()?;
    if actor_desktop_name != core.desktop_name() {
        return Err(io::Error::other(format!(
            "actor desktop affinity failed: expected {:?}, got {:?}",
            core.desktop_name(),
            actor_desktop_name
        )));
    }

    let desktop_addr = core.desktop_handle_addr();
    let attach_failure: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    let attach_failure_for_hook = attach_failure.clone();

    let mut builder = tokio::runtime::Builder::new_current_thread();
    let startup_failure = attach_failure.clone();
    builder
        .enable_all()
        // Actor commands are serialized already. A single long-lived blocking
        // worker guarantees that A/B visual baseline, UIA/native mutation and C
        // post-capture stay on the same private-desktop STA, matching the B3
        // worker process contract instead of drifting across worker apartments.
        .max_blocking_threads(1)
        .thread_name("cua-private-blocking")
        .on_thread_start(move || {
            if let Err(error) = attach_current_thread_to(desktop_addr) {
                let mut slot = attach_failure_for_hook.lock().unwrap();
                if slot.is_none() {
                    *slot = Some(error.to_string());
                }
                return;
            }
            let hr = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
            if hr.is_ok() {
                PRIVATE_COM_STA_INITIALIZED.with(|initialized| initialized.set(true));
            } else {
                let mut slot = startup_failure.lock().unwrap();
                if slot.is_none() {
                    *slot = Some(format!(
                        "private blocking worker COM STA initialization failed: {hr:?}"
                    ));
                }
            }
        })
        .on_thread_stop(|| {
            PRIVATE_COM_STA_INITIALIZED.with(|initialized| {
                if initialized.replace(false) {
                    unsafe { CoUninitialize() };
                }
            });
        });
    let runtime = builder.build()?;

    let blocking_desktop_name = runtime.block_on(async {
        tokio::task::spawn_blocking(current_thread_desktop_name)
            .await
            .map_err(|error| io::Error::other(error.to_string()))?
    })?;

    if let Some(error) = attach_failure.lock().unwrap().clone() {
        return Err(io::Error::other(format!(
            "blocking worker SetThreadDesktop failed: {error}"
        )));
    }
    if blocking_desktop_name != core.desktop_name() {
        return Err(io::Error::other(format!(
            "blocking worker desktop affinity failed: expected {:?}, got {:?}",
            core.desktop_name(),
            blocking_desktop_name
        )));
    }

    let startup = StartupReceipt {
        desktop_name: core.desktop_name().to_owned(),
        actor_desktop_name,
        blocking_desktop_name,
    };
    Ok((core, runtime, startup))
}

enum SessionEnvironment {
    Shared,
    Private(Arc<PrivateDesktopRuntime>),
}

pub struct ExecutionEnvironmentRegistry {
    inner: Mutex<HashMap<String, SessionEnvironment>>,
}

impl ExecutionEnvironmentRegistry {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
        }
    }

    pub fn claim_shared(&self, session: &str) -> Result<(), String> {
        if session.is_empty() {
            return Ok(());
        }
        let mut inner = self.inner.lock().unwrap();
        match inner.get(session) {
            Some(SessionEnvironment::Private(_)) => Err(
                "session is already bound to private_desktop and cannot switch isolation mode"
                    .into(),
            ),
            Some(SessionEnvironment::Shared) => Ok(()),
            None => {
                inner.insert(session.to_owned(), SessionEnvironment::Shared);
                Ok(())
            }
        }
    }

    pub fn get_or_create_private(
        &self,
        session: &str,
    ) -> Result<Arc<PrivateDesktopRuntime>, String> {
        if session.is_empty() {
            return Err("private_desktop requires a concrete session".into());
        }

        {
            let inner = self.inner.lock().unwrap();
            match inner.get(session) {
                Some(SessionEnvironment::Private(environment)) => {
                    return Ok(environment.clone());
                }
                Some(SessionEnvironment::Shared) => {
                    return Err(
                        "session is already bound to shared_user_desktop and cannot switch isolation mode"
                            .into(),
                    );
                }
                None => {}
            }
        }

        let created = PrivateDesktopRuntime::start().map_err(|error| error.to_string())?;
        let mut inner = self.inner.lock().unwrap();
        match inner.entry(session.to_owned()) {
            std::collections::hash_map::Entry::Occupied(entry) => match entry.get() {
                SessionEnvironment::Private(existing) => Ok(existing.clone()),
                SessionEnvironment::Shared => Err(
                    "session became bound to shared_user_desktop while private environment was starting"
                        .into(),
                ),
            },
            std::collections::hash_map::Entry::Vacant(entry) => {
                entry.insert(SessionEnvironment::Private(created.clone()));
                Ok(created)
            }
        }
    }

    pub fn private_for_session(&self, session: &str) -> Option<Arc<PrivateDesktopRuntime>> {
        let inner = self.inner.lock().unwrap();
        match inner.get(session) {
            Some(SessionEnvironment::Private(environment)) => Some(environment.clone()),
            _ => None,
        }
    }

    pub fn isolation_for_session(&self, session: &str) -> Option<IsolationMode> {
        let inner = self.inner.lock().unwrap();
        match inner.get(session) {
            Some(SessionEnvironment::Shared) => Some(IsolationMode::SharedUserDesktop),
            Some(SessionEnvironment::Private(_)) => Some(IsolationMode::PrivateDesktop),
            None => None,
        }
    }

    pub fn remove_private(&self, session: &str) -> io::Result<bool> {
        let (removed, environment) = {
            let mut inner = self.inner.lock().unwrap();
            match inner.remove(session) {
                Some(SessionEnvironment::Private(environment)) => (true, Some(environment)),
                Some(SessionEnvironment::Shared) => (true, None),
                None => (false, None),
            }
        };
        if let Some(environment) = environment {
            environment.shutdown().map_err(io::Error::other)?;
        }
        Ok(removed)
    }

    pub fn private_count(&self) -> io::Result<usize> {
        Ok(self
            .inner
            .lock()
            .unwrap()
            .values()
            .filter(|environment| matches!(environment, SessionEnvironment::Private(_)))
            .count())
    }

    pub fn remove(&self, session: &str) -> Result<(), String> {
        self.remove_private(session)
            .map(|_| ())
            .map_err(|error| error.to_string())
    }
}

impl Default for ExecutionEnvironmentRegistry {
    fn default() -> Self {
        Self::new()
    }
}
