mod config;
mod counter;

pub(crate) use config::ServerWorkerConfig;
pub(crate) use counter::{Counter, WorkerCounterGuard};

use std::{
    future::Future,
    mem,
    pin::Pin,
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

use counter::WorkerCounter;

use crate::join_all;
use crate::service::{BoxedServerService, InternalServiceFactory};
use crate::waker_queue::WakerQueue;

pub(crate) enum WorkerMessage<C> {
    Conn(Conn<C>),
    Stop(Stop),
}

/// Connection message. token field is used to find the corresponding Service from
/// Worker that can handle C type correctly.
#[derive(Debug)]
pub(crate) struct Conn<C> {
    pub token: usize,
    pub io: C,
}

/// Stop worker message. Returns `true` on successful graceful shutdown.
/// and `false` if some connections still alive when shutdown execute.
pub(crate) enum Stop {
    Graceful(oneshot::Sender<bool>),
    Force,
}

// TOD: Remove default MioStream type.
/// Handle to worker that can send message to worker.
pub(crate) struct WorkerHandleAccept<C> {
    idx: usize,
    tx: UnboundedSender<WorkerMessage<C>>,
    counter: Counter,
}

impl<C> WorkerHandleAccept<C> {
    #[inline(always)]
    pub(crate) fn idx(&self) -> usize {
        self.idx
    }

    #[inline(always)]
    pub(crate) fn inc_counter(&self) -> bool {
        self.counter.inc()
    }

    #[inline(always)]
    pub(crate) fn send(&self, conn: Conn<C>) -> Result<(), Conn<C>> {
        self.tx
            .send(WorkerMessage::Conn(conn))
            .map_err(|msg| match msg.0 {
                WorkerMessage::Conn(conn) => conn,
                _ => unreachable!(),
            })
    }

    pub(crate) fn stop(&self, graceful: bool) -> Option<oneshot::Receiver<bool>> {
        let (stop, rx) = if graceful {
            let (tx, rx) = oneshot::channel();

            (Stop::Graceful(tx), Some(rx))
        } else {
            (Stop::Force, None)
        };

        let _ = self.tx.send(WorkerMessage::Stop(stop));

        rx
    }
}

/// Server worker.
///
/// Worker accepts Socket objects via unbounded channel and starts stream processing.
pub(crate) struct Worker<C> {
    worker: WorkerInner<C>,
    state: WorkerState<C>,
}

impl<C> Worker<C>
where
    C: Send + 'static,
{
    fn new(
        rx: UnboundedReceiver<WorkerMessage<C>>,
        counter: WorkerCounter<C>,
        services: Box<[WorkerService<C>]>,
        factories: Box<[Box<dyn InternalServiceFactory<C>>]>,
        shutdown_timeout: Duration,
    ) -> Self {
        Self {
            worker: WorkerInner {
                rx,
                counter,
                services,
                factories,
                shutdown_timeout,
            },
            state: WorkerState::default(),
        }
    }

    pub(crate) fn start(
        idx: usize,
        factories: Box<[Box<dyn InternalServiceFactory<C>>]>,
        waker_queue: WakerQueue<C>,
        config: ServerWorkerConfig,
    ) -> WorkerHandleAccept<C> {
        let (tx, rx) = unbounded_channel();

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

                let counter = WorkerCounter::new(idx, waker_queue, counter_clone);

                // a third spawn to make sure ServerWorker runs as non boxed future.
                spawn(Worker::new(
                    rx,
                    counter,
                    services,
                    factories,
                    config.shutdown_timeout,
                ));
            });
        });

        WorkerHandleAccept { idx, tx, counter }
    }
}

struct WorkerInner<C> {
    // UnboundedReceiver<Conn> should always be the first field.
    // It must be dropped as soon as ServerWorker dropping.
    rx: UnboundedReceiver<WorkerMessage<C>>,
    counter: WorkerCounter<C>,
    services: Box<[WorkerService<C>]>,
    factories: Box<[Box<dyn InternalServiceFactory<C>>]>,
    shutdown_timeout: Duration,
}

impl<C> WorkerInner<C> {
    /// `Conn` message and worker/service state switch handler
    fn poll_running(&mut self, running: &mut Running<C>, cx: &mut Context<'_>) -> Poll<Stop> {
        match *running {
            // Actively poll Conn channel and handle MioStream.
            Running::Available => loop {
                match self.poll_ready(cx) {
                    Ok(true) => match ready!(Pin::new(&mut self.rx).poll_recv(cx)) {
                        Some(WorkerMessage::Conn(conn)) => {
                            let guard = self.counter.guard();
                            let _ = self.services[conn.token].service.call((guard, conn.io));
                        }
                        Some(WorkerMessage::Stop(stop)) => return Poll::Ready(stop),
                        None => return Poll::Ready(Stop::Force),
                    },
                    Ok(false) => {
                        trace!("Worker is unavailable");
                        return Poll::Pending;
                    }
                    Err((token, idx)) => {
                        let restart = self.restart_service(token, idx);
                        *running = Running::Restart(restart);

                        return self.poll_running(running, cx);
                    }
                }
            },
            Running::Restart(Restart {
                factory_id,
                token,
                ref mut fut,
            }) => {
                let name = self.factories[factory_id].name(token);

                let (token_new, service) = ready!(fut.as_mut().poll(cx)).unwrap_or_else(|_| {
                    // Restart failure would result in a panic drop of ServerWorker.
                    // This would prevent busy loop of poll_running.
                    panic!("Can not restart {:?} service", name)
                });

                assert_eq!(token, token_new);

                trace!("Service {:?} has been restarted", name);

                self.services[token].created(service);
                *running = Running::Available;

                self.poll_running(running, cx)
            }
        }
    }

