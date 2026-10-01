use super::win32_private::{attach_current_thread_to, current_thread_desktop_name};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;
use windows::core::{Interface, VARIANT};
use windows::Win32::Foundation::HWND;
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED,
    SAFEARRAY,
};
use windows::Win32::UI::Accessibility::{
    CUIAutomation, IUIAutomation, IUIAutomationCacheRequest, IUIAutomationElement,
    IUIAutomationPropertyChangedEventHandler, IUIAutomationPropertyChangedEventHandler_Impl,
    IUIAutomationStructureChangedEventHandler, IUIAutomationStructureChangedEventHandler_Impl,
    StructureChangeType, TreeScope_Subtree, UIA_ControlTypePropertyId, UIA_NamePropertyId,
    UIA_PROPERTY_ID,
};

const OBSERVER_REPLY_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone, Debug)]
pub struct PrivateObservationEpoch {
    epoch: Arc<AtomicU64>,
}

impl PrivateObservationEpoch {
    pub fn current(&self) -> u64 {
        self.epoch.load(Ordering::Acquire)
    }
}

#[windows::core::implement(
    IUIAutomationStructureChangedEventHandler,
    IUIAutomationPropertyChangedEventHandler
)]
struct EpochEventHandler {
    epoch: Arc<AtomicU64>,
}

impl EpochEventHandler {
    fn invalidate(&self) {
        self.epoch.fetch_add(1, Ordering::AcqRel);
    }
}

impl IUIAutomationStructureChangedEventHandler_Impl for EpochEventHandler {
    fn HandleStructureChangedEvent(
        &self,
        _sender: Option<&IUIAutomationElement>,
        _changetype: StructureChangeType,
        _runtimeid: *const SAFEARRAY,
    ) -> windows::core::Result<()> {
        self.invalidate();
        Ok(())
    }
}

impl IUIAutomationPropertyChangedEventHandler_Impl for EpochEventHandler {
    fn HandlePropertyChangedEvent(
        &self,
        _sender: Option<&IUIAutomationElement>,
        _propertyid: UIA_PROPERTY_ID,
        _newvalue: &VARIANT,
    ) -> windows::core::Result<()> {
        self.invalidate();
        Ok(())
    }
}

struct WpfWatch {
    root: IUIAutomationElement,
    structure_handler: IUIAutomationStructureChangedEventHandler,
    property_handler: IUIAutomationPropertyChangedEventHandler,
    epoch: Arc<AtomicU64>,
}

enum ObserverCommand {
    WatchWpf {
        pid: u32,
        hwnd: u64,
        reply: mpsc::SyncSender<Result<PrivateObservationEpoch, String>>,
    },
    Reset {
        reply: mpsc::SyncSender<()>,
    },
    Shutdown {
        reply: mpsc::SyncSender<()>,
    },
}

pub struct PrivateObservationMonitor {
    sender: mpsc::Sender<ObserverCommand>,
    join: Mutex<Option<JoinHandle<()>>>,
}

impl PrivateObservationMonitor {
    pub fn start(desktop_addr: usize, expected_desktop: String) -> Result<Self, String> {
        let (sender, receiver) = mpsc::channel();
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let join = thread::Builder::new()
            .name("cua-private-observation".into())
            .spawn(move || {
                observer_main(desktop_addr, &expected_desktop, receiver, ready_tx);
            })
            .map_err(|error| format!("private observation thread spawn failed: {error}"))?;

        match ready_rx.recv_timeout(OBSERVER_REPLY_TIMEOUT) {
            Ok(Ok(())) => Ok(Self {
                sender,
                join: Mutex::new(Some(join)),
            }),
            Ok(Err(error)) => {
                let _ = join.join();
                Err(error)
            }
            Err(error) => {
                let _ = join.join();
                Err(format!(
                    "private observation startup reply timed out: {error}"
                ))
            }
        }
    }

    pub fn watch_wpf(&self, pid: u32, hwnd: u64) -> Result<PrivateObservationEpoch, String> {
        let (reply_tx, reply_rx) = mpsc::sync_channel(1);
        self.sender
            .send(ObserverCommand::WatchWpf {
                pid,
                hwnd,
                reply: reply_tx,
            })
            .map_err(|_| "private observation thread stopped".to_owned())?;
        reply_rx
            .recv_timeout(OBSERVER_REPLY_TIMEOUT)
            .map_err(|error| format!("private observation watch reply timed out: {error}"))?
    }

    pub fn reset(&self) -> Result<(), String> {
        let (reply_tx, reply_rx) = mpsc::sync_channel(1);
        self.sender
            .send(ObserverCommand::Reset { reply: reply_tx })
            .map_err(|_| "private observation thread stopped".to_owned())?;
        reply_rx
            .recv_timeout(OBSERVER_REPLY_TIMEOUT)
            .map_err(|error| format!("private observation reset reply timed out: {error}"))
    }

