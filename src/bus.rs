//! Address-space access trait.
//!
//! The CPU is generic over this trait so it can be unit-tested against
//! the lightweight [`crate::memory::Memory`] while the binary plugs in
//! the full [`crate::mmu::Mmu`].

pub trait Bus {
    fn read8(&self, addr: u16) -> u8;
    fn write8(&mut self, addr: u16, value: u8);
}
