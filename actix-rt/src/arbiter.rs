use std::any::{Any, TypeId};
use std::cell::RefCell;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::task::{Context, Poll};
use std::{fmt, thread};

use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use tokio::sync::oneshot::{channel, error::RecvError as Canceled, Sender};
// use futures_util::stream::FuturesUnordered;
// use tokio::task::JoinHandle;
// use tokio::stream::StreamExt;
use tokio::task::LocalSet;

use crate::runtime::Runtime;
use crate::system::System;

thread_local!(
    static ADDR: RefCell<Option<Arbiter>> = RefCell::new(None);
    // TODO: Commented out code are for Arbiter::local_join function.
    // It can be safely removed if this function is not used in actix-*.
    //
    // /// stores join handle for spawned async tasks.
    // static HANDLE: RefCell<FuturesUnordered<JoinHandle<()>>> =
    //     RefCell::new(FuturesUnordered::new());
    static STORAGE: RefCell<HashMap<TypeId, Box<dyn Any>>> = RefCell::new(HashMap::new());
);

pub(crate) static COUNT: AtomicUsize = AtomicUsize::new(0);

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

#[derive(Debug)]
/// Arbiters provide an asynchronous execution environment for actors, functions
/// and futures. When an Arbiter is created, it spawns a new OS thread, and
/// hosts an event loop. Some Arbiter functions execute on the current thread.
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

                    // unregister arbiter
                    let _ = System::current()
                        .sys()
                        .send(SystemCommand::UnregisterArbiter(id));
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

    /// Spawn a future on the current thread. This does not create a new Arbiter
    /// or Arbiter address, it is simply a helper for spawning futures on the current
    /// thread.
    pub fn spawn<F>(future: F)
    where
        F: Future<Output = ()> + 'static,
    {
        // HANDLE.with(|handle| {
        //     let handle = handle.borrow();
        //     handle.push(tokio::task::spawn_local(future));
        // });
        // let _ = tokio::task::spawn_local(CleanupPending);
        let _ = tokio::task::spawn_local(future);
    }

    /// Executes a future on the current thread. This does not create a new Arbiter
    /// or Arbiter address, it is simply a helper for executing futures on the current
    /// thread.
    pub fn spawn_fn<F, R>(f: F)
    where
        F: FnOnce() -> R + 'static,
        R: Future<Output = ()> + 'static,
    {
        Arbiter::spawn(async {
            f();
        })
    }

    /// Send a future to the Arbiter's thread, and spawn it.
    pub fn send<F>(&self, future: F)
    where
        F: Future<Output = ()> + Send + Unpin + 'static,
    {
        let _ = self.sender.send(ArbiterCommand::Execute(Box::new(future)));
    }

    /// Send a function to the Arbiter's thread, and execute it. Any result from the function
    /// is discarded.
    pub fn exec_fn<F>(&self, f: F)
    where
        F: FnOnce() + Send + 'static,
    {
        let _ = self
            .sender
            .send(ArbiterCommand::ExecuteFn(Box::new(move || {
                f();
            })));
    }

    /// Send a function to the Arbiter's thread. This function will be executed asynchronously.
    /// A future is created, and when resolved will contain the result of the function sent
    /// to the Arbiters thread.
    pub fn exec<F, R>(&self, f: F) -> impl Future<Output = Result<R, Canceled>>
    where
        F: FnOnce() -> R + Send + 'static,
        R: Send + 'static,
    {
        let (tx, rx) = channel();
        let _ = self
            .sender
            .send(ArbiterCommand::ExecuteFn(Box::new(move || {
                if !tx.is_closed() {
                    let _ = tx.send(f());
                }
            })));
        rx
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

    /// Returns a future that will be completed once all currently spawned futures
    /// have completed.
    #[deprecated(since = "1.2.0", note = "Arbiter::local_join function is removed.")]
    pub async fn local_join() {
        // let handle = HANDLE.with(|fut| std::mem::take(&mut *fut.borrow_mut()));
        // async move {
        //     handle.collect::<Vec<_>>().await;
        // }
        unimplemented!("Arbiter::local_join function is removed.")
    }
}

// /// Future used for cleaning-up already finished `JoinHandle`s
// /// from the `PENDING` list so the vector doesn't grow indefinitely
// struct CleanupPending;
//
// impl Future for CleanupPending {
//     type Output = ();
//
//     fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
//         HANDLE.with(move |handle| {
//             recycle_join_handle(&mut *handle.borrow_mut(), cx);
//         });
//
//         Poll::Ready(())
//     }
// }

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
        loop {
            match Pin::new(&mut self.rx).poll_recv(cx) {
                Poll::Ready(None) => return Poll::Ready(()),
                Poll::Ready(Some(item)) => match item {
                    ArbiterCommand::Stop => return Poll::Ready(()),
                    ArbiterCommand::Execute(fut) => {
                        // HANDLE.with(|handle| {
                        //     let mut handle = handle.borrow_mut();
                        //     handle.push(tokio::task::spawn_local(fut));
                        //     recycle_join_handle(&mut *handle, cx);
                        // });
                        tokio::task::spawn_local(fut);
                    }
                    ArbiterCommand::ExecuteFn(f) => {
                        f.call_box();
                    }
                },
                Poll::Pending => return Poll::Pending,
            }
        }
    }
}

// fn recycle_join_handle(handle: &mut FuturesUnordered<JoinHandle<()>>, cx: &mut Context<'_>) {
//     let _ = Pin::new(&mut *handle).poll_next(cx);
//
//     // Try to recycle more join handles and free up memory.
//     //
//     // this is a guess. The yield limit for FuturesUnordered is 32.
//     // So poll an extra 3 times would make the total poll below 128.
//     if handle.len() > 64 {
//         (0..3).for_each(|_| {
//             let _ = Pin::new(&mut *handle).poll_next(cx);
//         })
//     }
// }

#[derive(Debug)]
pub(crate) enum SystemCommand {
    Exit(i32),
    RegisterArbiter(usize, Arbiter),
    UnregisterArbiter(usize),
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
        loop {
            match Pin::new(&mut self.commands).poll_recv(cx) {
                Poll::Ready(None) => return Poll::Ready(()),
                Poll::Ready(Some(cmd)) => match cmd {
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
                    SystemCommand::UnregisterArbiter(name) => {
                        self.arbiters.remove(&name);
                    }
                },
                Poll::Pending => return Poll::Pending,
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
