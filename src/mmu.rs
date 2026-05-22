use crate::bus::Bus;
use crate::cartridge::Cartridge;
use crate::joypad::Joypad;
use crate::ppu::{Framebuffer, Ppu};
use crate::serial::Serial;
use crate::timer::Timer;

pub struct Mmu {
    cartridge: Box<dyn Cartridge>,
    timer: Timer,
    ppu: Ppu,
    serial: Serial,
    joypad: Joypad,
    wram: [u8; 0x2000],
    io: [u8; 0x80],
    hram: [u8; 0x7f],
    ie: u8,
    // Last value written to 0xFF46. Reads return this verbatim; writes
    // also kick off an OAM DMA copy of 160 bytes from (value << 8) to
    // OAM (0xFE00-0xFE9F).
    dma: u8,
    // Set when the PPU just entered VBlank (i.e. completed a frame).
    // The display loop consumes this via take_frame_ready().
    frame_ready: bool,
}

impl Mmu {
    pub fn new(cartridge: Box<dyn Cartridge>) -> Self {
        Self {
            cartridge,
            timer: Timer::new(),
            ppu: Ppu::new(),
            serial: Serial::new(),
            joypad: Joypad::new(),
            wram: [0; 0x2000],
            io: [0; 0x80],
            hram: [0; 0x7f],
            ie: 0,
            dma: 0,
            frame_ready: false,
        }
    }

    /// Push the current set of pressed buttons (a bitmask of joypad::BUTTON_*)
    /// into the joypad. Raises IF bit 4 if any button just became pressed.
    pub fn set_buttons(&mut self, pressed: u8) {
        self.joypad.set_pressed(pressed);
        if self.joypad.take_interrupt() {
            self.io[0x0f] |= 0x10; // Joypad -> IF bit 4
        }
    }

