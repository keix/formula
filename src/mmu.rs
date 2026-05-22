use crate::bus::Bus;
use crate::cartridge::Cartridge;
use crate::ppu::Ppu;
use crate::serial::Serial;
use crate::timer::Timer;

pub struct Mmu {
    cartridge: Box<dyn Cartridge>,
    timer: Timer,
    ppu: Ppu,
    serial: Serial,
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
            ppu: Ppu::new(),
            serial: Serial::new(),
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
        let ppu_if = self.ppu.tick(u32::from(cycles));
        if ppu_if != 0 {
            self.io[0x0f] |= ppu_if;
        }
        if self.serial.tick(cycles) {
            self.io[0x0f] |= 0x08; // Serial interrupt -> IF bit 3
        }
    }

    /// Consume any bytes that the CPU has shipped out via the serial port
    /// since the last call.
    pub fn drain_serial_output(&mut self) -> Vec<u8> {
        self.serial.drain_output()
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
            0xff01..=0xff02 => self.serial.read(addr),
            0xff04..=0xff07 => self.timer.read(addr),
            0xff40..=0xff4b => self.ppu.read(addr),
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
            0xff01..=0xff02 => self.serial.write(addr, value),
            0xff04..=0xff07 => self.timer.write(addr, value),
            0xff40..=0xff4b => self.ppu.write(addr, value),
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
    fn vram_covers_the_full_8kib_region() {
        let mut mmu = make_mmu();
        // Sweep across the 8 KiB window at 0x100-byte stride so every
        // 256-byte page in VRAM is exercised.
        for (i, addr) in (0x8000_u16..=0x9fff).step_by(0x100).enumerate() {
            mmu.write8(addr, i as u8);
        }
        for (i, addr) in (0x8000_u16..=0x9fff).step_by(0x100).enumerate() {
            assert_eq!(mmu.read8(addr), i as u8, "mismatch at {addr:#06x}");
        }
    }

    #[test]
    fn vram_does_not_alias_external_ram() {
        let mut mmu = make_mmu();
        // The 0x9fff/0xa000 boundary is the easiest place to write a
        // routing off-by-one. Pin both sides with distinct sentinels.
        mmu.write8(0x9fff, 0x11);
        mmu.write8(0xa000, 0x22);
        assert_eq!(mmu.read8(0x9fff), 0x11);
        assert_eq!(mmu.read8(0xa000), 0x22);
    }

    #[test]
    fn vram_does_not_alias_wram() {
        let mut mmu = make_mmu();
        mmu.write8(0x8000, 0xaa);
        mmu.write8(0xc000, 0x55);
        assert_eq!(mmu.read8(0x8000), 0xaa);
        assert_eq!(mmu.read8(0xc000), 0x55);
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
    fn oam_covers_the_full_a0_byte_region() {
        let mut mmu = make_mmu();
        for (i, addr) in (0xfe00_u16..=0xfe9f).step_by(0x10).enumerate() {
            mmu.write8(addr, i as u8);
        }
        for (i, addr) in (0xfe00_u16..=0xfe9f).step_by(0x10).enumerate() {
            assert_eq!(mmu.read8(addr), i as u8, "mismatch at {addr:#06x}");
        }
    }

    #[test]
    fn oam_does_not_leak_into_unusable_area() {
        let mut mmu = make_mmu();
        // Last byte of OAM stays distinct from the first byte of the
        // unusable region, which always reads 0xFF and swallows writes.
        mmu.write8(0xfe9f, 0x42);
        mmu.write8(0xfea0, 0x42);
        assert_eq!(mmu.read8(0xfe9f), 0x42);
        assert_eq!(mmu.read8(0xfea0), 0xff);
    }

    #[test]
    fn oam_does_not_alias_wram_echo_tail() {
        let mut mmu = make_mmu();
        // 0xFDFF is the last byte of the echo-of-WRAM region; 0xFE00 is the
        // first byte of OAM. Two completely different stores.
        mmu.write8(0xfdff, 0x77);
        mmu.write8(0xfe00, 0x88);
        assert_eq!(mmu.read8(0xfdff), 0x77);
        assert_eq!(mmu.read8(0xfe00), 0x88);
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

    #[test]
    fn ppu_registers_route_to_ppu_not_io() {
        let mut mmu = make_mmu();

        mmu.write8(0xff40, 0x91); // LCDC
        mmu.write8(0xff42, 0x42); // SCY
        mmu.write8(0xff43, 0x10); // SCX
        mmu.write8(0xff45, 0x66); // LYC
        mmu.write8(0xff47, 0xfc); // BGP
        mmu.write8(0xff4a, 0x07); // WY
        mmu.write8(0xff4b, 0x08); // WX

        assert_eq!(mmu.read8(0xff40), 0x91);
        assert_eq!(mmu.read8(0xff42), 0x42);
        assert_eq!(mmu.read8(0xff43), 0x10);
        assert_eq!(mmu.read8(0xff45), 0x66);
        assert_eq!(mmu.read8(0xff47), 0xfc);
        assert_eq!(mmu.read8(0xff4a), 0x07);
        assert_eq!(mmu.read8(0xff4b), 0x08);
    }

    #[test]
    fn ly_is_readable_through_mmu() {
        let mut mmu = make_mmu();
        mmu.write8(0xff40, 0x80); // enable LCD

        // Initial LY is 0
        assert_eq!(mmu.read8(0xff44), 0);

        // After one full scanline (456 dots = 456 T-cycles), LY = 1.
        // Mmu::tick takes u8, so chunk it.
        for _ in 0..(456 / 8) {
            mmu.tick(8);
        }
        assert_eq!(mmu.read8(0xff44), 1);
    }

    #[test]
    fn stat_interrupt_propagates_to_if_bit_1() {
        let mut mmu = make_mmu();
        mmu.write8(0xff40, 0x80); // enable LCD
        // Enable LYC source with LYC = LY = 0; first PPU tick raises the line.
        mmu.write8(0xff45, 0);
        mmu.write8(0xff41, 0b0100_0000);

        mmu.tick(8);

        assert_eq!(mmu.read8(0xff0f) & 0x02, 0x02);
    }

    #[test]
    fn serial_transfer_routes_to_serial_and_raises_if_bit_3() {
        let mut mmu = make_mmu();

        mmu.write8(0xff01, 0x41); // SB = 'A'
        mmu.write8(0xff02, 0x81); // SC = transfer start

        // Bit 7 of SC reads as 0 immediately (instant-transfer stub).
        assert_eq!(mmu.read8(0xff02) & 0x80, 0);

        // Next tick propagates the queued interrupt into IF.
        mmu.tick(4);
        assert_eq!(mmu.read8(0xff0f) & 0x08, 0x08);

        assert_eq!(mmu.drain_serial_output(), b"A");
    }

    #[test]
    fn vblank_entry_sets_if_bit_0() {
        let mut mmu = make_mmu();
        mmu.write8(0xff40, 0x80); // enable LCD

        // 144 lines × 456 dots = 65664 T-cycles to reach VBlank.
        // Chunk through Mmu::tick (u8).
        let mut remaining = 144_u32 * 456;
        while remaining > 0 {
            let chunk = remaining.min(255) as u8;
            mmu.tick(chunk);
            remaining -= u32::from(chunk);
        }

        assert_eq!(mmu.read8(0xff0f) & 0x01, 0x01);
    }
}
