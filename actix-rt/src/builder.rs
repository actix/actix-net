use std::borrow::Cow;
use std::future::Future;
use std::io;
use std::marker::PhantomData;

use futures_channel::mpsc::unbounded;
use futures_channel::oneshot::{channel, Receiver};

use crate::arbiter::{Arbiter, SystemArbiter};
use crate::runtime::ExecFactory;
use crate::system::System;

/// Builder struct for a actix runtime.
///
/// Either use `Builder::build` to create a system and start actors.
/// Alternatively, use `Builder::run` to start the tokio runtime and
/// run a function in its context.
pub struct Builder<E> {
    /// Name of the System. Defaults to "actix" if unset.
    name: Cow<'static, str>,

    /// Whether the Arbiter will stop the whole System on uncaught panic. Defaults to false.
    stop_on_panic: bool,
    exec: PhantomData<E>,
}

impl<E: ExecFactory> Builder<E> {
    pub(crate) fn new() -> Builder<E> {
        Builder {
            name: Cow::Borrowed("actix"),
            stop_on_panic: false,
            exec: PhantomData,
        }
    }

    /// Sets the name of the System.
    pub fn name<T: Into<String>>(mut self, name: T) -> Self {
        self.name = Cow::Owned(name.into());
        self
    }

    /// Sets the option 'stop_on_panic' which controls whether the System is stopped when an
    /// uncaught panic is thrown from a worker thread.
    ///
    /// Defaults to false.
    pub fn stop_on_panic(mut self, stop_on_panic: bool) -> Self {
        self.stop_on_panic = stop_on_panic;
        self
    }

    /// Create new System.
    ///
    /// This method panics if it can not create tokio runtime
    pub fn build(self) -> SystemRunner<E> {
        self.create_runtime(|| {})
    }

    /// This function will start tokio runtime and will finish once the
    /// `System::stop()` message get called.
    /// Function `f` get called within tokio runtime context.
    pub fn run<F>(self, f: F) -> io::Result<()>
    where
        F: FnOnce() + 'static,
    {
        self.create_runtime(f).run()
    }

    /// Create runtime with a given instance of type that impl `ExecFactory::Executor` trait.
    pub fn create_with_runtime<F>(self, mut rt: E::Executor, f: F) -> SystemRunner<E>
    where
        F: FnOnce() + 'static,
    {
        let (stop_tx, stop) = channel();
        let (sys_sender, sys_receiver) = unbounded();

        let system = System::construct(
            sys_sender,
            Arbiter::new_system::<E>(&mut rt),
            self.stop_on_panic,
        );

        // system arbiter
        let arb = SystemArbiter::new(stop_tx, sys_receiver);

        E::spawn_ref(&mut rt, arb);

        // init system arbiter and run configuration method
        E::block_on(&mut rt, async { f() });

        SystemRunner { rt, stop, system }
    }

    fn create_runtime<F>(self, f: F) -> SystemRunner<E>
    where
        F: FnOnce() + 'static,
    {
        let rt = E::build().unwrap();
        self.create_with_runtime(rt, f)
    }
}

/// Helper object that runs System's event loop
#[must_use = "SystemRunner must be run"]
#[derive(Debug)]
pub struct SystemRunner<E: ExecFactory> {
    rt: E::Executor,
    stop: Receiver<i32>,
    system: System,
}

impl<E: ExecFactory> SystemRunner<E> {
    /// This function will start event loop and will finish once the
    /// `System::stop()` function is called.
    pub fn run(self) -> io::Result<()> {
        let SystemRunner { mut rt, stop, .. } = self;

        // run loop
        match E::block_on(&mut rt, stop) {
            Ok(code) => {
                if code != 0 {
                    Err(io::Error::new(
                        io::ErrorKind::Other,
                        format!("Non-zero exit code: {}", code),
                    ))
                } else {
                    Ok(())
                }
            }
            Err(e) => Err(io::Error::new(io::ErrorKind::Other, e)),
        }
    }

    pub fn spawn<F>(&mut self, fut: F)
    where
        F: Future<Output = ()> + 'static,
    {
        E::spawn_ref(&mut self.rt, fut);
    }

    /// Execute a future and wait for result.
    pub fn block_on<F, O>(&mut self, fut: F) -> O
    where
        F: Future<Output = O>,
    {
        E::block_on(&mut self.rt, fut)
    }
}
