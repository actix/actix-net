//! Simple composite-service TCP echo server.
//!
//! Using the following command:
//!
//! ```sh
//! nc 127.0.0.1 8080
//! ```
//!
//! Start typing. When you press enter the typed line will be echoed back. The server will log
//! the length of each line it echos and the total size of data sent when the connection is closed.

use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use std::{env, io};

use actix_rt::ExecFactory;
use actix_server::{FromStream, Server, StdStream};
use actix_service::pipeline_factory;
use futures_util::future::ok;
use log::{error, info};

fn main() -> io::Result<()> {
    // annotate executor type and block_on it
    actix_rt::System::new_with::<AsyncStdExec, _>("actix").block_on(async {
        env::set_var("RUST_LOG", "actix=trace,basic=trace");
        env_logger::init();

        let count = Arc::new(AtomicUsize::new(0));

        let addr = ("127.0.0.1", 8080);
        info!("starting server on port: {}", &addr.0);

        // annotate again as the server would want to spawn tasks on executor.
        Server::build_with::<AsyncStdExec>()
            .bind("echo", addr, move || {
                let count = Arc::clone(&count);
                let num2 = Arc::clone(&count);

                // stream type must impl actix_server::FromStream trait for handling the stream.
                // at this point stream type can be generic if you want to passing it though
                // pipeline but at last a service must give it a concrete type to work with.
                pipeline_factory(move |mut stream: AsyncStdTcpStream| {
                    let count = Arc::clone(&count);

                    async move {
                        let num = count.fetch_add(1, Ordering::SeqCst);
                        let num = num + 1;

                        let mut size = 0;
                        let mut buf = vec![0; 1024];

                        use async_std::prelude::*;

                        loop {
                            match stream.0.read(&mut buf).await {
                                // end of stream; bail from loop
                                Ok(0) => break,

                                // more bytes to process
                                Ok(bytes_read) => {
                                    info!("[{}] read {} bytes", num, bytes_read);
                                    stream.0.write_all(&buf[size..]).await.unwrap();
                                    size += bytes_read;
                                }

                                // stream error; bail from loop with error
                                Err(err) => {
                                    error!("Stream Error: {:?}", err);
                                    return Err(());
                                }
                            }
                        }

                        // send data down service pipeline
                        Ok((buf.len(), size))
                    }
                })
                .map_err(|err| error!("Service Error: {:?}", err))
                .and_then(move |(_, size)| {
                    let num = num2.load(Ordering::SeqCst);
                    info!("[{}] total bytes read: {}", num, size);
                    ok(size)
                })
            })?
            .workers(1)
            .run()
            .await
    })
}

// custom executor
struct AsyncStdExec;

// stream type can work on the custom executor
struct AsyncStdTcpStream(async_std::net::TcpStream);

// server would pass a StdStream enum and you have to convert it to your stream type.
impl FromStream for AsyncStdTcpStream {
    fn from_stdstream(stream: StdStream) -> std::io::Result<Self> {
        match stream {
            StdStream::Tcp(tcp) => Ok(AsyncStdTcpStream(async_std::net::TcpStream::from(tcp))),
            StdStream::Uds(_) => unimplemented!(),
        }
    }
}

// impl trait for custom executor so server can block_on/spawn tasks
impl ExecFactory for AsyncStdExec {
    type Executor = ();
    type Sleep = Pin<Box<dyn Future<Output = ()> + Send + 'static>>;

    fn build() -> std::io::Result<Self::Executor> {
        Ok(())
    }

    fn block_on<F: Future>(_: &mut Self::Executor, f: F) -> <F as Future>::Output {
        async_std::task::block_on(f)
    }

    fn spawn<F: Future<Output = ()> + 'static>(f: F) {
        async_std::task::spawn_local(f);
    }

    fn spawn_on<F: Future<Output = ()> + 'static>(_: &mut Self::Executor, f: F) {
        async_std::task::spawn_local(f);
    }

    fn sleep(dur: Duration) -> Self::Sleep {
        Box::pin(async_std::task::sleep(dur))
    }
}
