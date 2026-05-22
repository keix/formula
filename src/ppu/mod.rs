mod framebuffer;
mod mode;
mod registers;

pub use framebuffer::{Framebuffer, HEIGHT, WIDTH};
pub use mode::PpuMode;
pub use registers::Registers;

pub struct Ppu {
    pub regs: Registers,
    framebuffer: Framebuffer,
    mode: PpuMode,
    dots: u16,
}

impl Ppu {
    pub fn new() -> Self {
        Self {
            regs: Registers::new(),
            framebuffer: Framebuffer::new(),
            mode: PpuMode::OamSearch,
            dots: 0,
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
            0xff40 => self.regs.lcdc = value,
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

    /// Advance the PPU by `cycles` dots and return any IF bits to set.
    pub fn tick(&mut self, cycles: u32) -> u8 {
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
        assert_eq!(ppu.regs.ly, 1);
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

        assert_eq!(ppu.regs.ly, 144);
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

        assert_eq!(ppu.regs.ly, 0);
        assert_eq!(ppu.mode(), PpuMode::OamSearch);
    }

    #[test]
    fn tick_returns_vblank_bit_once_per_frame() {
        let mut ppu = Ppu::new();

        assert_eq!(ppu.tick(456 * 144 - 1), 0);
        assert_eq!(ppu.tick(1), 0x01);
        assert_eq!(ppu.tick(456 * 10 - 1), 0);
        assert_eq!(ppu.tick(1), 0);
    }

    #[test]
    fn stat_read_carries_mode_bits() {
        let mut ppu = Ppu::new();
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
        let mut ppu = Ppu::new();
        ppu.write(0xff45, 0);
        assert_eq!(ppu.read(0xff41) & 0b100, 0b100);

        ppu.tick(456);
        assert_eq!(ppu.read(0xff41) & 0b100, 0);

        ppu.write(0xff45, 1);
        assert_eq!(ppu.read(0xff41) & 0b100, 0b100);
    }

    #[test]
    fn stat_write_only_touches_interrupt_enable_bits() {
        let mut ppu = Ppu::new();
        ppu.write(0xff41, 0xff);

        let read = ppu.read(0xff41);
        assert_eq!(read & 0b1111_1000, 0b1111_1000);
    }

    #[test]
    fn write_to_ly_resets_to_zero() {
        let mut ppu = Ppu::new();
        ppu.tick(456 * 5);
        assert_eq!(ppu.regs.ly, 5);

        ppu.write(0xff44, 42);
        assert_eq!(ppu.regs.ly, 0);
    }
}
