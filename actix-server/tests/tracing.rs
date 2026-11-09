#![allow(missing_docs)]

use std::{
    collections::BTreeMap,
    fmt,
    sync::{mpsc, Arc, Mutex},
    time::Duration,
};

use actix_server::{Server, TestServer};
use actix_service::fn_service;
use tokio::io::AsyncReadExt as _;
use tracing::{field::Visit, span, Event, Subscriber};
use tracing_subscriber::{layer::Context, prelude::*, registry::LookupSpan, Layer};

#[derive(Clone, Debug, Default)]
struct Fields(BTreeMap<String, String>);

impl Visit for Fields {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn fmt::Debug) {
        self.0.insert(field.name().to_owned(), format!("{value:?}"));
    }
}

#[derive(Clone, Debug)]
struct ConnectionSpan {
    id: span::Id,
    fields: Fields,
    root: bool,
}

struct Capture(mpsc::Sender<Vec<ConnectionSpan>>);

impl<S: Subscriber + for<'a> LookupSpan<'a>> Layer<S> for Capture {
    fn on_new_span(&self, attrs: &span::Attributes<'_>, id: &span::Id, ctx: Context<'_, S>) {
        let span = ctx.span(id).unwrap();
        let mut fields = Fields::default();
        attrs.record(&mut fields);
        span.extensions_mut().insert(fields);
    }

    fn on_event(&self, event: &Event<'_>, ctx: Context<'_, S>) {
        if event.metadata().target() != "connection_test" {
            return;
        }

        let spans = ctx
            .event_scope(event)
            .into_iter()
            .flatten()
            .filter(|span| span.name() == "connection")
            .map(|span| ConnectionSpan {
                id: span.id(),
                fields: span.extensions().get::<Fields>().unwrap().clone(),
                root: span.parent().is_none(),
            })
            .collect();

        self.0.send(spans).unwrap();
    }
}

#[test]
fn connection_spans_cover_service_calls_and_async_polls() {
    let (events_tx, events_rx) = mpsc::channel();
    tracing::subscriber::set_global_default(
        tracing_subscriber::registry().with(Capture(events_tx)),
    )
    .unwrap();

    for actix_runtime in [true, false] {
        let (listener, addr) = TestServer::unused_listener();
        let (handle_tx, handle_rx) = mpsc::channel();
        let gates = Arc::new(Mutex::new(Vec::new()));
        let service_gates = Arc::clone(&gates);

        let thread = std::thread::spawn(move || {
            let run = async move {
                let server = Server::build()
                    .workers(2)
                    .disable_signals()
                    .listen("traced", listener, move || {
                        let gates = Arc::clone(&service_gates);

                        fn_service(move |mut stream: actix_rt::net::TcpStream| {
                            tracing::info!(target: "connection_test", "synchronous call");
                            let (tx, rx) = tokio::sync::oneshot::channel::<()>();
                            gates.lock().unwrap().push(tx);

                            async move {
                                tracing::info!(target: "connection_test", "first poll");
                                rx.await.unwrap();

                                tracing::debug_span!("protocol").in_scope(|| {
                                    tracing::info!(target: "connection_test", "resumed poll");
                                });

                                let mut byte = [0];
                                stream.read_exact(&mut byte).await?;
                                Ok::<_, std::io::Error>(())
                            }
                        })
                    })
                    .unwrap()
                    .run();

                handle_tx.send(server.handle()).unwrap();
                server.await.unwrap();
            };

            if actix_runtime {
                actix_rt::System::new().block_on(run);
            } else {
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .unwrap()
                    .block_on(run);
            }
        });

        let handle = handle_rx.recv_timeout(Duration::from_secs(10)).unwrap();
        let mut observed = Vec::new();
        let mut streams = Vec::new();

        for _ in 0..4 {
            streams.push(std::net::TcpStream::connect(addr).unwrap());
            observed.push(events_rx.recv_timeout(Duration::from_secs(10)).unwrap());
            observed.push(events_rx.recv_timeout(Duration::from_secs(10)).unwrap());
        }

        for gate in gates.lock().unwrap().drain(..) {
            gate.send(()).unwrap();
        }

        for _ in 0..4 {
            observed.push(events_rx.recv_timeout(Duration::from_secs(10)).unwrap());
        }

        drop(streams);
        actix_rt::System::new().block_on(handle.stop(true));
        thread.join().unwrap();

        let mut counts = BTreeMap::new();
        let mut workers = std::collections::BTreeSet::new();
        let mut connections = std::collections::BTreeSet::new();

        for spans in observed {
            assert_eq!(spans.len(), 1, "each event must have one connection span");
            let span = &spans[0];
            assert!(span.root);
            assert_eq!(span.fields.0["service"], "\"traced\"");
            assert_eq!(span.fields.0["local_addr"], addr.to_string());
            workers.insert(span.fields.0["worker_id"].clone());
            connections.insert((
                span.fields.0["worker_id"].clone(),
                span.fields.0["connection_id"].clone(),
            ));
            *counts.entry(span.id.clone().into_u64()).or_insert(0) += 1;
        }

        assert_eq!(workers, ["0".to_owned(), "1".to_owned()].into());
        assert_eq!(counts.len(), 4);
        assert_eq!(connections.len(), 4);
        assert!(counts.values().all(|&count| count == 3));
    }
}
