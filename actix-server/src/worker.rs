use std::{
    future::Future,
    io, mem,
    pin::Pin,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    task::{Context, Poll},
    thread,
    time::Duration,
};

use actix_rt::{
    task::yield_now,
    time::{sleep, Instant, Sleep},
    System,
};
use actix_utils::counter::Counter;
use futures_core::{future::LocalBoxFuture, ready};
use log::{error, info, trace};
use tokio::sync::{
    mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender},
    oneshot,
};

use crate::service::{BoxedServerService, InternalServiceFactory};
use crate::socket::MioStream;
use crate::waker_queue::{WakerInterest, WakerQueue};
use crate::Token;

/// Stop worker message. Returns `true` on successful graceful shutdown.
/// and `false` if some connections still alive when shutdown execute.
struct Stop {
    graceful: bool,
    tx: oneshot::Sender<bool>,
}

#[derive(Debug)]
pub(crate) struct Conn {
    pub io: MioStream,
    pub token: Token,
}

fn handle_pair(
    idx: usize,
    tx1: UnboundedSender<Conn>,
    tx2: UnboundedSender<Stop>,
    avail: WorkerAvailability,
) -> (WorkerHandleAccept, WorkerHandleServer) {
    let accept = WorkerHandleAccept { tx: tx1, avail };

    let server = WorkerHandleServer { idx, tx: tx2 };

    (accept, server)
}

/// Handle to worker that can send connection message to worker and share the
/// availability of worker to other thread.
///
/// Held by [Accept](crate::accept::Accept).
pub(crate) struct WorkerHandleAccept {
    tx: UnboundedSender<Conn>,
    avail: WorkerAvailability,
}

impl WorkerHandleAccept {
    #[inline(always)]
    pub(crate) fn idx(&self) -> usize {
        self.avail.idx
    }

    #[inline(always)]
    pub(crate) fn send(&self, msg: Conn) -> Result<(), Conn> {
        self.tx.send(msg).map_err(|msg| msg.0)
    }

    #[inline(always)]
    pub(crate) fn available(&self) -> bool {
        self.avail.available()
    }
}

/// Handle to worker than can send stop message to worker.
///
/// Held by [ServerBuilder](crate::builder::ServerBuilder).
pub(crate) struct WorkerHandleServer {
    pub idx: usize,
    tx: UnboundedSender<Stop>,
}

impl WorkerHandleServer {
    pub(crate) fn stop(&self, graceful: bool) -> oneshot::Receiver<bool> {
        let (tx, rx) = oneshot::channel();
        let _ = self.tx.send(Stop { graceful, tx });
        rx
    }
}

#[derive(Clone)]
pub(crate) struct WorkerAvailability {
    idx: usize,
    waker: WakerQueue,
    available: Arc<AtomicBool>,
}

impl WorkerAvailability {
    pub fn new(idx: usize, waker: WakerQueue) -> Self {
        WorkerAvailability {
            idx,
            waker,
            available: Arc::new(AtomicBool::new(false)),
        }
    }

    #[inline(always)]
    pub fn available(&self) -> bool {
        self.available.load(Ordering::Acquire)
    }

    pub fn set(&self, val: bool) {
        // Ordering:
        //
        // There could be multiple set calls happen in one <ServerWorker as Future>::poll.
        // Order is important between them.
        let old = self.available.swap(val, Ordering::AcqRel);
        // Notify the accept on switched to available.
        if !old && val {
            self.waker.wake(WakerInterest::WorkerAvailable(self.idx));
        }
    }
}

/// Service worker.
///
/// Worker accepts Socket objects via unbounded channel and starts stream processing.
pub(crate) struct ServerWorker {
    // UnboundedReceiver<Conn> should always be the first field.
    // It must be dropped as soon as ServerWorker dropping.
    rx: UnboundedReceiver<Conn>,
    rx2: UnboundedReceiver<Stop>,
    services: Box<[WorkerService]>,
    availability: WorkerAvailability,
    conns: Counter,
    factories: Box<[Box<dyn InternalServiceFactory>]>,
    state: WorkerState,
    shutdown_timeout: Duration,
}

struct WorkerService {
    factory: usize,
    status: WorkerServiceStatus,
    service: BoxedServerService,
}

