//! The four PPU pipeline states and their STAT-register encoding.

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum PpuMode {
    HBlank,
    VBlank,
    OamSearch,
    Drawing,
}

impl PpuMode {
    /// Encoding for the low two bits of the STAT register (0xFF41).
    pub fn stat_bits(self) -> u8 {
        match self {
            PpuMode::HBlank => 0b00,
            PpuMode::VBlank => 0b01,
            PpuMode::OamSearch => 0b10,
            PpuMode::Drawing => 0b11,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stat_bits_match_hardware_encoding() {
        assert_eq!(PpuMode::HBlank.stat_bits(), 0b00);
        assert_eq!(PpuMode::VBlank.stat_bits(), 0b01);
        assert_eq!(PpuMode::OamSearch.stat_bits(), 0b10);
        assert_eq!(PpuMode::Drawing.stat_bits(), 0b11);
    }
}
