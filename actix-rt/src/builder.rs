use std::{borrow::Cow, future::Future, io};

use tokio::sync::{
    mpsc::unbounded_channel,
    oneshot::{channel, Receiver},
};

use crate::{
    runtime::Runtime,
    system::{System, SystemWorker},
    worker::Worker,
};

/// Builder an actix runtime.
///
/// Either use `Builder::build` to create a system and start actors. Alternatively, use
/// `Builder::run` to start the Tokio runtime and run a function in its context.
pub struct Builder {
    /// Name of the System. Defaults to "actix-rt" if unset.
    name: Cow<'static, str>,

    /// Whether the Arbiter will stop the whole System on uncaught panic. Defaults to false.
    stop_on_panic: bool,
}

impl Builder {
    pub(crate) fn new() -> Self {
        Builder {
            name: Cow::Borrowed("actix-rt"),
            stop_on_panic: false,
        }
    }

    /// Sets the name of the System.
    pub fn name(mut self, name: impl Into<String>) -> Self {
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
    /// This method panics if it can not create Tokio runtime
    pub fn build(self) -> SystemRunner {
        self.create_runtime(|| {})
    }

    /// This function will start Tokio runtime and will finish once the `System::stop()` message
    /// is called. Function `f` is called within Tokio runtime context.
    pub fn run<F>(self, f: F) -> io::Result<()>
    where
        F: FnOnce(),
    {
        self.create_runtime(f).run()
    }

    fn create_runtime<F>(self, f: F) -> SystemRunner
    where
        F: FnOnce(),
    {
        let (stop_tx, stop) = channel();
        let (sys_sender, sys_receiver) = unbounded_channel();

        let rt = Runtime::new().unwrap();

        let system = System::construct(
            sys_sender,
            Worker::new_system(rt.local()),
            self.stop_on_panic,
        );

        let arb = SystemWorker::new(sys_receiver, stop_tx);

        rt.spawn(arb);

        // init system arbiter and run configuration method
        rt.block_on(async { f() });

        SystemRunner { rt, stop, system }
    }
}

/// Helper object that runs System's event loop
#[must_use = "SystemRunner must be run"]
#[derive(Debug)]
pub struct SystemRunner {
    rt: Runtime,
    stop: Receiver<i32>,
    system: System,
}

impl SystemRunner {
    /// This function will start event loop and will finish once the
    /// `System::stop()` function is called.
    pub fn run(self) -> io::Result<()> {
        let SystemRunner { rt, stop, .. } = self;

        // run loop
        match rt.block_on(stop) {
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

    /// Execute a future and wait for result.
    #[inline]
    pub fn block_on<F: Future>(&self, fut: F) -> F::Output {
        self.rt.block_on(fut)
    }
}
