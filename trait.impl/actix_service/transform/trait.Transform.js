(function() {
    var implementors = Object.fromEntries([["actix_service",[]],["actix_tracing",[["impl&lt;S, Req, U, F&gt; Transform&lt;S, Req&gt; for <a class=\"struct\" href=\"actix_tracing/struct.TracingTransform.html\" title=\"struct actix_tracing::TracingTransform\">TracingTransform</a>&lt;S, U, F&gt;<div class=\"where\">where\n    S: Service&lt;Req&gt;,\n    U: ServiceFactory&lt;Req, Response = S::Response, Error = S::Error, Service = S&gt;,\n    F: <a class=\"trait\" href=\"https://doc.rust-lang.org/nightly/core/ops/function/trait.Fn.html\" title=\"trait core::ops::function::Fn\">Fn</a>(<a class=\"primitive\" href=\"https://doc.rust-lang.org/nightly/std/primitive.reference.html\">&amp;Req</a>) -&gt; <a class=\"enum\" href=\"https://doc.rust-lang.org/nightly/core/option/enum.Option.html\" title=\"enum core::option::Option\">Option</a>&lt;Span&gt; + <a class=\"trait\" href=\"https://doc.rust-lang.org/nightly/core/clone/trait.Clone.html\" title=\"trait core::clone::Clone\">Clone</a>,</div>"]]]]);
    if (window.register_implementors) {
        window.register_implementors(implementors);
    } else {
        window.pending_implementors = implementors;
    }
})()
//{"start":57,"fragment_lengths":[20,935]}