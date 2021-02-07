use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use actix_rt::time::{sleep_until, Instant, Sleep};
use actix_rt::{spawn, Arbiter};
use actix_utils::counter::Counter;
use futures_core::future::LocalBoxFuture;
use log::{error, info, trace};
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use tokio::sync::oneshot;

use crate::service::{BoxedServerService, InternalServiceFactory};
use crate::socket::{MioStream, SocketAddr};
use crate::waker_queue::{WakerInterest, WakerQueue};
use crate::{join_all, Token};

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
    pub peer: Option<SocketAddr>,
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
    ) -> WorkerHandle {
        let (tx1, rx) = unbounded_channel();
        let (tx2, rx2) = unbounded_channel();
        let avail = availability.clone();

        // every worker runs in it's own arbiter.
        // use a custom tokio runtime builder to change the settings of runtime.
        Arbiter::with_tokio_rt(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .max_blocking_threads(config.max_blocking_threads)
                .build()
                .unwrap()
        })
        .spawn(async move {
            availability.set(false);
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

            let fut = wrk
                .factories
                .iter()
                .enumerate()
                .map(|(idx, factory)| {
                    let fut = factory.create();
                    async move {
                        fut.await.map(|r| {
                            r.into_iter().map(|(t, s)| (idx, t, s)).collect::<Vec<_>>()
                        })
                    }
                })
                .collect::<Vec<_>>();

            // a second spawn to make sure worker future runs as non boxed future.
            // As Arbiter::spawn would box the future before send it to arbiter.
            spawn(async move {
                let res: Result<Vec<_>, _> = join_all(fut).await.into_iter().collect();
                match res {
                    Ok(services) => {
                        for item in services {
                            for (factory, token, service) in item {
                                assert_eq!(token.0, wrk.services.len());
                                wrk.services.push(WorkerService {
                                    factory,
                                    service,
                                    status: WorkerServiceStatus::Unavailable,
                                });
                            }
                        }
                    }
                    Err(e) => {
                        error!("Can not start worker: {:?}", e);
                        Arbiter::current().stop();
                    }
                }
                wrk.await
            });
        });

        WorkerHandle::new(idx, tx1, tx2, avail)
    }

    fn shutdown(&mut self, force: bool) {
        if force {
            self.services.iter_mut().for_each(|srv| {
                if srv.status == WorkerServiceStatus::Available {
                    srv.status = WorkerServiceStatus::Stopped;
                }
            });
        } else {
            self.services.iter_mut().for_each(move |srv| {
                if srv.status == WorkerServiceStatus::Available {
                    srv.status = WorkerServiceStatus::Stopping;
                }
            });
        }
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
                let num = num_connections();
                if num != 0 {
                    info!("Graceful worker shutdown, {} connections", num);
                    self.state = WorkerState::Shutdown(
                        Box::pin(sleep_until(Instant::now() + Duration::from_secs(1))),
                        Box::pin(sleep_until(Instant::now() + self.config.shutdown_timeout)),
                        Some(result),
                    );
                } else {
                    let _ = result.send(true);
                    return Poll::Ready(());
                }
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
                    trace!(
                        "Service {:?} failed, restarting",
                        self.factories[idx].name(token)
                    );
                    self.services[token.0].status = WorkerServiceStatus::Restarting;
                    self.state =
                        WorkerState::Restarting(idx, token, self.factories[idx].create());
                    self.poll(cx)
                }
            },
            WorkerState::Restarting(idx, token, ref mut fut) => {
                match fut.as_mut().poll(cx) {
                    Poll::Ready(Ok(item)) => {
                        // only interest in the first item?
                        if let Some((token, service)) = item.into_iter().next() {
                            trace!(
                                "Service {:?} has been restarted",
                                self.factories[idx].name(token)
                            );
                            self.services[token.0].created(service);
                            self.state = WorkerState::Unavailable;
                            return self.poll(cx);
                        }
                    }
                    Poll::Ready(Err(_)) => {
                        panic!(
                            "Can not restart {:?} service",
                            self.factories[idx].name(token)
                        );
                    }
                    Poll::Pending => return Poll::Pending,
                }
                self.poll(cx)
            }
            WorkerState::Shutdown(ref mut t1, ref mut t2, ref mut tx) => {
                let num = num_connections();
                if num == 0 {
                    let _ = tx.take().unwrap().send(true);
                    Arbiter::current().stop();
                    return Poll::Ready(());
                }

                // check graceful timeout
                if Pin::new(t2).poll(cx).is_ready() {
                    let _ = tx.take().unwrap().send(false);
                    self.shutdown(true);
                    Arbiter::current().stop();
                    return Poll::Ready(());
                }

                // sleep for 1 second and then check again
                if t1.as_mut().poll(cx).is_ready() {
                    *t1 = Box::pin(sleep_until(Instant::now() + Duration::from_secs(1)));
                    let _ = t1.as_mut().poll(cx);
                }

                Poll::Pending
            }
            // actively poll stream and handle worker command
            WorkerState::Available => loop {
                match self.check_readiness(cx) {
                    Ok(true) => (),
                    Ok(false) => {
                        trace!("Worker is unavailable");
                        self.availability.set(false);
                        self.state = WorkerState::Unavailable;
                        return self.poll(cx);
                    }
                    Err((token, idx)) => {
                        trace!(
                            "Service {:?} failed, restarting",
                            self.factories[idx].name(token)
                        );
                        self.availability.set(false);
                        self.services[token.0].status = WorkerServiceStatus::Restarting;
                        self.state =
                            WorkerState::Restarting(idx, token, self.factories[idx].create());
                        return self.poll(cx);
                    }
                }

                match Pin::new(&mut self.rx).poll_recv(cx) {
                    // handle incoming io stream
                    Poll::Ready(Some(WorkerCommand(msg))) => {
                        let guard = self.conns.get();
                        let _ = self.services[msg.token.0]
                            .service
                            .call((Some(guard), msg.io));
                    }
                    Poll::Pending => return Poll::Pending,
                    Poll::Ready(None) => return Poll::Ready(()),
                };
            },
        }
    }
}
