# Formula


A Game Boy emulator written in Rust.  
Status: **work in progress.** The bus and 64 KB memory are in place; the CPU is not implemented yet.

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
