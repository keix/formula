//! DMG APU register file and coarse frame-sequencer state.
//!
//! This models the CPU-visible behavior needed by the early Blargg sound
//! ROMs: register read masks, NR52 power control, wave RAM, channel status
//! bits, and the 256 Hz length counters driven by the 512 Hz frame
//! sequencer. Audio generation, envelopes, sweep, and wave playback timing
//! are layered on later.

const REG_START: u16 = 0xff10;
const REG_END: u16 = 0xff26;
const WAVE_START: u16 = 0xff30;
const WAVE_END: u16 = 0xff3f;

const NR10: u16 = 0xff10;
const NR11: u16 = 0xff11;
const NR12: u16 = 0xff12;
const NR14: u16 = 0xff14;
const NR21: u16 = 0xff16;
const NR22: u16 = 0xff17;
const NR24: u16 = 0xff19;
const NR30: u16 = 0xff1a;
const NR31: u16 = 0xff1b;
const NR34: u16 = 0xff1e;
const NR41: u16 = 0xff20;
const NR42: u16 = 0xff21;
const NR44: u16 = 0xff23;
const NR50: u16 = 0xff24;
const NR51: u16 = 0xff25;
const NR52: u16 = 0xff26;

// Readback masks from Blargg's dmg_sound 01-registers test.
const READ_MASKS: [u8; (REG_END - REG_START + 1) as usize] = [
    0x80, 0x3f, 0x00, 0xff, 0xbf, // FF10-FF14
    0xff, 0x3f, 0x00, 0xff, 0xbf, // FF15-FF19
    0x7f, 0xff, 0x9f, 0xff, 0xbf, // FF1A-FF1E
    0xff, 0xff, 0x00, 0x00, 0xbf, // FF1F-FF23
    0x00, 0x00, 0x70,             // FF24-FF26
];

const FRAME_SEQ_PERIOD_T: u16 = 8192;

#[derive(Clone, Copy)]
struct Channel {
    enabled: bool,
    dac_enabled: bool,
    length_enabled: bool,
    length_counter: u16,
    max_length: u16,
}

impl Channel {
    fn new(max_length: u16) -> Self {
        Self {
            enabled: false,
            dac_enabled: false,
            length_enabled: false,
            length_counter: 0,
            max_length,
        }
    }

    fn load_length(&mut self, raw: u8, mask: u8) {
        self.length_counter = self.max_length - u16::from(raw & mask);
    }

    fn trigger(&mut self) {
        if self.length_counter == 0 {
            self.length_counter = self.max_length;
        }
        self.enabled = self.dac_enabled;
    }

    fn clock_length(&mut self) {
        if !self.length_enabled || self.length_counter == 0 {
            return;
        }
        self.length_counter -= 1;
        if self.length_counter == 0 {
            self.enabled = false;
        }
    }
}

pub struct Apu {
    regs: [u8; READ_MASKS.len()],
    wave_ram: [u8; 0x10],
    powered: bool,
    frame_seq_div: u16,
    frame_seq_step: u8,
    ch1: Channel,
    ch2: Channel,
    ch3: Channel,
    ch4: Channel,
}

impl Apu {
    pub fn new() -> Self {
        Self {
            regs: [0; READ_MASKS.len()],
            wave_ram: [0; 0x10],
            powered: false,
            frame_seq_div: 0,
            frame_seq_step: 0,
            ch1: Channel::new(64),
            ch2: Channel::new(64),
            ch3: Channel::new(256),
            ch4: Channel::new(64),
        }
    }

    pub fn read(&self, addr: u16) -> u8 {
        match addr {
            REG_START..=REG_END => {
                let idx = (addr - REG_START) as usize;
                let mask = READ_MASKS[idx];
                if addr == NR52 {
                    let power = if self.powered { 0x80 } else { 0x00 };
                    let status = (self.ch1.enabled as u8)
                        | ((self.ch2.enabled as u8) << 1)
                        | ((self.ch3.enabled as u8) << 2)
                        | ((self.ch4.enabled as u8) << 3);
                    power | status | mask
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
                    self.power_off();
                } else {
                    self.power_on();
                }
            }
            REG_START..=REG_END => {
                if !self.powered {
                    return;
                }
                self.write_powered(addr, value);
            }
            WAVE_START..=WAVE_END => {
                self.wave_ram[(addr - WAVE_START) as usize] = value;
            }
            _ => {}
        }
    }

    pub fn tick(&mut self, cycles: u8) {
        if !self.powered {
            return;
        }

        self.frame_seq_div += u16::from(cycles);
        while self.frame_seq_div >= FRAME_SEQ_PERIOD_T {
            self.frame_seq_div -= FRAME_SEQ_PERIOD_T;
            self.clock_frame_sequencer();
        }
    }

    fn power_off(&mut self) {
        self.powered = false;
        self.regs = [0; READ_MASKS.len()];
        self.frame_seq_div = 0;
        self.frame_seq_step = 0;
        self.ch1 = Channel::new(64);
        self.ch2 = Channel::new(64);
        self.ch3 = Channel::new(256);
        self.ch4 = Channel::new(64);
    }

    fn power_on(&mut self) {
        self.powered = true;
    }

    fn reg_index(addr: u16) -> usize {
        (addr - REG_START) as usize
    }

    fn write_reg_raw(&mut self, addr: u16, value: u8) {
        let idx = Self::reg_index(addr);
        self.regs[idx] = value & !READ_MASKS[idx];
    }