impl WorkerService {
    fn created(&mut self, service: BoxedServerService) {
        self.service = service;
        self.status = WorkerServiceStatus::Unavailable;
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum WorkerServiceStatus {
    Available,
    Unavailable,
    Failed,
    Restarting,
    Stopping,
    Stopped,
}

/// Config for worker behavior passed down from server builder.
#[derive(Copy, Clone)]
pub(crate) struct ServerWorkerConfig {
    shutdown_timeout: Duration,
    max_blocking_threads: usize,
    max_concurrent_connections: usize,
}

impl Default for ServerWorkerConfig {
    fn default() -> Self {
        // 512 is the default max blocking thread count of tokio runtime.
        let max_blocking_threads = std::cmp::max(512 / num_cpus::get(), 1);
        Self {
            shutdown_timeout: Duration::from_secs(30),
            max_blocking_threads,
            max_concurrent_connections: 25600,
        }
    }
}

impl ServerWorkerConfig {
    pub(crate) fn max_blocking_threads(&mut self, num: usize) {
        self.max_blocking_threads = num;
    }

    pub(crate) fn max_concurrent_connections(&mut self, num: usize) {
        self.max_concurrent_connections = num;
    }

    pub(crate) fn shutdown_timeout(&mut self, dur: Duration) {
        self.shutdown_timeout = dur;
    }
}

impl ServerWorker {
    /// Start server worker in sync.
    pub(crate) fn start(
        idx: usize,
        factories: Box<[Box<dyn InternalServiceFactory>]>,
        avail: WorkerAvailability,
        config: ServerWorkerConfig,
    ) -> io::Result<(WorkerHandleAccept, WorkerHandleServer)> {
        let (factory_tx, factory_rx) = std::sync::mpsc::sync_channel(1);

        let (tx1, tx2) = Self::_start(idx, factories, avail.clone(), config, move |err| {
            factory_tx.send(err).unwrap()
        })?;

        factory_rx
            .recv()
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?
            .map(Err)
            .unwrap_or_else(|| Ok(handle_pair(idx, tx1, tx2, avail)))
    }

    // TODO: Use start_non_blocking when restarting worker.
    #[allow(dead_code)]
    /// Start server worker in async.
    pub(crate) fn start_non_blocking(
        idx: usize,
        factories: Box<[Box<dyn InternalServiceFactory>]>,
        avail: WorkerAvailability,
        config: ServerWorkerConfig,
    ) -> impl Future<Output = io::Result<(WorkerHandleAccept, WorkerHandleServer)>> {
        let (factory_tx, factory_rx) = oneshot::channel();

        let res = Self::_start(idx, factories, avail.clone(), config, move |err| {
            factory_tx.send(err).unwrap()
        });

        async move {
            let (tx1, tx2) = res?;
            factory_rx
                .await
                .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?
                .map(Err)
                .unwrap_or_else(|| Ok(handle_pair(idx, tx1, tx2, avail)))
        }
    }

    fn _start<F>(
        idx: usize,
        factories: Box<[Box<dyn InternalServiceFactory>]>,
        availability: WorkerAvailability,
        config: ServerWorkerConfig,
        f: F,
    ) -> io::Result<(UnboundedSender<Conn>, UnboundedSender<Stop>)>
    where
        F: FnOnce(Option<io::Error>) + Send + 'static,
    {
        assert!(!availability.available());

        // Try to get actix system.
        let system = System::try_current();

        // Try to get tokio runtime handle.
        let rt_handle = tokio::runtime::Handle::try_current();

        let (tx1, rx) = unbounded_channel();
        let (tx2, rx2) = unbounded_channel();

        thread::Builder::new()
            .name(format!("actix-server-worker-{}", idx))
            .spawn(move || {
                // Prepare service construct future.
                let fut = async {
                    let mut services = Vec::new();

                    for (idx, factory) in factories.iter().enumerate() {
                        let (token, service) = factory.create().await.map_err(|_| {
                            io::Error::new(io::ErrorKind::Other, "Can not start worker service")
                        })?;

                        assert_eq!(token, services.len());
                        services.push(WorkerService {
                            factory: idx,
                            service,
                            status: WorkerServiceStatus::Unavailable,
                        });
                    }
                    Ok::<_, io::Error>(services)
                };

                // All future runs in a LocalSet for being able to run !Send future.
                let local = tokio::task::LocalSet::new();

                match (rt_handle, system) {
                    (Ok(handle), None) => {
                        // Use existing tokio runtime with handle.
                        let res = handle.block_on(local.run_until(fut));

                        match res {
                            Ok(services) => {
                                let worker = ServerWorker {
                                    rx,
                                    rx2,
                                    services: services.into_boxed_slice(),
                                    availability,
                                    conns: Counter::new(config.max_concurrent_connections),
                                    factories,
                                    state: Default::default(),
                                    shutdown_timeout: config.shutdown_timeout,
                                };

                                let worker = local.spawn_local(worker);

                                handle.block_on(local.run_until(async {
                                    yield_now().await;
                                    f(None);
                                    let _ = worker.await;
                                }));
                            }
                            Err(e) => f(Some(e)),
                        }
                    }
                    (_, system) => {
                        // Conditionally setup actix system.
                        if let Some(system) = system {
                            System::set_current(system);
                        }

                        // No existing tokio runtime found. Start new runtime.
                        let rt = tokio::runtime::Builder::new_current_thread()
                            .enable_all()
                            .max_blocking_threads(config.max_blocking_threads)
                            .build();

                        let res = rt.and_then(|rt| {
                            local.block_on(&rt, fut).map(|services| (services, rt))
                        });

                        match res {
                            Ok((services, rt)) => {
                                let worker = ServerWorker {
                                    rx,
                                    rx2,
                                    services: services.into_boxed_slice(),
                                    availability,
                                    conns: Counter::new(config.max_concurrent_connections),
                                    factories,
                                    state: Default::default(),
                                    shutdown_timeout: config.shutdown_timeout,
                                };

                                let handle = local.spawn_local(worker);

                                local.block_on(&rt, async {
                                    yield_now().await;
                                    f(None);
                                    let _ = handle.await;
                                });
                            }
                            Err(e) => f(Some(e)),
                        }
                    }
                }
            })
            .map(|_| (tx1, tx2))
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))
    }

