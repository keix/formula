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

// LCDC register bits.
const LCDC_LCD_ENABLE: u8 = 1 << 7;
const LCDC_BG_TILE_MAP: u8 = 1 << 3; // 0 = 0x9800-0x9BFF, 1 = 0x9C00-0x9FFF
const LCDC_TILE_DATA: u8 = 1 << 4; // 0 = signed (base 0x9000), 1 = unsigned (base 0x8000)
const LCDC_BG_ENABLE: u8 = 1 << 0;

pub struct Ppu {
    pub regs: Registers,
    framebuffer: Framebuffer,
    vram: [u8; 0x2000],
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
            vram: [0; 0x2000],
            mode: PpuMode::OamSearch,
            dots: 0,
            stat_line: false,
        }
    }

    pub fn read_vram(&self, addr: u16) -> u8 {
        match addr {
            0x8000..=0x9fff => self.vram[(addr - 0x8000) as usize],
            _ => panic!("PPU: read_vram out of range at {:#06x}", addr),
        }
    }

    pub fn write_vram(&mut self, addr: u16, value: u8) {
        match addr {
            0x8000..=0x9fff => self.vram[(addr - 0x8000) as usize] = value,
            _ => panic!("PPU: write_vram out of range at {:#06x}", addr),
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

            // Render the current scanline at the moment we enter HBlank.
            // Hardware draws pixel-by-pixel through mode 3; we composite
            // the whole line in one shot at the transition.
            if self.dots == 80 + 172 && self.regs.ly < 144 {
                self.render_bg_scanline();
            }

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

    fn render_bg_scanline(&mut self) {
        let ly = self.regs.ly as usize;

        // LCDC bit 0: when clear, the BG renders as shade 0 across the line.
        if (self.regs.lcdc & LCDC_BG_ENABLE) == 0 {
            for x in 0..WIDTH {
                self.framebuffer.set_pixel(x, ly, 0);
            }
            return;
        }

        let map_base: u16 = if (self.regs.lcdc & LCDC_BG_TILE_MAP) != 0 {
            0x9c00
        } else {
            0x9800
        };
        let unsigned_tile_data = (self.regs.lcdc & LCDC_TILE_DATA) != 0;
        let bgp = self.regs.bgp;

        let bg_y = self.regs.ly.wrapping_add(self.regs.scy);
        let tile_row = (bg_y / 8) as u16;
        let fine_y = (bg_y % 8) as u16;

        for x in 0..WIDTH {
            let bg_x = (x as u8).wrapping_add(self.regs.scx);
            let tile_col = (bg_x / 8) as u16;
            let fine_x = bg_x % 8;

            let tile_index = self.read_vram(map_base + tile_row * 32 + tile_col);
            let tile_data_addr = if unsigned_tile_data {
                0x8000 + (tile_index as u16) * 16
            } else {
                // Signed addressing: 0x9000 is bank 0, with indices
                // -128..=-1 mapping back into 0x8800-0x8FFF.
                (0x9000_i32 + (tile_index as i8 as i32) * 16) as u16
            };

            let lo = self.read_vram(tile_data_addr + fine_y * 2);
            let hi = self.read_vram(tile_data_addr + fine_y * 2 + 1);

            let bit = 7 - fine_x;
            let color_index = (((hi >> bit) & 1) << 1) | ((lo >> bit) & 1);
            let shade = (bgp >> (color_index * 2)) & 0b11;

            self.framebuffer.set_pixel(x, ly, shade);
        }
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

    // Lock in the register-window contract before VRAM lands in PPU: the
    // read/write methods cover 0xFF40-0xFF4B and panic on anything else.
    // The MMU is the only caller and it never strays outside, but pinning
    // the bounds keeps the upcoming refactor honest.

    #[test]
    #[should_panic(expected = "PPU: unmapped read")]
    fn ppu_read_just_below_register_window_panics() {
        let ppu = Ppu::new();
        let _ = ppu.read(0xff3f);
    }

    #[test]
    #[should_panic(expected = "PPU: unmapped read")]
    fn ppu_read_just_above_register_window_panics() {
        let ppu = Ppu::new();
        let _ = ppu.read(0xff4c);
    }

    #[test]
    #[should_panic(expected = "PPU: unmapped write")]
    fn ppu_write_just_below_register_window_panics() {
        let mut ppu = Ppu::new();
        ppu.write(0xff3f, 0);
    }

    #[test]
    #[should_panic(expected = "PPU: unmapped write")]
    fn ppu_write_just_above_register_window_panics() {
        let mut ppu = Ppu::new();
        ppu.write(0xff4c, 0);
    }

    #[test]
    fn vram_roundtrips_through_ppu_directly() {
        let mut ppu = Ppu::new();
        ppu.write_vram(0x8000, 0xab);
        ppu.write_vram(0x9fff, 0xcd);
        assert_eq!(ppu.read_vram(0x8000), 0xab);
        assert_eq!(ppu.read_vram(0x9fff), 0xcd);
    }

    #[test]
    #[should_panic(expected = "PPU: read_vram out of range")]
    fn vram_read_below_8000_panics() {
        let ppu = Ppu::new();
        let _ = ppu.read_vram(0x7fff);
    }

    #[test]
    #[should_panic(expected = "PPU: write_vram out of range")]
    fn vram_write_above_9fff_panics() {
        let mut ppu = Ppu::new();
        ppu.write_vram(0xa000, 0);
    }

    // Cycles needed to land on the first HBlank entry (dot 252 of line 0).
    const TO_HBLANK: u32 = 80 + 172;

    fn ppu_with_bg() -> Ppu {
        let mut ppu = Ppu::new();
        // LCD on (bit 7), unsigned tile data at 0x8000 (bit 4), BG on (bit 0).
        ppu.write(0xff40, 0x91);
        // Identity BGP: color 0->shade 0, 1->1, 2->2, 3->3.
        ppu.write(0xff47, 0xe4);
        ppu
    }

    #[test]
    fn bg_renders_solid_shade_three_from_a_full_tile() {
        let mut ppu = ppu_with_bg();
        // Tile 0 row 0: both planes 0xFF -> every pixel resolves to color 3.
        ppu.write_vram(0x8000, 0xff);
        ppu.write_vram(0x8001, 0xff);
        // BG map at 0x9800 stays zero, so every map slot picks tile 0.

        ppu.tick(TO_HBLANK);

        let fb = ppu.framebuffer();
        for x in 0..WIDTH {
            assert_eq!(fb.pixel(x, 0), 3, "x={x}");
        }
    }

    #[test]
    fn bg_disable_paints_shade_zero_even_when_tile_is_set() {
        let mut ppu = Ppu::new();
        // LCD on, BG off.
        ppu.write(0xff40, LCDC_LCD_ENABLE);
        ppu.write(0xff47, 0xff); // every color would map to shade 3 if BG ran
        ppu.write_vram(0x8000, 0xff);
        ppu.write_vram(0x8001, 0xff);

        ppu.tick(TO_HBLANK);

        let fb = ppu.framebuffer();
        for x in 0..WIDTH {
            assert_eq!(fb.pixel(x, 0), 0);
        }
    }

    #[test]
    fn bgp_remaps_color_indices_to_shades() {
        let mut ppu = ppu_with_bg();
        // BGP = 0b00_00_00_11: every color index but 0 stays 0, but color
        // index 0 maps to shade 3. Tile 0 row 0 is all zeros, so every
        // pixel has color index 0 -> shade 3 after the remap.
        ppu.write(0xff47, 0b0000_0011);

        ppu.tick(TO_HBLANK);

        let fb = ppu.framebuffer();
        for x in 0..WIDTH {
            assert_eq!(fb.pixel(x, 0), 3);
        }
    }

    #[test]
    fn signed_tile_addressing_finds_tile_at_0x9000() {
        // LCDC.4 = 0 -> tile index is signed; index 0 maps to 0x9000.
        let mut ppu = Ppu::new();
        ppu.write(0xff40, LCDC_LCD_ENABLE | LCDC_BG_ENABLE); // bit 4 stays 0
        ppu.write(0xff47, 0xe4);

        // Tile -1 lives at 0x8FF0; row 0 = all-3 pixels.
        ppu.write_vram(0x8ff0, 0xff);
        ppu.write_vram(0x8ff1, 0xff);
        // Tilemap at 0x9800 stores 0xFF for every cell -> signed index -1.
        for cell in 0x9800_u16..=0x9bff {
            ppu.write_vram(cell, 0xff);
        }

        ppu.tick(TO_HBLANK);

        let fb = ppu.framebuffer();
        for x in 0..WIDTH {
            assert_eq!(fb.pixel(x, 0), 3, "x={x}");
        }
    }

    #[test]
    fn scx_scrolls_the_visible_pixels_horizontally() {
        let mut ppu = ppu_with_bg();
        // Tile 0: a column at fine_x = 0 only (bit 7 of each row).
        // lo=0x80, hi=0x80 -> color index 3 only when bit 7 is read.
        for row in 0..8 {
            ppu.write_vram(0x8000 + row * 2, 0x80);
            ppu.write_vram(0x8001 + row * 2, 0x80);
        }

        ppu.write(0xff43, 0); // SCX = 0
        ppu.tick(TO_HBLANK);
        assert_eq!(ppu.framebuffer().pixel(0, 0), 3);
        assert_eq!(ppu.framebuffer().pixel(1, 0), 0);
        assert_eq!(ppu.framebuffer().pixel(8, 0), 3); // next tile, fine_x = 0

        // Re-render with SCX = 1 -> the column that was at x=0 now sits at x=-1
        // (i.e. just off the left edge), and the next tile's column lands at x=7.
        ppu.regs.ly = 0;
        ppu.dots = 0;
        ppu.write(0xff43, 1);
        ppu.tick(TO_HBLANK);
        assert_eq!(ppu.framebuffer().pixel(0, 0), 0);
        assert_eq!(ppu.framebuffer().pixel(7, 0), 3);
    }
}
