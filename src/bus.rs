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
    /// CPU-originated read. Full-system buses may attach DMG-specific side
    /// effects such as the OAM corruption bug.
    fn read8_cpu(&mut self, addr: u16) -> u8 {
        self.read8(addr)
    }
    /// CPU-originated write. Full-system buses can use this to attach timing
    /// quirks that should not affect direct test/setup writes.
    fn write8_cpu(&mut self, addr: u16, value: u8) {
        self.write8(addr, value);
    }
    /// CPU read in an M-cycle that also performs a 16-bit inc/dec on the same
    /// register pair (`LD A,[HLI]`, `LD A,[HLD]`).
    fn read8_cpu_idu(&mut self, addr: u16, idu_addr: u16) -> u8 {
        let _ = idu_addr;
        self.read8_cpu(addr)
    }
    /// CPU write in an M-cycle that also performs a 16-bit inc/dec on the
    /// same register pair (`LD [HLI],A`, `LD [HLD],A`).
    fn write8_cpu_idu(&mut self, addr: u16, value: u8, idu_addr: u16) {
        let _ = idu_addr;
        self.write8_cpu(addr, value);
    }
    /// Bare 16-bit inc/dec bus exposure with no asserted read/write
    /// (`INC rr`, `DEC rr`, `INC SP`, `DEC SP`).
    fn idu_glitch_cpu(&mut self, addr: u16) {
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
