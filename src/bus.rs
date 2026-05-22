//! Address-space access trait.
//!
//! The CPU is generic over this trait so it can be unit-tested against
//! the lightweight [`crate::memory::Memory`] while the binary plugs in
//! the full [`crate::mmu::Mmu`].

pub trait Bus {
    /// Read one byte from `addr`. Reads must never panic — out-of-range
    /// or unmapped reads return a defined value (`0xFF` in the MMU).
    fn read8(&self, addr: u16) -> u8;
    /// Write one byte to `addr`. Writes to ROM or unmapped regions are
    /// silently dropped; writes to memory-mapped IO have side effects.
    fn write8(&mut self, addr: u16, value: u8);
}