    pub fn shutdown(&self) -> Result<(), String> {
        let join = self.join.lock().unwrap().take();
        let Some(join) = join else {
            return Ok(());
        };

        let (reply_tx, reply_rx) = mpsc::sync_channel(1);
        if self
            .sender
            .send(ObserverCommand::Shutdown { reply: reply_tx })
            .is_ok()
        {
            let _ = reply_rx.recv_timeout(OBSERVER_REPLY_TIMEOUT);
        }
        join.join()
            .map_err(|_| "private observation thread panicked during shutdown".to_owned())
    }
}

impl Drop for PrivateObservationMonitor {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

fn unregister_watch(automation: &IUIAutomation, watch: WpfWatch) {
    unsafe {
        let _ = automation.RemovePropertyChangedEventHandler(&watch.root, &watch.property_handler);
        let _ =
            automation.RemoveStructureChangedEventHandler(&watch.root, &watch.structure_handler);
    }
}

fn clear_watches(automation: &IUIAutomation, watches: &mut HashMap<(u32, u64), WpfWatch>) {
    for (_, watch) in watches.drain() {
        unregister_watch(automation, watch);
    }
}

fn register_wpf_watch(automation: &IUIAutomation, hwnd: u64) -> Result<WpfWatch, String> {
    let root = unsafe {
        automation
            .ElementFromHandle(HWND(hwnd as *mut _))
            .map_err(|error| format!("private WPF observer ElementFromHandle failed: {error}"))?
    };
    let epoch = Arc::new(AtomicU64::new(1));
    let handler_impl = EpochEventHandler {
        epoch: epoch.clone(),
    };
    let structure_handler: IUIAutomationStructureChangedEventHandler = handler_impl.into();
    let property_handler: IUIAutomationPropertyChangedEventHandler = structure_handler
        .cast()
        .map_err(|error| format!("private WPF observer handler cast failed: {error}"))?;

    unsafe {
        automation
            .AddStructureChangedEventHandler(
                &root,
                TreeScope_Subtree,
                None::<&IUIAutomationCacheRequest>,
                &structure_handler,
            )
            .map_err(|error| {
                format!("private WPF structure handler registration failed: {error}")
            })?;

        if let Err(error) = automation.AddPropertyChangedEventHandlerNativeArray(
            &root,
            TreeScope_Subtree,
            None::<&IUIAutomationCacheRequest>,
            &property_handler,
            &[UIA_NamePropertyId, UIA_ControlTypePropertyId],
        ) {
            let _ = automation.RemoveStructureChangedEventHandler(&root, &structure_handler);
            return Err(format!(
                "private WPF property handler registration failed: {error}"
            ));
        }
    }

    Ok(WpfWatch {
        root,
        structure_handler,
        property_handler,
        epoch,
    })
}

fn observer_main(
    desktop_addr: usize,
    expected_desktop: &str,
    receiver: mpsc::Receiver<ObserverCommand>,
    ready: mpsc::SyncSender<Result<(), String>>,
) {
    if let Err(error) = attach_current_thread_to(desktop_addr) {
        let _ = ready.send(Err(format!(
            "private observation SetThreadDesktop failed: {error}"
        )));
        return;
    }
    match current_thread_desktop_name() {
        Ok(actual) if actual == expected_desktop => {}
        Ok(actual) => {
            let _ = ready.send(Err(format!(
                "private observation desktop mismatch: expected {expected_desktop:?}, got {actual:?}"
            )));
            return;
        }
        Err(error) => {
            let _ = ready.send(Err(format!(
                "private observation desktop probe failed: {error}"
            )));
            return;
        }
    }

    let com = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
    if let Err(error) = com {
        let _ = ready.send(Err(format!(
            "private observation COM MTA initialization failed: {error}"
        )));
        return;
    }

    let automation: IUIAutomation =
        match unsafe { CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER) } {
            Ok(automation) => automation,
            Err(error) => {
                let _ = ready.send(Err(format!(
                    "private observation UIA initialization failed: {error}"
                )));
                unsafe { CoUninitialize() };
                return;
            }
        };

    if ready.send(Ok(())).is_err() {
        unsafe { CoUninitialize() };
        return;
    }

    let mut watches: HashMap<(u32, u64), WpfWatch> = HashMap::new();
    while let Ok(command) = receiver.recv() {
        match command {
            ObserverCommand::WatchWpf { pid, hwnd, reply } => {
                let key = (pid, hwnd);
                let result = if let Some(watch) = watches.get(&key) {
                    Ok(PrivateObservationEpoch {
                        epoch: watch.epoch.clone(),
                    })
                } else {
                    register_wpf_watch(&automation, hwnd).map(|watch| {
                        let epoch = PrivateObservationEpoch {
                            epoch: watch.epoch.clone(),
                        };
                        watches.insert(key, watch);
                        epoch
                    })
                };
                let _ = reply.send(result);
            }
            ObserverCommand::Reset { reply } => {
                clear_watches(&automation, &mut watches);
                let _ = reply.send(());
            }
            ObserverCommand::Shutdown { reply } => {
                clear_watches(&automation, &mut watches);
                let _ = reply.send(());
                break;
            }
        }
    }

    clear_watches(&automation, &mut watches);
    drop(automation);
    unsafe { CoUninitialize() };
}
