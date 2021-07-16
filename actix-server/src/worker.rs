use std::{
    future::Future,
    mem,
    pin::Pin,
    rc::Rc,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    task::{Context, Poll},
    time::Duration,
};

use actix_rt::{
    spawn,
    time::{sleep, Instant, Sleep},
    Arbiter,
};
use futures_core::{future::LocalBoxFuture, ready};
use log::{error, info, trace};
use tokio::sync::{
    mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender},
    oneshot,
};

use crate::join_all;
use crate::service::{BoxedServerService, InternalServiceFactory};
use crate::socket::MioStream;
use crate::waker_queue::{WakerInterest, WakerQueue};

/// Stop worker message. Returns `true` on successful graceful shutdown.
/// and `false` if some connections still alive when shutdown execute.
pub(crate) struct Stop {
    graceful: bool,
    tx: oneshot::Sender<bool>,
}

#[derive(Debug)]
pub(crate) struct Conn {
    pub io: MioStream,
    pub token: usize,
}

fn handle_pair(
    idx: usize,
    tx1: UnboundedSender<Conn>,
    tx2: UnboundedSender<Stop>,
    counter: Counter,
) -> (WorkerHandleAccept, WorkerHandleServer) {
    let accept = WorkerHandleAccept {
        idx,
        tx: tx1,
        counter,
    };

    let server = WorkerHandleServer { idx, tx: tx2 };

    (accept, server)
}

/// counter: Arc<AtomicUsize> field is owned by `Accept` thread and `ServerWorker` thread.
///
/// `Accept` would increment the counter and `ServerWorker` would decrement it.
///
/// # Atomic Ordering:
///
/// `Accept` always look into it's cached `Availability` field for `ServerWorker` state.
/// It lazily increment counter after successful dispatching new work to `ServerWorker`.
/// On reaching counter limit `Accept` update it's cached `Availability` and mark worker as
/// unable to accept any work.
///
/// `ServerWorker` always decrement the counter when every work received from `Accept` is done.
/// On reaching counter limit worker would use `mio::Waker` and `WakerQueue` to wake up `Accept`
/// and notify it to update cached `Availability` again to mark worker as able to accept work again.
///
/// Hence, a wake up would only happen after `Accept` increment it to limit.
/// And a decrement to limit always wake up `Accept`.
#[derive(Clone)]
pub(crate) struct Counter {
    counter: Arc<AtomicUsize>,
    limit: usize,
}

impl Counter {
    pub(crate) fn new(limit: usize) -> Self {
        Self {
            counter: Arc::new(AtomicUsize::new(1)),
            limit,
        }
    }

    /// Increment counter by 1 and return true when hitting limit
    #[inline(always)]
    pub(crate) fn inc(&self) -> bool {
        self.counter.fetch_add(1, Ordering::Relaxed) != self.limit
    }

    /// Decrement counter by 1 and return true if crossing limit.
    #[inline(always)]
    pub(crate) fn dec(&self) -> bool {
        self.counter.fetch_sub(1, Ordering::Relaxed) == self.limit
    }

    pub(crate) fn total(&self) -> usize {
        self.counter.load(Ordering::SeqCst) - 1
    }
}

pub(crate) struct WorkerCounter {
    idx: usize,
    inner: Rc<(WakerQueue, Counter)>,
}

impl Clone for WorkerCounter {
    fn clone(&self) -> Self {
        Self {
            idx: self.idx,
            inner: self.inner.clone(),
        }
    }
}

impl WorkerCounter {
    pub(crate) fn new(idx: usize, waker_queue: WakerQueue, counter: Counter) -> Self {
        Self {
            idx,
            inner: Rc::new((waker_queue, counter)),
        }
    }

    #[inline(always)]
    pub(crate) fn guard(&self) -> WorkerCounterGuard {
        WorkerCounterGuard(self.clone())
    }

    fn total(&self) -> usize {
        self.inner.1.total()
    }
}

pub(crate) struct WorkerCounterGuard(WorkerCounter);

impl Drop for WorkerCounterGuard {
    fn drop(&mut self) {
        let (waker_queue, counter) = &*self.0.inner;
        if counter.dec() {
            waker_queue.wake(WakerInterest::WorkerAvailable(self.0.idx));
        }
    }
}

/// Handle to worker that can send connection message to worker and share the
/// availability of worker to other thread.
///
/// Held by [Accept](crate::accept::Accept).
pub(crate) struct WorkerHandleAccept {
    idx: usize,
    tx: UnboundedSender<Conn>,
    counter: Counter,
}

impl WorkerHandleAccept {
    #[inline(always)]
    pub(crate) fn idx(&self) -> usize {
        self.idx
    }

    #[inline(always)]
    pub(crate) fn send(&self, msg: Conn) -> Result<(), Conn> {
        self.tx.send(msg).map_err(|msg| msg.0)
    }

    #[inline(always)]
    pub(crate) fn inc_counter(&self) -> bool {
        self.counter.inc()
    }
}

/// Handle to worker than can send stop message to worker.
///
/// Held by [ServerBuilder](crate::builder::ServerBuilder).
#[derive(Debug)]
pub(crate) struct WorkerHandleServer {
    idx: usize,
    tx: UnboundedSender<Stop>,
}

