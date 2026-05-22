#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum PpuMode {
    HBlank,
    VBlank,
    OamSearch,
    Drawing,
}

pub struct Ppu {
    pub lcdc: u8,
    pub stat: u8,
    pub scy: u8,
    pub scx: u8,
    pub ly: u8,
    pub lyc: u8,
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
            stat: 0,
            scy: 0,
            scx: 0,
            ly: 0,
            lyc: 0,
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

    /// Advance the PPU by `cycles` dots (1 dot == 1 T-cycle on DMG).
    pub fn tick(&mut self, cycles: u32) {
        for _ in 0..cycles {
            self.dots += 1;
            if self.dots == 456 {
                self.dots = 0;
                self.ly += 1;
                if self.ly == 154 {
                    self.ly = 0;
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

        // Mode 2 (OAM Search) covers dots 0..80
        assert_eq!(ppu.mode(), PpuMode::OamSearch);
        ppu.tick(79);
        assert_eq!(ppu.mode(), PpuMode::OamSearch);

        // Mode 3 (Drawing) starts at dot 80
        ppu.tick(1);
        assert_eq!(ppu.mode(), PpuMode::Drawing);

        // Drawing until dot 251
        ppu.tick(171);
        assert_eq!(ppu.mode(), PpuMode::Drawing);

        // Mode 0 (HBlank) starts at dot 252
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

        // Mode stays VBlank for the whole 10-line VBlank region
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
}