    /// Delay shutdown and drain all unhandled `Conn`.
    fn poll_shutdown(
        &mut self,
        delay: &mut DelayShutdown,
        running: &mut Running<C>,
        cx: &mut Context<'_>,
    ) -> Poll<bool> {
        let num = self.counter.total();
        if num == 0 {
            // Graceful shutdown.
            info!("Graceful worker shutdown, 0 connections unhandled");
            Poll::Ready(true)
        } else if delay.start_from.elapsed() >= self.shutdown_timeout {
            // Timeout forceful shutdown.
            info!(
                "Graceful worker shutdown timeout, {} connections unhandled",
                num
            );
            Poll::Ready(false)
        } else {
            // Poll Running state and try to drain all `Conn` from channel.
            let _ = self.poll_running(running, cx);

            // Wait for 1 second.
            ready!(delay.timer.as_mut().poll(cx));

            // Reset timer and try again.
            let time = Instant::now() + Duration::from_secs(1);
            delay.timer.as_mut().reset(time);

            delay.timer.as_mut().poll(cx).map(|_| false)
        }
    }

    /// Check readiness of services.
    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Result<bool, (usize, usize)> {
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

    /// `Stop` message handler.
    ///
    /// Return Some<DelayShutdown> when worker should enter delay shutdown.
    /// Return None when worker is ready to shutdown in place.
    fn try_shutdown(&mut self, stop: Stop) -> Option<DelayShutdown> {
        let num = self.counter.total();

        match stop {
            Stop::Graceful(tx) => {
                self.shutdown_service(false);

                let shutdown = DelayShutdown {
                    timer: Box::pin(sleep(Duration::from_secs(1))),
                    start_from: Instant::now(),
                    tx,
                };

                Some(shutdown)
            }
            Stop::Force => {
                info!("Force worker shutdown, {} connections unhandled", num);

                self.shutdown_service(true);

                None
            }
        }
    }

    fn restart_service(&mut self, idx: usize, factory_id: usize) -> Restart<C> {
        let factory = &self.factories[factory_id];
        trace!("Service {:?} failed, restarting", factory.name(idx));
        self.services[idx].status = WorkerServiceStatus::Restarting;
        Restart {
            factory_id,
            token: idx,
            fut: factory.create(),
        }
    }

    fn shutdown_service(&mut self, force: bool) {
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
}

struct WorkerService<C> {
    factory: usize,
    status: WorkerServiceStatus,
    service: BoxedServerService<C>,
}

impl<C> WorkerService<C> {
    fn created(&mut self, service: BoxedServerService<C>) {
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

enum WorkerState<C> {
    Running(Running<C>),
    DelayShutdown(DelayShutdown, Running<C>),
}

impl<C> Default for WorkerState<C> {
    fn default() -> Self {
        Self::Running(Running::Available)
    }
}

enum Running<C> {
    Available,
    Restart(Restart<C>),
}

struct Restart<C> {
    factory_id: usize,
    token: usize,
    fut: LocalBoxFuture<'static, Result<(usize, BoxedServerService<C>), ()>>,
}

// Keep states necessary for delayed server shutdown:
// Sleep for interval check the shutdown progress.
// Instant for the start time of shutdown.
// Sender for send back the shutdown outcome(force/grace) to `Stop` caller.
struct DelayShutdown {
    timer: Pin<Box<Sleep>>,
    start_from: Instant,
    tx: oneshot::Sender<bool>,
}

impl<C> Drop for Worker<C> {
    fn drop(&mut self) {
        // Stop the Arbiter ServerWorker runs on on drop.
        Arbiter::current().stop();
    }
}

impl<C> Future for Worker<C> {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.as_mut().get_mut();

        match this.state {
            WorkerState::Running(ref mut running) => {
                let stop = ready!(this.worker.poll_running(running, cx));
                match this.worker.try_shutdown(stop) {
                    Some(delay) => {
                        // Take running state and pass it to DelayShutdown.
                        // During shutdown there could be unhandled `Conn` message left in channel.
                        // They should be drained and worker would try to handle them all until
                        // delay shutdown timeout met.
                        this.state = match mem::take(&mut this.state) {
                            WorkerState::Running(running) => {
                                WorkerState::DelayShutdown(delay, running)
                            }
                            _ => unreachable!("ServerWorker enter DelayShutdown already"),
                        };

                        self.poll(cx)
                    }
                    None => Poll::Ready(()),
                }
            }
            WorkerState::DelayShutdown(ref mut delay, ref mut running) => {
                let is_graceful = ready!(this.worker.poll_shutdown(delay, running, cx));

                // Report back shutdown outcome to caller.
                if let WorkerState::DelayShutdown(delay, _) = mem::take(&mut this.state) {
                    let _ = delay.tx.send(is_graceful);
                }

                Poll::Ready(())
            }
        }
    }
}
