//! Actix tracing - support for tokio tracing with Actix services.

#![deny(rust_2018_idioms, nonstandard_style)]
#![warn(future_incompatible)]
#![doc(html_logo_url = "https://actix.rs/img/logo.png")]
#![doc(html_favicon_url = "https://actix.rs/favicon.ico")]

use core::marker::PhantomData;

use actix_service::{
    apply, ApplyTransform, IntoServiceFactory, Service, ServiceFactory, Transform,
};
use actix_utils::future::{ok, Either, Ready};
use tracing_futures::{Instrument, Instrumented};

/// A `Service` implementation that automatically enters/exits tracing spans
/// for the wrapped inner service.
#[derive(Clone)]
pub struct TracingService<S, F> {
    inner: S,
    make_span: F,
}

impl<S, F> TracingService<S, F> {
    pub fn new(inner: S, make_span: F) -> Self {
        TracingService { inner, make_span }
    }
}

impl<S, Req, F> Service<Req> for TracingService<S, F>
where
    S: Service<Req>,
    F: Fn(&Req) -> Option<tracing::Span>,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = Either<S::Future, Instrumented<S::Future>>;

    actix_service::forward_ready!(inner);

    fn call(&self, req: Req) -> Self::Future {
        let span = (self.make_span)(&req);
        let _enter = span.as_ref().map(|s| s.enter());

        let fut = self.inner.call(req);

        // make a child span to track the future's execution
        if let Some(span) = span
            .clone()
            .map(|span| tracing::span!(parent: &span, tracing::Level::INFO, "future"))
        {
            Either::right(fut.instrument(span))
        } else {
            Either::left(fut)
        }
    }
}

/// A `Transform` implementation that wraps services with a [`TracingService`].
pub struct TracingTransform<S, U, F> {
    make_span: F,
    _p: PhantomData<fn(S, U)>,
}

impl<S, U, F> TracingTransform<S, U, F> {
    pub fn new(make_span: F) -> Self {
        TracingTransform {
            make_span,
            _p: PhantomData,
        }
    }
}

impl<S, Req, U, F> Transform<S, Req> for TracingTransform<S, U, F>
where
    S: Service<Req>,
    U: ServiceFactory<Req, Response = S::Response, Error = S::Error, Service = S>,
    F: Fn(&Req) -> Option<tracing::Span> + Clone,
{
    type Response = S::Response;
    type Error = S::Error;
    type Transform = TracingService<S, F>;
    type InitError = U::InitError;
    type Future = Ready<Result<Self::Transform, Self::InitError>>;

    fn new_transform(&self, service: S) -> Self::Future {
        ok(TracingService::new(service, self.make_span.clone()))
    }
}

/// Wraps the provided service factory with a transform that automatically
/// enters/exits the given span.
///
/// The span to be entered/exited can be provided via a closure. The closure
/// is passed in a reference to the request being handled by the service.
///
/// For example:
/// ```ignore
/// let traced_service = trace(
///     web_service,
///     |req: &Request| Some(span!(Level::INFO, "request", req.id))
/// );
/// ```
pub fn trace<S, Req, I, F>(
    service_factory: I,
    make_span: F,
) -> ApplyTransform<TracingTransform<S::Service, S, F>, S, Req>
where
    I: IntoServiceFactory<S, Req>,
    S: ServiceFactory<Req>,
    F: Fn(&Req) -> Option<tracing::Span> + Clone,
{
    apply(
        TracingTransform::new(make_span),
        service_factory.into_factory(),
    )
}

#[cfg(test)]
mod test {
    use std::cell::RefCell;
    use std::collections::{BTreeMap, BTreeSet};
    use std::sync::{Arc, RwLock};

    use actix_service::{fn_factory, fn_service};
    use slab::Slab;
    use tracing::{span, Event, Level, Metadata, Subscriber};

    use super::*;

    thread_local! {
        static SPAN: RefCell<Vec<span::Id>> = RefCell::new(Vec::new());
    }

    #[derive(Default)]
    struct Stats {
        entered_spans: BTreeSet<u64>,
        exited_spans: BTreeSet<u64>,
        events_count: BTreeMap<u64, usize>,
    }

    #[derive(Default)]
    struct Inner {
        spans: Slab<&'static Metadata<'static>>,
        stats: Stats,
    }

    #[derive(Clone, Default)]
    struct TestSubscriber {
        inner: Arc<RwLock<Inner>>,
    }

    impl Subscriber for TestSubscriber {
        fn enabled(&self, _metadata: &Metadata<'_>) -> bool {
            true
        }

        fn new_span(&self, span: &span::Attributes<'_>) -> span::Id {
            let id = self.inner.write().unwrap().spans.insert(span.metadata());
            span::Id::from_u64(id as u64 + 1)
        }

        fn record(&self, _span: &span::Id, _values: &span::Record<'_>) {}

        fn record_follows_from(&self, _span: &span::Id, _follows: &span::Id) {}

        fn event(&self, event: &Event<'_>) {
            let id = event
                .parent()
                .cloned()
                .or_else(|| SPAN.with(|current_span| current_span.borrow().last().cloned()))
                .unwrap();

            *self
                .inner
                .write()
                .unwrap()
                .stats
                .events_count
                .entry(id.into_u64())
                .or_insert(0) += 1;
        }

        fn enter(&self, span: &span::Id) {
            self.inner
                .write()
                .unwrap()
                .stats
                .entered_spans
                .insert(span.into_u64());

            SPAN.with(|current_span| {
                current_span.borrow_mut().push(span.clone());
            });
        }

        fn exit(&self, span: &span::Id) {
            self.inner
                .write()
                .unwrap()
                .stats
                .exited_spans
                .insert(span.into_u64());

            // we are guaranteed that on any given thread, spans are exited in reverse order
            SPAN.with(|current_span| {
                let leaving = current_span
                    .borrow_mut()
                    .pop()
                    .expect("told to exit span when not in span");
                assert_eq!(
                    &leaving, span,
                    "told to exit span that was not most recently entered"
                );
            });
        }
    }

    #[actix_rt::test]
    async fn service_call() {
        let service_factory = fn_factory(|| {
            ok::<_, ()>(fn_service(|req: &'static str| {
                tracing::event!(Level::TRACE, "It's happening - {}!", req);
                ok::<_, ()>(())
            }))
        });

        let subscriber = TestSubscriber::default();
        let _guard = tracing::subscriber::set_default(subscriber.clone());

        let span_svc = span!(Level::TRACE, "span_svc");
        let trace_service_factory = trace(service_factory, |_: &&str| Some(span_svc.clone()));
        let service = trace_service_factory.new_service(()).await.unwrap();
        service.call("boo").await.unwrap();

        let id = span_svc.id().unwrap().into_u64();
        assert!(subscriber
            .inner
            .read()
            .unwrap()
            .stats
            .entered_spans
            .contains(&id));
        assert!(subscriber
            .inner
            .read()
            .unwrap()
            .stats
            .exited_spans
            .contains(&id));
        assert_eq!(subscriber.inner.read().unwrap().stats.events_count[&id], 1);
    }
}
