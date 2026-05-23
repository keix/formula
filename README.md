# Formula

Formula is not an abstraction.  
It is the law of motion beneath the machine.

## Overview
Formula is a Game Boy emulator written in Rust.

It starts close to C: bytes, registers, memory, and explicit state changes.  
Rust is used not to hide the machine, but to make its boundaries visible and testable.

AI writes.  
Rust reduces the review surface.

Status: **work in progress.** CPU, MMU, PPU, timer, serial, and joypad run on DMG-accurate timing. The APU is landing now.

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

## Current Status

Implemented:

- full CPU instruction set, including HALT, STOP, and the DMG OAM-bug edge cases
- interrupt dispatch with IME/IE/IF and the post-push vector resolve
- MMU with per-M-cycle subsystem ticking
- cartridge support for MBC0 and MBC1
- timer with DMG-accurate overflow timing
- serial port on real DMG timing
- joypad register wired through to IF bit 4
- PPU: BG, window, and sprite layers with priorities, palettes, flips, 8x16, OAM DMA — passes `dmg-acid2`
- integration suite covering the CPU, PPU, timer, serial, and OAM-bug behaviors

In progress (on `impl-apu`):

- APU: NR10–NR52 register file, frame sequencer, length counters, envelope, sweep, wave channel, mixed output through `aplay`

Not yet complete:

- additional MBCs (MBC2/3/5, RTC, battery-backed save)
- full cycle accuracy across every subsystem
- Game Boy Color features

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