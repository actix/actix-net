use std::{
    any::{Any, TypeId},
    cell::RefCell,
    collections::HashMap,
    fmt,
    future::Future,
    pin::Pin,
    sync::atomic::{AtomicUsize, Ordering},
    task::{Context, Poll},
    thread,
};

use futures_core::ready;
use tokio::{sync::mpsc, task::LocalSet};

use crate::{
    runtime::Runtime,
    system::{System, SystemCommand},
};

pub(crate) static COUNT: AtomicUsize = AtomicUsize::new(0);

thread_local!(
    static HANDLE: RefCell<Option<ArbiterHandle>> = RefCell::new(None);
    static STORAGE: RefCell<HashMap<TypeId, Box<dyn Any>>> = RefCell::new(HashMap::new());
);

pub(crate) enum ArbiterCommand {
    Stop,
    Execute(Pin<Box<dyn Future<Output = ()> + Send>>),
}

impl fmt::Debug for ArbiterCommand {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ArbiterCommand::Stop => write!(f, "ArbiterCommand::Stop"),
            ArbiterCommand::Execute(_) => write!(f, "ArbiterCommand::Execute"),
        }
    }
}

/// A handle for sending spawn and stop messages to an [Arbiter].
#[derive(Debug, Clone)]
pub struct ArbiterHandle {
    tx: mpsc::UnboundedSender<ArbiterCommand>,
}

impl ArbiterHandle {
    pub(crate) fn new(tx: mpsc::UnboundedSender<ArbiterCommand>) -> Self {
        Self { tx }
    }

    /// Send a future to the [Arbiter]'s thread and spawn it.
    ///
    /// If you require a result, include a response channel in the future.
    ///
    /// Returns true if future was sent successfully and false if the [Arbiter] has died.
    pub fn spawn<Fut>(&self, future: Fut) -> bool
    where
        Fut: Future<Output = ()> + Send + 'static,
    {
        self.tx
            .send(ArbiterCommand::Execute(Box::pin(future)))
            .is_ok()
    }

    /// Send a function to the [Arbiter]'s thread and execute it.
    ///
    /// Any result from the function is discarded. If you require a result, include a response
    /// channel in the function.
    ///
    /// Returns true if function was sent successfully and false if the [Arbiter] has died.
    pub fn spawn_fn<F>(&self, f: F) -> bool
    where
        F: FnOnce() + Send + 'static,
    {
        self.spawn(async { f() })
    }

    /// Instruct [Arbiter] to stop processing it's event loop.
    ///
    /// Returns true if stop message was sent successfully and false if the [Arbiter] has
    /// been dropped.
    pub fn stop(&self) -> bool {
        self.tx.send(ArbiterCommand::Stop).is_ok()
    }
}

/// An Arbiter represents a thread that provides an asynchronous execution environment for futures
/// and functions.
///
/// When an arbiter is created, it spawns a new [OS thread](thread), and hosts an event loop.
#[derive(Debug)]
pub struct Arbiter {
    tx: mpsc::UnboundedSender<ArbiterCommand>,
    thread_handle: thread::JoinHandle<()>,
}

impl Arbiter {
    /// Spawn new Arbiter thread and start its event loop.
    ///
    /// # Panics
    /// Panics if a [System] is not registered on the current thread.
    #[allow(clippy::new_without_default)]
    pub fn new() -> Arbiter {
        let id = COUNT.fetch_add(1, Ordering::Relaxed);
        let system_id = System::current().id();
        let name = format!("actix-rt|system:{}|arbiter:{}", system_id, id);
        let sys = System::current();
        let (tx, rx) = mpsc::unbounded_channel();

        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<()>();

        let thread_handle = thread::Builder::new()
            .name(name.clone())
            .spawn({
                let tx = tx.clone();
                move || {
                    let rt = Runtime::new().expect("Cannot create new Arbiter's Runtime.");
                    let hnd = ArbiterHandle::new(tx);

                    System::set_current(sys);

                    STORAGE.with(|cell| cell.borrow_mut().clear());
                    HANDLE.with(|cell| *cell.borrow_mut() = Some(hnd.clone()));

                    // register arbiter
                    let _ = System::current()
                        .tx()
                        .send(SystemCommand::RegisterArbiter(id, hnd));

                    ready_tx.send(()).unwrap();

                    // run arbiter event processing loop
                    rt.block_on(ArbiterRunner { rx });

                    // deregister arbiter
                    let _ = System::current()
                        .tx()
                        .send(SystemCommand::DeregisterArbiter(id));
                }
            })
            .unwrap_or_else(|err| {
                panic!("Cannot spawn Arbiter's thread: {:?}. {:?}", &name, err)
            });

        ready_rx.recv().unwrap();

        Arbiter { tx, thread_handle }
    }