    fn restart_service(&mut self, token: Token, factory_id: usize) {
        let factory = &self.factories[factory_id];
        trace!("Service {:?} failed, restarting", factory.name(token));
        self.services[token.0].status = WorkerServiceStatus::Restarting;
        self.state = WorkerState::Restarting(Restart {
            factory_id,
            token,
            fut: factory.create(),
        });
    }

    fn shutdown(&mut self, force: bool) {
        self.services
            .iter_mut()
            .filter(|srv| srv.status == WorkerServiceStatus::Available)
            .for_each(|srv| {
                srv.status = if force {
                    WorkerServiceStatus::Stopped
                } else {
                    WorkerServiceStatus::Stopping
                };
            });
    }

    fn check_readiness(&mut self, cx: &mut Context<'_>) -> Result<bool, (Token, usize)> {
        let mut ready = self.conns.available(cx);
        for (idx, srv) in self.services.iter_mut().enumerate() {
            if srv.status == WorkerServiceStatus::Available
                || srv.status == WorkerServiceStatus::Unavailable
            {
                match srv.service.poll_ready(cx) {
                    Poll::Ready(Ok(_)) => {
                        if srv.status == WorkerServiceStatus::Unavailable {
                            trace!(
                                "Service {:?} is available",
                                self.factories[srv.factory].name(Token(idx))
                            );
                            srv.status = WorkerServiceStatus::Available;
                        }
                    }
                    Poll::Pending => {
                        ready = false;

                        if srv.status == WorkerServiceStatus::Available {
                            trace!(
                                "Service {:?} is unavailable",
                                self.factories[srv.factory].name(Token(idx))
                            );
                            srv.status = WorkerServiceStatus::Unavailable;
                        }
                    }
                    Poll::Ready(Err(_)) => {
                        error!(
                            "Service {:?} readiness check returned error, restarting",
                            self.factories[srv.factory].name(Token(idx))
                        );
                        srv.status = WorkerServiceStatus::Failed;
                        return Err((Token(idx), srv.factory));
                    }
                }
            }
        }

        Ok(ready)
    }
}

enum WorkerState {
    Available,
    Unavailable,
    Restarting(Restart),
    Shutdown(Shutdown),
}

struct Restart {
    factory_id: usize,
    token: Token,
    fut: LocalBoxFuture<'static, Result<(Token, BoxedServerService), ()>>,
}

