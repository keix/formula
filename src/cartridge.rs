pub trait Cartridge {
    fn read_rom(&self, addr: u16) -> u8;
    fn write_rom(&mut self, addr: u16, value: u8);
    fn read_ram(&self, addr: u16) -> u8;
    fn write_ram(&mut self, addr: u16, value: u8);
}

fn ram_size_from_header(byte: u8) -> usize {
    match byte {
        0x00 => 0,
        0x01 => 2 * 1024,
        0x02 => 8 * 1024,
        0x03 => 32 * 1024,
        0x04 => 128 * 1024,
        0x05 => 64 * 1024,
        _ => 0,
    }
}

pub struct Mbc0 {
    rom: Vec<u8>,
    ram: Vec<u8>,
}

impl Mbc0 {
    pub fn new(rom: Vec<u8>) -> Self {
        let ram_size = ram_size_from_header(rom.get(0x0149).copied().unwrap_or(0));
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

/// MBC1: up to 2 MiB ROM (128 banks) + 32 KiB RAM (4 banks).
///
/// Bank registers form an 8-bit address: `(bank_hi << 5) | bank_lo`.
/// - `bank_lo` (5 bits): hardware substitutes a written zero with one, so
///   the 0x4000-0x7FFF window can never select bank 0/0x20/0x40/0x60 —
///   those slots show up as 0x01/0x21/0x41/0x61 instead.
/// - `mode == 0` (default): 0x0000-0x3FFF is bank 0, RAM is bank 0.
/// - `mode == 1`: 0x0000-0x3FFF reads `bank_hi << 5`, RAM bank = `bank_hi`.
pub struct Mbc1 {
    rom: Vec<u8>,
    ram: Vec<u8>,
    ram_enabled: bool,
    bank_lo: u8,
    bank_hi: u8,
    mode: u8,
}

impl Mbc1 {
    pub fn new(rom: Vec<u8>) -> Self {
        let ram_size = ram_size_from_header(rom.get(0x0149).copied().unwrap_or(0));
        Self {
            rom,
            ram: vec![0; ram_size],
            ram_enabled: false,
            bank_lo: 1,
            bank_hi: 0,
            mode: 0,
        }
    }

    fn rom_bank_count(&self) -> usize {
        (self.rom.len() / 0x4000).max(1)
    }

    fn ram_bank_count(&self) -> usize {
        (self.ram.len() / 0x2000).max(1)
    }
}

impl Cartridge for Mbc1 {
    fn read_rom(&self, addr: u16) -> u8 {
        let bank = match addr {
            0x0000..=0x3fff => {
                if self.mode == 1 {
                    (self.bank_hi as usize) << 5
                } else {
                    0
                }
            }
            0x4000..=0x7fff => ((self.bank_hi as usize) << 5) | (self.bank_lo as usize),
            _ => return 0xff,
        };
        let bank = bank & (self.rom_bank_count() - 1);
        let offset = bank * 0x4000 + (addr as usize & 0x3fff);
        self.rom.get(offset).copied().unwrap_or(0xff)
    }

    fn write_rom(&mut self, addr: u16, value: u8) {
        match addr {
            0x0000..=0x1fff => self.ram_enabled = (value & 0x0f) == 0x0a,
            0x2000..=0x3fff => {
                let v = value & 0x1f;
                self.bank_lo = if v == 0 { 1 } else { v };
            }
            0x4000..=0x5fff => self.bank_hi = value & 0x03,
            0x6000..=0x7fff => self.mode = value & 0x01,
            _ => {}
        }
    }

    fn read_ram(&self, addr: u16) -> u8 {
        if !self.ram_enabled || self.ram.is_empty() {
            return 0xff;
        }
        let bank = if self.mode == 1 {
            self.bank_hi as usize
        } else {
            0
        };
        let bank = bank & (self.ram_bank_count() - 1);
        let offset = bank * 0x2000 + (addr as usize & 0x1fff);
        self.ram.get(offset).copied().unwrap_or(0xff)
    }

    fn write_ram(&mut self, addr: u16, value: u8) {
        if !self.ram_enabled || self.ram.is_empty() {
            return;
        }
        let bank = if self.mode == 1 {
            self.bank_hi as usize
        } else {
            0
        };
        let bank = bank & (self.ram_bank_count() - 1);
        let offset = bank * 0x2000 + (addr as usize & 0x1fff);
        if let Some(b) = self.ram.get_mut(offset) {
            *b = value;
        }
    }
}

pub fn load_cartridge(rom: Vec<u8>) -> Box<dyn Cartridge> {
    let mbc_type = *rom
        .get(0x0147)
        .expect("ROM too small to contain a cartridge header");
    match mbc_type {
        0x00 => Box::new(Mbc0::new(rom)),
        0x01..=0x03 => Box::new(Mbc1::new(rom)),
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

    fn mbc1_rom(banks: usize, ram_size: u8) -> Vec<u8> {
        let mut rom = vec![0; banks * 0x4000];
        rom[0x0147] = 0x01; // MBC1
        rom[0x0149] = ram_size;
        // Tag each 16 KiB bank with its index at offset 0 so reads can identify it.
        for bank in 0..banks {
            rom[bank * 0x4000] = bank as u8;
        }
        rom
    }

    #[test]
    fn mbc1_bank0_is_always_visible_at_0000_in_default_mode() {
        let cart = Mbc1::new(mbc1_rom(4, 0));
        assert_eq!(cart.read_rom(0x0000), 0x00);
    }

    #[test]
    fn mbc1_window_at_4000_defaults_to_bank_1() {
        let cart = Mbc1::new(mbc1_rom(4, 0));
        assert_eq!(cart.read_rom(0x4000), 0x01);
    }

    #[test]
    fn mbc1_bank_select_writes_the_lower_five_bits() {
        let mut cart = Mbc1::new(mbc1_rom(8, 0));

        cart.write_rom(0x2000, 0x03);
        assert_eq!(cart.read_rom(0x4000), 0x03);

        cart.write_rom(0x2000, 0x07);
        assert_eq!(cart.read_rom(0x4000), 0x07);
    }

    #[test]
    fn mbc1_writing_zero_to_bank_register_selects_bank_one() {
        let mut cart = Mbc1::new(mbc1_rom(8, 0));
        cart.write_rom(0x2000, 0x00);
        assert_eq!(cart.read_rom(0x4000), 0x01);
    }

    #[test]
    fn mbc1_bank_high_bits_extend_rom_address_in_default_mode() {
        // 128 banks ≈ 2 MiB. bank_hi 1 + bank_lo 1 → bank 33.
        let mut cart = Mbc1::new(mbc1_rom(64, 0));
        cart.write_rom(0x2000, 0x01); // bank_lo = 1
        cart.write_rom(0x4000, 0x01); // bank_hi = 1
        assert_eq!(cart.read_rom(0x4000), 33);

        // 0x0000-0x3FFF still maps to bank 0 in mode 0.
        assert_eq!(cart.read_rom(0x0000), 0);
    }

    #[test]
    fn mbc1_advanced_mode_maps_high_banks_into_low_window() {
        let mut cart = Mbc1::new(mbc1_rom(128, 0));
        cart.write_rom(0x4000, 0x01); // bank_hi = 1
        cart.write_rom(0x6000, 0x01); // mode = 1
                                      // Low window now sees bank (1 << 5) = 32.
        assert_eq!(cart.read_rom(0x0000), 32);
        // High window: (1 << 5) | bank_lo(1) = 33.
        assert_eq!(cart.read_rom(0x4000), 33);
    }

    #[test]
    fn mbc1_bank_index_wraps_to_rom_size() {
        // 4-bank ROM but software requests bank 7 — should wrap to bank 3.
        let mut cart = Mbc1::new(mbc1_rom(4, 0));
        cart.write_rom(0x2000, 0x07);
        assert_eq!(cart.read_rom(0x4000), 0x03);
    }

    #[test]
    fn mbc1_ram_is_disabled_until_unlocked() {
        let mut cart = Mbc1::new(mbc1_rom(2, 0x02)); // 8 KiB RAM
        cart.write_ram(0x0000, 0x42);
        assert_eq!(cart.read_ram(0x0000), 0xff);
    }

    #[test]
    fn mbc1_ram_roundtrips_when_enabled_with_magic_value() {
        let mut cart = Mbc1::new(mbc1_rom(2, 0x02));
        cart.write_rom(0x0000, 0x0a); // unlock RAM
        cart.write_ram(0x0100, 0x42);
        assert_eq!(cart.read_ram(0x0100), 0x42);
    }

    #[test]
    fn mbc1_ram_re_locks_on_non_magic_write() {
        let mut cart = Mbc1::new(mbc1_rom(2, 0x02));
        cart.write_rom(0x0000, 0x0a);
        cart.write_ram(0x0100, 0x42);

        cart.write_rom(0x0000, 0x00);
        assert_eq!(cart.read_ram(0x0100), 0xff);
    }

    #[test]
    fn mbc1_ram_banks_under_advanced_mode() {
        let mut cart = Mbc1::new(mbc1_rom(2, 0x03)); // 32 KiB RAM (4 banks)
        cart.write_rom(0x0000, 0x0a);
        cart.write_rom(0x6000, 0x01); // mode = 1

        cart.write_rom(0x4000, 0x00); // RAM bank 0
        cart.write_ram(0x0000, 0x11);

        cart.write_rom(0x4000, 0x02); // RAM bank 2
        cart.write_ram(0x0000, 0x22);

        cart.write_rom(0x4000, 0x00);
        assert_eq!(cart.read_ram(0x0000), 0x11);
        cart.write_rom(0x4000, 0x02);
        assert_eq!(cart.read_ram(0x0000), 0x22);
    }

    #[test]
    fn load_cartridge_returns_mbc1_for_types_01_through_03() {
        for ty in [0x01_u8, 0x02, 0x03] {
            let cart = load_cartridge(rom_with_header(ty, 0x00));
            assert_eq!(cart.read_rom(0x0147), ty);
        }
    }
}