    fn start_oam_dma(&mut self, value: u8) {
        self.dma = value;
        let base = (value as u16) << 8;
        // Real hardware bus-blocks the CPU for ~160 M-cycles and only HRAM
        // stays accessible. We do an instant 160-byte copy; ROMs that rely
        // on a HRAM bounce-routine still work because read8 below covers
        // every address space the source value can legally point at.
        for i in 0..0xa0_u16 {
            let byte = self.read8(base + i);
            self.ppu.write_oam(0xfe00 + i, byte);
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
            if ppu_if & 0x01 != 0 {
                self.frame_ready = true;
            }
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

    /// Returns true once per frame, when the PPU has just entered VBlank.
    /// Reading the flag clears it so the next frame can be detected.
    pub fn take_frame_ready(&mut self) -> bool {
        std::mem::take(&mut self.frame_ready)
    }

    pub fn framebuffer(&self) -> &Framebuffer {
        self.ppu.framebuffer()
    }
}

impl Bus for Mmu {
    // The IO range 0xFF00-0xFF7F is the catch-all for the generic io[] array;
    // earlier arms intentionally carve out specific routed sub-addresses
    // (joypad, serial, timer, ppu, dma). First-match-wins semantics make
    // this idiomatic, but clippy still flags exact-start overlaps.
    #[allow(clippy::match_overlapping_arm)]
    fn read8(&self, addr: u16) -> u8 {
        match addr {
            0x0000..=0x7fff => self.cartridge.read_rom(addr),
            0x8000..=0x9fff => self.ppu.read_vram(addr),
            0xa000..=0xbfff => self.cartridge.read_ram(addr - 0xa000),
            0xc000..=0xdfff => self.wram[(addr - 0xc000) as usize],
            0xe000..=0xfdff => self.wram[(addr - 0xe000) as usize],
            0xfe00..=0xfe9f => self.ppu.read_oam(addr),
            0xfea0..=0xfeff => 0xff,
            0xff00 => self.joypad.read(addr),
            0xff01..=0xff02 => self.serial.read(addr),
            0xff04..=0xff07 => self.timer.read(addr),
            0xff46 => self.dma,
            0xff40..=0xff4b => self.ppu.read(addr),
            0xff00..=0xff7f => self.io[(addr - 0xff00) as usize],
            0xff80..=0xfffe => self.hram[(addr - 0xff80) as usize],
            0xffff => self.ie,
        }
    }

    #[allow(clippy::match_overlapping_arm)]
    fn write8(&mut self, addr: u16, value: u8) {
        match addr {
            0x0000..=0x7fff => self.cartridge.write_rom(addr, value),
            0x8000..=0x9fff => self.ppu.write_vram(addr, value),
            0xa000..=0xbfff => self.cartridge.write_ram(addr - 0xa000, value),
            0xc000..=0xdfff => self.wram[(addr - 0xc000) as usize] = value,
            0xe000..=0xfdff => self.wram[(addr - 0xe000) as usize] = value,
            0xfe00..=0xfe9f => self.ppu.write_oam(addr, value),
            0xfea0..=0xfeff => {}
            0xff00 => self.joypad.write(addr, value),
            0xff01..=0xff02 => self.serial.write(addr, value),
            0xff04..=0xff07 => self.timer.write(addr, value),
            0xff46 => self.start_oam_dma(value),
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
        // 0xFF00 (joypad) and 0xFF46 (DMA) bypass the generic io[] array;
        // they each have dedicated tests above. The other unmapped IO
        // bytes still round-trip through the array.
        let mut mmu = make_mmu();

        mmu.write8(0xff03, 0xab);
        mmu.write8(0xff40, 0x91);
        mmu.write8(0xff7f, 0xcd);

        assert_eq!(mmu.read8(0xff03), 0xab);
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
    fn joypad_button_press_routes_to_if_bit_4() {
        use crate::joypad::BUTTON_A;
        let mut mmu = make_mmu();
        mmu.write8(0xff00, 0x10); // P15 = 0 (action row)

        // Press A: a fresh transition latches the Joypad IRQ in IF.
        mmu.set_buttons(BUTTON_A);
        assert_eq!(mmu.read8(0xff0f) & 0x10, 0x10);

        // Bit 0 of the joypad register reflects A pressed (active low).
        assert_eq!(mmu.read8(0xff00) & 0x01, 0);
    }

    #[test]
    fn oam_dma_register_reads_back_last_written_value() {
        let mut mmu = make_mmu();
        // The value addresses ROM where MBC0 starts as zeros — safe target.
        mmu.write8(0xff46, 0x00);
        assert_eq!(mmu.read8(0xff46), 0x00);

        mmu.write8(0xff46, 0xc0);
        assert_eq!(mmu.read8(0xff46), 0xc0);
    }

    #[test]
    fn oam_dma_copies_160_bytes_from_source_into_oam() {
        let mut mmu = make_mmu();
        // Seed WRAM at 0xC000..0xC09F with a recognisable pattern.
        for i in 0..0xa0_u8 {
            mmu.write8(0xc000 + i as u16, i ^ 0xa5);
        }
        // Make sure OAM is empty first.
        for i in 0..0xa0_u16 {
            mmu.write8(0xfe00 + i, 0);
        }

        // Trigger DMA from page 0xC0.
        mmu.write8(0xff46, 0xc0);

        for i in 0..0xa0_u8 {
            assert_eq!(
                mmu.read8(0xfe00 + i as u16),
                i ^ 0xa5,
                "OAM byte {i} should have been copied from WRAM"
            );
        }
    }

    #[test]
    fn oam_dma_can_source_from_vram() {
        // Verify the DMA loop walks every region the value can legally point
        // at — VRAM is the most common one for sprite tile bytes.
        let mut mmu = make_mmu();
        mmu.write8(0xff40, 0x80); // enable LCD so VRAM is writable in our model
        for i in 0..0xa0_u8 {
            mmu.write8(0x8000 + i as u16, i);
        }

        mmu.write8(0xff46, 0x80);

        for i in 0..0xa0_u8 {
            assert_eq!(mmu.read8(0xfe00 + i as u16), i);
        }
    }

    #[test]
    fn serial_transfer_routes_to_serial_and_raises_if_bit_3() {
        let mut mmu = make_mmu();

        mmu.write8(0xff01, 0x41); // SB = 'A'
        mmu.write8(0xff02, 0x81); // SC = transfer start

        // Start bit stays high until the internal-clock transfer finishes.
        assert_eq!(mmu.read8(0xff02) & 0x80, 0x80);

        // Internal-clock DMG serial takes 8 bits * 512 T-cycles.
        let mut remaining = 8_u32 * 512;
        while remaining > 0 {
            let chunk = remaining.min(255) as u8;
            mmu.tick(chunk);
            remaining -= u32::from(chunk);
        }

        assert_eq!(mmu.read8(0xff0f) & 0x08, 0x08);
        assert_eq!(mmu.read8(0xff02) & 0x80, 0);

        assert_eq!(mmu.drain_serial_output(), b"A");
    }

    #[test]
    fn take_frame_ready_fires_once_per_vblank_entry() {
        let mut mmu = make_mmu();
        mmu.write8(0xff40, 0x80); // enable LCD

        assert!(!mmu.take_frame_ready());

        // Tick through a full pre-VBlank frame.
        let mut remaining = 144_u32 * 456;
        while remaining > 0 {
            let chunk = remaining.min(255) as u8;
            mmu.tick(chunk);
            remaining -= u32::from(chunk);
        }

        assert!(
            mmu.take_frame_ready(),
            "frame should be ready at VBlank entry"
        );
        assert!(!mmu.take_frame_ready(), "the flag is one-shot");
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
