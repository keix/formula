//! 160x144 framebuffer of 2-bit DMG shades.
//!
//! Each pixel is one of {0, 1, 2, 3}; the host-side palette mapping
//! lives in the binary. The PPU writes into this every scanline and
//! the binary blits it to the window at VBlank.

pub const WIDTH: usize = 160;
pub const HEIGHT: usize = 144;

pub struct Framebuffer {
    pixels: [u8; WIDTH * HEIGHT],
}

impl Framebuffer {
    pub fn new() -> Self {
        Self {
            pixels: [0; WIDTH * HEIGHT],
        }
    }

    pub fn as_slice(&self) -> &[u8] {
        &self.pixels
    }

    pub fn pixel(&self, x: usize, y: usize) -> u8 {
        self.pixels[y * WIDTH + x]
    }

    pub fn set_pixel(&mut self, x: usize, y: usize, shade: u8) {
        self.pixels[y * WIDTH + x] = shade;
    }
}

impl Default for Framebuffer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn framebuffer_is_160_by_144() {
        let fb = Framebuffer::new();
        assert_eq!(fb.as_slice().len(), 160 * 144);
        assert!(fb.as_slice().iter().all(|&p| p == 0));
    }
}
