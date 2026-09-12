# TLS Credential Hot Reload

The `hot-reload-rustls.rs` and `hot-reload-openssl.rs` examples are each self-contained. Each file separates server setup, credential loading, and the polling task into functions.

The `hot-reload-rustls` example uses Rustls 0.23 to replace a certificate and private key without restarting the server or rebinding its listener. All workers share a certificate resolver backed by `arc-swap`. Each handshake loads an owned snapshot of the current credentials. Every two seconds, a blocking task reads the PEM files and validates that the key matches the leaf certificate. It installs the pair only after validation succeeds. Missing, incomplete, or mismatched files leave the current credentials in use. Invalid credentials at startup stop the server.

Install [mkcert](https://github.com/FiloSottile/mkcert#installation), then run these commands from the workspace root to generate test credentials in a separate directory:

```sh
mkdir -p /tmp/actix-tls-reload
mkcert -key-file /tmp/actix-tls-reload/key.pem -cert-file /tmp/actix-tls-reload/cert.pem localhost 127.0.0.1 ::1
cargo run -p actix-tls --example hot-reload-rustls --features rustls-0_23 -- /tmp/actix-tls-reload/cert.pem /tmp/actix-tls-reload/key.pem
```

To inspect the generated certificate, use [`inspect-cert-chain`](https://github.com/robjtede/inspect-cert-chain):

```sh
inspect-cert-chain --file /tmp/actix-tls-reload/cert.pem
```

In another terminal, inspect the certificate served by a fresh TLS connection:

```sh
inspect-cert-chain --host localhost --port 8443
```

Generate a replacement pair, then move both files into place:

```sh
mkcert -key-file /tmp/actix-tls-reload/key.next.pem -cert-file /tmp/actix-tls-reload/cert.next.pem localhost 127.0.0.1 ::1
mv /tmp/actix-tls-reload/key.next.pem /tmp/actix-tls-reload/key.pem
mv /tmp/actix-tls-reload/cert.next.pem /tmp/actix-tls-reload/cert.pem
```

After the server logs `TLS credentials reloaded`, repeat the inspection command. The serial and fingerprint change while the server process and listener stay in place. To test failure handling, replace `cert.pem` with an empty file. After a reload error, a fresh connection still receives the last valid certificate.

This is a TLS-only example: it closes each connection after the handshake and does not send an HTTP response. In an application, established TLS connections continue with their existing state. Only new full handshakes use the new certificate; resumed sessions can omit certificate exchange. This example does not revoke sessions, validate certificate expiry or trust, or select certificates by SNI. Use a certificate chain with the leaf certificate first.

The same resolver can be passed to the Rustls config builder's `with_cert_resolver` for Actix Web's `HttpServer::bind_rustls_0_23`. Keep the resolver shared with the reload task and configure the HTTP server as usual. No Actix TLS library change is required.

## OpenSSL

The `hot-reload-openssl` example provides the same behavior with OpenSSL. Run it with the test credentials generated above, after stopping the Rustls example:

```sh
cargo run -p actix-tls --example hot-reload-openssl --features openssl -- \
  /tmp/actix-tls-reload/cert.pem /tmp/actix-tls-reload/key.pem
```

Use the same replacement and inspection commands above. All workers share the current OpenSSL context. Every two seconds, a blocking task loads the certificate chain and private key into a new context and checks that they match. It replaces the shared context only when the validated certificate PEM contents change. Invalid files leave the current context in use; invalid startup credentials stop the server.

The [OpenSSL server name callback](https://docs.rs/openssl/latest/openssl/ssl/struct.SslAcceptorBuilder.html#method.set_servername_callback) selects the current context for each handshake. It does not select by hostname and also works for clients that omit SNI. The callback uses `arc-swap` to load a snapshot of the context. File access and key parsing finish before the reload task atomically publishes the replacement. As with the Rustls example, this server closes connections after the handshake and does not serve HTTP. The same limits on session resumption, certificate validation, and revocation apply. Private keys must be unencrypted PEM files.

For Actix Web, pass the callback-equipped builder to `HttpServer::bind_openssl` and keep the reload task running. No Actix TLS library change is required.

Related requests: [actix-net#13](https://github.com/actix/actix-net/issues/13) and [actix-web#754](https://github.com/actix/actix-web/issues/754).
