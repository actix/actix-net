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
    static HANDLE: RefCell<Option<WorkerHandle>> = RefCell::new(None);
    static STORAGE: RefCell<HashMap<TypeId, Box<dyn Any>>> = RefCell::new(HashMap::new());
);

pub(crate) enum WorkerCommand {
    Stop,
    Execute(Pin<Box<dyn Future<Output = ()> + Send>>),
}

impl fmt::Debug for WorkerCommand {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WorkerCommand::Stop => write!(f, "WorkerCommand::Stop"),
            WorkerCommand::Execute(_) => write!(f, "WorkerCommand::Execute"),
        }
    }
}

/// A handle for sending spawn and stop messages to a [Worker].
#[derive(Debug, Clone)]
pub struct WorkerHandle {
    sender: mpsc::UnboundedSender<WorkerCommand>,
}


impl WorkerHandle {
    pub(crate) fn new(sender: mpsc::UnboundedSender<WorkerCommand>) -> Self {
        Self { sender }
    }

    /// Send a future to the Worker's thread and spawn it.
    ///
    /// If you require a result, include a response channel in the future.
    ///
    /// Returns true if future was sent successfully and false if the Worker has died.
    pub fn spawn<Fut>(&self, future: Fut) -> bool
    where
        Fut: Future<Output = ()> + Send + 'static,
    {
        self.sender
            .send(WorkerCommand::Execute(Box::pin(future)))
            .is_ok()
    }

    /// Send a function to the Worker's thread and execute it.
    ///
    /// Any result from the function is discarded. If you require a result, include a response
    /// channel in the function.
    ///
    /// Returns true if function was sent successfully and false if the Worker has died.
    pub fn spawn_fn<F>(&self, f: F) -> bool
    where
        F: FnOnce() + Send + 'static,
    {
        self.spawn(async { f() })
    }

    /// Instruct worker to stop processing it's event loop.
    ///
    /// Returns true if stop message was sent successfully and false if the Worker has been dropped.
    pub fn stop(&self) -> bool {
        self.sender.send(WorkerCommand::Stop).is_ok()
    }
}

/// A worker represent a thread that provides an asynchronous execution environment for futures
/// and functions.
///
/// When a Worker is created, it spawns a new [OS thread](thread), and hosts an event loop.
#[derive(Debug)]
pub struct Worker {
    sender: mpsc::UnboundedSender<WorkerCommand>,
    thread_handle: thread::JoinHandle<()>,
}

impl Worker {
    /// Spawn new thread and run event loop in spawned thread.
    ///
    /// Returns handle of newly created worker.
    ///
    /// # Panics
    /// Panics if a [System] is not registered on the current thread.
    pub fn new() -> Worker {
        let id = COUNT.fetch_add(1, Ordering::Relaxed);
        let name = format!("actix-rt:worker:{}", id);
        let sys = System::current();
        let (tx, rx) = mpsc::unbounded_channel();

        let thread_handle = thread::Builder::new()
            .name(name.clone())
            .spawn({
                let tx = tx.clone();
                move || {
                    let rt = Runtime::new().expect("Can not create Runtime");
                    let hnd = WorkerHandle::new(tx);

                    System::set_current(sys);

                    STORAGE.with(|cell| cell.borrow_mut().clear());
                    HANDLE.with(|cell| *cell.borrow_mut() = Some(hnd.clone()));

                    // register worker
                    let _ = System::current()
                        .tx()
                        .send(SystemCommand::RegisterWorker(id, hnd));

                    // run worker event processing loop
                    rt.block_on(WorkerRunner { rx });

                    // deregister worker
                    let _ = System::current()
                        .tx()
                        .send(SystemCommand::DeregisterWorker(id));
                }
            })
            .unwrap_or_else(|err| {
                panic!("Cannot spawn a Worker's thread {:?}: {:?}", &name, err)
            });

        Worker {
            sender: tx,
            thread_handle,
        }
    }

    /// Sets up a worker runner on the current thread using the provided runtime local task set.
    pub(crate) fn new_current_thread(local: &LocalSet) -> WorkerHandle {
        let (tx, rx) = mpsc::unbounded_channel();

        let hnd = WorkerHandle::new(tx);

        HANDLE.with(|cell| *cell.borrow_mut() = Some(hnd.clone()));
        STORAGE.with(|cell| cell.borrow_mut().clear());

        local.spawn_local(WorkerRunner { rx });

        hnd
    }

    /// Return a handle to the worker.
    ///
    /// # Panics
    /// Panics if no Worker is running on the current thread.
    pub fn handle() -> WorkerHandle {
        HANDLE.with(|cell| match *cell.borrow() {
            Some(ref addr) => addr.clone(),
            None => panic!("Worker is not running."),
        })
    }

    /// Stop worker from continuing it's event loop.
    ///
    /// Returns true if stop message was sent successfully and false if the Worker has been dropped.
    pub fn stop(&self) -> bool {
        self.sender.send(WorkerCommand::Stop).is_ok()
    }

    /// Send a future to the Worker's thread and spawn it.
    ///
    /// If you require a result, include a response channel in the future.
    ///
    /// Returns true if future was sent successfully and false if the Worker has died.
    pub fn spawn<Fut>(&self, future: Fut) -> bool
    where
        Fut: Future<Output = ()> + Send + 'static,
    {
        self.sender
            .send(WorkerCommand::Execute(Box::pin(future)))
            .is_ok()
    }

    /// Send a function to the Worker's thread and execute it.
    ///
    /// Any result from the function is discarded. If you require a result, include a response
    /// channel in the function.
    ///
    /// Returns true if function was sent successfully and false if the Worker has died.
    pub fn spawn_fn<F>(&self, f: F) -> bool
    where
        F: FnOnce() + Send + 'static,
    {
        self.spawn(async { f() })
    }

    /// Wait for worker's event loop to complete.
    ///
    /// Joins the underlying OS thread handle. See [`JoinHandle::join`](thread::JoinHandle::join).
    pub fn join(self) -> thread::Result<()> {
        self.thread_handle.join()
    }

    /// Insert item into worker's thread-local storage.
    ///
    /// Overwrites any item of the same type previously inserted.
    pub fn set_item<T: 'static>(item: T) {
        STORAGE.with(move |cell| cell.borrow_mut().insert(TypeId::of::<T>(), Box::new(item)));
    }

    /// Check if worker's thread-local storage contains an item type.
    pub fn contains_item<T: 'static>() -> bool {
        STORAGE.with(move |cell| cell.borrow().contains_key(&TypeId::of::<T>()))
    }

    /// Call a function with a shared reference to an item in this worker's thread-local storage.
    ///
    /// # Panics
    /// Panics if item is not in worker's thread-local item storage.
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

    /// Call a function with a mutable reference to an item in this worker's thread-local storage.
    ///
    /// # Panics
    /// Panics if item is not in worker's thread-local item storage.
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

/// A persistent worker future that processes worker commands.
struct WorkerRunner {
    rx: mpsc::UnboundedReceiver<WorkerCommand>,
}


impl Future for WorkerRunner {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {

        // process all items currently buffered in channel
        loop {

            match ready!(Pin::new(&mut self.rx).poll_recv(cx)) {
                // channel closed; no more messages can be received
                None => return Poll::Ready(()),

                // process worker command
                Some(item) => match item {
                    WorkerCommand::Stop => {
                        return Poll::Ready(());
                    }
                    WorkerCommand::Execute(task_fut) => {
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