    /// Sets up an Arbiter runner in a new System using the provided runtime local task set.
    pub(crate) fn in_new_system(local: &LocalSet) -> ArbiterHandle {
        let (tx, rx) = mpsc::unbounded_channel();

        let hnd = ArbiterHandle::new(tx);

        HANDLE.with(|cell| *cell.borrow_mut() = Some(hnd.clone()));
        STORAGE.with(|cell| cell.borrow_mut().clear());

        local.spawn_local(ArbiterRunner { rx });

        hnd
    }

    /// Return a handle to the current thread's Arbiter's message sender.
    ///
    /// # Panics
    /// Panics if no Arbiter is running on the current thread.
    pub fn current() -> ArbiterHandle {
        HANDLE.with(|cell| match *cell.borrow() {
            Some(ref addr) => addr.clone(),
            None => panic!("Arbiter is not running."),
        })
    }

    /// Stop Arbiter from continuing it's event loop.
    ///
    /// Returns true if stop message was sent successfully and false if the Arbiter has been dropped.
    pub fn stop(&self) -> bool {
        self.tx.send(ArbiterCommand::Stop).is_ok()
    }

    /// Send a future to the Arbiter's thread and spawn it.
    ///
    /// If you require a result, include a response channel in the future.
    ///
    /// Returns true if future was sent successfully and false if the Arbiter has died.
    pub fn spawn<Fut>(&self, future: Fut) -> bool
    where
        Fut: Future<Output = ()> + Send + 'static,
    {
        self.tx
            .send(ArbiterCommand::Execute(Box::pin(future)))
            .is_ok()
    }

    /// Send a function to the Arbiter's thread and execute it.
    ///
    /// Any result from the function is discarded. If you require a result, include a response
    /// channel in the function.
    ///
    /// Returns true if function was sent successfully and false if the Arbiter has died.
    pub fn spawn_fn<F>(&self, f: F) -> bool
    where
        F: FnOnce() + Send + 'static,
    {
        self.spawn(async { f() })
    }

    /// Wait for Arbiter's event loop to complete.
    ///
    /// Joins the underlying OS thread handle. See [`JoinHandle::join`](thread::JoinHandle::join).
    pub fn join(self) -> thread::Result<()> {
        self.thread_handle.join()
    }

    /// Insert item into Arbiter's thread-local storage.
    ///
    /// Overwrites any item of the same type previously inserted.
    pub fn set_item<T: 'static>(item: T) {
        STORAGE.with(move |cell| cell.borrow_mut().insert(TypeId::of::<T>(), Box::new(item)));
    }

    /// Check if Arbiter's thread-local storage contains an item type.
    pub fn contains_item<T: 'static>() -> bool {
        STORAGE.with(move |cell| cell.borrow().contains_key(&TypeId::of::<T>()))
    }

    /// Call a function with a shared reference to an item in this Arbiter's thread-local storage.
    ///
    /// # Panics
    /// Panics if item is not in Arbiter's thread-local item storage.
    pub fn get_item<T: 'static, F, R>(mut f: F) -> R
    where
        F: FnMut(&T) -> R,
    {
        STORAGE.with(move |cell| {
            let st = cell.borrow();

            let type_id = TypeId::of::<T>();
            let item = st.get(&type_id).and_then(downcast_ref).unwrap();

            f(item)
        })
    }

    /// Call a function with a mutable reference to an item in this Arbiter's thread-local storage.
    ///
    /// # Panics
    /// Panics if item is not in Arbiter's thread-local item storage.
    pub fn get_mut_item<T: 'static, F, R>(mut f: F) -> R
    where
        F: FnMut(&mut T) -> R,
    {
        STORAGE.with(move |cell| {
            let mut st = cell.borrow_mut();

            let type_id = TypeId::of::<T>();
            let item = st.get_mut(&type_id).and_then(downcast_mut).unwrap();

            f(item)
        })
    }
}

/// A persistent future that processes [Arbiter] commands.
struct ArbiterRunner {
    rx: mpsc::UnboundedReceiver<ArbiterCommand>,
}

impl Future for ArbiterRunner {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // process all items currently buffered in channel
        loop {
            match ready!(Pin::new(&mut self.rx).poll_recv(cx)) {
                // channel closed; no more messages can be received
                None => return Poll::Ready(()),

                // process arbiter command
                Some(item) => match item {
                    ArbiterCommand::Stop => {
                        return Poll::Ready(());
                    }
                    ArbiterCommand::Execute(task_fut) => {
                        tokio::task::spawn_local(task_fut);
                    }
                },
            }
        }
    }
}

fn downcast_ref<T: 'static>(boxed: &Box<dyn Any>) -> Option<&T> {
    boxed.downcast_ref()
}

fn downcast_mut<T: 'static>(boxed: &mut Box<dyn Any>) -> Option<&mut T> {
    boxed.downcast_mut()
}
