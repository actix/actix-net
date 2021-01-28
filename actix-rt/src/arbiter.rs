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
use tokio::{
    sync::{
        mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender},
        oneshot::Sender,
    },
    task::LocalSet,
};

use crate::{runtime::Runtime, system::System};

pub(crate) static COUNT: AtomicUsize = AtomicUsize::new(0);

thread_local!(
    static ADDR: RefCell<Option<Arbiter>> = RefCell::new(None);
    static STORAGE: RefCell<HashMap<TypeId, Box<dyn Any>>> = RefCell::new(HashMap::new());
);

pub(crate) enum ArbiterCommand {
    Stop,
    Execute(Box<dyn Future<Output = ()> + Unpin + Send>),
    ExecuteFn(Box<dyn FnExec>),
}

impl fmt::Debug for ArbiterCommand {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ArbiterCommand::Stop => write!(f, "ArbiterCommand::Stop"),
            ArbiterCommand::Execute(_) => write!(f, "ArbiterCommand::Execute"),
            ArbiterCommand::ExecuteFn(_) => write!(f, "ArbiterCommand::ExecuteFn"),
        }
    }
}

/// Arbiters provide an asynchronous execution environment for actors, functions and futures. When
/// an Arbiter is created, it spawns a new OS thread, and hosts an event loop. Some Arbiter
/// functions execute on the current thread.
#[derive(Debug)]
pub struct Arbiter {
    sender: UnboundedSender<ArbiterCommand>,
    thread_handle: Option<thread::JoinHandle<()>>,
}

impl Clone for Arbiter {
    fn clone(&self) -> Self {
        Self::with_sender(self.sender.clone())
    }
}

impl Default for Arbiter {
    fn default() -> Self {
        Self::new()
    }
}

impl Arbiter {
    pub(crate) fn new_system(local: &LocalSet) -> Self {
        let (tx, rx) = unbounded_channel();

        let arb = Arbiter::with_sender(tx);
        ADDR.with(|cell| *cell.borrow_mut() = Some(arb.clone()));
        STORAGE.with(|cell| cell.borrow_mut().clear());

        local.spawn_local(ArbiterController { rx });

        arb
    }

    /// Returns the current thread's arbiter's address. If no Arbiter is present, then this
    /// function will panic!
    pub fn current() -> Arbiter {
        ADDR.with(|cell| match *cell.borrow() {
            Some(ref addr) => addr.clone(),
            None => panic!("Arbiter is not running"),
        })
    }

    /// Check if current arbiter is running.
    #[deprecated(note = "Thread local variables for running state of Arbiter is removed")]
    pub fn is_running() -> bool {
        false
    }

    /// Stop arbiter from continuing it's event loop.
    pub fn stop(&self) {
        let _ = self.sender.send(ArbiterCommand::Stop);
    }

    /// Spawn new thread and run event loop in spawned thread.
    /// Returns address of newly created arbiter.
    pub fn new() -> Arbiter {
        let id = COUNT.fetch_add(1, Ordering::Relaxed);
        let name = format!("actix-rt:worker:{}", id);
        let sys = System::current();
        let (tx, rx) = unbounded_channel();

        let handle = thread::Builder::new()
            .name(name.clone())
            .spawn({
                let tx = tx.clone();
                move || {
                    let rt = Runtime::new().expect("Can not create Runtime");
                    let arb = Arbiter::with_sender(tx);

                    STORAGE.with(|cell| cell.borrow_mut().clear());

                    System::set_current(sys);

                    ADDR.with(|cell| *cell.borrow_mut() = Some(arb.clone()));

                    // register arbiter
                    let _ = System::current()
                        .sys()
                        .send(SystemCommand::RegisterArbiter(id, arb));

                    // start arbiter controller
                    // run loop
                    rt.block_on(ArbiterController { rx });

                    // deregister arbiter
                    let _ = System::current()
                        .sys()
                        .send(SystemCommand::DeregisterArbiter(id));
                }
            })
            .unwrap_or_else(|err| {
                panic!("Cannot spawn an arbiter's thread {:?}: {:?}", &name, err)
            });

        Arbiter {
            sender: tx,
            thread_handle: Some(handle),
        }
    }

    /// Send a future to the Arbiter's thread and spawn it.
    ///
    /// If you require a result, include a response channel in the future.
    ///
    /// Returns true if function was sent successfully and false if the Arbiter has died.
    pub fn spawn<Fut>(&self, future: Fut) -> bool
    where
        Fut: Future<Output = ()> + Unpin + Send + 'static,
    {
        match self.sender.send(ArbiterCommand::Execute(Box::new(future))) {
            Ok(_) => true,
            Err(_) => false,
        }
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
        match self.sender.send(ArbiterCommand::ExecuteFn(Box::new(f))) {
            Ok(_) => true,
            Err(_) => false,
        }
    }

