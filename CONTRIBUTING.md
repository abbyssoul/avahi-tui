# Contributing

Thanks for helping improve `avahi-tui`. This project is a Rust terminal UI for
browsing DNS-SD services and launching configured actions.

## Development Setup

Install the Rust toolchain from `rust-toolchain.toml`:

```sh
rustup show
```

On Debian or Ubuntu, install the native dependencies used by discovery and the
terminal UI stack:

```sh
sudo apt-get update
sudo apt-get install -y clang libavahi-client-dev libxcb-shape0-dev libxcb-xfixes0-dev xorg-dev
```

For real mDNS discovery, the Avahi daemon must be available on the system. For
UI and command development, you can run without Avahi by using fake discovery.

## Local Commands

Build the project:

```sh
cargo build --locked
```

Run tests:

```sh
cargo test --locked
```

Check formatting:

```sh
cargo fmt -- --check
```

Run lint checks:

```sh
cargo clippy --locked --all-targets -- -D warnings
```

Run the TUI with sample records:

```sh
cargo run -- --fake-discovery
```

Validate command configs:

```sh
cargo run -- list-commands
```

Validate command configs from a specific directory:

```sh
cargo run -- list-commands --config-dir ./actions
```

Build a Debian package if `cargo-deb` is installed:

```sh
cargo deb
```

## Project Layout

- `src/`: application, UI, discovery, action matching, process launching, and
  keybinding code.
- `actions/`: bundled command examples installed as system command defaults in
  the Debian package.
- `docs/actions.md`: custom command file reference.
- `docs/keybindings.md`: keybinding configuration reference.
- `.github/workflows/`: CI and release packaging workflows.

## Contribution Guidelines

Keep changes focused and easy to review. If a change affects command files,
keybindings, or user-facing behavior, update the matching README or `docs/`
page in the same pull request.

Before opening a pull request, run:

```sh
cargo fmt -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
```

When reporting bugs, include:

- operating system and version
- how `avahi-tui` was installed
- command used to run it
- whether `--fake-discovery` works
- relevant command or keybinding config snippets
- the full error output
