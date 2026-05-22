mod framebuffer;
mod mode;
mod registers;

pub use framebuffer::{Framebuffer, HEIGHT, WIDTH};
pub use mode::PpuMode;
pub use registers::Registers;

// STAT interrupt source enable bits (within the STAT register).
const STAT_INT_LYC: u8 = 1 << 6;
const STAT_INT_OAM: u8 = 1 << 5;
const STAT_INT_VBLANK: u8 = 1 << 4;
const STAT_INT_HBLANK: u8 = 1 << 3;

// LCD enable bit within the LCDC register.
const LCDC_LCD_ENABLE: u8 = 1 << 7;

pub struct Ppu {
    pub regs: Registers,
    framebuffer: Framebuffer,
    mode: PpuMode,
    dots: u16,
    // Cached STAT IRQ line. STAT interrupts fire on the LOW->HIGH transition
    // of any enabled source. Holding the previous level lets us detect that
    // rising edge and avoids spurious refires while the condition persists
    // (the hardware "STAT IRQ blocking" behavior).
    stat_line: bool,
}

impl Ppu {
    pub fn new() -> Self {
        Self {
            regs: Registers::new(),
            framebuffer: Framebuffer::new(),
            mode: PpuMode::OamSearch,
            dots: 0,
            stat_line: false,
        }
    }

    pub fn mode(&self) -> PpuMode {
        self.mode
    }

    pub fn framebuffer(&self) -> &Framebuffer {
        &self.framebuffer
    }

    pub fn read(&self, addr: u16) -> u8 {
        match addr {
            0xff40 => self.regs.lcdc,
            0xff41 => self.read_stat(),
            0xff42 => self.regs.scy,
            0xff43 => self.regs.scx,
            0xff44 => self.regs.ly,
            0xff45 => self.regs.lyc,
            0xff46 => self.regs.dma,
            0xff47 => self.regs.bgp,
            0xff48 => self.regs.obp0,
            0xff49 => self.regs.obp1,
            0xff4a => self.regs.wy,
            0xff4b => self.regs.wx,
            _ => panic!("PPU: unmapped read at {:#06x}", addr),
        }
    }

    pub fn write(&mut self, addr: u16, value: u8) {
        match addr {
            0xff40 => self.write_lcdc(value),
            0xff41 => self.write_stat(value),
            0xff42 => self.regs.scy = value,
            0xff43 => self.regs.scx = value,
            0xff44 => self.regs.ly = 0, // writing to LY resets it (DMG behavior)
            0xff45 => self.regs.lyc = value,
            // TODO: 0xFF46 should kick off an OAM DMA transfer (160 bytes at the
            // address value << 8). Stored as a byte for now; transfer comes later.
            0xff46 => self.regs.dma = value,
            0xff47 => self.regs.bgp = value,
            0xff48 => self.regs.obp0 = value,
            0xff49 => self.regs.obp1 = value,
            0xff4a => self.regs.wy = value,
            0xff4b => self.regs.wx = value,
            _ => panic!("PPU: unmapped write at {:#06x}", addr),
        }
    }

    fn read_stat(&self) -> u8 {
        let coincidence = if self.regs.ly == self.regs.lyc {
            0b100
        } else {
            0
        };
        // bit 7 always reads as 1 on DMG
        0x80 | self.regs.stat_select | coincidence | self.mode.stat_bits()
    }

    fn write_stat(&mut self, value: u8) {
        // Only the interrupt-enable bits (6..=3) are writable from the CPU
        self.regs.stat_select = value & 0b0111_1000;
    }

    fn write_lcdc(&mut self, value: u8) {
        let was_on = (self.regs.lcdc & LCDC_LCD_ENABLE) != 0;
        let now_on = (value & LCDC_LCD_ENABLE) != 0;
        self.regs.lcdc = value;
        if was_on && !now_on {
            // Disabling the LCD halts the PPU. Hardware reports LY=0 and the
            // STAT mode bits as 0 (HBlank) while the LCD is off; the STAT IRQ
            // line drops so re-enabling produces a fresh rising edge.
            self.regs.ly = 0;
            self.dots = 0;
            self.mode = PpuMode::HBlank;
            self.stat_line = false;
        }
    }

    fn compute_stat_line(&self) -> bool {
        let s = self.regs.stat_select;
        if (s & STAT_INT_LYC) != 0 && self.regs.ly == self.regs.lyc {
            return true;
        }
        match self.mode {
            PpuMode::HBlank => (s & STAT_INT_HBLANK) != 0,
            PpuMode::VBlank => (s & STAT_INT_VBLANK) != 0,
            PpuMode::OamSearch => (s & STAT_INT_OAM) != 0,
            PpuMode::Drawing => false,
        }
    }

