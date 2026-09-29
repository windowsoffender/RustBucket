# RustBucket

Learning C2 framework in Rust, not production ready. `README.md` has install (mingw) and usage.

## Workspace layout

- `rb` - shared lib. Wire types (`message`), `task`, `session` (a plain data struct), `store` (the `Store` trait + `MemoryStore`), command registry (`command`), implant-facing `listener::http_listener`. Everything else depends on it.
- `rb_server` - operator server. Newline-delimited JSON over TCP, optional mTLS, embeds actix HTTP listeners for implants, `store::SqliteStore` for persistence.
- `rb_client` - operator REPL (reedline). Talks to `rb_server`.
- `rb_implant` - Windows payload. HTTP check-in / poll / report loop.
- `rb_payload_build/` - generated at runtime by the `payload` command, gitignored and excluded from the workspace. Never edit by hand.

Two separate channels, don't mix them:
- operator <-> server: JSON lines over TCP (`LinesCodec` in `rb_server/src/server.rs`).
- implant <-> server: HTTP served by `HttpListener` (`/checkin`, `/tasks/{implant_id}`, `/results`, `/implants`) inside the server process.

## Commands

- Build all: `cargo build --release`
- One crate: `cargo build -p rb_server --release` (README's `--lib <name>` form is invalid cargo)
- Check: `cargo check --workspace`
- Test: `cargo test --workspace`. Tests live in `rb/src/session.rs` and `rb_implant/src/lib.rs`. Single test: `cargo test -p rb test_session_creation`
- Run: `cargo run -p rb_server` (reads `rb_server.toml` from CWD, else defaults; binds `0.0.0.0:6666`; `--config`, `--db-path`, `--mtls`, `--host`, `--port`, `--ca-path/--cert-path/--key-path/--crl-path` flags override), `cargo run -p rb_client -- --host localhost --port 6666`, `cargo run -p rb_implant -- --host <ip> --port 8080`
- No CI, no rustfmt/clippy config. `cargo check` + `cargo test` is the whole verification story.

Payload cross-compile needs `mingw-w64` and `rustup target add x86_64-pc-windows-gnu`.

Implants only work after the operator starts an HTTP listener from the client: `listeners start http -p 8080` (matches the implant's default port).

## Adding commands

Server commands: implement `RbCommand` in `rb/src/command/server_cmds/`, re-export in that `mod.rs`, register in `CommandRegistry::new` (`rb/src/command/mod.rs`). Implant commands use the same path, but their registrations are commented out so they never run. Uncomment to wire them up.

Routing: `CommandRegistry::execute` treats a request as an implant command when `session_id` is set, otherwise a server command.

`sessions use <id>` is handled in the client (`rb_client/src/main.rs`), not the server. The server's `sessions use` branch is dead code.

## Gotchas

- All state goes through `rb::store::Store`: implants, sessions, tasks, results and the listener registry. The SQLite impl (`rb_server::store::SqliteStore`) deliberately lives in `rb_server`, not `rb`, so the implant doesn't link SQLite. `db_path = ""` in the config means in-memory. Add state methods to the trait, not to the impls.
- Persisted listeners are re-bound by `RbServer::restore_listeners` at startup using their stored id. `listeners stop` deletes the record so it isn't restored. Persisted implants let an implant keep polling the same id across a restart; `get_tasks` reactivates its session.
- `mTLS` is a throwaway test PKI generated at server startup (`rb_server/src/certs`). Start the server with `--mtls`; it writes the CA, client cert/key and CRL to `certs/` (created if missing). The client must match: `--mtls --host localhost --ca-path certs/ca-cert.pem --cert-path certs/client-cert.pem --key-path certs/client-key.pem`. `*.pem`/`*.der` are gitignored.
- The server's mTLS path uses `tokio-rustls` 0.26 with `rustls` 0.23; `rb` stays TLS-free and `Client` in `rb/src/client.rs` is a metadata handle (id/addr/disconnect flag), not a stream. Transports are generic over `S: AsyncRead + AsyncWrite` in `server.rs::handle_client`.
- `*.sync-conflict-*` files at the root are Nextcloud sync artifacts, not source.
