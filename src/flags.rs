//! The CPU's F register, packed into a single byte.
//!
//! Layout follows the SM83 convention: bit 7 is Z (zero), 6 is N
//! (subtract), 5 is H (half-carry), 4 is C (carry). The low nibble
//! is always read as zero — the setters here keep that invariant
//! even if a caller stuffs garbage into it via [`Flags::from_bits`].

#[derive(Debug, Copy, Clone, Default, PartialEq, Eq)]
pub struct Flags(u8);

impl Flags {
    const Z: u8 = 0b1000_0000;
    const N: u8 = 0b0100_0000;
    const H: u8 = 0b0010_0000;
    const C: u8 = 0b0001_0000;

    pub fn z(self) -> bool {
        self.0 & Self::Z != 0
    }

    pub fn n(self) -> bool {
        self.0 & Self::N != 0
    }

    pub fn h(self) -> bool {
        self.0 & Self::H != 0
    }

    pub fn c(self) -> bool {
        self.0 & Self::C != 0
    }

    pub fn set_z(&mut self, v: bool) {
        self.set(Self::Z, v);
    }

    pub fn set_n(&mut self, v: bool) {
        self.set(Self::N, v);
    }

    pub fn set_h(&mut self, v: bool) {
        self.set(Self::H, v);
    }

    pub fn set_c(&mut self, v: bool) {
        self.set(Self::C, v);
    }

    fn set(&mut self, mask: u8, v: bool) {
        if v {
            self.0 |= mask;
        } else {
            self.0 &= !mask;
        }

        self.0 &= 0xf0;
    }

    pub fn bits(self) -> u8 {
        self.0
    }

    pub fn from_bits(bits: u8) -> Self {
        Self(bits & 0xf0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_all_clear() {
        let f = Flags::default();
        assert_eq!(f.bits(), 0x00);
        assert!(!f.z());
        assert!(!f.n());
        assert!(!f.h());
        assert!(!f.c());
    }

    #[test]
    fn each_setter_toggles_its_bit() {
        let mut f = Flags::default();

        f.set_z(true);
        assert!(f.z());
        assert_eq!(f.bits(), 0b1000_0000);

        f.set_n(true);
        assert_eq!(f.bits(), 0b1100_0000);

        f.set_h(true);
        assert_eq!(f.bits(), 0b1110_0000);

        f.set_c(true);
        assert_eq!(f.bits(), 0b1111_0000);

        f.set_z(false);
        assert!(!f.z());
        assert_eq!(f.bits(), 0b0111_0000);
    }

    #[test]
    fn from_bits_masks_lower_nibble() {
        assert_eq!(Flags::from_bits(0xff).bits(), 0xf0);
        assert_eq!(Flags::from_bits(0xa5).bits(), 0xa0);
        assert_eq!(Flags::from_bits(0x0f).bits(), 0x00);
    }

    #[test]
    fn setters_preserve_lower_nibble_invariant() {
        // even if from_bits is called with garbage, every setter must keep low nibble at zero
        let mut f = Flags::from_bits(0xff);
        assert_eq!(f.bits(), 0xf0);

        f.set_c(false);
        assert_eq!(f.bits() & 0x0f, 0x00);

        f.set_h(false);
        f.set_z(true);
        f.set_n(true);
        assert_eq!(f.bits() & 0x0f, 0x00);
    }
}
