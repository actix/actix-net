use std::{
    future::Future,
    io, mem,
    num::NonZeroUsize,
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
    Arbiter, ArbiterHandle, System,
};
use futures_core::{future::LocalBoxFuture, ready};
use tokio::sync::{
    mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender},
    oneshot,
};
use tracing::{error, info, trace};

use crate::{
    service::{BoxedServerService, InternalServiceFactory},
    socket::MioStream,
    waker_queue::{WakerInterest, WakerQueue},
};

/// Stop worker message. Returns `true` on successful graceful shutdown
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

/// Create accept and server worker handles.
fn handle_pair(
    idx: usize,
    conn_tx: UnboundedSender<Conn>,
    stop_tx: UnboundedSender<Stop>,
    counter: Counter,
) -> (WorkerHandleAccept, WorkerHandleServer) {
    let accept = WorkerHandleAccept {
        idx,
        conn_tx,
        counter,
    };

    let server = WorkerHandleServer { idx, stop_tx };

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

/// Handle to worker that can send connection message to worker and share the availability of worker
/// to other threads.
///
/// Held by [Accept](crate::accept::Accept).
pub(crate) struct WorkerHandleAccept {
    idx: usize,
    conn_tx: UnboundedSender<Conn>,
    counter: Counter,
}

impl WorkerHandleAccept {
    #[inline(always)]
    pub(crate) fn idx(&self) -> usize {
        self.idx
    }

    #[inline(always)]
    pub(crate) fn send(&self, conn: Conn) -> Result<(), Conn> {
        self.conn_tx.send(conn).map_err(|msg| msg.0)
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
    pub(crate) idx: usize,
    stop_tx: UnboundedSender<Stop>,
}

impl WorkerHandleServer {
    pub(crate) fn stop(&self, graceful: bool) -> oneshot::Receiver<bool> {
        let (tx, rx) = oneshot::channel();
        let _ = self.stop_tx.send(Stop { graceful, tx });
        rx
    }
}

/// Service worker.
///
/// Worker accepts Socket objects via unbounded channel and starts stream processing.
pub(crate) struct ServerWorker {
    // UnboundedReceiver<Conn> should always be the first field.
    // It must be dropped as soon as ServerWorker dropping.
    conn_rx: UnboundedReceiver<Conn>,
    stop_rx: UnboundedReceiver<Stop>,
    counter: WorkerCounter,
    services: Box<[WorkerService]>,
    factories: Box<[Box<dyn InternalServiceFactory>]>,
    state: WorkerState,
    shutdown_timeout: Duration,
}

struct WorkerService {
    factory_idx: usize,
    status: WorkerServiceStatus,
    service: BoxedServerService,
}

impl WorkerService {
    fn created(&mut self, service: BoxedServerService) {
        self.service = service;
        self.status = WorkerServiceStatus::Unavailable;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WorkerServiceStatus {
    Available,
    Unavailable,
    Failed,
    Restarting,
    Stopping,
    Stopped,
}

impl Default for WorkerServiceStatus {
    fn default() -> Self {
        Self::Unavailable
    }
}

/// Config for worker behavior passed down from server builder.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ServerWorkerConfig {
    shutdown_timeout: Duration,
    max_blocking_threads: usize,
    max_concurrent_connections: usize,
}

impl Default for ServerWorkerConfig {
    fn default() -> Self {
        let parallelism = std::thread::available_parallelism().map_or(2, NonZeroUsize::get);

        // 512 is the default max blocking thread count of a Tokio runtime.
        let max_blocking_threads = std::cmp::max(512 / parallelism, 1);

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
    ) -> io::Result<(WorkerHandleAccept, WorkerHandleServer)> {
        trace!("starting server worker {}", idx);

        let (tx1, conn_rx) = unbounded_channel();
        let (tx2, stop_rx) = unbounded_channel();

        let counter = Counter::new(config.max_concurrent_connections);
        let pair = handle_pair(idx, tx1, tx2, counter.clone());

        // get actix system context if it is set
        let actix_system = System::try_current();

        // get tokio runtime handle if it is set
        let tokio_handle = tokio::runtime::Handle::try_current().ok();

        // service factories initialization channel
        let (factory_tx, factory_rx) = std::sync::mpsc::sync_channel::<io::Result<()>>(1);

        // outline of following code:
        //
        // if system exists
        //   if uring enabled
        //     start arbiter using uring method
        //   else
        //     start arbiter with regular tokio
        // else
        //   if uring enabled
        //     start uring in spawned thread
        //   else
        //     start regular tokio in spawned thread

        // every worker runs in it's own thread and tokio runtime.
        // use a custom tokio runtime builder to change the settings of runtime.

        match (actix_system, tokio_handle) {
            (None, None) => {
                panic!("No runtime detected. Start a Tokio (or Actix) runtime.");
            }

            // no actix system
            (None, Some(rt_handle)) => {
                std::thread::Builder::new()
                    .name(format!("actix-server worker {}", idx))
                    .spawn(move || {
                        let (worker_stopped_tx, worker_stopped_rx) = oneshot::channel();

                        // local set for running service init futures and worker services
                        let ls = tokio::task::LocalSet::new();

                        // init services using existing Tokio runtime (so probably on main thread)
                        let services = rt_handle.block_on(ls.run_until(async {
                            let mut services = Vec::new();

                            for (idx, factory) in factories.iter().enumerate() {
                                match factory.create().await {
                                    Ok((token, svc)) => services.push((idx, token, svc)),

                                    Err(err) => {
                                        error!("can not start worker: {:?}", err);
                                        return Err(io::Error::new(
                                            io::ErrorKind::Other,
                                            format!("can not start server service {}", idx),
                                        ));
                                    }
                                }
                            }

                            Ok(services)
                        }));

                        let services = match services {
                            Ok(services) => {
                                factory_tx.send(Ok(())).unwrap();
                                services
                            }
                            Err(err) => {
                                factory_tx.send(Err(err)).unwrap();
                                return;
                            }
                        };

                        let worker_services = wrap_worker_services(services);

                        let worker_fut = async move {
                            // spawn to make sure ServerWorker runs as non boxed future.
                            spawn(async move {
                                ServerWorker {
                                    conn_rx,
                                    stop_rx,
                                    services: worker_services.into_boxed_slice(),
                                    counter: WorkerCounter::new(idx, waker_queue, counter),
                                    factories: factories.into_boxed_slice(),
                                    state: WorkerState::default(),
                                    shutdown_timeout: config.shutdown_timeout,
                                }
                                .await;

                                // wake up outermost task waiting for shutdown
                                worker_stopped_tx.send(()).unwrap();
                            });

                            worker_stopped_rx.await.unwrap();
                        };

                        #[cfg(all(target_os = "linux", feature = "io-uring"))]
                        {
                            // TODO: pass max blocking thread config when tokio-uring enable configuration
                            // on building runtime.
                            let _ = config.max_blocking_threads;
                            tokio_uring::start(worker_fut);
                        }

                        #[cfg(not(all(target_os = "linux", feature = "io-uring")))]
                        {
                            let rt = tokio::runtime::Builder::new_current_thread()
                                .enable_all()
                                .max_blocking_threads(config.max_blocking_threads)
                                .build()
                                .unwrap();

                            rt.block_on(ls.run_until(worker_fut));
                        }
                    })
                    .expect("cannot spawn server worker thread");
            }

            // with actix system
            (Some(_sys), _) => {
                #[cfg(all(target_os = "linux", feature = "io-uring"))]
                let arbiter = {
                    // TODO: pass max blocking thread config when tokio-uring enable configuration
                    // on building runtime.
                    let _ = config.max_blocking_threads;
                    Arbiter::new()
                };

                #[cfg(not(all(target_os = "linux", feature = "io-uring")))]
                let arbiter = {
                    Arbiter::with_tokio_rt(move || {
                        tokio::runtime::Builder::new_current_thread()
                            .enable_all()
                            .max_blocking_threads(config.max_blocking_threads)
                            .build()
                            .unwrap()
                    })
                };

                arbiter.spawn(async move {
                    // spawn_local to run !Send future tasks.
                    spawn(async move {
                        let mut services = Vec::new();

                        for (idx, factory) in factories.iter().enumerate() {
                            match factory.create().await {
                                Ok((token, svc)) => services.push((idx, token, svc)),

                                Err(err) => {
                                    error!("can not start worker: {:?}", err);
                                    Arbiter::current().stop();
                                    factory_tx
                                        .send(Err(io::Error::new(
                                            io::ErrorKind::Other,
                                            format!("can not start server service {}", idx),
                                        )))
                                        .unwrap();
                                    return;
                                }
                            }
                        }

                        factory_tx.send(Ok(())).unwrap();

                        let worker_services = wrap_worker_services(services);

                        // spawn to make sure ServerWorker runs as non boxed future.
                        spawn(ServerWorker {
                            conn_rx,
                            stop_rx,
                            services: worker_services.into_boxed_slice(),
                            counter: WorkerCounter::new(idx, waker_queue, counter),
                            factories: factories.into_boxed_slice(),
                            state: Default::default(),
                            shutdown_timeout: config.shutdown_timeout,
                        });
                    });
                });
            }
        };

        // wait for service factories initialization
        factory_rx.recv().unwrap()?;

        Ok(pair)
    }

    fn restart_service(&mut self, idx: usize, factory_id: usize) {
        let factory = &self.factories[factory_id];
        trace!("service {:?} failed, restarting", factory.name(idx));
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
                                "service {:?} is available",
                                self.factories[srv.factory_idx].name(idx)
                            );
                            srv.status = WorkerServiceStatus::Available;
                        }
                    }
                    Poll::Pending => {
                        ready = false;

                        if srv.status == WorkerServiceStatus::Available {
                            trace!(
                                "service {:?} is unavailable",
                                self.factories[srv.factory_idx].name(idx)
                            );
                            srv.status = WorkerServiceStatus::Unavailable;
                        }
                    }
                    Poll::Ready(Err(_)) => {
                        error!(
                            "service {:?} readiness check returned error, restarting",
                            self.factories[srv.factory_idx].name(idx)
                        );
                        srv.status = WorkerServiceStatus::Failed;
                        return Err((idx, srv.factory_idx));
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

/// State necessary for server shutdown.
struct Shutdown {
    // Interval for checking the shutdown progress.
    timer: Pin<Box<Sleep>>,

    /// Start time of shutdown.
    start_from: Instant,

    /// Notify caller of the shutdown outcome (graceful/force).
    tx: oneshot::Sender<bool>,
}

impl Default for WorkerState {
    fn default() -> Self {
        Self::Unavailable
    }
}

impl Drop for ServerWorker {
    fn drop(&mut self) {
        Arbiter::try_current().as_ref().map(ArbiterHandle::stop);
    }
}

impl Future for ServerWorker {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.as_mut().get_mut();

        // `StopWorker` message handler
        if let Poll::Ready(Some(Stop { graceful, tx })) = this.stop_rx.poll_recv(cx) {
            let num = this.counter.total();
            if num == 0 {
                info!("shutting down idle worker");
                let _ = tx.send(true);
                return Poll::Ready(());
            } else if graceful {
                info!("graceful worker shutdown; finishing {} connections", num);
                this.shutdown(false);

                this.state = WorkerState::Shutdown(Shutdown {
                    timer: Box::pin(sleep(Duration::from_secs(1))),
                    start_from: Instant::now(),
                    tx,
                });
            } else {
                info!("force shutdown worker, closing {} connections", num);
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

                let (token_new, service) =
                    ready!(restart.fut.as_mut().poll(cx)).unwrap_or_else(|_| {
                        panic!(
                            "Can not restart {:?} service",
                            this.factories[factory_id].name(token)
                        )
                    });

                assert_eq!(token, token_new);

                trace!(
                    "service {:?} has been restarted",
                    this.factories[factory_id].name(token)
                );

                this.services[token].created(service);
                this.state = WorkerState::Unavailable;

                self.poll(cx)
            }

            WorkerState::Shutdown(ref mut shutdown) => {
                // drop all pending connections in rx channel.
                while let Poll::Ready(Some(conn)) = this.conn_rx.poll_recv(cx) {
                    // WorkerCounterGuard is needed as Accept thread has incremented counter.
                    // It's guard's job to decrement the counter together with drop of Conn.
                    let guard = this.counter.guard();
                    drop((conn, guard));
                }

                // wait for 1 second
                ready!(shutdown.timer.as_mut().poll(cx));

                if this.counter.total() == 0 {
                    // graceful shutdown
                    if let WorkerState::Shutdown(shutdown) = mem::take(&mut this.state) {
                        let _ = shutdown.tx.send(true);
                    }

                    Poll::Ready(())
                } else if shutdown.start_from.elapsed() >= this.shutdown_timeout {
                    // timeout forceful shutdown
                    if let WorkerState::Shutdown(shutdown) = mem::take(&mut this.state) {
                        let _ = shutdown.tx.send(false);
                    }

                    Poll::Ready(())
                } else {
                    // reset timer and wait for 1 second
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
                        trace!("worker is unavailable");
                        this.state = WorkerState::Unavailable;
                        return self.poll(cx);
                    }
                    Err((token, idx)) => {
                        this.restart_service(token, idx);
                        return self.poll(cx);
                    }
                }

                // handle incoming io stream
                match ready!(this.conn_rx.poll_recv(cx)) {
                    Some(msg) => {
                        let guard = this.counter.guard();
                        let _ = this.services[msg.token]
                            .service
                            .call((guard, msg.io))
                            .into_inner();
                    }
                    None => return Poll::Ready(()),
                };
            },
        }
    }
}

fn wrap_worker_services(services: Vec<(usize, usize, BoxedServerService)>) -> Vec<WorkerService> {
    services
        .into_iter()
        .fold(Vec::new(), |mut services, (idx, token, service)| {
            assert_eq!(token, services.len());
            services.push(WorkerService {
                factory_idx: idx,
                service,
                status: WorkerServiceStatus::Unavailable,
            });
            services
        })
}