    /// Advance the PPU by `cycles` dots and return any IF bits to set.
    pub fn tick(&mut self, cycles: u32) -> u8 {
        if (self.regs.lcdc & LCDC_LCD_ENABLE) == 0 {
            return 0;
        }
        let mut interrupts = 0;
        for _ in 0..cycles {
            self.dots += 1;
            if self.dots == 456 {
                self.dots = 0;
                self.regs.ly += 1;
                if self.regs.ly == 154 {
                    self.regs.ly = 0;
                }
                if self.regs.ly == 144 {
                    interrupts |= 0x01; // VBlank -> IF bit 0
                }
            }
            self.mode = if self.regs.ly >= 144 {
                PpuMode::VBlank
            } else if self.dots < 80 {
                PpuMode::OamSearch
            } else if self.dots < 80 + 172 {
                PpuMode::Drawing
            } else {
                PpuMode::HBlank
            };

            // STAT IRQ line: rising edge raises IF bit 1 once; subsequent
            // cycles with the line still high do not refire.
            let new_line = self.compute_stat_line();
            if !self.stat_line && new_line {
                interrupts |= 0x02; // LCD STAT -> IF bit 1
            }
            self.stat_line = new_line;
        }
        interrupts
    }
}

impl Default for Ppu {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ppu_on() -> Ppu {
        let mut ppu = Ppu::new();
        ppu.write(0xff40, LCDC_LCD_ENABLE);
        ppu
    }

    #[test]
    fn ppu_advances_ly_after_456_dots() {
        let mut ppu = ppu_on();
        ppu.tick(456);
        assert_eq!(ppu.regs.ly, 1);
    }

    #[test]
    fn ppu_mode_transitions_within_a_line() {
        let mut ppu = ppu_on();

        assert_eq!(ppu.mode(), PpuMode::OamSearch);
        ppu.tick(79);
        assert_eq!(ppu.mode(), PpuMode::OamSearch);

        ppu.tick(1);
        assert_eq!(ppu.mode(), PpuMode::Drawing);

        ppu.tick(171);
        assert_eq!(ppu.mode(), PpuMode::Drawing);

        ppu.tick(1);
        assert_eq!(ppu.mode(), PpuMode::HBlank);
    }

    #[test]
    fn ppu_enters_vblank_at_ly_144() {
        let mut ppu = ppu_on();
        ppu.tick(456 * 144);

        assert_eq!(ppu.regs.ly, 144);
        assert_eq!(ppu.mode(), PpuMode::VBlank);
    }

    #[test]
    fn ppu_stays_in_vblank_throughout_lines_144_to_153() {
        let mut ppu = ppu_on();
        ppu.tick(456 * 144);
        assert_eq!(ppu.mode(), PpuMode::VBlank);

        for _ in 0..(456 * 10 - 1) {
            ppu.tick(1);
            assert_eq!(ppu.mode(), PpuMode::VBlank);
        }
    }

    #[test]
    fn ppu_wraps_ly_after_a_full_frame() {
        let mut ppu = ppu_on();
        ppu.tick(456 * 154);

        assert_eq!(ppu.regs.ly, 0);
        assert_eq!(ppu.mode(), PpuMode::OamSearch);
    }

    #[test]
    fn tick_returns_vblank_bit_once_per_frame() {
        let mut ppu = ppu_on();

        assert_eq!(ppu.tick(456 * 144 - 1), 0);
        assert_eq!(ppu.tick(1), 0x01);
        assert_eq!(ppu.tick(456 * 10 - 1), 0);
        assert_eq!(ppu.tick(1), 0);
    }

    #[test]
    fn stat_read_carries_mode_bits() {
        let mut ppu = ppu_on();
        assert_eq!(ppu.read(0xff41) & 0b11, 0b10);

        ppu.tick(80);
        assert_eq!(ppu.read(0xff41) & 0b11, 0b11);

        ppu.tick(172);
        assert_eq!(ppu.read(0xff41) & 0b11, 0b00);

        ppu.tick(456 * 144 - 252);
        assert_eq!(ppu.read(0xff41) & 0b11, 0b01);
    }

    #[test]
    fn stat_coincidence_bit_reflects_ly_eq_lyc() {
        let mut ppu = ppu_on();
        ppu.write(0xff45, 0);
        assert_eq!(ppu.read(0xff41) & 0b100, 0b100);

        ppu.tick(456);
        assert_eq!(ppu.read(0xff41) & 0b100, 0);

        ppu.write(0xff45, 1);
        assert_eq!(ppu.read(0xff41) & 0b100, 0b100);
    }

    #[test]
    fn stat_write_only_touches_interrupt_enable_bits() {
        let mut ppu = ppu_on();
        ppu.write(0xff41, 0xff);

        let read = ppu.read(0xff41);
        assert_eq!(read & 0b1111_1000, 0b1111_1000);
    }

    #[test]
    fn write_to_ly_resets_to_zero() {
        let mut ppu = ppu_on();
        ppu.tick(456 * 5);
        assert_eq!(ppu.regs.ly, 5);

        ppu.write(0xff44, 42);
        assert_eq!(ppu.regs.ly, 0);
    }

    #[test]
    fn stat_interrupt_fires_on_oam_search_entry() {
        let mut ppu = ppu_on();
        // Initial mode is OAM Search; line is LOW until source is enabled.
        ppu.write(0xff41, STAT_INT_OAM);
        // First tick raises the line LOW -> HIGH.
        assert_eq!(ppu.tick(1) & 0x02, 0x02);
    }

