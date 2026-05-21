# Formula

Formula is not an abstraction.  
It is the law of motion beneath the machine.

## Overview
Formula is a Game Boy emulator written in Rust.

It starts close to C: bytes, registers, memory, and explicit state changes.  
Rust is used not to hide the machine, but to make its boundaries visible and testable.

AI writes.  
Rust reduces the review surface.

Status: **work in progress.** Core CPU execution, memory, MMU, and basic cartridge support are in place.

## Philosophy

Formula is built from explicit boundaries.

The CPU does not own the world.
It fetches bytes, mutates registers, and talks to memory through the bus.

Memory is bytes.  
Registers are state.  
Instructions are mutations.  
Tests are executable specifications.

Formula reveals the machine, one boundary at a time.

## Architecture

Stable architectural notes live in [docs/formula-architecture.md](docs/formula-architecture.md).

The current core is organized around this shape:

```text
CPU -> MMU -> Cartridge / Memory / I/O
```

The emulator begins from the CPU core and grows outward:

```
CPU
Memory
MMU
Cartridge
Timer
Interrupts
PPU
Input
```

## Current Status

Implemented:

- core CPU execution
- register state
- instruction fetch
- memory
- MMU
- basic cartridge support
- unit tests for core behavior

Not yet complete:

- full instruction set
- interrupts
- timer
- PPU
- joypad input
- sound
- cycle-accurate behavior

## Development shell

A Nix flake is provided with `cargo`, `rustc`, `rustfmt`, `clippy`, and `rust-analyzer`:

```
nix develop
```

## Build

```
cargo build
```

## Test

```
cargo test
```

## License

Formula is released under the MIT License. Copyright (C) 2026 Master *void a.k.a. keix.