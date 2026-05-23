//! DMG APU register file, coarse frame sequencer, and a first-pass mixer.
//!
//! This models the CPU-visible behavior needed by the early Blargg sound
//! ROMs: register read masks, NR52 power control, wave RAM, channel status
//! bits, and the 256 Hz length counters driven by the 512 Hz frame
//! sequencer. It also exposes a pragmatic first audio path: basic square,
//! wave, and noise generation using the register file's current state,
//! mixed down to interleaved stereo `i16` samples. Envelope/sweep timing
//! is still incomplete, but commercial ROMs can now produce audible output.

const REG_START: u16 = 0xff10;
const REG_END: u16 = 0xff26;
const WAVE_START: u16 = 0xff30;
const WAVE_END: u16 = 0xff3f;

const NR10: u16 = 0xff10;
const NR11: u16 = 0xff11;
const NR13: u16 = 0xff13;
const NR12: u16 = 0xff12;
const NR14: u16 = 0xff14;
const NR21: u16 = 0xff16;
const NR23: u16 = 0xff18;
const NR22: u16 = 0xff17;
const NR24: u16 = 0xff19;
const NR30: u16 = 0xff1a;
const NR31: u16 = 0xff1b;
const NR32: u16 = 0xff1c;
const NR33: u16 = 0xff1d;
const NR34: u16 = 0xff1e;
const NR41: u16 = 0xff20;
const NR43: u16 = 0xff22;
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
    0x00, 0x00, 0x70, // FF24-FF26
];

const FRAME_SEQ_PERIOD_T: u16 = 8192;
const CPU_CLOCK_HZ: u32 = 4_194_304;
const OUTPUT_SAMPLE_RATE: u32 = 48_000;
const DC_BLOCK_COEFF: f32 = 0.996;
const LOW_PASS_COEFF: f32 = 0.82;
const WAVE_TRIGGER_DELAY_T: u16 = 3;
const DUTY_PATTERNS: [u8; 4] = [0b0000_0001, 0b1000_0001, 0b1000_0111, 0b0111_1110];
const NOISE_DIVISORS: [u16; 8] = [8, 16, 32, 48, 64, 80, 96, 112];

#[derive(Clone, Copy)]
struct Channel {
    enabled: bool,
    dac_enabled: bool,
    length_enabled: bool,
    length_counter: u16,
    max_length: u16,
    period_timer: u16,
    phase: u8,
    volume: u8,
    initial_volume: u8,
    envelope_period: u8,
    envelope_timer: u8,
    envelope_increase: bool,
    lfsr: u16,
}

