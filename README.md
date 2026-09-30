RustBucket
---

A simple C2 framework written in Rust.

This is basically learning project for me and my team, to learn Rust and how C2 frameworks work. It is in no way a finished project that's ready for a production environment.

# Requirements

## Rust

Follow [the official instructions](https://www.rust-lang.org/tools/install) to install Rust.

## Mingw

Mingw is needed to cross compile the windows payload. You can install it using your package manager.

### Debian Based Distros (Ubuntu, Kali, etc)

```bash
sudo apt update
sudo apt install mingw-w64
```


### Arch Linux / Arch Based Distros (Manjaro, etc)

```bash
sudo pacman -S mingw-w64-gcc
```

### RH Based Distros (Fedora, CentOS, etc)

```bash
sudo dnf install mingw32-gcc mingw64-gcc
```

## Cargo cross compilation capabilities

You also need to add windows cross compilation features to cargo to be able to cross compile the windows payload.

```bash
rustup target add x86_64-pc-windows-gnu
```

# Build

To build the entire project, you can use the following command:

```bash
cargo build --release
```

This builds all the binaries in the `target/release` folder.


To build a specific crate, use its package name:

```bash
cargo build -p <crate_name> --release
```

# Usage

1. Run the server

```bash
./rb_server
```

Runs the server on `0.0.0.0:6666` by default. Settings are read from `rb_server.toml` in the current directory (if present); CLI flags override it. See [Configuration](#configuration).

2. Run the client

```bash
./rb_client
```

This will connect to the default server on localhost:6666. You can change the connection details with command line flags. Run `./rb_client --help` for more details. The client has tab completion for commands and subcommands, syntax highlighting, and keeps history in `~/.rustbucket_history`.

# Configuration

The server reads `rb_server.toml` from the current directory by default. Pass `--config <file>` to use a different one. Any CLI flag (`--host`, `--port`, `--mtls`, `--ca-path`, `--ca-key-path`, `--cert-path`, `--key-path`, `--crl-path`) overrides the file. A missing default file is fine, the built-in defaults are used.

```toml
host = "0.0.0.0"
port = 6666
verbose = false
db_path = "rustbucket.sqlite"

[mtls]
enabled = false
ca_path = "certs/ca-cert.pem"
ca_key_path = "certs/ca-key.pem"
cert_path = "certs/client-cert.pem"
key_path = "certs/client-key.pem"
crl_path = "certs/crl.der"
crl_update_seconds = 5
```

Sessions, tasks, results and listener definitions are saved to the SQLite database at `db_path`, so they survive a restart. HTTP listeners are automatically bound again on startup. Set `db_path = ""` to keep everything in memory.

The CA key at `ca_key_path` is generated once and reused, so certificates issued to operators stay valid across restarts.

# Listeners

Start an HTTPS listener from the client before running an implant:

```
listeners start http -p 8080
```

The listener serves the implant endpoints over HTTPS with mTLS, so the implant must present a client cert signed by the implant CA. The server writes that CA and an implant cert/key pair to `certs/` (`implant-ca-cert.pem`, `implant-cert.pem`, `implant-key.pem`).

To run a standalone implant for testing, point it at those certs:

```bash
cargo run -p rb_implant -- --host <ip> --port 8080 --ca-path certs/implant-ca-cert.pem --cert-path certs/implant-cert.pem --key-path certs/implant-key.pem
```

# Implant commands

Once an implant has checked in, attach to it from the client with `sessions use <id>` and run one of:

File management:
- `pwd`, `cd <path>` - working directory
- `ls [path]`, `cat <file>`, `download <file>` (sends the file back to the operator)
- `upload <local-file> [remote-path]` (sends a local file to the implant)
- `mkdir <dir>`, `rm [-r] <path>`, `mv <src> <dst>`, `cp [-r] <src> <dst>`, `touch <file>`

System:
- `systeminfo`, `whoami`, `env`, `ps`, `kill <pid>`
- `sleep <seconds> [jitter-percent]` - set the beacon interval (and optional jitter)

Network:
- `netstat`, `ipconfig`

Execution:
- `shell <command>` - run a command through powershell on Windows, sh elsewhere

Commands run natively on the implant. Unknown commands are rejected, and there is no implicit shell fallback; use `shell` to run arbitrary commands explicitly. The implant beacons every `--interval` seconds at startup and the interval can be changed at runtime with `sleep`.

# Operator profiles

The server issues per-operator client certificates signed by its CA.

- `operator new <name> [--host <host>] [--port <port>]` writes `operators/<name>/client-cert.pem`, `operators/<name>/client-key.pem` and a profile at `operators/<name>.toml`.
- `operator list` lists profiles and whether they are revoked.
- `operator revoke <name>` revokes a profile; the server's CRL updater rejects it within a few seconds.

Connect with an issued profile (CLI flags override it):

```bash
./rb_client --profile operators/<name>.toml
```

# Payloads

From the client, build a Windows payload and have the server send it back:

```
payload new --lhost <listener-ip> --lport <listener-port>
```

The server cross-compiles the implant and the client saves the executable to its working directory (for example `rb_payload.exe`). This needs `mingw-w64` and the `x86_64-pc-windows-gnu` target, and the server must run from the repo root so it can find `rb_implant` when building.

The generated payload embeds the implant CA and client cert/key automatically, so it can talk to an HTTPS listener without any cert files on disk.
