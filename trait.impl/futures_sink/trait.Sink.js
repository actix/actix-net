(function() {
    var implementors = Object.fromEntries([["actix_codec",[["impl&lt;T, U, I&gt; Sink&lt;I&gt; for <a class=\"struct\" href=\"actix_codec/struct.Framed.html\" title=\"struct actix_codec::Framed\">Framed</a>&lt;T, U&gt;<div class=\"where\">where\n    T: <a class=\"trait\" href=\"actix_codec/trait.AsyncWrite.html\" title=\"trait actix_codec::AsyncWrite\">AsyncWrite</a>,\n    U: <a class=\"trait\" href=\"actix_codec/trait.Encoder.html\" title=\"trait actix_codec::Encoder\">Encoder</a>&lt;I&gt;,\n    U::<a class=\"associatedtype\" href=\"actix_codec/trait.Encoder.html#associatedtype.Error\" title=\"type actix_codec::Encoder::Error\">Error</a>: <a class=\"trait\" href=\"https://doc.rust-lang.org/nightly/core/convert/trait.From.html\" title=\"trait core::convert::From\">From</a>&lt;<a class=\"struct\" href=\"https://doc.rust-lang.org/nightly/std/io/error/struct.Error.html\" title=\"struct std::io::error::Error\">Error</a>&gt;,</div>"]]],["local_channel",[["impl&lt;T&gt; Sink&lt;T&gt; for <a class=\"struct\" href=\"local_channel/mpsc/struct.Sender.html\" title=\"struct local_channel::mpsc::Sender\">Sender</a>&lt;T&gt;"]]]]);
    if (window.register_implementors) {
        window.register_implementors(implementors);
    } else {
        window.pending_implementors = implementors;
    }
})()
//{"start":57,"fragment_lengths":[901,188]}