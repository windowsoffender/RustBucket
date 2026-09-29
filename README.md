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

This will connect to the default server on localhost:6666. You can change the connection details with command line flags. Run `./rb_client --help` for more details.

# Configuration

The server reads `rb_server.toml` from the current directory by default. Pass `--config <file>` to use a different one. Any CLI flag (`--host`, `--port`, `--mtls`, `--ca-path`, `--cert-path`, `--key-path`, `--crl-path`) overrides the file. A missing default file is fine, the built-in defaults are used.

```toml
host = "0.0.0.0"
port = 6666
verbose = false
db_path = "rustbucket.sqlite"

[mtls]
enabled = false
ca_path = "certs/ca-cert.pem"
cert_path = "certs/client-cert.pem"
key_path = "certs/client-key.pem"
crl_path = "certs/crl.der"
crl_update_seconds = 5
```

Sessions, tasks, results and listener definitions are saved to the SQLite database at `db_path`, so they survive a restart. HTTP listeners are automatically bound again on startup. Set `db_path = ""` to keep everything in memory.

# TODO

- Implement actual commands to do stuff instead of just powershell commands.
- Nicer cli experience (tab completion, syntax highlighting, etc).
- Operator profiles and a command to generate them.
- Make server send the generated payload to the client (currently it just stays on the server).
