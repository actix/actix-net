use std::future::Future;
use std::io;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll};
use std::thread;
use std::time::Duration;

use actix_rt::{
    time::{sleep, Sleep},
    System,
};
use actix_utils::counter::Counter;
use futures_core::{future::LocalBoxFuture, ready};
use log::{error, info, trace};
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use tokio::sync::oneshot;

use crate::service::{BoxedServerService, InternalServiceFactory};
use crate::socket::MioStream;
use crate::waker_queue::{WakerInterest, WakerQueue};
use crate::Token;

pub(crate) struct WorkerCommand(Conn);

/// Stop worker message. Returns `true` on successful shutdown
/// and `false` if some connections still alive.
pub(crate) struct StopCommand {
    graceful: bool,
    result: oneshot::Sender<bool>,
}

#[derive(Debug)]
pub(crate) struct Conn {
    pub io: MioStream,
    pub token: Token,
}

static MAX_CONNS: AtomicUsize = AtomicUsize::new(25600);

/// Sets the maximum per-worker number of concurrent connections.
///
/// All socket listeners will stop accepting connections when this limit is
/// reached for each worker.
///
/// By default max connections is set to a 25k per worker.
pub fn max_concurrent_connections(num: usize) {
    MAX_CONNS.store(num, Ordering::Relaxed);
}

thread_local! {
    static MAX_CONNS_COUNTER: Counter =
        Counter::new(MAX_CONNS.load(Ordering::Relaxed));
}

pub(crate) fn num_connections() -> usize {
    MAX_CONNS_COUNTER.with(|conns| conns.total())
}

// a handle to worker that can send message to worker and share the availability of worker to other
// thread.
#[derive(Clone)]
pub(crate) struct WorkerHandle {
    pub idx: usize,
    tx1: UnboundedSender<WorkerCommand>,
    tx2: UnboundedSender<StopCommand>,
    avail: WorkerAvailability,
}

impl WorkerHandle {
    pub fn new(
        idx: usize,
        tx1: UnboundedSender<WorkerCommand>,
        tx2: UnboundedSender<StopCommand>,
        avail: WorkerAvailability,
    ) -> Self {
        WorkerHandle {
            idx,
            tx1,
            tx2,
            avail,
        }
    }

    pub fn send(&self, msg: Conn) -> Result<(), Conn> {
        self.tx1.send(WorkerCommand(msg)).map_err(|msg| msg.0 .0)
    }

    pub fn available(&self) -> bool {
        self.avail.available()
    }

    pub fn stop(&self, graceful: bool) -> oneshot::Receiver<bool> {
        let (result, rx) = oneshot::channel();
        let _ = self.tx2.send(StopCommand { graceful, result });
        rx
    }
}

#[derive(Clone)]
pub(crate) struct WorkerAvailability {
    waker: WakerQueue,
    available: Arc<AtomicBool>,
}

impl WorkerAvailability {
    pub fn new(waker: WakerQueue) -> Self {
        WorkerAvailability {
            waker,
            available: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn available(&self) -> bool {
        self.available.load(Ordering::Acquire)
    }

    pub fn set(&self, val: bool) {
        let old = self.available.swap(val, Ordering::Release);
        // notify the accept on switched to available.
        if !old && val {
            self.waker.wake(WakerInterest::WorkerAvailable);
        }
    }
}

/// Service worker.
///
/// Worker accepts Socket objects via unbounded channel and starts stream processing.
pub(crate) struct ServerWorker {
    rx: UnboundedReceiver<WorkerCommand>,
    rx2: UnboundedReceiver<StopCommand>,
    services: Vec<WorkerService>,
    availability: WorkerAvailability,
    conns: Counter,
    factories: Vec<Box<dyn InternalServiceFactory>>,
    state: WorkerState,
    config: ServerWorkerConfig,
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
}

impl Default for ServerWorkerConfig {
    fn default() -> Self {
        // 512 is the default max blocking thread count of tokio runtime.
        let max_blocking_threads = std::cmp::max(512 / num_cpus::get(), 1);
        Self {
            shutdown_timeout: Duration::from_secs(30),
            max_blocking_threads,
        }
    }
}

impl ServerWorkerConfig {
    pub(crate) fn max_blocking_threads(&mut self, num: usize) {
        self.max_blocking_threads = num;
    }

    pub(crate) fn shutdown_timeout(&mut self, dur: Duration) {
        self.shutdown_timeout = dur;
    }
}

impl ServerWorker {
    pub(crate) fn start(
        idx: usize,
        factories: Vec<Box<dyn InternalServiceFactory>>,
        availability: WorkerAvailability,
        config: ServerWorkerConfig,
    ) -> io::Result<WorkerHandle> {
        let (tx1, rx) = unbounded_channel();
        let (tx2, rx2) = unbounded_channel();
        let avail = availability.clone();

        availability.set(false);

        // Try to get actix system when have one.
        let system = System::try_current();

        let (factory_tx, factory_rx) = std::sync::mpsc::sync_channel(1);

        // every worker runs in it's own thread.
        thread::Builder::new()
            .name(format!("actix-server-worker-{}", idx))
            .spawn(move || {
                // conditionally setup actix system.
                if let Some(system) = system {
                    System::set_current(system);
                }

                let mut wrk = MAX_CONNS_COUNTER.with(move |conns| ServerWorker {
                    rx,
                    rx2,
                    availability,
                    factories,
                    config,
                    services: Vec::new(),
                    conns: conns.clone(),
                    state: WorkerState::Unavailable,
                });

                // use a custom tokio runtime builder to change the settings of runtime.
                let local = tokio::task::LocalSet::new();
                let res = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .max_blocking_threads(config.max_blocking_threads)
                    .build()
                    .and_then(|rt| {
                        local.block_on(&rt, async {
                            for (idx, factory) in wrk.factories.iter().enumerate() {
                                let service = factory.create().await.map_err(|_| {
                                    io::Error::new(
                                        io::ErrorKind::Other,
                                        "Can not start worker service",
                                    )
                                })?;

                                for (token, service) in service {
                                    assert_eq!(token.0, wrk.services.len());
                                    wrk.services.push(WorkerService {
                                        factory: idx,
                                        service,
                                        status: WorkerServiceStatus::Unavailable,
                                    })
                                }
                            }
                            Ok::<_, io::Error>(())
                        })?;

                        Ok(rt)
                    });

                match res {
                    Ok(rt) => {
                        factory_tx.send(None).unwrap();
                        local.block_on(&rt, wrk)
                    }
                    Err(e) => factory_tx.send(Some(e)).unwrap(),
                }
            })
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;

        factory_rx
            .recv()
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?
            .map(Err)
            .unwrap_or_else(|| Ok(WorkerHandle::new(idx, tx1, tx2, avail)))
    }

    fn restart_service(&mut self, token: Token, idx: usize) {
        let factory = &self.factories[idx];
        trace!("Service {:?} failed, restarting", factory.name(token));
        self.services[token.0].status = WorkerServiceStatus::Restarting;
        self.state = WorkerState::Restarting(idx, token, factory.create());
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
        let mut failed = None;
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
                        failed = Some((Token(idx), srv.factory));
                        srv.status = WorkerServiceStatus::Failed;
                    }
                }
            }
        }
        if let Some(idx) = failed {
            Err(idx)
        } else {
            Ok(ready)
        }
    }
}

