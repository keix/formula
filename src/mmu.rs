use crate::bus::Bus;
use crate::cartridge::Cartridge;
use crate::timer::Timer;

pub struct Mmu {
    cartridge: Box<dyn Cartridge>,
    timer: Timer,
    vram: [u8; 0x2000],
    wram: [u8; 0x2000],
    oam: [u8; 0xa0],
    io: [u8; 0x80],
    hram: [u8; 0x7f],
    ie: u8,
}

impl Mmu {
    pub fn new(cartridge: Box<dyn Cartridge>) -> Self {
        Self {
            cartridge,
            timer: Timer::new(),
            vram: [0; 0x2000],
            wram: [0; 0x2000],
            oam: [0; 0xa0],
            io: [0; 0x80],
            hram: [0; 0x7f],
            ie: 0,
        }
    }

    /// Advance memory-mapped sub-systems by `cycles` T-cycles. Subsystems
    /// that raise interrupts set the corresponding bit in IF (0xFF0F).
    pub fn tick(&mut self, cycles: u8) {
        if self.timer.tick(cycles) {
            self.io[0x0f] |= 0x04; // Timer interrupt -> IF bit 2
        }
    }
}

impl Bus for Mmu {
    fn read8(&self, addr: u16) -> u8 {
        match addr {
            0x0000..=0x7fff => self.cartridge.read_rom(addr),
            0x8000..=0x9fff => self.vram[(addr - 0x8000) as usize],
            0xa000..=0xbfff => self.cartridge.read_ram(addr - 0xa000),
            0xc000..=0xdfff => self.wram[(addr - 0xc000) as usize],
            0xe000..=0xfdff => self.wram[(addr - 0xe000) as usize],
            0xfe00..=0xfe9f => self.oam[(addr - 0xfe00) as usize],
            0xfea0..=0xfeff => 0xff,
            0xff04..=0xff07 => self.timer.read(addr),
            0xff00..=0xff7f => self.io[(addr - 0xff00) as usize],
            0xff80..=0xfffe => self.hram[(addr - 0xff80) as usize],
            0xffff => self.ie,
        }
    }

