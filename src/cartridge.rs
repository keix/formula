pub trait Cartridge {
    fn read_rom(&self, addr: u16) -> u8;
    fn write_rom(&mut self, addr: u16, value: u8);
    fn read_ram(&self, addr: u16) -> u8;
    fn write_ram(&mut self, addr: u16, value: u8);
}

pub struct Mbc0 {
    rom: Vec<u8>,
    ram: Vec<u8>,
}

impl Mbc0 {
    pub fn new(rom: Vec<u8>) -> Self {
        let ram_size = match rom.get(0x0149).copied().unwrap_or(0) {
            0x00 => 0,
            0x01 => 2 * 1024,
            0x02 => 8 * 1024,
            _ => 0,
        };
        Self {
            rom,
            ram: vec![0; ram_size],
        }
    }
}

impl Cartridge for Mbc0 {
    fn read_rom(&self, addr: u16) -> u8 {
        self.rom.get(addr as usize).copied().unwrap_or(0xff)
    }

    fn write_rom(&mut self, _addr: u16, _value: u8) {}

    fn read_ram(&self, addr: u16) -> u8 {
        self.ram.get(addr as usize).copied().unwrap_or(0xff)
    }

    fn write_ram(&mut self, addr: u16, value: u8) {
        if let Some(byte) = self.ram.get_mut(addr as usize) {
            *byte = value;
        }
    }
}

pub fn load_cartridge(rom: Vec<u8>) -> Box<dyn Cartridge> {
    let mbc_type = *rom
        .get(0x0147)
        .expect("ROM too small to contain a cartridge header");
    match mbc_type {
        0x00 => Box::new(Mbc0::new(rom)),
        t => panic!("unsupported cartridge type: {:#04x}", t),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rom_with_header(mbc_type: u8, ram_size: u8) -> Vec<u8> {
        let mut rom = vec![0; 0x8000];
        rom[0x0147] = mbc_type;
        rom[0x0149] = ram_size;
        rom
    }

    #[test]
    fn mbc0_reads_rom_bytes() {
        let mut rom = rom_with_header(0x00, 0x02);
        rom[0x0100] = 0xab;
        let cart = Mbc0::new(rom);

        assert_eq!(cart.read_rom(0x0100), 0xab);
        assert_eq!(cart.read_rom(0x0000), 0x00);
    }

    #[test]
    fn mbc0_ignores_writes_to_rom() {
        let mut rom = rom_with_header(0x00, 0x00);
        rom[0x2000] = 0x42;
        let mut cart = Mbc0::new(rom);

        cart.write_rom(0x2000, 0xff);

        assert_eq!(cart.read_rom(0x2000), 0x42);
    }

    #[test]
    fn mbc0_external_ram_roundtrips() {
        let mut cart = Mbc0::new(rom_with_header(0x00, 0x02));

        cart.write_ram(0x0100, 0x42);

        assert_eq!(cart.read_ram(0x0100), 0x42);
    }

    #[test]
    fn mbc0_without_ram_reads_ff_and_swallows_writes() {
        let mut cart = Mbc0::new(rom_with_header(0x00, 0x00));

        cart.write_ram(0x0000, 0x42);

        assert_eq!(cart.read_ram(0x0000), 0xff);
    }

    #[test]
    fn mbc0_short_rom_reads_ff_past_end() {
        let cart = Mbc0::new(vec![0xab; 0x0150]);

        assert_eq!(cart.read_rom(0x0100), 0xab);
        assert_eq!(cart.read_rom(0x0200), 0xff);
    }

    #[test]
    fn load_cartridge_returns_mbc0_for_type_00() {
        let cart = load_cartridge(rom_with_header(0x00, 0x00));
        // smoke test — reads should succeed
        assert_eq!(cart.read_rom(0x0147), 0x00);
    }

    #[test]
    #[should_panic(expected = "unsupported cartridge type")]
    fn load_cartridge_panics_on_unknown_mbc() {
        load_cartridge(rom_with_header(0xff, 0x00));
    }

    #[test]
    #[should_panic(expected = "ROM too small")]
    fn load_cartridge_panics_on_short_rom() {
        load_cartridge(vec![0; 0x10]);
    }
}
