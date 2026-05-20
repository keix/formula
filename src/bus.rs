pub trait Bus {
    fn read8(&self, addr: u16) -> u8;
    fn write8(&mut self, addr: u16, value: u8);
}