impl Channel {
    fn new(max_length: u16) -> Self {
        Self {
            enabled: false,
            dac_enabled: false,
            length_enabled: false,
            length_counter: 0,
            max_length,
            period_timer: 0,
            phase: 0,
            volume: 0,
            initial_volume: 0,
            envelope_period: 0,
            envelope_timer: 0,
            envelope_increase: false,
            lfsr: 0x7fff,
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
        self.phase = 0;
        self.period_timer = 0;
        self.volume = self.initial_volume;
        self.envelope_timer = self.envelope_period.max(1);
        self.lfsr = 0x7fff;
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

    fn clock_envelope(&mut self) {
        if !self.enabled || self.envelope_period == 0 {
            return;
        }

        if self.envelope_timer > 0 {
            self.envelope_timer -= 1;
        }
        if self.envelope_timer != 0 {
            return;
        }

        self.envelope_timer = self.envelope_period;
        if self.envelope_increase {
            if self.volume < 15 {
                self.volume += 1;
            }
        } else if self.volume > 0 {
            self.volume -= 1;
        }
    }
}

#[derive(Clone, Copy)]
struct SweepState {
    enabled: bool,
    timer: u8,
    period: u8,
    shift: u8,
    negate: bool,
    negate_used: bool,
    shadow_freq: u16,
}

impl SweepState {
    fn new() -> Self {
        Self {
            enabled: false,
            timer: 8,
            period: 0,
            shift: 0,
            negate: false,
            negate_used: false,
            shadow_freq: 0,
        }
    }
}

fn sweep_period(period: u8) -> u8 {
    if period == 0 {
        8
    } else {
        period
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
    sweep: SweepState,
    wave_index: u8,
    wave_byte: u8,
    wave_output: u8,
    sample_phase: u32,
    sample_buffer: Vec<i16>,
    hp_prev_in_l: f32,
    hp_prev_in_r: f32,
    hp_prev_out_l: f32,
    hp_prev_out_r: f32,
    lp_prev_l: f32,
    lp_prev_r: f32,
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
            sweep: SweepState::new(),
            wave_index: 0,
            wave_byte: 0,
            wave_output: 0,
            sample_phase: 0,
            sample_buffer: Vec::new(),
            hp_prev_in_l: 0.0,
            hp_prev_in_r: 0.0,
            hp_prev_out_l: 0.0,
            hp_prev_out_r: 0.0,
            lp_prev_l: 0.0,
            lp_prev_r: 0.0,
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

        for _ in 0..cycles {
            self.advance_channels(1);

            self.frame_seq_div += 1;
            if self.frame_seq_div >= FRAME_SEQ_PERIOD_T {
                self.frame_seq_div -= FRAME_SEQ_PERIOD_T;
                self.clock_frame_sequencer();
            }

            self.sample_phase += OUTPUT_SAMPLE_RATE;
            while self.sample_phase >= CPU_CLOCK_HZ {
                self.sample_phase -= CPU_CLOCK_HZ;
                let (left, right) = self.render_sample();
                self.sample_buffer.push(left);
                self.sample_buffer.push(right);
            }
        }
    }

    pub fn sample_rate(&self) -> u32 {
        OUTPUT_SAMPLE_RATE
    }

    pub fn drain_samples(&mut self) -> Vec<i16> {
        std::mem::take(&mut self.sample_buffer)
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
        self.sweep = SweepState::new();
        self.wave_index = 0;
        self.wave_byte = 0;
        self.wave_output = 0;
        self.sample_phase = 0;
        self.hp_prev_in_l = 0.0;
        self.hp_prev_in_r = 0.0;
        self.hp_prev_out_l = 0.0;
        self.hp_prev_out_r = 0.0;
        self.lp_prev_l = 0.0;
        self.lp_prev_r = 0.0;
    }

    fn power_on(&mut self) {
        self.powered = true;
    }

    fn reg_index(addr: u16) -> usize {
        (addr - REG_START) as usize
    }

    fn write_reg_raw(&mut self, addr: u16, value: u8) {
        let idx = Self::reg_index(addr);
        self.regs[idx] = value;
    }

    fn write_powered(&mut self, addr: u16, value: u8) {
        match addr {
            NR10 => {
                let old_negate = self.regs[Self::reg_index(NR10)] & 0x08 != 0;
                self.write_reg_raw(addr, value);
                let new_negate = value & 0x08 != 0;
                if old_negate && !new_negate && self.sweep.negate_used {
                    self.ch1.enabled = false;
                }
                self.sweep.negate = new_negate;
                self.sweep.period = (value >> 4) & 0x07;
                self.sweep.shift = value & 0x07;
            }
            NR50 | NR51 => self.write_reg_raw(addr, value),
            NR11 => {
                self.write_reg_raw(addr, value);
                self.ch1.load_length(value, 0x3f);
            }
            NR12 => {
                self.write_reg_raw(addr, value);
                self.ch1.dac_enabled = value & 0xf8 != 0;
                self.ch1.initial_volume = value >> 4;
                self.ch1.volume = self.ch1.initial_volume;
                self.ch1.envelope_increase = value & 0x08 != 0;
                self.ch1.envelope_period = value & 0x07;
                self.ch1.envelope_timer = self.ch1.envelope_period.max(1);
                if !self.ch1.dac_enabled {
                    self.ch1.enabled = false;
                }
            }
            NR14 => {
                self.write_reg_raw(addr, value);
                self.ch1.length_enabled = value & 0x40 != 0;
                if value & 0x80 != 0 {
                    self.ch1.trigger();
                    self.trigger_sweep();
                }
            }
            NR21 => {
                self.write_reg_raw(addr, value);
                self.ch2.load_length(value, 0x3f);
            }
            NR22 => {
                self.write_reg_raw(addr, value);
                self.ch2.dac_enabled = value & 0xf8 != 0;
                self.ch2.initial_volume = value >> 4;
                self.ch2.volume = self.ch2.initial_volume;
                self.ch2.envelope_increase = value & 0x08 != 0;
                self.ch2.envelope_period = value & 0x07;
                self.ch2.envelope_timer = self.ch2.envelope_period.max(1);
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
                    self.wave_output = 0;
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
                    self.trigger_wave();
                }
            }
            NR41 => {
                self.write_reg_raw(addr, value);
                self.ch4.load_length(value, 0x3f);
            }
            NR42 => {
                self.write_reg_raw(addr, value);
                self.ch4.dac_enabled = value & 0xf8 != 0;
                self.ch4.initial_volume = value >> 4;
                self.ch4.volume = self.ch4.initial_volume;
                self.ch4.envelope_increase = value & 0x08 != 0;
                self.ch4.envelope_period = value & 0x07;
                self.ch4.envelope_timer = self.ch4.envelope_period.max(1);
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
        if self.frame_seq_step == 2 || self.frame_seq_step == 6 {
            self.clock_sweep();
        }
        if self.frame_seq_step == 7 {
            self.ch1.clock_envelope();
            self.ch2.clock_envelope();
            self.ch4.clock_envelope();
        }
    }

    fn advance_channels(&mut self, cycles: u8) {
        self.advance_square(1, cycles);
        self.advance_square(2, cycles);
        self.advance_wave(cycles);
        self.advance_noise(cycles);
    }

    fn advance_square(&mut self, channel: u8, cycles: u8) {
        let (ch, lo_reg, hi_reg) = match channel {
            1 => (&mut self.ch1, NR13, NR14),
            2 => (&mut self.ch2, NR23, NR24),
            _ => unreachable!(),
        };
        if !ch.enabled {
            return;
        }

        let period = Self::square_period(
            self.regs[Self::reg_index(lo_reg)],
            self.regs[Self::reg_index(hi_reg)],
        );
        Self::advance_periodic_channel(ch, cycles, period);
    }

    fn advance_wave(&mut self, cycles: u8) {
        if !self.ch3.enabled {
            return;
        }
        let period = Self::wave_period(
            self.regs[Self::reg_index(NR33)],
            self.regs[Self::reg_index(NR34)],
        );
        let mut remaining = u16::from(cycles);
        while remaining > 0 {
            if remaining > self.ch3.period_timer {
                remaining -= self.ch3.period_timer + 1;
                self.ch3.period_timer = period.saturating_sub(1);
                self.wave_index = (self.wave_index + 1) & 0x1f;
                self.wave_byte = self.wave_ram[(self.wave_index >> 1) as usize];
                self.wave_output = if self.wave_index & 1 == 0 {
                    self.wave_byte >> 4
                } else {
                    self.wave_byte & 0x0f
                };
                self.ch3.phase = self.wave_index;
            } else {
                self.ch3.period_timer -= remaining;
                remaining = 0;
            }
        }
    }

    fn advance_noise(&mut self, cycles: u8) {
        if !self.ch4.enabled {
            return;
        }

        let period = Self::noise_period(self.regs[Self::reg_index(NR43)]);
        let mut remaining = u16::from(cycles);
        while remaining > 0 {
            if self.ch4.period_timer == 0 {
                self.ch4.period_timer = period;
            }
            let step = remaining.min(self.ch4.period_timer);
            self.ch4.period_timer -= step;
            remaining -= step;
            if self.ch4.period_timer == 0 {
                let xor = ((self.ch4.lfsr & 0x01) ^ ((self.ch4.lfsr >> 1) & 0x01)) as u16;
                self.ch4.lfsr = (self.ch4.lfsr >> 1) | (xor << 14);
                if self.regs[Self::reg_index(NR43)] & 0x08 != 0 {
                    self.ch4.lfsr = (self.ch4.lfsr & !(1 << 6)) | (xor << 6);
                }
            }
        }
    }

    fn advance_periodic_channel(ch: &mut Channel, cycles: u8, period: u16) {
        let mut remaining = u16::from(cycles);
        while remaining > 0 {
            if ch.period_timer == 0 {
                ch.period_timer = period;
            }
            let step = remaining.min(ch.period_timer);
            ch.period_timer -= step;
            remaining -= step;
            if ch.period_timer == 0 {
                ch.phase = ch.phase.wrapping_add(1);
            }
        }
    }

    fn square_period(lo: u8, hi: u8) -> u16 {
        let freq = (((hi as u16) & 0x07) << 8) | lo as u16;
        ((2048 - freq).max(1)) * 4
    }

    fn wave_period(lo: u8, hi: u8) -> u16 {
        let freq = (((hi as u16) & 0x07) << 8) | lo as u16;
        ((2048 - freq).max(1)) * 2
    }

    fn noise_period(nr43: u8) -> u16 {
        let shift = nr43 >> 4;
        let divisor = NOISE_DIVISORS[(nr43 & 0x07) as usize];
        divisor << shift
    }

    fn trigger_sweep(&mut self) {
        let nr10 = self.regs[Self::reg_index(NR10)];
        self.sweep.period = (nr10 >> 4) & 0x07;
        self.sweep.shift = nr10 & 0x07;
        self.sweep.negate = nr10 & 0x08 != 0;
        self.sweep.negate_used = false;
        self.sweep.timer = sweep_period(self.sweep.period);
        self.sweep.shadow_freq = self.square_freq(
            self.regs[Self::reg_index(NR13)],
            self.regs[Self::reg_index(NR14)],
        );
        self.sweep.enabled = self.sweep.period != 0 || self.sweep.shift != 0;
        if self.sweep.shift != 0 {
            let _ = self.calculate_sweep_frequency(false);
        }
    }

    fn trigger_wave(&mut self) {
        self.wave_index = 0;
        self.wave_byte = self.wave_ram[0];
        self.wave_output = self.wave_byte >> 4;
        self.ch3.phase = 0;
        self.ch3.period_timer = Self::wave_period(
            self.regs[Self::reg_index(NR33)],
            self.regs[Self::reg_index(NR34)],
        ) + WAVE_TRIGGER_DELAY_T;
    }

    fn clock_sweep(&mut self) {
        if self.sweep.timer > 0 {
            self.sweep.timer -= 1;
        }
        if self.sweep.timer != 0 {
            return;
        }

        self.sweep.timer = sweep_period(self.sweep.period);
        if !self.sweep.enabled || self.sweep.period == 0 {
            return;
        }
        if let Some(new_freq) = self.calculate_sweep_frequency(true) {
            if self.sweep.shift != 0 {
                self.sweep.shadow_freq = new_freq;
                self.set_square1_freq(new_freq);
                let _ = self.calculate_sweep_frequency(false);
            }
        }
    }

    fn calculate_sweep_frequency(&mut self, update_negate_used: bool) -> Option<u16> {
        let delta = self.sweep.shadow_freq >> self.sweep.shift;
        let new_freq = if self.sweep.negate {
            if update_negate_used {
                self.sweep.negate_used = true;
            }
            self.sweep.shadow_freq.wrapping_sub(delta)
        } else {
            self.sweep.shadow_freq.saturating_add(delta)
        };

        if new_freq > 0x07ff {
            self.ch1.enabled = false;
            return None;
        }
        Some(new_freq)
    }

    fn square_freq(&self, lo: u8, hi: u8) -> u16 {
        (((hi as u16) & 0x07) << 8) | lo as u16
    }

    fn set_square1_freq(&mut self, freq: u16) {
        let lo_idx = Self::reg_index(NR13);
        let hi_idx = Self::reg_index(NR14);
        self.regs[lo_idx] = (freq & 0xff) as u8;
        self.regs[hi_idx] = (self.regs[hi_idx] & !0x07) | (((freq >> 8) as u8) & 0x07);
        self.ch1.period_timer = 0;
    }

    fn render_sample(&mut self) -> (i16, i16) {
        let nr50 = self.regs[Self::reg_index(NR50)];
        let nr51 = self.regs[Self::reg_index(NR51)];
        let left_volume = ((nr50 >> 4) & 0x07) + 1;
        let right_volume = (nr50 & 0x07) + 1;

        let channel_samples = [
            self.square_sample(&self.ch1, self.regs[Self::reg_index(NR11)]),
            self.square_sample(&self.ch2, self.regs[Self::reg_index(NR21)]),
            self.wave_sample(),
            self.noise_sample(),
        ];

        let mut left = 0.0f32;
        let mut right = 0.0f32;
        for (idx, sample) in channel_samples.into_iter().enumerate() {
            if nr51 & (1 << (idx + 4)) != 0 {
                left += sample;
            }
            if nr51 & (1 << idx) != 0 {
                right += sample;
            }
        }

        let left = left * (left_volume as f32 / 8.0) / 4.5;
        let right = right * (right_volume as f32 / 8.0) / 4.5;
        let left = self.low_pass_left(left);
        let right = self.low_pass_right(right);
        let left = self.high_pass_left(left).clamp(-1.0, 1.0);
        let right = self.high_pass_right(right).clamp(-1.0, 1.0);
        (
            (left * i16::MAX as f32) as i16,
            (right * i16::MAX as f32) as i16,
        )
    }

    fn low_pass_left(&mut self, input: f32) -> f32 {
        self.lp_prev_l += LOW_PASS_COEFF * (input - self.lp_prev_l);
        self.lp_prev_l
    }

    fn low_pass_right(&mut self, input: f32) -> f32 {
        self.lp_prev_r += LOW_PASS_COEFF * (input - self.lp_prev_r);
        self.lp_prev_r
    }

    fn high_pass_left(&mut self, input: f32) -> f32 {
        let output = input - self.hp_prev_in_l + DC_BLOCK_COEFF * self.hp_prev_out_l;
        self.hp_prev_in_l = input;
        self.hp_prev_out_l = output;
        output
    }

    fn high_pass_right(&mut self, input: f32) -> f32 {
        let output = input - self.hp_prev_in_r + DC_BLOCK_COEFF * self.hp_prev_out_r;
        self.hp_prev_in_r = input;
        self.hp_prev_out_r = output;
        output
    }

    fn square_sample(&self, ch: &Channel, duty_reg: u8) -> f32 {
        if !ch.enabled || !ch.dac_enabled || ch.volume == 0 {
            return 0.0;
        }
        let duty = DUTY_PATTERNS[((duty_reg >> 6) & 0x03) as usize];
        let high = (duty >> (7 - (ch.phase & 0x07))) & 0x01 != 0;
        if high {
            ch.volume as f32 / 15.0
        } else {
            0.0
        }
    }

    fn wave_sample(&self) -> f32 {
        if !self.ch3.enabled || !self.ch3.dac_enabled {
            return 0.0;
        }

        let level_code = (self.regs[Self::reg_index(NR32)] >> 5) & 0x03;
        if level_code == 0 {
            return 0.0;
        }

        let level_scale = match level_code {
            1 => 1.0,
            2 => 0.5,
            3 => 0.25,
            _ => 0.0,
        };
        (self.wave_output as f32 / 15.0) * level_scale
    }

    fn noise_sample(&self) -> f32 {
        if !self.ch4.enabled || !self.ch4.dac_enabled || self.ch4.volume == 0 {
            return 0.0;
        }
        if self.ch4.lfsr & 0x01 == 0 {
            self.ch4.volume as f32 / 15.0
        } else {
            0.0
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

    #[test]
    fn ticking_produces_stereo_samples_when_a_channel_is_routed() {
        let mut apu = Apu::new();
        apu.write(NR52, 0x80);
        apu.write(NR50, 0x77);
        apu.write(NR51, 0x11);
        apu.write(NR12, 0xf0);
        apu.write(NR11, 0x80);
        apu.write(NR13, 0xaa);
        apu.write(NR14, 0x87);

        apu.tick(128);
        let samples = apu.drain_samples();

        assert!(!samples.is_empty());
        assert_eq!(samples.len() % 2, 0);
        assert!(samples.iter().any(|&sample| sample != 0));
    }

    #[test]
    fn envelope_clocks_channel_volume_down() {
        let mut apu = Apu::new();
        apu.write(NR52, 0x80);
        apu.write(NR12, 0x51);
        apu.write(NR11, 0x80);
        apu.write(NR14, 0x80);

        assert_eq!(apu.ch1.volume, 5);
        for _ in 0..7 {
            for _ in 0..(FRAME_SEQ_PERIOD_T / 4) {
                apu.tick(4);
            }
        }
        assert_eq!(apu.ch1.volume, 4);
    }

    #[test]
    fn wave_channel_latches_samples_as_it_advances() {
        let mut apu = Apu::new();
        apu.write(NR52, 0x80);
        apu.write(NR30, 0x80);
        apu.write(NR32, 0x20);
        apu.write(0xff30, 0xf1);
        apu.write(0xff31, 0x23);
        apu.write(NR33, 0xff);
        apu.write(NR34, 0x87);

        assert_eq!(apu.wave_output, 0x0f);
        let period = Apu::wave_period(
            apu.regs[Apu::reg_index(NR33)],
            apu.regs[Apu::reg_index(NR34)],
        );
        for _ in 0..WAVE_TRIGGER_DELAY_T {
            apu.tick(1);
        }
        for _ in 0..=period {
            apu.tick(1);
        }
        assert_eq!(apu.wave_output, 0x01);
        for _ in 0..=period {
            apu.tick(1);
        }
        assert_eq!(apu.wave_output, 0x02);
    }

    #[test]
    fn batched_tick_matches_single_cycle_tick_for_audio_sampling() {
        let mut batched = Apu::new();
        let mut stepped = Apu::new();

        for apu in [&mut batched, &mut stepped] {
            apu.write(NR52, 0x80);
            apu.write(NR50, 0x77);
            apu.write(NR51, 0x11);
            apu.write(NR12, 0xf0);
            apu.write(NR11, 0x80);
            apu.write(NR13, 0xaa);
            apu.write(NR14, 0x87);
        }

        batched.tick(16);
        for _ in 0..16 {
            stepped.tick(1);
        }

        assert_eq!(batched.drain_samples(), stepped.drain_samples());
    }

    #[test]
    fn sweep_period_zero_reloads_as_eight() {
        assert_eq!(sweep_period(0), 8);
        assert_eq!(sweep_period(5), 5);
    }

    #[test]
    fn clearing_negate_after_negate_sweep_use_disables_channel1() {
        let mut apu = Apu::new();
        apu.write(NR52, 0x80);
        apu.write(NR12, 0xf0);
        apu.write(NR10, 0x19);
        apu.write(NR13, 0x40);
        apu.write(NR14, 0x83);
        assert_ne!(apu.read(NR52) & 0x01, 0);

        // Force a sweep calculation to observe negate mode once.
        apu.sweep.shadow_freq = 0x140;
        let _ = apu.calculate_sweep_frequency(true);
        assert!(apu.sweep.negate_used);

        apu.write(NR10, 0x11);

        assert_eq!(apu.read(NR52) & 0x01, 0);
    }

    #[test]
    fn write_only_frequency_bits_are_preserved_internally() {
        let mut apu = Apu::new();
        apu.write(NR52, 0x80);
        apu.write(NR13, 0xaa);
        apu.write(NR14, 0x87);
        apu.write(NR33, 0x55);
        apu.write(NR34, 0x83);

        assert_eq!(apu.regs[Apu::reg_index(NR13)], 0xaa);
        assert_eq!(apu.regs[Apu::reg_index(NR14)] & 0x07, 0x07);
        assert_eq!(apu.regs[Apu::reg_index(NR33)], 0x55);
        assert_eq!(apu.regs[Apu::reg_index(NR34)] & 0x07, 0x03);
    }
}
