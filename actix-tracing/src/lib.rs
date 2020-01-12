//! Actix tracing - support for tokio tracing with Actix services.
#![deny(rust_2018_idioms, warnings)]

use std::marker::PhantomData;
use std::task::{Context, Poll};

use actix_service::{apply, IntoServiceFactory, Service, ServiceFactory, Transform};
use futures_util::future::{ok, Either, Ready};
use tracing_futures::{Instrument, Instrumented};

/// Trait representing a type that is able to produce spans for a given value.
///
/// The generic argument `T` is typically the request type of a `Service`
/// implementation.
pub trait MakeSpan<T> {
    /// Creates a new span for the given request value.
    ///
    /// By default, this function returns `None`.
    fn span_for(&self, _req: &T) -> Option<tracing::Span> {
        None
    }
}

impl<T, F> MakeSpan<T> for F
where
    F: Fn(&T) -> Option<tracing::Span>,
{
    fn span_for(&self, req: &T) -> Option<tracing::Span> {
        (self)(req)
    }
}

impl<T> MakeSpan<T> for tracing::Span {
    fn span_for(&self, _: &T) -> Option<tracing::Span> {
        Some(self.clone())
    }
}

/// A `Service` implementation that automatically enters/exits tracing spans
/// for the wrapped inner service.
#[derive(Clone)]
pub struct TracingService<S, T> {
    inner: S,
    make_span: T,
}

impl<S, T> TracingService<S, T>
where
    S: Service,
    T: MakeSpan<S::Request>,
{
    pub fn new(inner: S, make_span: T) -> Self {
        TracingService { inner, make_span }
    }
}

impl<S, T> Service for TracingService<S, T>
where
    S: Service,
    T: MakeSpan<S::Request>,
{
    type Request = S::Request;
    type Response = S::Response;
    type Error = S::Error;
    type Future = Either<S::Future, Instrumented<S::Future>>;

    fn poll_ready(&mut self, ctx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(ctx)
    }

    fn call(&mut self, req: Self::Request) -> Self::Future {
        let span = self.make_span.span_for(&req);
        let _enter = span.as_ref().map(|s| s.enter());

        let fut = self.inner.call(req);

        // make a child span to track the future's execution
        if let Some(span) = span
            .clone()
            .map(|span| tracing::span!(parent: &span, tracing::Level::INFO, "future"))
        {
            Either::Right(fut.instrument(span))
        } else {
            Either::Left(fut)
        }
    }
}

/// A `Transform` implementation that wraps services with a [`TracingService`].
///
/// [`TracingService`]: struct.TracingService.html
pub struct TracingTransform<S, U, T> {
    make_span: T,
    _p: PhantomData<fn(S, U)>,
}

impl<S, U, T> TracingTransform<S, U, T> {
    pub fn new(make_span: T) -> Self {
        TracingTransform {
            make_span,
            _p: PhantomData,
        }
    }
}

impl<S, U, T> Transform<S> for TracingTransform<S, U, T>
where
    S: Service,
    U: ServiceFactory<
        Request = S::Request,
        Response = S::Response,
        Error = S::Error,
        Service = S,
    >,
    T: MakeSpan<S::Request> + Clone,
{
    type Request = S::Request;
    type Response = S::Response;
    type Error = S::Error;
    type InitError = U::InitError;
    type Transform = TracingService<S, T>;
    type Future = Ready<Result<Self::Transform, Self::InitError>>;

    fn new_transform(&self, service: S) -> Self::Future {
        ok(TracingService::new(service, self.make_span.clone()))
    }
}

/// Wraps the provided service factory with a transform that automatically
/// enters/exits the given span.
///
/// The span to be entered/exited can be provided via an implementation of
/// the [`MakeSpan`] trait. A blanket implementation of [`MakeSpan`] for
/// `Fn(&T) -> Option<Span>` allows you to pass a closure for the `make_span`
/// argument. The closure is passed in a reference to the request being
/// handled by the service.
///
/// For example:
/// ```rust,ignore
/// let traced_service = trace(
///     web_service,
///     |req: &Request| Some(span!(Level::INFO, "request", req.id))
/// );
/// ```
///
/// [`MakeSpan`]: trait.MakeSpan.html
pub fn trace<S, U, T>(
    service_factory: U,
    make_span: T,
) -> impl ServiceFactory<
    Config = S::Config,
    Request = S::Request,
    Response = S::Response,
    Error = S::Error,
    InitError = S::InitError,
>
where
    S: ServiceFactory,
    T: MakeSpan<S::Request> + Clone,
    U: IntoServiceFactory<S>,
{
    apply(
        TracingTransform::<_, S, _>::new(make_span),
        service_factory.into_factory(),
    )
}

#[cfg(test)]
mod test {
    use super::*;

    use std::cell::RefCell;
    use std::collections::{BTreeMap, BTreeSet};
    use std::sync::{Arc, RwLock};

    use actix_service::{fn_factory, fn_service};
    use slab::Slab;
    use tracing::{span, Event, Level, Metadata, Subscriber};

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
        let mut service = trace_service_factory.new_service(()).await.unwrap();
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
