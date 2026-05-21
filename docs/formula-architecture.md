# Formula Architecture

This document records the stable architectural assumptions for `formula`.

## Overview

`formula` is a Game Boy emulator written in Rust.
The codebase is organized around a CPU that executes SM83 instructions through a bus abstraction, plus memory and cartridge components that provide addressable storage.

## CPU Registers

The CPU model exposes the standard Game Boy register set:

| Register | Width | Notes |
| ------- | ----- | ----- |
| `A` | 8-bit | Accumulator |
| `F` | 8-bit | Flags register; low nibble is always zero |
| `B` | 8-bit | General-purpose |
| `C` | 8-bit | General-purpose |
| `D` | 8-bit | General-purpose |
| `E` | 8-bit | General-purpose |
| `H` | 8-bit | General-purpose; pairs with `L` |
| `L` | 8-bit | General-purpose; pairs with `H` |
| `SP` | 16-bit | Stack pointer |
| `PC` | 16-bit | Program counter |

The following 16-bit register pairs are treated as architectural units:

| Pair | Components |
| ---- | ---------- |
| `AF` | `A` + `F` |
| `BC` | `B` + `C` |
| `DE` | `D` + `E` |
| `HL` | `H` + `L` |

## Flags Register

The flags register uses the upper nibble only:

| Bit | Flag | Meaning |
| --- | ---- | ------- |
| 7 | `Z` | Zero |
| 6 | `N` | Subtract |
| 5 | `H` | Half-carry |
| 4 | `C` | Carry |
| 3-0 | - | Always zero |

The invariant that bits `3..=0` stay clear is part of the architecture, not just an implementation detail.

## Memory Model

The emulator uses a `Bus` abstraction for 8-bit reads and writes.
This lets the CPU operate against either a flat memory image or a full MMU-backed address space.

### Flat Memory

`Memory` provides a simple 64 KiB contiguous address space.
It is useful for focused CPU tests and program loading.

### MMU Layout

The MMU currently maps the address space as follows:

| Address range | Region |
| ------------- | ------ |
| `0x0000..=0x7FFF` | Cartridge ROM |
| `0x8000..=0x9FFF` | VRAM |
| `0xA000..=0xBFFF` | Cartridge external RAM |
| `0xC000..=0xDFFF` | WRAM |
| `0xE000..=0xFDFF` | Echo of WRAM |
| `0xFE00..=0xFE9F` | OAM |
| `0xFEA0..=0xFEFF` | Unusable area |
| `0xFF00..=0xFF7F` | I/O registers |
| `0xFF80..=0xFFFE` | HRAM |
| `0xFFFF` | Interrupt enable register |

The unusable area reads as `0xFF` and ignores writes.
The echo area mirrors WRAM.

## Cartridge Model

Cartridge access is abstracted behind a `Cartridge` trait with separate ROM and external RAM entry points.

The current implementation supports:

| Mapper | Support |
| ------ | ------- |
| `MBC0` | Supported |

Unsupported cartridge types are rejected explicitly.

## Instruction Model

The CPU executes one instruction per `step` and returns the elapsed time in T-cycles.
This makes timing visible at the CPU boundary and keeps instruction execution aligned with Game Boy timing terminology.

The instruction decoder follows two broad patterns:

- Irregular instructions are handled explicitly.
- Regular opcode families are decoded by bit pattern, especially register-to-register loads, ALU register forms, and CB-prefixed operations.

CB-prefixed instructions are treated as a separate opcode page.

## Current Scope

The current codebase already includes:

- CPU register state and stepping logic
- A flags type that preserves the `F` register invariant
- Flat 64 KiB memory for direct CPU tests
- An MMU with core Game Boy address regions
- Cartridge loading for `MBC0`

Areas that are intentionally not frozen in this document include implementation tactics, refactoring notes, and evolving opcode-by-opcode decisions.

## References

- Pan Docs: https://gbdev.io/pandocs/