    #[test]
    fn stat_interrupt_fires_on_hblank_entry() {
        let mut ppu = ppu_on();
        ppu.write(0xff41, STAT_INT_HBLANK);

        // No HBlank source matched yet (we are still in OAM/Drawing)
        assert_eq!(ppu.tick(251) & 0x02, 0);
        // Dot 252 enters HBlank
        assert_eq!(ppu.tick(1) & 0x02, 0x02);
    }

    #[test]
    fn stat_interrupt_fires_on_vblank_entry_alongside_vblank_if() {
        let mut ppu = ppu_on();
        ppu.write(0xff41, STAT_INT_VBLANK);

        let mut total = 0_u8;
        for _ in 0..(456 * 144) {
            total |= ppu.tick(1);
        }
        assert_eq!(total & 0x02, 0x02, "STAT interrupt should fire");
        assert_eq!(total & 0x01, 0x01, "VBlank IF bit should also fire");
    }

    #[test]
    fn stat_interrupt_fires_on_lyc_match() {
        let mut ppu = ppu_on();
        ppu.write(0xff45, 5); // LYC = 5
        ppu.write(0xff41, STAT_INT_LYC);

        let mut total = 0_u8;
        for _ in 0..(456 * 5) {
            total |= ppu.tick(1);
        }
        assert_eq!(total & 0x02, 0x02);
    }

    #[test]
    fn stat_interrupt_does_not_fire_when_sources_disabled() {
        let mut ppu = ppu_on();
        // No STAT sources enabled
        let mut total = 0_u8;
        for _ in 0..(456 * 144) {
            total |= ppu.tick(1);
        }
        assert_eq!(total & 0x02, 0, "STAT must stay quiet");
        assert_eq!(total & 0x01, 0x01, "but VBlank IF still fires independently");
    }

    #[test]
    fn stat_interrupt_does_not_refire_while_line_stays_high() {
        let mut ppu = ppu_on();
        ppu.write(0xff41, STAT_INT_OAM);

        // First cycle fires
        assert_eq!(ppu.tick(1) & 0x02, 0x02);

        // Remaining OAM Search cycles (dots 2..80) do not refire
        let mut total = 0_u8;
        for _ in 0..78 {
            total |= ppu.tick(1);
        }
        assert_eq!(total & 0x02, 0);
    }

    #[test]
    fn overlapping_stat_sources_fire_only_once_per_rising_edge() {
        let mut ppu = ppu_on();
        ppu.write(0xff45, 0); // LYC = LY = 0, so LYC source is true from the start
        ppu.write(0xff41, STAT_INT_LYC | STAT_INT_OAM);

        // Line was LOW, both conditions are now true: single rising edge -> 1 fire.
        assert_eq!(ppu.tick(1) & 0x02, 0x02);

        // Through the rest of line 0's OAM Search: line stays HIGH (LYC or OAM),
        // no refire.
        let mut total = 0_u8;
        for _ in 0..78 {
            total |= ppu.tick(1);
        }
        assert_eq!(total & 0x02, 0);
    }

    #[test]
    fn disabling_lcd_freezes_the_ppu() {
        let mut ppu = ppu_on();
        ppu.tick(456 * 3 + 100); // somewhere into line 3

        ppu.write(0xff40, 0); // LCD off
        assert_eq!(ppu.regs.ly, 0);
        assert_eq!(ppu.mode(), PpuMode::HBlank);

        // Subsequent ticks must not advance state or raise interrupts.
        let irqs = ppu.tick(456 * 200);
        assert_eq!(irqs, 0);
        assert_eq!(ppu.regs.ly, 0);
        assert_eq!(ppu.mode(), PpuMode::HBlank);
    }

    #[test]
    fn stat_reads_mode_zero_while_lcd_is_off() {
        let mut ppu = ppu_on();
        ppu.tick(80); // enter Drawing
        assert_eq!(ppu.read(0xff41) & 0b11, 0b11);

        ppu.write(0xff40, 0);
        assert_eq!(ppu.read(0xff41) & 0b11, 0b00);
    }

    #[test]
    fn re_enabling_lcd_restarts_from_line_zero() {
        let mut ppu = ppu_on();
        ppu.tick(456 * 50);
        ppu.write(0xff40, 0);

        ppu.write(0xff40, LCDC_LCD_ENABLE);
        assert_eq!(ppu.regs.ly, 0);
        assert_eq!(ppu.mode(), PpuMode::HBlank);

        ppu.tick(1);
        assert_eq!(ppu.mode(), PpuMode::OamSearch);
    }

    #[test]
    fn stat_interrupt_stays_quiet_while_lcd_is_off() {
        let mut ppu = ppu_on();
        ppu.write(0xff41, STAT_INT_OAM | STAT_INT_HBLANK | STAT_INT_VBLANK | STAT_INT_LYC);
        ppu.write(0xff40, 0);

        let mut total = 0_u8;
        for _ in 0..(456 * 200) {
            total |= ppu.tick(1);
        }
        assert_eq!(total, 0);
    }
}