    fn write8(&mut self, addr: u16, value: u8) {
        match addr {
            0x0000..=0x7fff => self.cartridge.write_rom(addr, value),
            0x8000..=0x9fff => self.vram[(addr - 0x8000) as usize] = value,
            0xa000..=0xbfff => self.cartridge.write_ram(addr - 0xa000, value),
            0xc000..=0xdfff => self.wram[(addr - 0xc000) as usize] = value,
            0xe000..=0xfdff => self.wram[(addr - 0xe000) as usize] = value,
            0xfe00..=0xfe9f => self.oam[(addr - 0xfe00) as usize] = value,
            0xfea0..=0xfeff => {}
            0xff04..=0xff07 => self.timer.write(addr, value),
            0xff00..=0xff7f => self.io[(addr - 0xff00) as usize] = value,
            0xff80..=0xfffe => self.hram[(addr - 0xff80) as usize] = value,
            0xffff => self.ie = value,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cartridge::Mbc0;

    fn make_mmu_with_rom(rom: Vec<u8>) -> Mmu {
        Mmu::new(Box::new(Mbc0::new(rom)))
    }

    fn make_mmu() -> Mmu {
        let mut rom = vec![0; 0x8000];
        rom[0x0147] = 0x00;
        rom[0x0149] = 0x02; // 8 KB external RAM
        make_mmu_with_rom(rom)
    }

    #[test]
    fn rom_reads_go_to_cartridge() {
        let mut rom = vec![0; 0x8000];
        rom[0x0147] = 0x00;
        rom[0x0100] = 0xab;
        let mmu = make_mmu_with_rom(rom);

        assert_eq!(mmu.read8(0x0100), 0xab);
    }

    #[test]
    fn vram_roundtrip() {
        let mut mmu = make_mmu();

        mmu.write8(0x8000, 0xab);
        mmu.write8(0x9fff, 0xcd);

        assert_eq!(mmu.read8(0x8000), 0xab);
        assert_eq!(mmu.read8(0x9fff), 0xcd);
    }

    #[test]
    fn external_ram_roundtrip() {
        let mut mmu = make_mmu();

        mmu.write8(0xa000, 0x42);
        mmu.write8(0xbfff, 0x66);

        assert_eq!(mmu.read8(0xa000), 0x42);
        assert_eq!(mmu.read8(0xbfff), 0x66);
    }

    #[test]
    fn wram_roundtrip() {
        let mut mmu = make_mmu();

        mmu.write8(0xc000, 0xab);
        mmu.write8(0xdfff, 0xcd);

        assert_eq!(mmu.read8(0xc000), 0xab);
        assert_eq!(mmu.read8(0xdfff), 0xcd);
    }

    #[test]
    fn echo_ram_mirrors_wram_both_directions() {
        let mut mmu = make_mmu();

        // write to WRAM, read from echo
        mmu.write8(0xc000, 0xab);
        assert_eq!(mmu.read8(0xe000), 0xab);

        // write to echo, read from WRAM
        mmu.write8(0xe100, 0xcd);
        assert_eq!(mmu.read8(0xc100), 0xcd);

        // echo only goes up to 0xfdff -> wram[0x1dff] -> wram addr 0xddff
        mmu.write8(0xddff, 0x77);
        assert_eq!(mmu.read8(0xfdff), 0x77);
    }

    #[test]
    fn oam_roundtrip() {
        let mut mmu = make_mmu();

        mmu.write8(0xfe00, 0xab);
        mmu.write8(0xfe9f, 0xcd);

        assert_eq!(mmu.read8(0xfe00), 0xab);
        assert_eq!(mmu.read8(0xfe9f), 0xcd);
    }

    #[test]
    fn unusable_area_reads_ff_and_drops_writes() {
        let mut mmu = make_mmu();

        mmu.write8(0xfea0, 0x42); // no-op, no panic
        mmu.write8(0xfeff, 0x42);

        assert_eq!(mmu.read8(0xfea0), 0xff);
        assert_eq!(mmu.read8(0xfeff), 0xff);
    }

    #[test]
    fn io_area_roundtrip() {
        let mut mmu = make_mmu();

        mmu.write8(0xff00, 0xab);
        mmu.write8(0xff40, 0x91);
        mmu.write8(0xff7f, 0xcd);

        assert_eq!(mmu.read8(0xff00), 0xab);
        assert_eq!(mmu.read8(0xff40), 0x91);
        assert_eq!(mmu.read8(0xff7f), 0xcd);
    }

    #[test]
    fn hram_roundtrip() {
        let mut mmu = make_mmu();

        mmu.write8(0xff80, 0xab);
        mmu.write8(0xfffe, 0xcd);

        assert_eq!(mmu.read8(0xff80), 0xab);
        assert_eq!(mmu.read8(0xfffe), 0xcd);
    }

    #[test]
    fn ie_register_is_distinct_from_hram() {
        let mut mmu = make_mmu();

        mmu.write8(0xfffe, 0x11); // last byte of HRAM
        mmu.write8(0xffff, 0x1f); // IE

        assert_eq!(mmu.read8(0xfffe), 0x11);
        assert_eq!(mmu.read8(0xffff), 0x1f);
    }

    #[test]
    fn timer_registers_route_to_timer_not_io() {
        let mut mmu = make_mmu();

        // Writing to DIV resets the internal counter regardless of value
        mmu.write8(0xff04, 0xab);
        assert_eq!(mmu.read8(0xff04), 0);

        // TIMA, TMA, TAC are stored values
        mmu.write8(0xff05, 0x42);
        mmu.write8(0xff06, 0x99);
        mmu.write8(0xff07, 0x05);

        assert_eq!(mmu.read8(0xff05), 0x42);
        assert_eq!(mmu.read8(0xff06), 0x99);
        assert_eq!(mmu.read8(0xff07), 0xfd); // 0xf8 | 0x05
    }

    #[test]
    fn tick_sets_timer_interrupt_in_if_on_tima_overflow() {
        let mut mmu = make_mmu();

        mmu.write8(0xff05, 0xff); // TIMA on the brink
        mmu.write8(0xff06, 0x00); // TMA
        mmu.write8(0xff07, 0x05); // enable, clock 01 (every 16 cycles)

        // Run 16 T-cycles to trigger overflow
        mmu.tick(16);

        // IF (0xff0f) should have bit 2 set
        assert_eq!(mmu.read8(0xff0f) & 0x04, 0x04);
        assert_eq!(mmu.read8(0xff05), 0x00); // reloaded from TMA
    }

    #[test]
    fn tick_does_not_clear_other_if_bits() {
        let mut mmu = make_mmu();

        mmu.write8(0xff0f, 0x01); // VBlank already pending
        mmu.write8(0xff05, 0xff);
        mmu.write8(0xff07, 0x05);

        mmu.tick(16);

        // Both VBlank (bit 0) and Timer (bit 2) should be set
        assert_eq!(mmu.read8(0xff0f) & 0x05, 0x05);
    }
}
