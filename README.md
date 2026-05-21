# Formula


A Game Boy emulator written in Rust.  
Status: **work in progress.** Core CPU execution, memory, MMU, and basic cartridge support are in place.

## Architecture

Stable architectural notes live in [docs/formula-architecture.md](docs/formula-architecture.md).
Working design notes and implementation decisions remain in `design.md`.

## Build

```sh
cargo build
```

## Test

```sh
cargo test
```

## Development shell

A Nix flake is provided with `cargo`, `rustc`, `rustfmt`, `clippy`, and `rust-analyzer`:

```sh
nix develop
```

## License

MIT — see [LICENSE](LICENSE).