impl WorkerHandleServer {
    pub(crate) fn stop(&self, graceful: bool) -> oneshot::Receiver<bool> {
        let (tx, rx) = oneshot::channel();
        let _ = self.tx.send(Stop { graceful, tx });
        rx
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
    counter: WorkerCounter,
    services: Box<[WorkerService]>,
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
    pub(crate) fn start(
        idx: usize,
        factories: Vec<Box<dyn InternalServiceFactory>>,
        waker_queue: WakerQueue,
        config: ServerWorkerConfig,
    ) -> (WorkerHandleAccept, WorkerHandleServer) {
        let (tx1, rx) = unbounded_channel();
        let (tx2, rx2) = unbounded_channel();

        let counter = Counter::new(config.max_concurrent_connections);

        let counter_clone = counter.clone();
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
            let fut = factories
                .iter()
                .enumerate()
                .map(|(idx, factory)| {
                    let fut = factory.create();
                    async move { fut.await.map(|(t, s)| (idx, t, s)) }
                })
                .collect::<Vec<_>>();

            // a second spawn to run !Send future tasks.
            spawn(async move {
                let res = join_all(fut)
                    .await
                    .into_iter()
                    .collect::<Result<Vec<_>, _>>();
                let services = match res {
                    Ok(res) => res
                        .into_iter()
                        .fold(Vec::new(), |mut services, (factory, token, service)| {
                            assert_eq!(token, services.len());
                            services.push(WorkerService {
                                factory,
                                service,
                                status: WorkerServiceStatus::Unavailable,
                            });
                            services
                        })
                        .into_boxed_slice(),
                    Err(e) => {
                        error!("Can not start worker: {:?}", e);
                        Arbiter::current().stop();
                        return;
                    }
                };

                // a third spawn to make sure ServerWorker runs as non boxed future.
                spawn(ServerWorker {
                    rx,
                    rx2,
                    services,
                    counter: WorkerCounter::new(idx, waker_queue, counter_clone),
                    factories: factories.into_boxed_slice(),
                    state: Default::default(),
                    shutdown_timeout: config.shutdown_timeout,
                });
            });
        });

        handle_pair(idx, tx1, tx2, counter)
    }

    fn restart_service(&mut self, idx: usize, factory_id: usize) {
        let factory = &self.factories[factory_id];
        trace!("Service {:?} failed, restarting", factory.name(idx));
        self.services[idx].status = WorkerServiceStatus::Restarting;
        self.state = WorkerState::Restarting(Restart {
            factory_id,
            token: idx,
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

    fn check_readiness(&mut self, cx: &mut Context<'_>) -> Result<bool, (usize, usize)> {
        let mut ready = true;
        for (idx, srv) in self.services.iter_mut().enumerate() {
            if srv.status == WorkerServiceStatus::Available
                || srv.status == WorkerServiceStatus::Unavailable
            {
                match srv.service.poll_ready(cx) {
                    Poll::Ready(Ok(_)) => {
                        if srv.status == WorkerServiceStatus::Unavailable {
                            trace!(
                                "Service {:?} is available",
                                self.factories[srv.factory].name(idx)
                            );
                            srv.status = WorkerServiceStatus::Available;
                        }
                    }
                    Poll::Pending => {
                        ready = false;

                        if srv.status == WorkerServiceStatus::Available {
                            trace!(
                                "Service {:?} is unavailable",
                                self.factories[srv.factory].name(idx)
                            );
                            srv.status = WorkerServiceStatus::Unavailable;
                        }
                    }
                    Poll::Ready(Err(_)) => {
                        error!(
                            "Service {:?} readiness check returned error, restarting",
                            self.factories[srv.factory].name(idx)
                        );
                        srv.status = WorkerServiceStatus::Failed;
                        return Err((idx, srv.factory));
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
    token: usize,
    fut: LocalBoxFuture<'static, Result<(usize, BoxedServerService), ()>>,
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
        // Stop the Arbiter ServerWorker runs on on drop.
        Arbiter::current().stop();
    }
}

impl Future for ServerWorker {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.as_mut().get_mut();

        // `StopWorker` message handler
        if let Poll::Ready(Some(Stop { graceful, tx })) = Pin::new(&mut this.rx2).poll_recv(cx)
        {
            let num = this.counter.total();
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

                this.services[token].created(service);
                this.state = WorkerState::Unavailable;

                self.poll(cx)
            }
            WorkerState::Shutdown(ref mut shutdown) => {
                // Wait for 1 second.
                ready!(shutdown.timer.as_mut().poll(cx));

                if this.counter.total() == 0 {
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
                        this.state = WorkerState::Unavailable;
                        return self.poll(cx);
                    }
                    Err((token, idx)) => {
                        this.restart_service(token, idx);
                        return self.poll(cx);
                    }
                }

                // handle incoming io stream
                match ready!(Pin::new(&mut this.rx).poll_recv(cx)) {
                    Some(msg) => {
                        let guard = this.counter.guard();
                        let _ = this.services[msg.token].service.call((guard, msg.io));
                    }
                    None => return Poll::Ready(()),
                };
            },
        }
    }
}