// Shutdown keep states necessary for server shutdown:
// Sleep for interval check the shutdown progress.
// Instant for the start time of shutdown.
// Sender for send back the shutdown outcome(force/grace) to StopCommand caller.
struct Shutdown {
    timer: Pin<Box<Sleep>>,
    start_from: Instant,
    tx: oneshot::Sender<bool>,
}

impl Default for WorkerState {
    fn default() -> Self {
        Self::Unavailable
    }
}

impl Drop for ServerWorker {
    fn drop(&mut self) {
        // Set availability to true so if accept try to send connection to this worker
        // it would find worker is gone and remove it.
        // This is helpful when worker is dropped unexpected.
        self.availability.set(true);
    }
}

impl Future for ServerWorker {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.as_mut().get_mut();

        // `StopWorker` message handler
        if let Poll::Ready(Some(Stop { graceful, tx })) = Pin::new(&mut this.rx2).poll_recv(cx)
        {
            this.availability.set(false);
            let num = this.conns.total();
            if num == 0 {
                info!("Shutting down worker, 0 connections");
                let _ = tx.send(true);
                return Poll::Ready(());
            } else if graceful {
                info!("Graceful worker shutdown, {} connections", num);
                this.shutdown(false);

                this.state = WorkerState::Shutdown(Shutdown {
                    timer: Box::pin(sleep(Duration::from_secs(1))),
                    start_from: Instant::now(),
                    tx,
                });
            } else {
                info!("Force shutdown worker, {} connections", num);
                this.shutdown(true);

                let _ = tx.send(false);
                return Poll::Ready(());
            }
        }

        match this.state {
            WorkerState::Unavailable => match this.check_readiness(cx) {
                Ok(true) => {
                    this.state = WorkerState::Available;
                    this.availability.set(true);
                    self.poll(cx)
                }
                Ok(false) => Poll::Pending,
                Err((token, idx)) => {
                    this.restart_service(token, idx);
                    self.poll(cx)
                }
            },
            WorkerState::Restarting(ref mut restart) => {
                let factory_id = restart.factory_id;
                let token = restart.token;

                let (token_new, service) = ready!(restart.fut.as_mut().poll(cx))
                    .unwrap_or_else(|_| {
                        panic!(
                            "Can not restart {:?} service",
                            this.factories[factory_id].name(token)
                        )
                    });

                assert_eq!(token, token_new);

                trace!(
                    "Service {:?} has been restarted",
                    this.factories[factory_id].name(token)
                );

                this.services[token.0].created(service);
                this.state = WorkerState::Unavailable;

                self.poll(cx)
            }
            WorkerState::Shutdown(ref mut shutdown) => {
                // Wait for 1 second.
                ready!(shutdown.timer.as_mut().poll(cx));

                if this.conns.total() == 0 {
                    // Graceful shutdown.
                    if let WorkerState::Shutdown(shutdown) = mem::take(&mut this.state) {
                        let _ = shutdown.tx.send(true);
                    }
                    Poll::Ready(())
                } else if shutdown.start_from.elapsed() >= this.shutdown_timeout {
                    // Timeout forceful shutdown.
                    if let WorkerState::Shutdown(shutdown) = mem::take(&mut this.state) {
                        let _ = shutdown.tx.send(false);
                    }
                    Poll::Ready(())
                } else {
                    // Reset timer and wait for 1 second.
                    let time = Instant::now() + Duration::from_secs(1);
                    shutdown.timer.as_mut().reset(time);
                    shutdown.timer.as_mut().poll(cx)
                }
            }
            // actively poll stream and handle worker command
            WorkerState::Available => loop {
                match this.check_readiness(cx) {
                    Ok(true) => {}
                    Ok(false) => {
                        trace!("Worker is unavailable");
                        this.availability.set(false);
                        this.state = WorkerState::Unavailable;
                        return self.poll(cx);
                    }
                    Err((token, idx)) => {
                        this.restart_service(token, idx);
                        this.availability.set(false);
                        return self.poll(cx);
                    }
                }

                match ready!(Pin::new(&mut this.rx).poll_recv(cx)) {
                    // handle incoming io stream
                    Some(msg) => {
                        let guard = this.conns.get();
                        let _ = this.services[msg.token.0].service.call((guard, msg.io));
                    }
                    None => return Poll::Ready(()),
                };
            },
        }
    }
}
