//! formula - a Game Boy (DMG) emulator core
//!
//! The crate is split along the hardware lines a real DMG has: [`cpu`]
//! decodes and executes opcodes against [`bus::Bus`], [`mmu`] arbitrates
//! the address bus, and per-subsystem modules ([`ppu`], [`timer`],
//! [`serial`], [`joypad`], [`cartridge`]) own their own state behind a
//! narrow API the MMU dispatches to. The binary at `src/main.rs` wires
//! this library to a minifb window and the host keyboard.

pub mod bus;
pub mod cartridge;
pub mod cpu;
pub mod flags;
pub mod joypad;
pub mod memory;
pub mod mmu;
pub mod ppu;
pub mod serial;
pub mod timer;
