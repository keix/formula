#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum PpuMode {
    HBlank,
    VBlank,
    OamSearch,
    Drawing,
}

pub struct Ppu {
    pub lcdc: u8,
    stat_select: u8, // only bits 6..=3 are writable; others computed on read
    pub scy: u8,
    pub scx: u8,
    pub ly: u8,
    pub lyc: u8,
    pub dma: u8,
    pub bgp: u8,
    pub obp0: u8,
    pub obp1: u8,
    pub wy: u8,
    pub wx: u8,

    mode: PpuMode,
    dots: u16,
    framebuffer: [u8; 160 * 144],
}

impl Ppu {
    pub fn new() -> Self {
        Self {
            lcdc: 0,
            stat_select: 0,
            scy: 0,
            scx: 0,
            ly: 0,
            lyc: 0,
            dma: 0,
            bgp: 0,
            obp0: 0,
            obp1: 0,
            wy: 0,
            wx: 0,
            mode: PpuMode::OamSearch,
            dots: 0,
            framebuffer: [0; 160 * 144],
        }
    }

    pub fn mode(&self) -> PpuMode {
        self.mode
    }

    pub fn framebuffer(&self) -> &[u8; 160 * 144] {
        &self.framebuffer
    }

    pub fn read(&self, addr: u16) -> u8 {
        match addr {
            0xff40 => self.lcdc,
            0xff41 => self.read_stat(),
            0xff42 => self.scy,
            0xff43 => self.scx,
            0xff44 => self.ly,
            0xff45 => self.lyc,
            0xff46 => self.dma,
            0xff47 => self.bgp,
            0xff48 => self.obp0,
            0xff49 => self.obp1,
            0xff4a => self.wy,
            0xff4b => self.wx,
            _ => panic!("PPU: unmapped read at {:#06x}", addr),
        }
    }

    pub fn write(&mut self, addr: u16, value: u8) {
        match addr {
            0xff40 => self.lcdc = value,
            0xff41 => self.write_stat(value),
            0xff42 => self.scy = value,
            0xff43 => self.scx = value,
            0xff44 => self.ly = 0, // writing to LY resets it (DMG behavior)
            0xff45 => self.lyc = value,
            // TODO: 0xFF46 should kick off an OAM DMA transfer (160 bytes at the
            // address value << 8). Stored as a byte for now; transfer comes later.
            0xff46 => self.dma = value,
            0xff47 => self.bgp = value,
            0xff48 => self.obp0 = value,
            0xff49 => self.obp1 = value,
            0xff4a => self.wy = value,
            0xff4b => self.wx = value,
            _ => panic!("PPU: unmapped write at {:#06x}", addr),
        }
    }

    fn read_stat(&self) -> u8 {
        let mode_bits = match self.mode {
            PpuMode::HBlank => 0b00,
            PpuMode::VBlank => 0b01,
            PpuMode::OamSearch => 0b10,
            PpuMode::Drawing => 0b11,
        };
        let coincidence = if self.ly == self.lyc { 0b100 } else { 0 };
        // bit 7 always reads as 1 on DMG
        0x80 | self.stat_select | coincidence | mode_bits
    }

    fn write_stat(&mut self, value: u8) {
        // Only the interrupt-enable bits (6..=3) are writable from the CPU
        self.stat_select = value & 0b0111_1000;
    }

    /// Advance the PPU by `cycles` dots and return any IF bits to set.
    pub fn tick(&mut self, cycles: u32) -> u8 {
        let mut interrupts = 0;
        for _ in 0..cycles {
            self.dots += 1;
            if self.dots == 456 {
                self.dots = 0;
                self.ly += 1;
                if self.ly == 154 {
                    self.ly = 0;
                }
                if self.ly == 144 {
                    interrupts |= 0x01; // VBlank -> IF bit 0
                }
            }
            self.mode = if self.ly >= 144 {
                PpuMode::VBlank
            } else if self.dots < 80 {
                PpuMode::OamSearch
            } else if self.dots < 80 + 172 {
                PpuMode::Drawing
            } else {
                PpuMode::HBlank
            };
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

    #[test]
    fn ppu_advances_ly_after_456_dots() {
        let mut ppu = Ppu::new();
        ppu.tick(456);
        assert_eq!(ppu.ly, 1);
    }

    #[test]
    fn ppu_mode_transitions_within_a_line() {
        let mut ppu = Ppu::new();

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
        let mut ppu = Ppu::new();
        ppu.tick(456 * 144);

        assert_eq!(ppu.ly, 144);
        assert_eq!(ppu.mode(), PpuMode::VBlank);
    }

    #[test]
    fn ppu_stays_in_vblank_throughout_lines_144_to_153() {
        let mut ppu = Ppu::new();
        ppu.tick(456 * 144);
        assert_eq!(ppu.mode(), PpuMode::VBlank);

        for _ in 0..(456 * 10 - 1) {
            ppu.tick(1);
            assert_eq!(ppu.mode(), PpuMode::VBlank);
        }
    }

    #[test]
    fn ppu_wraps_ly_after_a_full_frame() {
        let mut ppu = Ppu::new();
        ppu.tick(456 * 154);

        assert_eq!(ppu.ly, 0);
        assert_eq!(ppu.mode(), PpuMode::OamSearch);
    }

    #[test]
    fn tick_returns_vblank_bit_once_per_frame() {
        let mut ppu = Ppu::new();

        // Lines 0..143: no VBlank yet
        assert_eq!(ppu.tick(456 * 144 - 1), 0);

        // The dot that enters line 144 raises VBlank
        assert_eq!(ppu.tick(1), 0x01);

        // VBlank does not refire while LY stays in 144..=153
        assert_eq!(ppu.tick(456 * 10 - 1), 0);

        // Wrapping back to LY=0 doesn't raise VBlank
        assert_eq!(ppu.tick(1), 0);
    }

    #[test]
    fn stat_read_carries_mode_bits() {
        let mut ppu = Ppu::new();
        assert_eq!(ppu.read(0xff41) & 0b11, 0b10); // OAM Search

        ppu.tick(80);
        assert_eq!(ppu.read(0xff41) & 0b11, 0b11); // Drawing

        ppu.tick(172);
        assert_eq!(ppu.read(0xff41) & 0b11, 0b00); // HBlank

        ppu.tick(456 * 144 - 252);
        assert_eq!(ppu.read(0xff41) & 0b11, 0b01); // VBlank
    }

    #[test]
    fn stat_coincidence_bit_reflects_ly_eq_lyc() {
        let mut ppu = Ppu::new();
        ppu.write(0xff45, 0); // LYC = LY = 0
        assert_eq!(ppu.read(0xff41) & 0b100, 0b100);

        ppu.tick(456); // LY = 1, LYC still 0
        assert_eq!(ppu.read(0xff41) & 0b100, 0);

        ppu.write(0xff45, 1);
        assert_eq!(ppu.read(0xff41) & 0b100, 0b100);
    }

    #[test]
    fn stat_write_only_touches_interrupt_enable_bits() {
        let mut ppu = Ppu::new();
        ppu.write(0xff41, 0xff);

        // Bits 6..=3 stored from write; bit 7 reads back as 1; bits 2..=0 are PPU state.
        let read = ppu.read(0xff41);
        assert_eq!(read & 0b1111_1000, 0b1111_1000);
    }

    #[test]
    fn write_to_ly_resets_to_zero() {
        let mut ppu = Ppu::new();
        ppu.tick(456 * 5);
        assert_eq!(ppu.ly, 5);

        ppu.write(0xff44, 42); // value ignored
        assert_eq!(ppu.ly, 0);
    }
}