enum WorkerState {
    Available,
    Unavailable,
    Restarting(
        usize,
        Token,
        LocalBoxFuture<'static, Result<Vec<(Token, BoxedServerService)>, ()>>,
    ),
    Shutdown(
        Pin<Box<Sleep>>,
        Pin<Box<Sleep>>,
        Option<oneshot::Sender<bool>>,
    ),
}

impl Future for ServerWorker {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // `StopWorker` message handler
        if let Poll::Ready(Some(StopCommand { graceful, result })) =
            Pin::new(&mut self.rx2).poll_recv(cx)
        {
            self.availability.set(false);
            let num = num_connections();
            if num == 0 {
                info!("Shutting down worker, 0 connections");
                let _ = result.send(true);
                return Poll::Ready(());
            } else if graceful {
                self.shutdown(false);
                info!("Graceful worker shutdown, {} connections", num);
                self.state = WorkerState::Shutdown(
                    Box::pin(sleep(Duration::from_secs(1))),
                    Box::pin(sleep(self.config.shutdown_timeout)),
                    Some(result),
                );
            } else {
                info!("Force shutdown worker, {} connections", num);
                self.shutdown(true);
                let _ = result.send(false);
                return Poll::Ready(());
            }
        }

        match self.state {
            WorkerState::Unavailable => match self.check_readiness(cx) {
                Ok(true) => {
                    self.state = WorkerState::Available;
                    self.availability.set(true);
                    self.poll(cx)
                }
                Ok(false) => Poll::Pending,
                Err((token, idx)) => {
                    self.restart_service(token, idx);
                    self.poll(cx)
                }
            },
            WorkerState::Restarting(idx, token, ref mut fut) => {
                let item = ready!(fut.as_mut().poll(cx)).unwrap_or_else(|_| {
                    panic!(
                        "Can not restart {:?} service",
                        self.factories[idx].name(token)
                    )
                });

                // Only interest in the first item?
                let (token, service) = item
                    .into_iter()
                    .next()
                    .expect("No BoxedServerService. Restarting can not progress");

                trace!(
                    "Service {:?} has been restarted",
                    self.factories[idx].name(token)
                );

                self.services[token.0].created(service);
                self.state = WorkerState::Unavailable;

                self.poll(cx)
            }
            WorkerState::Shutdown(ref mut t1, ref mut t2, ref mut tx) => {
                let num = num_connections();
                if num == 0 {
                    let _ = tx.take().unwrap().send(true);
                    return Poll::Ready(());
                }

                // check graceful timeout
                if Pin::new(t2).poll(cx).is_ready() {
                    let _ = tx.take().unwrap().send(false);
                    self.shutdown(true);
                    return Poll::Ready(());
                }

                // sleep for 1 second and then check again
                if t1.as_mut().poll(cx).is_ready() {
                    *t1 = Box::pin(sleep(Duration::from_secs(1)));
                    let _ = t1.as_mut().poll(cx);
                }

                Poll::Pending
            }
            // actively poll stream and handle worker command
            WorkerState::Available => loop {
                match self.check_readiness(cx) {
                    Ok(true) => {}
                    Ok(false) => {
                        trace!("Worker is unavailable");
                        self.availability.set(false);
                        self.state = WorkerState::Unavailable;
                        return self.poll(cx);
                    }
                    Err((token, idx)) => {
                        self.restart_service(token, idx);
                        self.availability.set(false);
                        return self.poll(cx);
                    }
                }

                match ready!(Pin::new(&mut self.rx).poll_recv(cx)) {
                    // handle incoming io stream
                    Some(WorkerCommand(msg)) => {
                        let guard = self.conns.get();
                        let _ = self.services[msg.token.0].service.call((guard, msg.io));
                    }
                    None => return Poll::Ready(()),
                };
            },
        }
    }
}
