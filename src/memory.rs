//! Flat 64 KiB memory backing the CPU's [`Bus`] in unit tests.
//!
//! The real address space is decoded by [`crate::mmu::Mmu`]; this
//! type exists so the CPU can be exercised against a single
//! contiguous array, with no banks, IO routing, or memory-mapped
//! subsystems in the way.

use crate::bus::Bus;

pub struct Memory {
    data: [u8; 0x10000],
}

impl Memory {
    pub fn new() -> Self {
        Self { data: [0; 0x10000] }
    }

    /// Copy `bytes` into memory starting at `start`. Used by tests to
    /// seed program code or fixture data; panics if the slice would
    /// run past the end of the address space.
    pub fn load(&mut self, start: u16, bytes: &[u8]) {
        let start = start as usize;
        let end = start + bytes.len();
        self.data[start..end].copy_from_slice(bytes);
    }
}

impl Default for Memory {
    fn default() -> Self {
        Self::new()
    }
}

impl Bus for Memory {
    fn read8(&self, addr: u16) -> u8 {
        self.data[addr as usize]
    }

    fn write8(&mut self, addr: u16, value: u8) {
        self.data[addr as usize] = value;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_write_memory() {
        let mut mem = Memory::new();

        mem.write8(0x1234, 0x42);

        assert_eq!(mem.read8(0x1234), 0x42);
    }

    #[test]
    fn load_program() {
        let mut mem = Memory::new();

        mem.load(0x0000, &[0x3e, 0x42, 0x76]);

        assert_eq!(mem.read8(0x0000), 0x3e);
        assert_eq!(mem.read8(0x0001), 0x42);
        assert_eq!(mem.read8(0x0002), 0x76);
    }

    #[test]
    fn uninitialized_memory_reads_zero() {
        let mem = Memory::new();

        assert_eq!(mem.read8(0x0000), 0x00);
        assert_eq!(mem.read8(0x1234), 0x00);
        assert_eq!(mem.read8(0xffff), 0x00);
    }

    #[test]
    fn load_at_high_boundary() {
        let mut mem = Memory::new();

        mem.load(0xfffe, &[0xab, 0xcd]);

        assert_eq!(mem.read8(0xfffe), 0xab);
        assert_eq!(mem.read8(0xffff), 0xcd);
    }

    #[test]
    fn write_at_high_boundary() {
        let mut mem = Memory::new();

        mem.write8(0xffff, 0x7f);

        assert_eq!(mem.read8(0xffff), 0x7f);
    }
}
