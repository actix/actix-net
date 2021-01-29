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

/// System builder.
///
/// Either use `Builder::build` to create a system and start actors. Alternatively, use
/// `Builder::run` to start the Tokio runtime and run a function in its context.
pub struct Builder {
    /// Name of the System. Defaults to "actix-rt" if unset.
    name: Cow<'static, str>,
}

impl Builder {
    pub(crate) fn new() -> Self {
        Builder {
            name: Cow::Borrowed("actix-rt"),
        }
    }

    /// Sets the name of the System.
    pub fn name(mut self, name: impl Into<Cow<'static, str>>) -> Self {
        self.name = name.into();
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
    pub fn run<F>(self, init_fn: F) -> io::Result<()>
    where
        F: FnOnce(),
    {
        self.create_runtime(init_fn).run()
    }

    fn create_runtime<F>(self, init_fn: F) -> SystemRunner
    where
        F: FnOnce(),
    {
        let (stop_tx, stop_rx) = channel();
        let (sys_sender, sys_receiver) = unbounded_channel();

        let rt = Runtime::new().unwrap();

        let system = System::construct(sys_sender, Worker::new_system(rt.local()));

        // init system worker
        let sys_worker = SystemWorker::new(sys_receiver, stop_tx);
        rt.spawn(sys_worker);

        // run system init method
        rt.block_on(async { init_fn() });

        SystemRunner {
            rt,
            stop_rx,
            system,
        }
    }
}

/// System runner object that keeps event loop alive and running until stop message is received.
#[must_use = "A SystemRunner does nothing unless `run` is called."]
#[derive(Debug)]
pub struct SystemRunner {
    rt: Runtime,
    stop_rx: Receiver<i32>,
    system: System,
}

impl SystemRunner {
    /// Starts event loop and will finish once [`System::stop()`] is called.
    pub fn run(self) -> io::Result<()> {
        let SystemRunner { rt, stop_rx, .. } = self;

        // run loop
        match rt.block_on(stop_rx) {
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

    /// Runs the provided future, blocking the current thread until the future completes.
    #[inline]
    pub fn block_on<F: Future>(&self, fut: F) -> F::Output {
        self.rt.block_on(fut)
    }
}
