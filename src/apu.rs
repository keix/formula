//! DMG APU register file.
//!
//! This is the minimal "CPU-visible" subset needed to stop treating the
//! sound window as anonymous IO bytes: register read masks, NR52 power
//! control, and wave RAM storage. Timing, channel state, and audio mixing
//! are layered on later.

const REG_START: u16 = 0xff10;
const REG_END: u16 = 0xff26;
const WAVE_START: u16 = 0xff30;
const WAVE_END: u16 = 0xff3f;
const NR52: u16 = 0xff26;

// Readback masks from Blargg's dmg_sound 01-registers test.
const READ_MASKS: [u8; (REG_END - REG_START + 1) as usize] = [
    0x80, 0x3f, 0x00, 0xff, 0xbf, // FF10-FF14
    0xff, 0x3f, 0x00, 0xff, 0xbf, // FF15-FF19
    0x7f, 0xff, 0x9f, 0xff, 0xbf, // FF1A-FF1E
    0xff, 0xff, 0x00, 0x00, 0xbf, // FF1F-FF23
    0x00, 0x00, 0x70, // FF24-FF26
];

pub struct Apu {
    regs: [u8; READ_MASKS.len()],
    wave_ram: [u8; 0x10],
    powered: bool,
}

impl Apu {
    pub fn new() -> Self {
        Self {
            regs: [0; READ_MASKS.len()],
            wave_ram: [0; 0x10],
            powered: false,
        }
    }

    pub fn read(&self, addr: u16) -> u8 {
        match addr {
            REG_START..=REG_END => {
                let idx = (addr - REG_START) as usize;
                let mask = READ_MASKS[idx];
                if addr == NR52 {
                    let power = if self.powered { 0x80 } else { 0x00 };
                    power | mask
                } else if self.powered {
                    self.regs[idx] | mask
                } else {
                    mask
                }
            }
            WAVE_START..=WAVE_END => self.wave_ram[(addr - WAVE_START) as usize],
            _ => 0xff,
        }
    }

    pub fn write(&mut self, addr: u16, value: u8) {
        match addr {
            NR52 => {
                if value & 0x80 == 0 {
                    self.powered = false;
                    self.regs = [0; READ_MASKS.len()];
                } else {
                    self.powered = true;
                }
            }
            REG_START..=REG_END => {
                if self.powered {
                    let idx = (addr - REG_START) as usize;
                    self.regs[idx] = value & !READ_MASKS[idx];
                }
            }
            WAVE_START..=WAVE_END => {
                self.wave_ram[(addr - WAVE_START) as usize] = value;
            }
            _ => {}
        }
    }

    pub fn tick(&mut self, cycles: u8) {
        let _ = cycles;
    }
}

impl Default for Apu {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn powered_on_registers_read_back_with_hardware_masks() {
        let mut apu = Apu::new();
        apu.write(NR52, 0x80);

        apu.write(0xff10, 0x12);
        apu.write(0xff11, 0x34);
        apu.write(0xff12, 0x56);
        apu.write(0xff24, 0x78);
        apu.write(0xff25, 0x9a);

        assert_eq!(apu.read(0xff10), 0x92);
        assert_eq!(apu.read(0xff11), 0x3f);
        assert_eq!(apu.read(0xff12), 0x56);
        assert_eq!(apu.read(0xff24), 0x78);
        assert_eq!(apu.read(0xff25), 0x9a);
        assert_eq!(apu.read(NR52), 0xf0);
    }

    #[test]
    fn powering_off_clears_registers_but_keeps_wave_ram() {
        let mut apu = Apu::new();
        apu.write(NR52, 0x80);
        apu.write(0xff12, 0xff);
        apu.write(0xff30, 0x37);

        apu.write(NR52, 0x00);

        assert_eq!(apu.read(0xff12), 0x00);
        assert_eq!(apu.read(NR52), 0x70);
        assert_eq!(apu.read(0xff30), 0x37);
    }

    #[test]
    fn writes_are_ignored_while_powered_off_except_wave_ram() {
        let mut apu = Apu::new();

        apu.write(0xff12, 0xff);
        apu.write(0xff24, 0xff);
        apu.write(0xff30, 0x44);

        assert_eq!(apu.read(0xff12), 0x00);
        assert_eq!(apu.read(0xff24), 0x00);
        assert_eq!(apu.read(0xff30), 0x44);
    }
}
