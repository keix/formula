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

    /// CPU-visible read. The default is a plain read, but full-system buses
    /// can attach CPU-side effects such as the DMG OAM corruption bug.
    fn cpu_read8(&mut self, addr: u16) -> u8 {
        self.read8(addr)
    }

    /// CPU-visible write. The default is a plain write, but full-system buses
    /// can attach CPU-side effects such as the DMG OAM corruption bug.
    fn cpu_write8(&mut self, addr: u16, value: u8) {
        self.write8(addr, value);
    }

    /// CPU read in an M-cycle that also performed a 16-bit increment or
    /// decrement on the same register (e.g. `LD A, [HLI]`).
    fn cpu_read8_idu(&mut self, addr: u16) -> u8 {
        self.cpu_read8(addr)
    }

    /// CPU write in an M-cycle that also performed a 16-bit increment or
    /// decrement on the same register (e.g. `LD [HLD], A`).
    fn cpu_write8_idu(&mut self, addr: u16, value: u8) {
        self.cpu_write8(addr, value);
    }

    /// Bare 16-bit increment/decrement bus exposure with no asserted memory
    /// read or write in that M-cycle (e.g. `INC DE`).
    fn cpu_idu_glitch(&mut self, addr: u16) {
        let _ = addr;
    }
    /// Advance the bus-attached subsystems by `cycles` T-cycles.
    ///
    /// Flat test memory has no time-based side effects, so the default
    /// implementation is a no-op.
    fn tick(&mut self, cycles: u8) {
        let _ = cycles;
    }
}