    fn write_powered(&mut self, addr: u16, value: u8) {
        match addr {
            NR10 | NR50 | NR51 => self.write_reg_raw(addr, value),
            NR11 => {
                self.write_reg_raw(addr, value);
                self.ch1.load_length(value, 0x3f);
            }
            NR12 => {
                self.write_reg_raw(addr, value);
                self.ch1.dac_enabled = value & 0xf8 != 0;
                if !self.ch1.dac_enabled {
                    self.ch1.enabled = false;
                }
            }
            NR14 => {
                self.write_reg_raw(addr, value);
                self.ch1.length_enabled = value & 0x40 != 0;
                if value & 0x80 != 0 {
                    self.ch1.trigger();
                }
            }
            NR21 => {
                self.write_reg_raw(addr, value);
                self.ch2.load_length(value, 0x3f);
            }
            NR22 => {
                self.write_reg_raw(addr, value);
                self.ch2.dac_enabled = value & 0xf8 != 0;
                if !self.ch2.dac_enabled {
                    self.ch2.enabled = false;
                }
            }
            NR24 => {
                self.write_reg_raw(addr, value);
                self.ch2.length_enabled = value & 0x40 != 0;
                if value & 0x80 != 0 {
                    self.ch2.trigger();
                }
            }
            NR30 => {
                self.write_reg_raw(addr, value);
                self.ch3.dac_enabled = value & 0x80 != 0;
                if !self.ch3.dac_enabled {
                    self.ch3.enabled = false;
                }
            }
            NR31 => {
                self.write_reg_raw(addr, value);
                self.ch3.load_length(value, 0xff);
            }
            NR34 => {
                self.write_reg_raw(addr, value);
                self.ch3.length_enabled = value & 0x40 != 0;
                if value & 0x80 != 0 {
                    self.ch3.trigger();
                }
            }
            NR41 => {
                self.write_reg_raw(addr, value);
                self.ch4.load_length(value, 0x3f);
            }
            NR42 => {
                self.write_reg_raw(addr, value);
                self.ch4.dac_enabled = value & 0xf8 != 0;
                if !self.ch4.dac_enabled {
                    self.ch4.enabled = false;
                }
            }
            NR44 => {
                self.write_reg_raw(addr, value);
                self.ch4.length_enabled = value & 0x40 != 0;
                if value & 0x80 != 0 {
                    self.ch4.trigger();
                }
            }
            _ => self.write_reg_raw(addr, value),
        }
    }

    fn clock_frame_sequencer(&mut self) {
        self.frame_seq_step = (self.frame_seq_step + 1) & 0x07;
        if self.frame_seq_step & 1 == 0 {
            self.ch1.clock_length();
            self.ch2.clock_length();
            self.ch3.clock_length();
            self.ch4.clock_length();
        }
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

        apu.write(NR10, 0x12);
        apu.write(NR11, 0x34);
        apu.write(NR12, 0x56);
        apu.write(NR50, 0x78);
        apu.write(NR51, 0x9a);

        assert_eq!(apu.read(NR10), 0x92);
        assert_eq!(apu.read(NR11), 0x3f);
        assert_eq!(apu.read(NR12), 0x56);
        assert_eq!(apu.read(NR50), 0x78);
        assert_eq!(apu.read(NR51), 0x9a);
        assert_eq!(apu.read(NR52), 0xf0);
    }

    #[test]
    fn powering_off_clears_registers_but_keeps_wave_ram() {
        let mut apu = Apu::new();
        apu.write(NR52, 0x80);
        apu.write(NR12, 0xff);
        apu.write(0xff30, 0x37);

        apu.write(NR52, 0x00);

        assert_eq!(apu.read(NR12), 0x00);
        assert_eq!(apu.read(NR52), 0x70);
        assert_eq!(apu.read(0xff30), 0x37);
    }

    #[test]
    fn writes_are_ignored_while_powered_off_except_wave_ram() {
        let mut apu = Apu::new();

        apu.write(NR12, 0xff);
        apu.write(NR50, 0xff);
        apu.write(0xff30, 0x44);

        assert_eq!(apu.read(NR12), 0x00);
        assert_eq!(apu.read(NR50), 0x00);
        assert_eq!(apu.read(0xff30), 0x44);
    }

    #[test]
    fn trigger_sets_status_and_length_clock_eventually_disables_channel() {
        let mut apu = Apu::new();
        apu.write(NR52, 0x80);
        apu.write(NR22, 0x08); // DAC on
        apu.write(NR21, 0x3e); // length = 2
        apu.write(NR24, 0xc0); // trigger + enable length

        assert_eq!(apu.read(NR52) & 0x02, 0x02);

        for _ in 0..(FRAME_SEQ_PERIOD_T / 4 * 4) {
            apu.tick(4);
        }

        assert_eq!(apu.read(NR52) & 0x02, 0x00);
    }

    #[test]
    fn disabling_dac_drops_channel_status_immediately() {
        let mut apu = Apu::new();
        apu.write(NR52, 0x80);
        apu.write(NR22, 0x08);
        apu.write(NR21, 0x3f);
        apu.write(NR24, 0x80);
        assert_eq!(apu.read(NR52) & 0x02, 0x02);

        apu.write(NR22, 0x07);

        assert_eq!(apu.read(NR52) & 0x02, 0x00);
    }
}