    /// Set item to arbiter storage
    pub fn set_item<T: 'static>(item: T) {
        STORAGE.with(move |cell| cell.borrow_mut().insert(TypeId::of::<T>(), Box::new(item)));
    }

    /// Check if arbiter storage contains item
    pub fn contains_item<T: 'static>() -> bool {
        STORAGE.with(move |cell| cell.borrow().get(&TypeId::of::<T>()).is_some())
    }

    /// Get a reference to a type previously inserted on this arbiter's storage.
    ///
    /// Panics is item is not inserted
    pub fn get_item<T: 'static, F, R>(mut f: F) -> R
    where
        F: FnMut(&T) -> R,
    {
        STORAGE.with(move |cell| {
            let st = cell.borrow();
            let item = st
                .get(&TypeId::of::<T>())
                .and_then(|boxed| (&**boxed as &(dyn Any + 'static)).downcast_ref())
                .unwrap();
            f(item)
        })
    }

    /// Get a mutable reference to a type previously inserted on this arbiter's storage.
    ///
    /// Panics is item is not inserted
    pub fn get_mut_item<T: 'static, F, R>(mut f: F) -> R
    where
        F: FnMut(&mut T) -> R,
    {
        STORAGE.with(move |cell| {
            let mut st = cell.borrow_mut();
            let item = st
                .get_mut(&TypeId::of::<T>())
                .and_then(|boxed| (&mut **boxed as &mut (dyn Any + 'static)).downcast_mut())
                .unwrap();
            f(item)
        })
    }

    fn with_sender(sender: UnboundedSender<ArbiterCommand>) -> Self {
        Self {
            sender,
            thread_handle: None,
        }
    }

    /// Wait for the event loop to stop by joining the underlying thread (if have Some).
    pub fn join(&mut self) -> thread::Result<()> {
        if let Some(thread_handle) = self.thread_handle.take() {
            thread_handle.join()
        } else {
            Ok(())
        }
    }
}

struct ArbiterController {
    rx: UnboundedReceiver<ArbiterCommand>,
}

impl Drop for ArbiterController {
    fn drop(&mut self) {
        if thread::panicking() {
            if System::current().stop_on_panic() {
                eprintln!("Panic in Arbiter thread, shutting down system.");
                System::current().stop_with_code(1)
            } else {
                eprintln!("Panic in Arbiter thread.");
            }
        }
    }
}

impl Future for ArbiterController {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // process all items currently buffered in channel
        loop {
            match ready!(Pin::new(&mut self.rx).poll_recv(cx)) {
                // channel closed; no more messages can be received
                None => return Poll::Ready(()),

                // process arbiter command
                Some(item) => match item {
                    ArbiterCommand::Stop => return Poll::Ready(()),
                    ArbiterCommand::Execute(fut) => {
                        tokio::task::spawn_local(fut);
                    }
                    ArbiterCommand::ExecuteFn(f) => {
                        f.call_box();
                    }
                },
            }
        }
    }
}

#[derive(Debug)]
pub(crate) enum SystemCommand {
    Exit(i32),
    RegisterArbiter(usize, Arbiter),
    DeregisterArbiter(usize),
}

#[derive(Debug)]
pub(crate) struct SystemArbiter {
    stop: Option<Sender<i32>>,
    commands: UnboundedReceiver<SystemCommand>,
    arbiters: HashMap<usize, Arbiter>,
}

impl SystemArbiter {
    pub(crate) fn new(stop: Sender<i32>, commands: UnboundedReceiver<SystemCommand>) -> Self {
        SystemArbiter {
            commands,
            stop: Some(stop),
            arbiters: HashMap::new(),
        }
    }
}

impl Future for SystemArbiter {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // process all items currently buffered in channel
        loop {
            match ready!(Pin::new(&mut self.commands).poll_recv(cx)) {
                // channel closed; no more messages can be received
                None => return Poll::Ready(()),

                // process system command
                Some(cmd) => match cmd {
                    SystemCommand::Exit(code) => {
                        // stop arbiters
                        for arb in self.arbiters.values() {
                            arb.stop();
                        }
                        // stop event loop
                        if let Some(stop) = self.stop.take() {
                            let _ = stop.send(code);
                        }
                    }
                    SystemCommand::RegisterArbiter(name, hnd) => {
                        self.arbiters.insert(name, hnd);
                    }
                    SystemCommand::DeregisterArbiter(name) => {
                        self.arbiters.remove(&name);
                    }
                },
            }
        }
    }
}

pub trait FnExec: Send + 'static {
    fn call_box(self: Box<Self>);
}

impl<F> FnExec for F
where
    F: FnOnce() + Send + 'static,
{
    #[allow(clippy::boxed_local)]
    fn call_box(self: Box<Self>) {
        (*self)()
    }
}
