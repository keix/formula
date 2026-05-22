//! Memory-mapped PPU registers (0xFF40-0xFF4B except DMA).
//!
//! `stat_select` holds only the writable STAT bits (6..3); the mode
//! and coincidence bits are synthesised on read. DMA (0xFF46) lives
//! on the MMU because the transfer needs bus access.

pub struct Registers {
    pub lcdc: u8,
    pub(super) stat_select: u8,
    pub scy: u8,
    pub scx: u8,
    pub ly: u8,
    pub lyc: u8,
    pub bgp: u8,
    pub obp0: u8,
    pub obp1: u8,
    pub wy: u8,
    pub wx: u8,
}

impl Registers {
    pub fn new() -> Self {
        Self {
            lcdc: 0,
            stat_select: 0,
            scy: 0,
            scx: 0,
            ly: 0,
            lyc: 0,
            bgp: 0,
            obp0: 0,
            obp1: 0,
            wy: 0,
            wx: 0,
        }
    }
}

impl Default for Registers {
    fn default() -> Self {
        Self::new()
    }
}
