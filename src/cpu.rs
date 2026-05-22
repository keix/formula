//! SM83 CPU
//!
//! Decodes and executes the DMG instruction set against a [`Bus`].
//! Owns the register file (A/F/BC/DE/HL/SP/PC), the HALT and IME
//! state, and a couple of small latches for the HALT bug and the
//! one-instruction EI delay. Every [`Cpu::step`] returns the T-cycle
//! cost of the instruction it just ran (or 4 for a HALT cycle, or 20
//! when it serviced an interrupt) so the MMU can advance the rest
//! of the system in lockstep.
//!
//! Design notes:
//! - All 256 unprefixed opcodes plus the full CB-prefix block are
//!   handled. The eleven illegal opcodes (0xD3, 0xDB, 0xDD, 0xE3,
//!   0xE4, 0xEB, 0xEC, 0xED, 0xF4, 0xFC, 0xFD) flip `locked` instead
//!   of panicking — matching the hardware quirk where the CPU bus-
//!   stalls forever and giving the binary a clean exit path.
//! - Interrupt servicing is the standard 20 T-cycles (push PC, jump
//!   to vector, clear IF and IME). The HALT bug is modelled by the
//!   `halt_bug` flag: if HALT executes with IME=0 and a pending
//!   interrupt, the next opcode fetch reads the byte at PC twice.
//! - EI takes one instruction to enable IME (`ime_delay`).

use crate::bus::Bus;
use crate::flags::Flags;

pub struct Cpu {
    pub a: u8,
    pub f: Flags,
    pub b: u8,
    pub c: u8,
    pub d: u8,
    pub e: u8,
    pub h: u8,
    pub l: u8,
    pub sp: u16,
    pub pc: u16,
    pub halted: bool,
    pub ime: bool,
    pub halt_bug: bool,
    pub locked: bool,
    ime_delay: u8,
}

impl Cpu {
    pub fn new() -> Self {
        Self {
            a: 0,
            f: Flags::default(),
            b: 0,
            c: 0,
            d: 0,
            e: 0,
            h: 0,
            l: 0,
            sp: 0,
            pc: 0,
            halted: false,
            ime: false,
            halt_bug: false,
            locked: false,
            ime_delay: 0,
        }
    }

    pub fn af(&self) -> u16 {
        u16::from_be_bytes([self.a, self.f.bits()])
    }

    pub fn set_af(&mut self, value: u16) {
        let [hi, lo] = value.to_be_bytes();
        self.a = hi;
        self.f = Flags::from_bits(lo);
    }

    pub fn bc(&self) -> u16 {
        u16::from_be_bytes([self.b, self.c])
    }

    pub fn de(&self) -> u16 {
        u16::from_be_bytes([self.d, self.e])
    }

    pub fn hl(&self) -> u16 {
        u16::from_be_bytes([self.h, self.l])
    }

    pub fn set_bc(&mut self, value: u16) {
        [self.b, self.c] = value.to_be_bytes();
    }

    pub fn set_de(&mut self, value: u16) {
        [self.d, self.e] = value.to_be_bytes();
    }

    pub fn set_hl(&mut self, value: u16) {
        [self.h, self.l] = value.to_be_bytes();
    }

    fn mcycle(&mut self, bus: &mut impl Bus) {
        bus.tick(4);
    }

    fn idle(&mut self, bus: &mut impl Bus) {
        self.mcycle(bus);
    }

    fn read8_timed(&mut self, bus: &mut impl Bus, addr: u16) -> u8 {
        self.mcycle(bus);
        bus.read8_cpu(addr)
    }

    fn write8_timed(&mut self, bus: &mut impl Bus, addr: u16, value: u8) {
        self.mcycle(bus);
        bus.write8_cpu(addr, value);
    }

    fn read8_timed_idu(&mut self, bus: &mut impl Bus, addr: u16, idu_addr: u16) -> u8 {
        self.mcycle(bus);
        bus.read8_cpu_idu(addr, idu_addr)
    }

    fn write8_timed_idu(&mut self, bus: &mut impl Bus, addr: u16, value: u8, idu_addr: u16) {
        self.mcycle(bus);
        bus.write8_cpu_idu(addr, value, idu_addr);
    }

    fn idu_timed(&mut self, bus: &mut impl Bus, addr: u16) {
        self.mcycle(bus);
        bus.idu_glitch_cpu(addr);
    }

    fn fetch8(&mut self, bus: &mut impl Bus) -> u8 {
        let byte = self.read8_timed(bus, self.pc);
        if self.halt_bug {
            self.halt_bug = false;
        } else {
            self.pc = self.pc.wrapping_add(1);
        }
        byte
    }

    fn consume_operand_untimed(&mut self, bus: &impl Bus) -> u8 {
        let byte = bus.read8(self.pc);
        self.pc = self.pc.wrapping_add(1);
        byte
    }

    fn fetch16(&mut self, bus: &mut impl Bus) -> u16 {
        let lo = self.fetch8(bus);
        let hi = self.fetch8(bus);
        u16::from_be_bytes([hi, lo])
    }

    fn push16(&mut self, bus: &mut impl Bus, value: u16) {
        let [hi, lo] = value.to_be_bytes();
        let idu_addr = self.sp;
        self.sp = self.sp.wrapping_sub(1);
        self.write8_timed_idu(bus, self.sp, hi, idu_addr);
        let idu_addr = self.sp;
        self.sp = self.sp.wrapping_sub(1);
        self.write8_timed_idu(bus, self.sp, lo, idu_addr);
    }

    fn pop16(&mut self, bus: &mut impl Bus) -> u16 {
        // POP rp triggers only the read-side OAM bug (the dispatcher in
        // Mmu::apply_oam_read_corruption picks the right pattern per row).
        // The IDU bus stays quiet during POP's reads — matching SameBoy's
        // `pop_rr` which only emits cycle_read calls.
        let lo = self.read8_timed(bus, self.sp);
        self.sp = self.sp.wrapping_add(1);
        let hi = self.read8_timed(bus, self.sp);
        self.sp = self.sp.wrapping_add(1);
        u16::from_be_bytes([hi, lo])
    }

    fn read_r(&mut self, idx: u8, bus: &mut impl Bus) -> u8 {
        match idx & 7 {
            0 => self.b,
            1 => self.c,
            2 => self.d,
            3 => self.e,
            4 => self.h,
            5 => self.l,
            6 => self.read8_timed(bus, self.hl()),
            7 => self.a,
            _ => unreachable!(),
        }
    }

    fn write_r(&mut self, idx: u8, value: u8, bus: &mut impl Bus) {
        match idx & 7 {
            0 => self.b = value,
            1 => self.c = value,
            2 => self.d = value,
            3 => self.e = value,
            4 => self.h = value,
            5 => self.l = value,
            6 => self.write8_timed(bus, self.hl(), value),
            7 => self.a = value,
            _ => unreachable!(),
        }
    }

    fn add_a(&mut self, value: u8) {
        let (result, carry) = self.a.overflowing_add(value);
        let half = (self.a & 0xf) + (value & 0xf) > 0xf;
        self.a = result;
        self.f.set_z(result == 0);
        self.f.set_n(false);
        self.f.set_h(half);
        self.f.set_c(carry);
    }

    fn adc_a(&mut self, value: u8) {
        let carry_in = u8::from(self.f.c());
        let result = self.a.wrapping_add(value).wrapping_add(carry_in);
        let carry = u16::from(self.a) + u16::from(value) + u16::from(carry_in) > 0xff;
        let half = (self.a & 0xf) + (value & 0xf) + carry_in > 0xf;
        self.a = result;
        self.f.set_z(result == 0);
        self.f.set_n(false);
        self.f.set_h(half);
        self.f.set_c(carry);
    }

    fn sub_a(&mut self, value: u8) {
        let (result, borrow) = self.a.overflowing_sub(value);
        let half = (self.a & 0xf) < (value & 0xf);
        self.a = result;
        self.f.set_z(result == 0);
        self.f.set_n(true);
        self.f.set_h(half);
        self.f.set_c(borrow);
    }

    fn sbc_a(&mut self, value: u8) {
        let carry_in = u8::from(self.f.c());
        let result = self.a.wrapping_sub(value).wrapping_sub(carry_in);
        let borrow = u16::from(self.a) < u16::from(value) + u16::from(carry_in);
        let half = (self.a & 0xf) < (value & 0xf) + carry_in;
        self.a = result;
        self.f.set_z(result == 0);
        self.f.set_n(true);
        self.f.set_h(half);
        self.f.set_c(borrow);
    }

    fn and_a(&mut self, value: u8) {
        self.a &= value;
        self.f.set_z(self.a == 0);
        self.f.set_n(false);
        self.f.set_h(true);
        self.f.set_c(false);
    }

    fn xor_a(&mut self, value: u8) {
        self.a ^= value;
        self.f.set_z(self.a == 0);
        self.f.set_n(false);
        self.f.set_h(false);
        self.f.set_c(false);
    }

    fn or_a(&mut self, value: u8) {
        self.a |= value;
        self.f.set_z(self.a == 0);
        self.f.set_n(false);
        self.f.set_h(false);
        self.f.set_c(false);
    }

    fn cp_a(&mut self, value: u8) {
        let (result, borrow) = self.a.overflowing_sub(value);
        let half = (self.a & 0xf) < (value & 0xf);
        self.f.set_z(result == 0);
        self.f.set_n(true);
        self.f.set_h(half);
        self.f.set_c(borrow);
    }

    fn inc_r(&mut self, idx: u8, bus: &mut impl Bus) -> u8 {
        let v = self.read_r(idx, bus);
        let result = v.wrapping_add(1);
        self.write_r(idx, result, bus);
        self.f.set_z(result == 0);
        self.f.set_n(false);
        self.f.set_h((v & 0xf) == 0xf);
        // C unchanged
        if idx == 6 {
            12
        } else {
            4
        }
    }

    fn dec_r(&mut self, idx: u8, bus: &mut impl Bus) -> u8 {
        let v = self.read_r(idx, bus);
        let result = v.wrapping_sub(1);
        self.write_r(idx, result, bus);
        self.f.set_z(result == 0);
        self.f.set_n(true);
        self.f.set_h((v & 0xf) == 0);
        // C unchanged
        if idx == 6 {
            12
        } else {
            4
        }
    }

    fn add_hl(&mut self, value: u16) {
        let hl = self.hl();
        let (result, carry) = hl.overflowing_add(value);
        let half = ((hl & 0x0fff) + (value & 0x0fff)) > 0x0fff;
        self.set_hl(result);
        // Z preserved
        self.f.set_n(false);
        self.f.set_h(half);
        self.f.set_c(carry);
    }

    fn cc_satisfied(&self, cc: u8) -> bool {
        match cc & 3 {
            0 => !self.f.z(), // NZ
            1 => self.f.z(),  // Z
            2 => !self.f.c(), // NC
            3 => self.f.c(),  // C
            _ => unreachable!(),
        }
    }

    fn jr_cc(&mut self, cc: u8, bus: &mut impl Bus) -> u8 {
        let e = self.fetch8(bus) as i8;
        if self.cc_satisfied(cc) {
            self.idle(bus);
            self.pc = self.pc.wrapping_add_signed(i16::from(e));
            12
        } else {
            8
        }
    }

    fn jp_cc(&mut self, cc: u8, bus: &mut impl Bus) -> u8 {
        let nn = self.fetch16(bus);
        if self.cc_satisfied(cc) {
            self.idle(bus);
            self.pc = nn;
            16
        } else {
            12
        }
    }

    fn call_nn(&mut self, bus: &mut impl Bus) -> u8 {
        let nn = self.fetch16(bus);
        // The internal M-cycle before the two pushes asserts SP on the IDU
        // bus, so an SP that sits inside OAM trips the write-side bug just
        // like the pushes themselves do.
        self.idu_timed(bus, self.sp);
        self.push16(bus, self.pc);
        self.pc = nn;
        24
    }

    fn call_cc(&mut self, cc: u8, bus: &mut impl Bus) -> u8 {
        let nn = self.fetch16(bus);
        if self.cc_satisfied(cc) {
            self.idu_timed(bus, self.sp);
            self.push16(bus, self.pc);
            self.pc = nn;
            24
        } else {
            12
        }
    }

    fn ret_cc(&mut self, cc: u8, bus: &mut impl Bus) -> u8 {
        if self.cc_satisfied(cc) {
            self.idle(bus);
            self.pc = self.pop16(bus);
            self.idle(bus);
            20
        } else {
            self.idle(bus);
            8
        }
    }

    fn rst(&mut self, vector: u16, bus: &mut impl Bus) -> u8 {
        // Same IDU exposure as PUSH/CALL before the two pushes.
        self.idu_timed(bus, self.sp);
        self.push16(bus, self.pc);
        self.pc = vector;
        16
    }

    fn add_sp_e(&mut self, e: i8) -> u16 {
        // Flags are computed as if e were an unsigned 8-bit addend to SP's low byte.
        // The 16-bit result uses signed extension. This asymmetry is a SM83 quirk.
        let sp = self.sp;
        let e_u = e as u8;
        let result = sp.wrapping_add_signed(i16::from(e));
        self.f.set_z(false);
        self.f.set_n(false);
        self.f.set_h(((sp & 0xf) + u16::from(e_u & 0xf)) > 0xf);
        self.f.set_c(((sp & 0xff) + u16::from(e_u)) > 0xff);
        result
    }

    fn rlc(&mut self, value: u8) -> u8 {
        let result = value.rotate_left(1);
        self.f.set_z(result == 0);
        self.f.set_n(false);
        self.f.set_h(false);
        self.f.set_c(value & 0x80 != 0);
        result
    }

    fn rrc(&mut self, value: u8) -> u8 {
        let result = value.rotate_right(1);
        self.f.set_z(result == 0);
        self.f.set_n(false);
        self.f.set_h(false);
        self.f.set_c(value & 0x01 != 0);
        result
    }

    fn rl(&mut self, value: u8) -> u8 {
        let carry_in = u8::from(self.f.c());
        let result = (value << 1) | carry_in;
        self.f.set_z(result == 0);
        self.f.set_n(false);
        self.f.set_h(false);
        self.f.set_c(value & 0x80 != 0);
        result
    }

    fn rr(&mut self, value: u8) -> u8 {
        let carry_in = u8::from(self.f.c());
        let result = (value >> 1) | (carry_in << 7);
        self.f.set_z(result == 0);
        self.f.set_n(false);
        self.f.set_h(false);
        self.f.set_c(value & 0x01 != 0);
        result
    }

    fn sla(&mut self, value: u8) -> u8 {
        let result = value << 1;
        self.f.set_z(result == 0);
        self.f.set_n(false);
        self.f.set_h(false);
        self.f.set_c(value & 0x80 != 0);
        result
    }

    fn sra(&mut self, value: u8) -> u8 {
        let result = (value >> 1) | (value & 0x80);
        self.f.set_z(result == 0);
        self.f.set_n(false);
        self.f.set_h(false);
        self.f.set_c(value & 0x01 != 0);
        result
    }

    fn swap(&mut self, value: u8) -> u8 {
        let result = value.rotate_left(4);
        self.f.set_z(result == 0);
        self.f.set_n(false);
        self.f.set_h(false);
        self.f.set_c(false);
        result
    }

    fn srl(&mut self, value: u8) -> u8 {
        let result = value >> 1;
        self.f.set_z(result == 0);
        self.f.set_n(false);
        self.f.set_h(false);
        self.f.set_c(value & 0x01 != 0);
        result
    }

    fn daa(&mut self) {
        let mut adjust = 0_u8;
        let mut carry = self.f.c();
        if self.f.n() {
            if self.f.h() {
                adjust |= 0x06;
            }
            if self.f.c() {
                adjust |= 0x60;
            }
            self.a = self.a.wrapping_sub(adjust);
        } else {
            if self.f.h() || (self.a & 0x0f) > 0x09 {
                adjust |= 0x06;
            }
            if self.f.c() || self.a > 0x99 {
                adjust |= 0x60;
                carry = true;
            }
            self.a = self.a.wrapping_add(adjust);
        }
        self.f.set_z(self.a == 0);
        self.f.set_h(false);
        self.f.set_c(carry);
    }

    fn step_cb(&mut self, bus: &mut impl Bus) -> u8 {
        let opcode = self.fetch8(bus);
        let src = opcode & 7;
        let value = self.read_r(src, bus);

        match opcode {
            0x00..=0x3f => {
                let op = (opcode >> 3) & 7;
                let result = match op {
                    0 => self.rlc(value),
                    1 => self.rrc(value),
                    2 => self.rl(value),
                    3 => self.rr(value),
                    4 => self.sla(value),
                    5 => self.sra(value),
                    6 => self.swap(value),
                    7 => self.srl(value),
                    _ => unreachable!(),
                };
                self.write_r(src, result, bus);
                if src == 6 {
                    16
                } else {
                    8
                }
            }
            0x40..=0x7f => {
                let bit = (opcode >> 3) & 7;
                self.f.set_z(value & (1 << bit) == 0);
                self.f.set_n(false);
                self.f.set_h(true);
                if src == 6 {
                    12
                } else {
                    8
                }
            }
            0x80..=0xbf => {
                let bit = (opcode >> 3) & 7;
                self.write_r(src, value & !(1 << bit), bus);
                if src == 6 {
                    16
                } else {
                    8
                }
            }
            0xc0..=0xff => {
                let bit = (opcode >> 3) & 7;
                self.write_r(src, value | (1 << bit), bus);
                if src == 6 {
                    16
                } else {
                    8
                }
            }
        }
    }

    fn service_interrupt(&mut self, bus: &mut impl Bus, pending: u8) -> u8 {
        let bit = pending.trailing_zeros() as u8;
        let vector = 0x40_u16 + u16::from(bit) * 8;

        self.ime = false;
        let if_ = bus.read8(0xff0f);
        bus.write8(0xff0f, if_ & !(1u8 << bit));

        self.idle(bus);
        self.idle(bus);
        self.idle(bus);
        self.push16(bus, self.pc);
        self.pc = vector;

        20
    }

    fn tick_ime_delay(&mut self) {
        if self.ime_delay > 0 {
            self.ime_delay -= 1;
            if self.ime_delay == 0 {
                self.ime = true;
            }
        }
    }

    /// Execute one instruction (or service one pending interrupt, or
    /// burn one HALT cycle, or one stall cycle when the CPU has
    /// locked on an illegal opcode). Returns the number of T-cycles
    /// the rest of the system should advance by.
    pub fn step(&mut self, bus: &mut impl Bus) -> u8 {
        if self.locked {
            self.idle(bus);
            return 4;
        }

        let pending = bus.read8(0xff0f) & bus.read8(0xffff) & 0x1f;

        if self.halted && pending != 0 {
            self.halted = false;
        }

        if self.ime && pending != 0 {
            return self.service_interrupt(bus, pending);
        }

        if self.halted {
            self.idle(bus);
            return 4;
        }

        let opcode = self.fetch8(bus);
        let cycles = match opcode {
            0x00 => 4,
            0x01 => {
                let v = self.fetch16(bus);
                self.set_bc(v);
                12
            }
            0x02 => {
                self.write8_timed(bus, self.bc(), self.a);
                8
            }
            0x03 => {
                self.idu_timed(bus, self.bc());
                self.set_bc(self.bc().wrapping_add(1));
                8
            }
            0x04 => self.inc_r(0, bus),
            0x05 => self.dec_r(0, bus),
            0x06 => {
                self.b = self.fetch8(bus);
                8
            }
            0x07 => {
                self.a = self.rlc(self.a);
                self.f.set_z(false);
                4
            }
            0x08 => {
                let nn = self.fetch16(bus);
                let [hi, lo] = self.sp.to_be_bytes();
                self.write8_timed(bus, nn, lo);
                self.write8_timed(bus, nn.wrapping_add(1), hi);
                20
            }
            0x09 => {
                self.idle(bus);
                self.add_hl(self.bc());
                8
            }
            0x0a => {
                self.a = self.read8_timed(bus, self.bc());
                8
            }
            0x0b => {
                self.idu_timed(bus, self.bc());
                self.set_bc(self.bc().wrapping_sub(1));
                8
            }
            0x0c => self.inc_r(1, bus),
            0x0d => self.dec_r(1, bus),
            0x0e => {
                self.c = self.fetch8(bus);
                8
            }
            0x0f => {
                self.a = self.rrc(self.a);
                self.f.set_z(false);
                4
            }
            0x10 => {
                // STOP — encoded as 0x10 0x00. We consume the operand byte
                // and reset DIV. The low-power / LCD-off semantics aren't
                // modeled; the CPU keeps executing the next instruction.
                let _ = self.consume_operand_untimed(bus);
                bus.write8(0xff04, 0);
                4
            }
            0x11 => {
                let v = self.fetch16(bus);
                self.set_de(v);
                12
            }
            0x12 => {
                self.write8_timed(bus, self.de(), self.a);
                8
            }
            0x13 => {
                self.idu_timed(bus, self.de());
                self.set_de(self.de().wrapping_add(1));
                8
            }
            0x14 => self.inc_r(2, bus),
            0x15 => self.dec_r(2, bus),
            0x16 => {
                self.d = self.fetch8(bus);
                8
            }
            0x17 => {
                self.a = self.rl(self.a);
                self.f.set_z(false);
                4
            }
            0x18 => {
                let e = self.fetch8(bus) as i8;
                self.idle(bus);
                self.pc = self.pc.wrapping_add_signed(i16::from(e));
                12
            }
            0x19 => {
                self.idle(bus);
                self.add_hl(self.de());
                8
            }
            0x1a => {
                self.a = self.read8_timed(bus, self.de());
                8
            }
            0x1b => {
                self.idu_timed(bus, self.de());
                self.set_de(self.de().wrapping_sub(1));
                8
            }
            0x1c => self.inc_r(3, bus),
            0x1d => self.dec_r(3, bus),
            0x1e => {
                self.e = self.fetch8(bus);
                8
            }
            0x1f => {
                self.a = self.rr(self.a);
                self.f.set_z(false);
                4
            }
            0x20 => self.jr_cc(0, bus), // JR NZ, e
            0x21 => {
                let v = self.fetch16(bus);
                self.set_hl(v);
                12
            }
            0x22 => {
                let hl = self.hl();
                self.write8_timed_idu(bus, hl, self.a, hl);
                self.set_hl(self.hl().wrapping_add(1));
                8
            }
            0x23 => {
                self.idu_timed(bus, self.hl());
                self.set_hl(self.hl().wrapping_add(1));
                8
            }
            0x24 => self.inc_r(4, bus),
            0x25 => self.dec_r(4, bus),
            0x26 => {
                self.h = self.fetch8(bus);
                8
            }
            0x27 => {
                self.daa();
                4
            }
            0x28 => self.jr_cc(1, bus), // JR Z, e
            0x29 => {
                self.idle(bus);
                self.add_hl(self.hl());
                8
            }
            0x2a => {
                let hl = self.hl();
                self.a = self.read8_timed_idu(bus, hl, hl);
                self.set_hl(self.hl().wrapping_add(1));
                8
            }
            0x2b => {
                self.idu_timed(bus, self.hl());
                self.set_hl(self.hl().wrapping_sub(1));
                8
            }
            0x2c => self.inc_r(5, bus),
            0x2d => self.dec_r(5, bus),
            0x2e => {
                self.l = self.fetch8(bus);
                8
            }
            0x2f => {
                self.a = !self.a;
                self.f.set_n(true);
                self.f.set_h(true);
                4
            }
            0x30 => self.jr_cc(2, bus), // JR NC, e
            0x31 => {
                self.sp = self.fetch16(bus);
                12
            }
            0x32 => {
                let hl = self.hl();
                self.write8_timed_idu(bus, hl, self.a, hl);
                self.set_hl(self.hl().wrapping_sub(1));
                8
            }
            0x33 => {
                self.idu_timed(bus, self.sp);
                self.sp = self.sp.wrapping_add(1);
                8
            }
            0x34 => self.inc_r(6, bus),
            0x35 => self.dec_r(6, bus),
            0x36 => {
                let n = self.fetch8(bus);
                self.write8_timed(bus, self.hl(), n);
                12
            }
            0x37 => {
                self.f.set_n(false);
                self.f.set_h(false);
                self.f.set_c(true);
                4
            }
            0x38 => self.jr_cc(3, bus), // JR C, e
            0x39 => {
                self.idle(bus);
                self.add_hl(self.sp);
                8
            }
            0x3a => {
                let hl = self.hl();
                self.a = self.read8_timed_idu(bus, hl, hl);
                self.set_hl(self.hl().wrapping_sub(1));
                8
            }
            0x3b => {
                self.idu_timed(bus, self.sp);
                self.sp = self.sp.wrapping_sub(1);
                8
            }
            0x3c => self.inc_r(7, bus),
            0x3d => self.dec_r(7, bus),
            0x3e => {
                self.a = self.fetch8(bus);
                8
            }
            0x3f => {
                let c = self.f.c();
                self.f.set_n(false);
                self.f.set_h(false);
                self.f.set_c(!c);
                4
            }
            0x76 => {
                // Re-sample IE & IF here: the opcode fetch above ticked the
                // bus 4 cycles, which can have raised a new IF bit (timer
                // overflow, PPU mode transition, etc.). Blargg's halt_bug
                // test cares about the pending state *at HALT execution*,
                // not at the start of the step.
                let pending_now = bus.read8(0xff0f) & bus.read8(0xffff) & 0x1f;
                if pending_now != 0 {
                    if self.ime {
                        // HALT yields immediately to the pending IRQ. PC
                        // rewinds so the ISR's return address points at the
                        // HALT itself — matching real DMG's "re-run HALT
                        // after RETI" semantics.
                        self.pc = self.pc.wrapping_sub(1);
                    } else {
                        self.halt_bug = true;
                    }
                } else {
                    self.halted = true;
                }
                4
            }
            0x40..=0x7f => {
                let dst = (opcode >> 3) & 7;
                let src = opcode & 7;
                let value = self.read_r(src, bus);
                self.write_r(dst, value, bus);
                if dst == 6 || src == 6 {
                    8
                } else {
                    4
                }
            }
            0x80..=0xbf => {
                let op = (opcode >> 3) & 7;
                let src = opcode & 7;
                let value = self.read_r(src, bus);
                match op {
                    0 => self.add_a(value),
                    1 => self.adc_a(value),
                    2 => self.sub_a(value),
                    3 => self.sbc_a(value),
                    4 => self.and_a(value),
                    5 => self.xor_a(value),
                    6 => self.or_a(value),
                    7 => self.cp_a(value),
                    _ => unreachable!(),
                }
                if src == 6 {
                    8
                } else {
                    4
                }
            }
            0xc0 => self.ret_cc(0, bus), // RET NZ
            0xc1 => {
                let v = self.pop16(bus);
                self.set_bc(v);
                12
            }
            0xc2 => self.jp_cc(0, bus), // JP NZ, nn
            0xc3 => {
                self.pc = self.fetch16(bus);
                self.idle(bus);
                16
            }
            0xc4 => self.call_cc(0, bus), // CALL NZ
            0xc5 => {
                self.idu_timed(bus, self.sp);
                self.push16(bus, self.bc());
                16
            }
            0xc6 => {
                let n = self.fetch8(bus);
                self.add_a(n);
                8
            }
            0xc7 => self.rst(0x0000, bus),
            0xc8 => self.ret_cc(1, bus), // RET Z
            0xc9 => {
                self.pc = self.pop16(bus);
                self.idle(bus);
                16
            }
            0xca => self.jp_cc(1, bus), // JP Z, nn
            0xcb => self.step_cb(bus),
            0xcc => self.call_cc(1, bus), // CALL Z
            0xcd => self.call_nn(bus),    // CALL nn
            0xce => {
                let n = self.fetch8(bus);
                self.adc_a(n);
                8
            }
            0xcf => self.rst(0x0008, bus),
            0xd0 => self.ret_cc(2, bus), // RET NC
            0xd1 => {
                let v = self.pop16(bus);
                self.set_de(v);
                12
            }
            0xd2 => self.jp_cc(2, bus),   // JP NC, nn
            0xd4 => self.call_cc(2, bus), // CALL NC
            0xd5 => {
                self.idu_timed(bus, self.sp);
                self.push16(bus, self.de());
                16
            }
            0xd6 => {
                let n = self.fetch8(bus);
                self.sub_a(n);
                8
            }
            0xd7 => self.rst(0x0010, bus),
            0xd8 => self.ret_cc(3, bus), // RET C
            0xd9 => {
                self.pc = self.pop16(bus);
                self.idle(bus);
                self.ime = true;
                self.ime_delay = 0;
                16
            }
            0xda => self.jp_cc(3, bus),   // JP C, nn
            0xdc => self.call_cc(3, bus), // CALL C
            0xde => {
                let n = self.fetch8(bus);
                self.sbc_a(n);
                8
            }
            0xdf => self.rst(0x0018, bus),
            0xe0 => {
                let n = self.fetch8(bus);
                self.write8_timed(bus, 0xff00 + u16::from(n), self.a);
                12
            }
            0xe1 => {
                let v = self.pop16(bus);
                self.set_hl(v);
                12
            }
            0xe2 => {
                self.write8_timed(bus, 0xff00 + u16::from(self.c), self.a);
                8
            }
            0xe5 => {
                self.idu_timed(bus, self.sp);
                self.push16(bus, self.hl());
                16
            }
            0xe6 => {
                let n = self.fetch8(bus);
                self.and_a(n);
                8
            }
            0xe7 => self.rst(0x0020, bus),
            0xe8 => {
                let e = self.fetch8(bus) as i8;
                self.idle(bus);
                self.idle(bus);
                self.sp = self.add_sp_e(e);
                16
            }
            0xe9 => {
                self.pc = self.hl();
                4
            }
            0xea => {
                let nn = self.fetch16(bus);
                self.write8_timed(bus, nn, self.a);
                16
            }
            0xee => {
                let n = self.fetch8(bus);
                self.xor_a(n);
                8
            }
            0xef => self.rst(0x0028, bus),
            0xf0 => {
                let n = self.fetch8(bus);
                self.a = self.read8_timed(bus, 0xff00 + u16::from(n));
                12
            }
            0xf1 => {
                let v = self.pop16(bus);
                self.set_af(v);
                12
            }
            0xf2 => {
                self.a = self.read8_timed(bus, 0xff00 + u16::from(self.c));
                8
            }
            0xf3 => {
                self.ime = false;
                self.ime_delay = 0;
                4
            }
            0xf5 => {
                self.idu_timed(bus, self.sp);
                self.push16(bus, self.af());
                16
            }
            0xf6 => {
                let n = self.fetch8(bus);
                self.or_a(n);
                8
            }
            0xf7 => self.rst(0x0030, bus),
            0xf8 => {
                let e = self.fetch8(bus) as i8;
                self.idle(bus);
                let v = self.add_sp_e(e);
                self.set_hl(v);
                12
            }
            0xf9 => {
                self.idle(bus);
                self.sp = self.hl();
                8
            }
            0xfa => {
                let nn = self.fetch16(bus);
                self.a = self.read8_timed(bus, nn);
                16
            }
            0xfb => {
                self.ime_delay = 2;
                4
            }
            0xfe => {
                let n = self.fetch8(bus);
                self.cp_a(n);
                8
            }
            0xff => self.rst(0x0038, bus),
            // Illegal opcodes lock the CPU on real hardware
            0xd3 | 0xdb | 0xdd | 0xe3 | 0xe4 | 0xeb | 0xec | 0xed | 0xf4 | 0xfc | 0xfd => {
                self.locked = true;
                4
            }
        };

        self.tick_ime_delay();

        cycles
    }
}

impl Default for Cpu {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::Memory;
    use std::cell::Cell;

    struct TimingBus {
        ticks: Cell<u32>,
        hl_addr: u16,
        wrote_at: Cell<u32>,
        wrote_value: Cell<u8>,
    }

    impl TimingBus {
        fn new(hl_addr: u16) -> Self {
            Self {
                ticks: Cell::new(0),
                hl_addr,
                wrote_at: Cell::new(u32::MAX),
                wrote_value: Cell::new(0),
            }
        }
    }

    impl Bus for TimingBus {
        fn read8(&self, addr: u16) -> u8 {
            match addr {
                0x0000 => 0xf0, // LDH A, (n)
                0x0001 => 0x05, // TIMA
                0x0100 => 0x34, // INC (HL)
                0xff05 => {
                    if self.ticks.get() >= 5 {
                        0x34
                    } else {
                        0x12
                    }
                }
                addr if addr == self.hl_addr => 0x7f,
                _ => 0x00,
            }
        }

        fn write8(&mut self, addr: u16, value: u8) {
            if addr == self.hl_addr {
                self.wrote_at.set(self.ticks.get());
                self.wrote_value.set(value);
            }
        }

        fn tick(&mut self, cycles: u8) {
            self.ticks.set(self.ticks.get() + u32::from(cycles));
        }
    }

    #[test]
    fn nop_then_halt() {
        let mut cpu = Cpu::new();
        let mut mem = Memory::new();
        mem.load(0x0000, &[0x00, 0x76]);

        cpu.step(&mut mem);
        assert_eq!(cpu.pc, 0x0001);
        assert!(!cpu.halted);

        cpu.step(&mut mem);
        assert_eq!(cpu.pc, 0x0002);
        assert!(cpu.halted);
    }

    #[test]
    fn stop_consumes_operand_and_continues() {
        let mut cpu = Cpu::new();
        let mut mem = Memory::new();
        // STOP 0x00, then LD A, 0x42 — the operand byte must not execute.
        mem.load(0x0000, &[0x10, 0x00, 0x3e, 0x42]);

        cpu.step(&mut mem);
        assert_eq!(cpu.pc, 0x0002);
        assert!(!cpu.halted);

        cpu.step(&mut mem);
        assert_eq!(cpu.a, 0x42);
        assert_eq!(cpu.pc, 0x0004);
    }

    #[test]
    fn ld_a_n() {
        let mut cpu = Cpu::new();
        let mut mem = Memory::new();
        mem.load(0x0000, &[0x3e, 0x42, 0x76]);

        cpu.step(&mut mem);
        assert_eq!(cpu.a, 0x42);
        assert_eq!(cpu.pc, 0x0002);

        cpu.step(&mut mem);
        assert!(cpu.halted);
    }

    #[test]
    fn halted_cpu_does_not_advance_pc() {
        let mut cpu = Cpu::new();
        let mut mem = Memory::new();
        mem.load(0x0000, &[0x76]);

        cpu.step(&mut mem);
        let pc = cpu.pc;
        assert!(cpu.halted);

        let cycles = cpu.step(&mut mem);
        assert_eq!(cpu.pc, pc);
        assert_eq!(cycles, 4);
    }

    #[test]
    fn step_returns_cycle_counts() {
        let mut cpu = Cpu::new();
        let mut mem = Memory::new();
        mem.load(0x0000, &[0x00, 0x3e, 0x42, 0x76]);

        assert_eq!(cpu.step(&mut mem), 4); // NOP
        assert_eq!(cpu.step(&mut mem), 8); // LD A, n
        assert_eq!(cpu.step(&mut mem), 4); // HALT
        assert_eq!(cpu.step(&mut mem), 4); // halted -> 1 M-cycle
    }

    #[test]
    fn ldh_a_n_reads_after_the_intermediate_m_cycles() {
        let mut cpu = Cpu::new();
        let mut bus = TimingBus::new(0xc000);

        let cycles = cpu.step(&mut bus);

        assert_eq!(cycles, 12);
        assert_eq!(cpu.a, 0x34);
        assert_eq!(bus.ticks.get(), 12);
    }

    #[test]
    fn inc_indirect_hl_reads_then_writes_on_later_m_cycle() {
        let mut cpu = Cpu::new();
        let mut bus = TimingBus::new(0xc000);
        cpu.pc = 0x0100;
        cpu.set_hl(0xc000);

        let cycles = cpu.step(&mut bus);

        assert_eq!(cycles, 12);
        assert_eq!(bus.wrote_value.get(), 0x80);
        assert_eq!(bus.wrote_at.get(), 12);
        assert_eq!(bus.ticks.get(), 12);
    }

    #[test]
    fn pc_wraps_around_at_0xffff() {
        let mut cpu = Cpu::new();
        let mut mem = Memory::new();
        mem.write8(0xffff, 0x00);
        cpu.pc = 0xffff;

        cpu.step(&mut mem);

        assert_eq!(cpu.pc, 0x0000);
    }

    #[test]
    fn ld_r_n_loads_all_8bit_registers() {
        let mut cpu = Cpu::new();
        let mut mem = Memory::new();
        mem.load(
            0x0000,
            &[
                0x06, 0xb0, // LD B, 0xB0
                0x0e, 0xc0, // LD C, 0xC0
                0x16, 0xd0, // LD D, 0xD0
                0x1e, 0xe0, // LD E, 0xE0
                0x26, 0x40, // LD H, 0x40
                0x2e, 0x50, // LD L, 0x50
                0x3e, 0xa0, // LD A, 0xA0
                0x76, // HALT
            ],
        );

        for _ in 0..8 {
            cpu.step(&mut mem);
        }

        assert_eq!(cpu.b, 0xb0);
        assert_eq!(cpu.c, 0xc0);
        assert_eq!(cpu.d, 0xd0);
        assert_eq!(cpu.e, 0xe0);
        assert_eq!(cpu.h, 0x40);
        assert_eq!(cpu.l, 0x50);
        assert_eq!(cpu.a, 0xa0);
        assert!(cpu.halted);
        assert_eq!(cpu.pc, 0x000f);
    }

    #[test]
    fn ld_a_n_overwrites_previous_value() {
        let mut cpu = Cpu::new();
        let mut mem = Memory::new();
        mem.load(0x0000, &[0x3e, 0x11, 0x3e, 0x22]);

        cpu.step(&mut mem);
        assert_eq!(cpu.a, 0x11);

        cpu.step(&mut mem);
        assert_eq!(cpu.a, 0x22);
        assert_eq!(cpu.pc, 0x0004);
    }

    #[test]
    fn register_pairs_roundtrip() {
        let mut cpu = Cpu::new();

        cpu.set_bc(0x1234);
        assert_eq!((cpu.b, cpu.c), (0x12, 0x34));
        assert_eq!(cpu.bc(), 0x1234);

        cpu.set_de(0x5678);
        assert_eq!((cpu.d, cpu.e), (0x56, 0x78));
        assert_eq!(cpu.de(), 0x5678);

        cpu.set_hl(0x9abc);
        assert_eq!((cpu.h, cpu.l), (0x9a, 0xbc));
        assert_eq!(cpu.hl(), 0x9abc);
    }

    #[test]
    fn set_af_masks_lower_nibble_of_f() {
        let mut cpu = Cpu::new();

        cpu.set_af(0x12ff);

        assert_eq!(cpu.a, 0x12);
        assert_eq!(cpu.f.bits(), 0xf0);
        assert_eq!(cpu.af(), 0x12f0);
    }

    #[test]
    fn read_r_dispatches_to_all_registers() {
        let mut cpu = Cpu::new();
        let mut mem = Memory::new();
        cpu.b = 0xb0;
        cpu.c = 0xc0;
        cpu.d = 0xd0;
        cpu.e = 0xe0;
        cpu.set_hl(0x4050);
        cpu.a = 0xa0;
        mem.write8(0x4050, 0x66);

        assert_eq!(cpu.read_r(0, &mut mem), 0xb0); // B
        assert_eq!(cpu.read_r(1, &mut mem), 0xc0); // C
        assert_eq!(cpu.read_r(2, &mut mem), 0xd0); // D
        assert_eq!(cpu.read_r(3, &mut mem), 0xe0); // E
        assert_eq!(cpu.read_r(4, &mut mem), 0x40); // H
        assert_eq!(cpu.read_r(5, &mut mem), 0x50); // L
        assert_eq!(cpu.read_r(6, &mut mem), 0x66); // (HL)
        assert_eq!(cpu.read_r(7, &mut mem), 0xa0); // A
    }

    #[test]
    fn ld_b_c() {
        let mut cpu = Cpu::new();
        let mut mem = Memory::new();
        cpu.c = 0x42;
        mem.load(0x0000, &[0x41]); // LD B, C

        let cycles = cpu.step(&mut mem);

        assert_eq!(cpu.b, 0x42);
        assert_eq!(cycles, 4);
    }

    #[test]
    fn ld_a_indirect_hl() {
        let mut cpu = Cpu::new();
        let mut mem = Memory::new();
        cpu.set_hl(0xc000);
        mem.write8(0xc000, 0x99);
        mem.load(0x0000, &[0x7e]); // LD A, (HL)

        let cycles = cpu.step(&mut mem);

        assert_eq!(cpu.a, 0x99);
        assert_eq!(cycles, 8);
    }

    #[test]
    fn ld_indirect_hl_a() {
        let mut cpu = Cpu::new();
        let mut mem = Memory::new();
        cpu.set_hl(0xc000);
        cpu.a = 0x88;
        mem.load(0x0000, &[0x77]); // LD (HL), A

        let cycles = cpu.step(&mut mem);

        assert_eq!(mem.read8(0xc000), 0x88);
        assert_eq!(cycles, 8);
    }

    #[test]
    fn write_r_dispatches_to_all_registers() {
        let mut cpu = Cpu::new();
        let mut mem = Memory::new();
        cpu.set_hl(0xc000);

        cpu.write_r(6, 0x66, &mut mem); // (HL) before H/L get overwritten
        cpu.write_r(0, 0xb0, &mut mem);
        cpu.write_r(1, 0xc0, &mut mem);
        cpu.write_r(2, 0xd0, &mut mem);
        cpu.write_r(3, 0xe0, &mut mem);
        cpu.write_r(4, 0x40, &mut mem);
        cpu.write_r(5, 0x50, &mut mem);
        cpu.write_r(7, 0xa0, &mut mem);

        assert_eq!(cpu.b, 0xb0);
        assert_eq!(cpu.c, 0xc0);
        assert_eq!(cpu.d, 0xd0);
        assert_eq!(cpu.e, 0xe0);
        assert_eq!(cpu.h, 0x40);
        assert_eq!(cpu.l, 0x50);
        assert_eq!(cpu.a, 0xa0);
        assert_eq!(mem.read8(0xc000), 0x66);
    }

    #[test]
    fn add_a_b_sets_half_carry_only() {
        let mut cpu = Cpu::new();
        let mut mem = Memory::new();
        cpu.a = 0x0f;
        cpu.b = 0x01;
        mem.load(0x0000, &[0x80]); // ADD A, B

        let cycles = cpu.step(&mut mem);

        assert_eq!(cpu.a, 0x10);
        assert!(!cpu.f.z());
        assert!(!cpu.f.n());
        assert!(cpu.f.h());
        assert!(!cpu.f.c());
        assert_eq!(cycles, 4);
    }

    #[test]
    fn add_a_b_overflow_sets_zero_and_carry() {
        let mut cpu = Cpu::new();
        let mut mem = Memory::new();
        cpu.a = 0xff;
        cpu.b = 0x01;
        mem.load(0x0000, &[0x80]); // ADD A, B

        cpu.step(&mut mem);

        assert_eq!(cpu.a, 0x00);
        assert!(cpu.f.z());
        assert!(!cpu.f.n());
        assert!(cpu.f.h());
        assert!(cpu.f.c());
    }

    #[test]
    fn adc_a_b_propagates_carry_in() {
        let mut cpu = Cpu::new();
        let mut mem = Memory::new();
        cpu.a = 0x10;
        cpu.b = 0x20;
        cpu.f.set_c(true);
        mem.load(0x0000, &[0x88]); // ADC A, B

        cpu.step(&mut mem);

        assert_eq!(cpu.a, 0x31);
        assert!(!cpu.f.c());
    }

    #[test]
    fn sub_a_b_to_zero_sets_z_and_n() {
        let mut cpu = Cpu::new();
        let mut mem = Memory::new();
        cpu.a = 0x42;
        cpu.b = 0x42;
        mem.load(0x0000, &[0x90]); // SUB B

        cpu.step(&mut mem);

        assert_eq!(cpu.a, 0x00);
        assert!(cpu.f.z());
        assert!(cpu.f.n());
        assert!(!cpu.f.h());
        assert!(!cpu.f.c());
    }

    #[test]
    fn sub_a_b_underflow_sets_carry_and_half_carry() {
        let mut cpu = Cpu::new();
        let mut mem = Memory::new();
        cpu.a = 0x00;
        cpu.b = 0x01;
        mem.load(0x0000, &[0x90]); // SUB B

        cpu.step(&mut mem);

        assert_eq!(cpu.a, 0xff);
        assert!(!cpu.f.z());
        assert!(cpu.f.n());
        assert!(cpu.f.h());
        assert!(cpu.f.c());
    }

    #[test]
    fn sbc_a_b_propagates_borrow_in() {
        let mut cpu = Cpu::new();
        let mut mem = Memory::new();
        cpu.a = 0x10;
        cpu.b = 0x01;
        cpu.f.set_c(true);
        mem.load(0x0000, &[0x98]); // SBC A, B

        cpu.step(&mut mem);

        assert_eq!(cpu.a, 0x0e);
        assert!(cpu.f.n());
    }

    #[test]
    fn and_a_b_always_sets_half_carry() {
        let mut cpu = Cpu::new();
        let mut mem = Memory::new();
        cpu.a = 0xff;
        cpu.b = 0xf0;
        mem.load(0x0000, &[0xa0]); // AND B

        cpu.step(&mut mem);

        assert_eq!(cpu.a, 0xf0);
        assert!(!cpu.f.z());
        assert!(!cpu.f.n());
        assert!(cpu.f.h());
        assert!(!cpu.f.c());
    }

    #[test]
    fn xor_a_a_zeroes_register() {
        let mut cpu = Cpu::new();
        let mut mem = Memory::new();
        cpu.a = 0x42;
        mem.load(0x0000, &[0xaf]); // XOR A

        cpu.step(&mut mem);

        assert_eq!(cpu.a, 0x00);
        assert!(cpu.f.z());
        assert!(!cpu.f.n());
        assert!(!cpu.f.h());
        assert!(!cpu.f.c());
    }

    #[test]
    fn or_a_b_combines_bits() {
        let mut cpu = Cpu::new();
        let mut mem = Memory::new();
        cpu.a = 0xf0;
        cpu.b = 0x0f;
        mem.load(0x0000, &[0xb0]); // OR B

        cpu.step(&mut mem);

        assert_eq!(cpu.a, 0xff);
        assert!(!cpu.f.z());
        assert!(!cpu.f.h());
        assert!(!cpu.f.c());
    }

    #[test]
    fn cp_a_b_does_not_modify_a() {
        let mut cpu = Cpu::new();
        let mut mem = Memory::new();
        cpu.a = 0x10;
        cpu.b = 0x20;
        mem.load(0x0000, &[0xb8]); // CP B

        cpu.step(&mut mem);

        assert_eq!(cpu.a, 0x10);
        assert!(!cpu.f.z());
        assert!(cpu.f.n());
        assert!(cpu.f.c()); // a < b
    }

    #[test]
    fn cp_a_a_sets_zero() {
        let mut cpu = Cpu::new();
        let mut mem = Memory::new();
        cpu.a = 0x42;
        mem.load(0x0000, &[0xbf]); // CP A

        cpu.step(&mut mem);

        assert_eq!(cpu.a, 0x42);
        assert!(cpu.f.z());
        assert!(cpu.f.n());
    }

    #[test]
    fn alu_indirect_hl_takes_8_cycles() {
        let mut cpu = Cpu::new();
        let mut mem = Memory::new();
        cpu.a = 0x10;
        cpu.set_hl(0xc000);
        mem.write8(0xc000, 0x20);
        mem.load(0x0000, &[0x86]); // ADD A, (HL)

        let cycles = cpu.step(&mut mem);

        assert_eq!(cpu.a, 0x30);
        assert_eq!(cycles, 8);
    }

    #[test]
    fn cb_rlc_b_rotates_left_and_captures_msb_in_carry() {
        let mut cpu = Cpu::new();
        let mut mem = Memory::new();
        cpu.b = 0b1010_1010;
        mem.load(0x0000, &[0xcb, 0x00]); // RLC B

        let cycles = cpu.step(&mut mem);

        assert_eq!(cpu.b, 0b0101_0101);
        assert!(!cpu.f.z());
        assert!(!cpu.f.n());
        assert!(!cpu.f.h());
        assert!(cpu.f.c());
        assert_eq!(cycles, 8);
    }

    #[test]
    fn cb_rrc_b_rotates_right_and_captures_lsb_in_carry() {
        let mut cpu = Cpu::new();
        let mut mem = Memory::new();
        cpu.b = 0b0000_0001;
        mem.load(0x0000, &[0xcb, 0x08]); // RRC B

        cpu.step(&mut mem);

        assert_eq!(cpu.b, 0b1000_0000);
        assert!(cpu.f.c());
    }

    #[test]
    fn cb_rl_b_inserts_carry_at_bit0() {
        let mut cpu = Cpu::new();
        let mut mem = Memory::new();
        cpu.b = 0b1000_0000;
        cpu.f.set_c(true);
        mem.load(0x0000, &[0xcb, 0x10]); // RL B

        cpu.step(&mut mem);

        assert_eq!(cpu.b, 0b0000_0001);
        assert!(cpu.f.c());
    }

    #[test]
    fn cb_rr_b_inserts_carry_at_bit7() {
        let mut cpu = Cpu::new();
        let mut mem = Memory::new();
        cpu.b = 0b0000_0001;
        cpu.f.set_c(true);
        mem.load(0x0000, &[0xcb, 0x18]); // RR B

        cpu.step(&mut mem);

        assert_eq!(cpu.b, 0b1000_0000);
        assert!(cpu.f.c());
    }

    #[test]
    fn cb_sla_b_shifts_in_zero() {
        let mut cpu = Cpu::new();
        let mut mem = Memory::new();
        cpu.b = 0b1010_1010;
        mem.load(0x0000, &[0xcb, 0x20]); // SLA B

        cpu.step(&mut mem);

        assert_eq!(cpu.b, 0b0101_0100);
        assert!(cpu.f.c());
    }

    #[test]
    fn cb_sra_b_preserves_sign_bit() {
        let mut cpu = Cpu::new();
        let mut mem = Memory::new();
        cpu.b = 0b1000_0001;
        mem.load(0x0000, &[0xcb, 0x28]); // SRA B

        cpu.step(&mut mem);

        assert_eq!(cpu.b, 0b1100_0000);
        assert!(cpu.f.c());
    }

    #[test]
    fn cb_swap_b_swaps_nibbles_and_clears_carry() {
        let mut cpu = Cpu::new();
        let mut mem = Memory::new();
        cpu.b = 0xab;
        cpu.f.set_c(true);
        mem.load(0x0000, &[0xcb, 0x30]); // SWAP B

        cpu.step(&mut mem);

        assert_eq!(cpu.b, 0xba);
        assert!(!cpu.f.c());
    }

    #[test]
    fn cb_srl_b_shifts_in_zero_at_top() {
        let mut cpu = Cpu::new();
        let mut mem = Memory::new();
        cpu.b = 0b1000_0001;
        mem.load(0x0000, &[0xcb, 0x38]); // SRL B

        cpu.step(&mut mem);

        assert_eq!(cpu.b, 0b0100_0000);
        assert!(cpu.f.c());
    }

    #[test]
    fn cb_bit_b_clears_z_when_bit_set() {
        let mut cpu = Cpu::new();
        let mut mem = Memory::new();
        cpu.b = 0b0000_0100;
        mem.load(0x0000, &[0xcb, 0x50]); // BIT 2, B

        cpu.step(&mut mem);

        assert!(!cpu.f.z());
        assert!(!cpu.f.n());
        assert!(cpu.f.h());
    }

    #[test]
    fn cb_bit_b_sets_z_when_bit_clear() {
        let mut cpu = Cpu::new();
        let mut mem = Memory::new();
        cpu.b = 0b0000_0100;
        mem.load(0x0000, &[0xcb, 0x48]); // BIT 1, B

        cpu.step(&mut mem);

        assert!(cpu.f.z());
    }

    #[test]
    fn cb_bit_indirect_hl_takes_12_cycles() {
        let mut cpu = Cpu::new();
        let mut mem = Memory::new();
        cpu.set_hl(0xc000);
        mem.write8(0xc000, 0xff);
        mem.load(0x0000, &[0xcb, 0x46]); // BIT 0, (HL)

        let cycles = cpu.step(&mut mem);

        assert!(!cpu.f.z());
        assert_eq!(cycles, 12);
    }

    #[test]
    fn cb_res_b_clears_bit() {
        let mut cpu = Cpu::new();
        let mut mem = Memory::new();
        cpu.b = 0xff;
        mem.load(0x0000, &[0xcb, 0x90]); // RES 2, B

        cpu.step(&mut mem);

        assert_eq!(cpu.b, 0b1111_1011);
    }

    #[test]
    fn cb_set_b_sets_bit() {
        let mut cpu = Cpu::new();
        let mut mem = Memory::new();
        cpu.b = 0x00;
        mem.load(0x0000, &[0xcb, 0xd0]); // SET 2, B

        cpu.step(&mut mem);

        assert_eq!(cpu.b, 0b0000_0100);
    }

    #[test]
    fn cb_rotate_indirect_hl_takes_16_cycles() {
        let mut cpu = Cpu::new();
        let mut mem = Memory::new();
        cpu.set_hl(0xc000);
        mem.write8(0xc000, 0x01);
        mem.load(0x0000, &[0xcb, 0x06]); // RLC (HL)

        let cycles = cpu.step(&mut mem);

        assert_eq!(mem.read8(0xc000), 0x02);
        assert_eq!(cycles, 16);
    }

    #[test]
    fn cb_res_indirect_hl_takes_16_cycles() {
        let mut cpu = Cpu::new();
        let mut mem = Memory::new();
        cpu.set_hl(0xc000);
        mem.write8(0xc000, 0xff);
        mem.load(0x0000, &[0xcb, 0x86]); // RES 0, (HL)

        let cycles = cpu.step(&mut mem);

        assert_eq!(mem.read8(0xc000), 0xfe);
        assert_eq!(cycles, 16);
    }

    #[test]
    fn interrupt_vblank_pushes_pc_and_jumps_to_vector() {
        let mut cpu = Cpu::new();
        let mut mem = Memory::new();
        cpu.pc = 0x1234;
        cpu.sp = 0xfffe;
        cpu.ime = true;
        mem.write8(0xff0f, 0x01); // VBlank pending
        mem.write8(0xffff, 0x01); // VBlank enabled

        let cycles = cpu.step(&mut mem);

        assert_eq!(cycles, 20);
        assert_eq!(cpu.pc, 0x0040);
        assert_eq!(cpu.sp, 0xfffc);
        assert_eq!(mem.read8(0xfffc), 0x34); // low byte
        assert_eq!(mem.read8(0xfffd), 0x12); // high byte
        assert_eq!(mem.read8(0xff0f), 0x00); // IF cleared
        assert!(!cpu.ime);
    }

    #[test]
    fn interrupt_priority_picks_lowest_bit() {
        let mut cpu = Cpu::new();
        let mut mem = Memory::new();
        cpu.pc = 0x1000;
        cpu.sp = 0xfffe;
        cpu.ime = true;
        mem.write8(0xff0f, 0x14); // bit 2 (Timer) + bit 4 (Joypad) pending
        mem.write8(0xffff, 0xff);

        cpu.step(&mut mem);

        assert_eq!(cpu.pc, 0x0050); // Timer vector
        assert_eq!(mem.read8(0xff0f), 0x10); // only bit 2 cleared
    }

    #[test]
    fn interrupt_not_serviced_when_ime_disabled() {
        let mut cpu = Cpu::new();
        let mut mem = Memory::new();
        cpu.pc = 0x1000;
        cpu.ime = false;
        mem.write8(0xff0f, 0x01);
        mem.write8(0xffff, 0x01);
        mem.write8(0x1000, 0x00); // NOP

        let cycles = cpu.step(&mut mem);

        assert_eq!(cycles, 4); // just NOP
        assert_eq!(cpu.pc, 0x1001);
        assert_eq!(mem.read8(0xff0f), 0x01); // IF unchanged
    }

    #[test]
    fn halt_wakes_on_pending_with_ime_disabled_then_executes_next() {
        let mut cpu = Cpu::new();
        let mut mem = Memory::new();
        cpu.pc = 0x1000;
        cpu.halted = true;
        cpu.ime = false;
        mem.write8(0x1000, 0x00); // NOP at PC

        // halted, no pending — idles
        assert_eq!(cpu.step(&mut mem), 4);
        assert!(cpu.halted);

        // raise interrupt
        mem.write8(0xff0f, 0x01);
        mem.write8(0xffff, 0x01);

        // wake, IME=0 so no service, execute NOP
        let cycles = cpu.step(&mut mem);
        assert!(!cpu.halted);
        assert_eq!(cpu.pc, 0x1001);
        assert_eq!(cycles, 4);
    }

    #[test]
    fn halt_wakes_and_services_with_ime_enabled() {
        let mut cpu = Cpu::new();
        let mut mem = Memory::new();
        cpu.pc = 0x1000;
        cpu.sp = 0xfffe;
        cpu.halted = true;
        cpu.ime = true;
        mem.write8(0xff0f, 0x01);
        mem.write8(0xffff, 0x01);

        let cycles = cpu.step(&mut mem);

        assert!(!cpu.halted);
        assert_eq!(cpu.pc, 0x0040);
        assert_eq!(cycles, 20);
    }

    #[test]
    fn ei_enables_ime_after_one_instruction_delay() {
        let mut cpu = Cpu::new();
        let mut mem = Memory::new();
        mem.load(0x0000, &[0xfb, 0x00, 0x00]); // EI; NOP; NOP

        assert!(!cpu.ime);

        cpu.step(&mut mem); // EI
        assert!(!cpu.ime, "IME should not be set immediately after EI");

        cpu.step(&mut mem); // NOP, IME still 0 during this instruction
        assert!(
            cpu.ime,
            "IME should be set after the instruction following EI"
        );
    }

    #[test]
    fn di_immediately_clears_ime_and_cancels_pending_ei() {
        let mut cpu = Cpu::new();
        let mut mem = Memory::new();
        cpu.ime = true;
        mem.load(0x0000, &[0xfb, 0xf3]); // EI; DI

        cpu.step(&mut mem); // EI
        cpu.step(&mut mem); // DI

        assert!(!cpu.ime);
        // EI's pending delay should be canceled by DI; one more step shouldn't re-enable.
        mem.load(0x0002, &[0x00]); // NOP
        cpu.step(&mut mem);
        assert!(!cpu.ime);
    }

    #[test]
    fn reti_pops_pc_and_enables_ime_immediately() {
        let mut cpu = Cpu::new();
        let mut mem = Memory::new();
        cpu.sp = 0xfffc;
        mem.write8(0xfffc, 0x34); // low byte
        mem.write8(0xfffd, 0x12); // high byte
        mem.load(0x0000, &[0xd9]); // RETI

        let cycles = cpu.step(&mut mem);

        assert_eq!(cpu.pc, 0x1234);
        assert_eq!(cpu.sp, 0xfffe);
        assert!(cpu.ime);
        assert_eq!(cycles, 16);
    }

    #[test]
    fn illegal_opcodes_lock_cpu() {
        for opcode in [
            0xd3_u8, 0xdb, 0xdd, 0xe3, 0xe4, 0xeb, 0xec, 0xed, 0xf4, 0xfc, 0xfd,
        ] {
            let mut cpu = Cpu::new();
            let mut mem = Memory::new();
            mem.load(0x0000, &[opcode]);

            let cycles = cpu.step(&mut mem);
            assert!(cpu.locked, "opcode {:#04x} should lock", opcode);
            assert_eq!(cycles, 4, "opcode {:#04x}", opcode);

            // Subsequent steps return 4 with PC frozen
            let pc_at_lock = cpu.pc;
            assert_eq!(cpu.step(&mut mem), 4, "opcode {:#04x}", opcode);
            assert_eq!(cpu.pc, pc_at_lock, "opcode {:#04x}: PC must freeze", opcode);
        }
    }

    #[test]
    fn locked_cpu_ignores_pending_interrupts() {
        let mut cpu = Cpu::new();
        let mut mem = Memory::new();
        cpu.locked = true;
        cpu.ime = true;
        cpu.pc = 0x1000;
        cpu.sp = 0xfffe;
        mem.write8(0xff0f, 0x01); // VBlank pending
        mem.write8(0xffff, 0x01); // VBlank enabled

        let cycles = cpu.step(&mut mem);

        assert_eq!(cycles, 4);
        assert_eq!(cpu.pc, 0x1000); // no jump to vector
        assert!(cpu.ime); // IME unchanged
        assert_eq!(mem.read8(0xff0f), 0x01); // IF unchanged
        assert_eq!(cpu.sp, 0xfffe); // no push
    }

    #[test]
    fn ld_indirect_nn_sp_writes_little_endian() {
        let mut cpu = Cpu::new();
        let mut mem = Memory::new();
        cpu.sp = 0x1234;
        mem.load(0x0000, &[0x08, 0x00, 0xc0]); // LD (0xC000), SP

        let cycles = cpu.step(&mut mem);

        assert_eq!(mem.read8(0xc000), 0x34); // low byte
        assert_eq!(mem.read8(0xc001), 0x12); // high byte
        assert_eq!(cycles, 20);
    }

    #[test]
    fn add_sp_e_positive_offset_half_carry() {
        let mut cpu = Cpu::new();
        let mut mem = Memory::new();
        cpu.sp = 0x000f;
        cpu.f.set_z(true); // Z should be cleared
        mem.load(0x0000, &[0xe8, 0x01]); // ADD SP, 1

        let cycles = cpu.step(&mut mem);

        assert_eq!(cpu.sp, 0x0010);
        assert!(!cpu.f.z());
        assert!(!cpu.f.n());
        assert!(cpu.f.h()); // (0x0f & 0xf) + (0x01 & 0xf) = 0x10 > 0xf
        assert!(!cpu.f.c());
        assert_eq!(cycles, 16);
    }

    #[test]
    fn add_sp_e_positive_offset_byte_carry() {
        let mut cpu = Cpu::new();
        let mut mem = Memory::new();
        cpu.sp = 0x00ff;
        mem.load(0x0000, &[0xe8, 0x01]); // ADD SP, 1

        cpu.step(&mut mem);

        assert_eq!(cpu.sp, 0x0100);
        assert!(cpu.f.h());
        assert!(cpu.f.c()); // 0xff + 0x01 > 0xff
    }

    #[test]
    fn add_sp_e_negative_offset_with_carry_from_low_byte_unsigned_add() {
        let mut cpu = Cpu::new();
        let mut mem = Memory::new();
        cpu.sp = 0x0010;
        mem.load(0x0000, &[0xe8, 0xff]); // ADD SP, -1

        cpu.step(&mut mem);

        assert_eq!(cpu.sp, 0x000f);
        // Flag math uses e as unsigned 8-bit:
        // H: (0x10 & 0xf) + (0xff & 0xf) = 0x0 + 0xf = 0xf, NOT > 0xf, so H = 0
        assert!(!cpu.f.h());
        // C: 0x10 + 0xff = 0x10f > 0xff, so C = 1
        assert!(cpu.f.c());
    }

    #[test]
    fn ld_hl_sp_e_does_not_modify_sp() {
        let mut cpu = Cpu::new();
        let mut mem = Memory::new();
        cpu.sp = 0x0010;
        mem.load(0x0000, &[0xf8, 0x02]); // LD HL, SP+2

        let cycles = cpu.step(&mut mem);

        assert_eq!(cpu.hl(), 0x0012);
        assert_eq!(cpu.sp, 0x0010);
        assert!(!cpu.f.z());
        assert!(!cpu.f.n());
        assert_eq!(cycles, 12);
    }

    #[test]
    fn ld_sp_hl_copies_hl_to_sp() {
        let mut cpu = Cpu::new();
        let mut mem = Memory::new();
        cpu.set_hl(0x1234);
        mem.load(0x0000, &[0xf9]); // LD SP, HL

        let cycles = cpu.step(&mut mem);

        assert_eq!(cpu.sp, 0x1234);
        assert_eq!(cycles, 8);
    }

    #[test]
    fn ldh_n_a_and_ldh_a_n_use_ff00_offset() {
        let mut cpu = Cpu::new();
        let mut mem = Memory::new();
        cpu.a = 0xab;
        mem.load(0x0000, &[0xe0, 0x40]); // LDH (0x40), A → 0xFF40

        let cycles = cpu.step(&mut mem);
        assert_eq!(mem.read8(0xff40), 0xab);
        assert_eq!(cycles, 12);
        assert_eq!(cpu.pc, 0x0002);

        cpu.a = 0;
        mem.write8(0xff40, 0x66);
        mem.load(0x0002, &[0xf0, 0x40]); // LDH A, (0x40)
        let cycles = cpu.step(&mut mem);
        assert_eq!(cpu.a, 0x66);
        assert_eq!(cycles, 12);
    }

    #[test]
    fn ld_indirect_c_uses_ff00_plus_c_register() {
        let mut cpu = Cpu::new();
        let mut mem = Memory::new();
        cpu.a = 0xab;
        cpu.c = 0x40;
        mem.load(0x0000, &[0xe2]); // LD (C), A → 0xFF40

        let cycles = cpu.step(&mut mem);
        assert_eq!(mem.read8(0xff40), 0xab);
        assert_eq!(cycles, 8);

        cpu.a = 0;
        mem.write8(0xff40, 0x66);
        mem.load(0x0001, &[0xf2]); // LD A, (C)
        let cycles = cpu.step(&mut mem);
        assert_eq!(cpu.a, 0x66);
        assert_eq!(cycles, 8);
    }

    #[test]
    fn ld_indirect_nn_roundtrips_through_absolute_address() {
        let mut cpu = Cpu::new();
        let mut mem = Memory::new();
        cpu.a = 0xab;
        mem.load(0x0000, &[0xea, 0x00, 0xc0]); // LD (0xC000), A

        let cycles = cpu.step(&mut mem);
        assert_eq!(mem.read8(0xc000), 0xab);
        assert_eq!(cycles, 16);
        assert_eq!(cpu.pc, 0x0003);

        cpu.a = 0;
        mem.write8(0xc000, 0x66);
        mem.load(0x0003, &[0xfa, 0x00, 0xc0]); // LD A, (0xC000)
        let cycles = cpu.step(&mut mem);
        assert_eq!(cpu.a, 0x66);
        assert_eq!(cycles, 16);
    }

    #[test]
    fn call_nn_pushes_next_pc_and_jumps() {
        let mut cpu = Cpu::new();
        let mut mem = Memory::new();
        cpu.pc = 0x1000;
        cpu.sp = 0xfffe;
        mem.write8(0x1000, 0xcd);
        mem.write8(0x1001, 0x34);
        mem.write8(0x1002, 0x12); // CALL 0x1234

        let cycles = cpu.step(&mut mem);

        assert_eq!(cpu.pc, 0x1234);
        assert_eq!(cpu.sp, 0xfffc);
        assert_eq!(mem.read8(0xfffc), 0x03); // PC was 0x1003 after fetch
        assert_eq!(mem.read8(0xfffd), 0x10);
        assert_eq!(cycles, 24);
    }

    #[test]
    fn call_cc_taken_and_not_taken_have_different_cycles() {
        // not taken
        let mut cpu = Cpu::new();
        let mut mem = Memory::new();
        cpu.pc = 0x1000;
        cpu.sp = 0xfffe;
        cpu.f.set_z(false);
        mem.write8(0x1000, 0xcc); // CALL Z, 0x1234 (not taken)
        mem.write8(0x1001, 0x34);
        mem.write8(0x1002, 0x12);
        let cycles = cpu.step(&mut mem);
        assert_eq!(cpu.pc, 0x1003);
        assert_eq!(cpu.sp, 0xfffe); // SP unchanged
        assert_eq!(cycles, 12);

        // taken
        let mut cpu = Cpu::new();
        let mut mem = Memory::new();
        cpu.pc = 0x1000;
        cpu.sp = 0xfffe;
        cpu.f.set_z(true);
        mem.write8(0x1000, 0xcc);
        mem.write8(0x1001, 0x34);
        mem.write8(0x1002, 0x12);
        let cycles = cpu.step(&mut mem);
        assert_eq!(cpu.pc, 0x1234);
        assert_eq!(cpu.sp, 0xfffc);
        assert_eq!(cycles, 24);
    }

    #[test]
    fn ret_pops_pc() {
        let mut cpu = Cpu::new();
        let mut mem = Memory::new();
        cpu.pc = 0x1000;
        cpu.sp = 0xfffc;
        mem.write8(0xfffc, 0x34); // low
        mem.write8(0xfffd, 0x12); // high
        mem.write8(0x1000, 0xc9); // RET

        let cycles = cpu.step(&mut mem);

        assert_eq!(cpu.pc, 0x1234);
        assert_eq!(cpu.sp, 0xfffe);
        assert_eq!(cycles, 16);
    }

    #[test]
    fn ret_cc_taken_takes_20_cycles_not_taken_takes_8() {
        // not taken
        let mut cpu = Cpu::new();
        let mut mem = Memory::new();
        cpu.pc = 0x1000;
        cpu.sp = 0xfffc;
        cpu.f.set_z(false);
        mem.write8(0xfffc, 0x34);
        mem.write8(0xfffd, 0x12);
        mem.write8(0x1000, 0xc8); // RET Z (not taken)
        let cycles = cpu.step(&mut mem);
        assert_eq!(cpu.pc, 0x1001); // just past RET cc
        assert_eq!(cpu.sp, 0xfffc); // SP unchanged
        assert_eq!(cycles, 8);

        // taken
        let mut cpu = Cpu::new();
        let mut mem = Memory::new();
        cpu.pc = 0x1000;
        cpu.sp = 0xfffc;
        cpu.f.set_z(true);
        mem.write8(0xfffc, 0x34);
        mem.write8(0xfffd, 0x12);
        mem.write8(0x1000, 0xc8);
        let cycles = cpu.step(&mut mem);
        assert_eq!(cpu.pc, 0x1234);
        assert_eq!(cpu.sp, 0xfffe);
        assert_eq!(cycles, 20);
    }

    #[test]
    fn rst_pushes_pc_and_jumps_to_vector_for_all_8_opcodes() {
        for (opcode, vector) in [
            (0xc7_u8, 0x0000_u16),
            (0xcf, 0x0008),
            (0xd7, 0x0010),
            (0xdf, 0x0018),
            (0xe7, 0x0020),
            (0xef, 0x0028),
            (0xf7, 0x0030),
            (0xff, 0x0038),
        ] {
            let mut cpu = Cpu::new();
            let mut mem = Memory::new();
            cpu.pc = 0x1000;
            cpu.sp = 0xfffe;
            mem.write8(0x1000, opcode);

            let cycles = cpu.step(&mut mem);

            assert_eq!(cpu.pc, vector, "opcode {:#04x}", opcode);
            assert_eq!(cpu.sp, 0xfffc, "opcode {:#04x}", opcode);
            assert_eq!(mem.read8(0xfffc), 0x01, "opcode {:#04x}", opcode);
            assert_eq!(mem.read8(0xfffd), 0x10, "opcode {:#04x}", opcode);
            assert_eq!(cycles, 16, "opcode {:#04x}", opcode);
        }
    }

    #[test]
    fn call_then_ret_roundtrips() {
        let mut cpu = Cpu::new();
        let mut mem = Memory::new();
        cpu.pc = 0x1000;
        cpu.sp = 0xfffe;
        mem.write8(0x1000, 0xcd); // CALL 0x2000
        mem.write8(0x1001, 0x00);
        mem.write8(0x1002, 0x20);
        mem.write8(0x2000, 0xc9); // RET

        cpu.step(&mut mem); // CALL
        assert_eq!(cpu.pc, 0x2000);
        cpu.step(&mut mem); // RET
        assert_eq!(cpu.pc, 0x1003);
        assert_eq!(cpu.sp, 0xfffe);
    }

    #[test]
    fn jr_e_positive_offset() {
        let mut cpu = Cpu::new();
        let mut mem = Memory::new();
        mem.load(0x0000, &[0x18, 0x05]); // JR +5

        let cycles = cpu.step(&mut mem);

        assert_eq!(cpu.pc, 0x0007); // 0x0002 (after fetch) + 5
        assert_eq!(cycles, 12);
    }

    #[test]
    fn jr_e_negative_offset_to_self() {
        let mut cpu = Cpu::new();
        let mut mem = Memory::new();
        mem.load(0x0000, &[0x18, 0xfe]); // JR -2  (loop back to itself)

        cpu.step(&mut mem);

        assert_eq!(cpu.pc, 0x0000);
    }

    #[test]
    fn jr_cc_taken_when_condition_true() {
        let mut cpu = Cpu::new();
        let mut mem = Memory::new();
        cpu.f.set_z(true);
        mem.load(0x0000, &[0x28, 0x10]); // JR Z, +0x10

        let cycles = cpu.step(&mut mem);

        assert_eq!(cpu.pc, 0x0012);
        assert_eq!(cycles, 12);
    }

    #[test]
    fn jr_cc_not_taken_when_condition_false_advances_past_operand() {
        let mut cpu = Cpu::new();
        let mut mem = Memory::new();
        cpu.f.set_z(false); // Z clear
        mem.load(0x0000, &[0x28, 0x10]); // JR Z, +0x10 (not taken)

        let cycles = cpu.step(&mut mem);

        assert_eq!(cpu.pc, 0x0002); // advanced past operand but no jump
        assert_eq!(cycles, 8);
    }

    #[test]
    fn jr_cc_covers_all_four_conditions() {
        // each row sets flags such that the condition is taken
        for (opcode, z, c) in [
            (0x20_u8, false, false), // JR NZ
            (0x28, true, false),     // JR Z
            (0x30, false, false),    // JR NC
            (0x38, false, true),     // JR C
        ] {
            let mut cpu = Cpu::new();
            let mut mem = Memory::new();
            cpu.f.set_z(z);
            cpu.f.set_c(c);
            mem.load(0x0000, &[opcode, 0x10]);

            assert_eq!(cpu.step(&mut mem), 12, "opcode {:#04x}", opcode);
            assert_eq!(cpu.pc, 0x0012, "opcode {:#04x}", opcode);
        }
    }

    #[test]
    fn jp_nn_jumps_absolute() {
        let mut cpu = Cpu::new();
        let mut mem = Memory::new();
        mem.load(0x0000, &[0xc3, 0x34, 0x12]); // JP 0x1234

        let cycles = cpu.step(&mut mem);

        assert_eq!(cpu.pc, 0x1234);
        assert_eq!(cycles, 16);
    }

    #[test]
    fn jp_cc_taken_and_not_taken_have_different_cycles() {
        // not taken
        let mut cpu = Cpu::new();
        let mut mem = Memory::new();
        cpu.f.set_z(false);
        mem.load(0x0000, &[0xca, 0x34, 0x12]); // JP Z, 0x1234 (not taken)
        let cycles = cpu.step(&mut mem);
        assert_eq!(cpu.pc, 0x0003);
        assert_eq!(cycles, 12);

        // taken
        let mut cpu = Cpu::new();
        let mut mem = Memory::new();
        cpu.f.set_z(true);
        mem.load(0x0000, &[0xca, 0x34, 0x12]); // JP Z, 0x1234 (taken)
        let cycles = cpu.step(&mut mem);
        assert_eq!(cpu.pc, 0x1234);
        assert_eq!(cycles, 16);
    }

    #[test]
    fn jp_hl_uses_hl_as_pc() {
        let mut cpu = Cpu::new();
        let mut mem = Memory::new();
        cpu.set_hl(0x4321);
        mem.load(0x0000, &[0xe9]); // JP HL

        let cycles = cpu.step(&mut mem);

        assert_eq!(cpu.pc, 0x4321);
        assert_eq!(cycles, 4);
    }

    #[test]
    fn add_hl_dispatches_to_all_pairs() {
        // ADD HL, BC
        let mut cpu = Cpu::new();
        let mut mem = Memory::new();
        cpu.set_hl(0x0100);
        cpu.set_bc(0x0023);
        mem.load(0x0000, &[0x09]);
        assert_eq!(cpu.step(&mut mem), 8);
        assert_eq!(cpu.hl(), 0x0123);

        // ADD HL, DE
        cpu.set_hl(0x0100);
        cpu.set_de(0x0023);
        cpu.pc = 0;
        mem.load(0x0000, &[0x19]);
        cpu.step(&mut mem);
        assert_eq!(cpu.hl(), 0x0123);

        // ADD HL, HL (doubling)
        cpu.set_hl(0x1234);
        cpu.pc = 0;
        mem.load(0x0000, &[0x29]);
        cpu.step(&mut mem);
        assert_eq!(cpu.hl(), 0x2468);

        // ADD HL, SP
        cpu.set_hl(0x0100);
        cpu.sp = 0x0023;
        cpu.pc = 0;
        mem.load(0x0000, &[0x39]);
        cpu.step(&mut mem);
        assert_eq!(cpu.hl(), 0x0123);
    }

    #[test]
    fn add_hl_sets_half_carry_on_bit_11_overflow() {
        let mut cpu = Cpu::new();
        let mut mem = Memory::new();
        cpu.set_hl(0x0fff);
        cpu.set_bc(0x0001);
        mem.load(0x0000, &[0x09]); // ADD HL, BC

        cpu.step(&mut mem);

        assert_eq!(cpu.hl(), 0x1000);
        assert!(!cpu.f.n());
        assert!(cpu.f.h());
        assert!(!cpu.f.c());
    }

    #[test]
    fn add_hl_sets_carry_on_bit_15_overflow() {
        let mut cpu = Cpu::new();
        let mut mem = Memory::new();
        cpu.set_hl(0xffff);
        cpu.set_bc(0x0001);
        mem.load(0x0000, &[0x09]); // ADD HL, BC

        cpu.step(&mut mem);

        assert_eq!(cpu.hl(), 0x0000);
        assert!(!cpu.f.n());
        assert!(cpu.f.h()); // 0xFFF + 1 carries from bit 11
        assert!(cpu.f.c());
    }

    #[test]
    fn add_hl_preserves_z_and_clears_n() {
        let mut cpu = Cpu::new();
        let mut mem = Memory::new();
        cpu.set_hl(0x0100);
        cpu.set_bc(0x0023);
        cpu.f.set_z(true);
        cpu.f.set_n(true);
        mem.load(0x0000, &[0x09]);

        cpu.step(&mut mem);

        assert!(cpu.f.z(), "Z must be preserved by ADD HL");
        assert!(!cpu.f.n());
    }

    #[test]
    fn ld_indirect_hl_n_writes_immediate_to_memory() {
        let mut cpu = Cpu::new();
        let mut mem = Memory::new();
        cpu.set_hl(0xc000);
        mem.load(0x0000, &[0x36, 0x42]); // LD (HL), 0x42

        let cycles = cpu.step(&mut mem);

        assert_eq!(cycles, 12);
        assert_eq!(mem.read8(0xc000), 0x42);
        assert_eq!(cpu.pc, 0x0002);
    }

    #[test]
    fn ld_indirect_bc_and_de_pair_roundtrip() {
        let mut cpu = Cpu::new();
        let mut mem = Memory::new();
        cpu.set_bc(0xc000);
        cpu.set_de(0xc001);
        cpu.a = 0xab;
        mem.load(0x0000, &[0x02, 0x12]); // LD (BC), A; LD (DE), A

        assert_eq!(cpu.step(&mut mem), 8);
        assert_eq!(cpu.step(&mut mem), 8);

        assert_eq!(mem.read8(0xc000), 0xab);
        assert_eq!(mem.read8(0xc001), 0xab);

        // Reads
        mem.write8(0xc000, 0x11);
        mem.write8(0xc001, 0x22);
        mem.load(0x0002, &[0x0a, 0x1a]); // LD A, (BC); LD A, (DE)

        cpu.step(&mut mem);
        assert_eq!(cpu.a, 0x11);
        cpu.step(&mut mem);
        assert_eq!(cpu.a, 0x22);
    }

    #[test]
    fn ld_indirect_hl_inc_a_writes_then_increments_hl() {
        let mut cpu = Cpu::new();
        let mut mem = Memory::new();
        cpu.set_hl(0xc000);
        cpu.a = 0xab;
        mem.load(0x0000, &[0x22]); // LD (HL+), A

        let cycles = cpu.step(&mut mem);

        assert_eq!(cycles, 8);
        assert_eq!(mem.read8(0xc000), 0xab);
        assert_eq!(cpu.hl(), 0xc001);
    }

    #[test]
    fn ld_indirect_hl_dec_a_writes_then_decrements_hl() {
        let mut cpu = Cpu::new();
        let mut mem = Memory::new();
        cpu.set_hl(0xc000);
        cpu.a = 0xab;
        mem.load(0x0000, &[0x32]); // LD (HL-), A

        cpu.step(&mut mem);

        assert_eq!(mem.read8(0xc000), 0xab);
        assert_eq!(cpu.hl(), 0xbfff);
    }

    #[test]
    fn ld_a_indirect_hl_inc_reads_then_increments_hl() {
        let mut cpu = Cpu::new();
        let mut mem = Memory::new();
        cpu.set_hl(0xc000);
        mem.write8(0xc000, 0xab);
        mem.load(0x0000, &[0x2a]); // LD A, (HL+)

        cpu.step(&mut mem);

        assert_eq!(cpu.a, 0xab);
        assert_eq!(cpu.hl(), 0xc001);
    }

    #[test]
    fn ld_a_indirect_hl_dec_reads_then_decrements_hl() {
        let mut cpu = Cpu::new();
        let mut mem = Memory::new();
        cpu.set_hl(0xc000);
        mem.write8(0xc000, 0xab);
        mem.load(0x0000, &[0x3a]); // LD A, (HL-)

        cpu.step(&mut mem);

        assert_eq!(cpu.a, 0xab);
        assert_eq!(cpu.hl(), 0xbfff);
    }

    #[test]
    fn ld_indirect_pair_preserves_flags() {
        let mut cpu = Cpu::new();
        let mut mem = Memory::new();
        cpu.set_bc(0xc000);
        cpu.set_hl(0xc010);
        cpu.a = 0x42;
        cpu.f.set_z(true);
        cpu.f.set_n(true);
        cpu.f.set_h(true);
        cpu.f.set_c(true);
        mem.load(0x0000, &[0x02, 0x22, 0x32]); // LD (BC),A; LD (HL+),A; LD (HL-),A

        for _ in 0..3 {
            cpu.step(&mut mem);
        }

        assert!(cpu.f.z() && cpu.f.n() && cpu.f.h() && cpu.f.c());
    }

    #[test]
    fn inc_r_dispatches_to_all_8bit_registers() {
        for idx in 0..8_u8 {
            let opcode = 0x04 | (idx << 3); // 00 ddd 100
            let mut cpu = Cpu::new();
            let mut mem = Memory::new();
            cpu.set_hl(0xc000);
            cpu.f.set_c(true); // C must survive
            cpu.write_r(idx, 0x42, &mut mem);
            mem.load(0x0000, &[opcode]);

            let cycles = cpu.step(&mut mem);

            assert_eq!(cpu.read_r(idx, &mut mem), 0x43, "idx {idx}");
            assert!(!cpu.f.n(), "N must be 0 for INC, idx {idx}");
            assert!(cpu.f.c(), "C must be preserved, idx {idx}");
            let expected = if idx == 6 { 12 } else { 4 };
            assert_eq!(cycles, expected, "idx {idx}");
        }
    }

    #[test]
    fn dec_r_dispatches_to_all_8bit_registers() {
        for idx in 0..8_u8 {
            let opcode = 0x05 | (idx << 3); // 00 ddd 101
            let mut cpu = Cpu::new();
            let mut mem = Memory::new();
            cpu.set_hl(0xc000);
            cpu.f.set_c(true);
            cpu.write_r(idx, 0x42, &mut mem);
            mem.load(0x0000, &[opcode]);

            let cycles = cpu.step(&mut mem);

            assert_eq!(cpu.read_r(idx, &mut mem), 0x41, "idx {idx}");
            assert!(cpu.f.n(), "N must be 1 for DEC, idx {idx}");
            assert!(cpu.f.c(), "C must be preserved, idx {idx}");
            let expected = if idx == 6 { 12 } else { 4 };
            assert_eq!(cycles, expected, "idx {idx}");
        }
    }

    #[test]
    fn inc_r_half_carry_on_low_nibble_overflow() {
        let mut cpu = Cpu::new();
        let mut mem = Memory::new();
        cpu.b = 0x0f;
        mem.load(0x0000, &[0x04]); // INC B

        cpu.step(&mut mem);

        assert_eq!(cpu.b, 0x10);
        assert!(!cpu.f.z());
        assert!(!cpu.f.n());
        assert!(cpu.f.h());
    }

    #[test]
    fn inc_r_wraps_to_zero_with_z_and_h() {
        let mut cpu = Cpu::new();
        let mut mem = Memory::new();
        cpu.b = 0xff;
        mem.load(0x0000, &[0x04]); // INC B

        cpu.step(&mut mem);

        assert_eq!(cpu.b, 0x00);
        assert!(cpu.f.z());
        assert!(cpu.f.h());
    }

    #[test]
    fn dec_r_half_carry_on_low_nibble_borrow() {
        let mut cpu = Cpu::new();
        let mut mem = Memory::new();
        cpu.b = 0x10;
        mem.load(0x0000, &[0x05]); // DEC B

        cpu.step(&mut mem);

        assert_eq!(cpu.b, 0x0f);
        assert!(cpu.f.n());
        assert!(cpu.f.h());
    }

    #[test]
    fn dec_r_wraps_from_zero_to_ff_with_h() {
        let mut cpu = Cpu::new();
        let mut mem = Memory::new();
        cpu.b = 0x00;
        mem.load(0x0000, &[0x05]); // DEC B

        cpu.step(&mut mem);

        assert_eq!(cpu.b, 0xff);
        assert!(!cpu.f.z());
        assert!(cpu.f.n());
        assert!(cpu.f.h());
    }

    #[test]
    fn inc_rr_increments_pairs_and_preserves_flags() {
        let mut cpu = Cpu::new();
        let mut mem = Memory::new();
        cpu.set_bc(0x1234);
        cpu.set_de(0x5678);
        cpu.set_hl(0x9abc);
        cpu.sp = 0xfffe;
        cpu.f.set_z(true); // every flag should survive
        cpu.f.set_n(true);
        cpu.f.set_h(true);
        cpu.f.set_c(true);
        mem.load(0x0000, &[0x03, 0x13, 0x23, 0x33]); // INC BC/DE/HL/SP

        for _ in 0..4 {
            assert_eq!(cpu.step(&mut mem), 8);
        }

        assert_eq!(cpu.bc(), 0x1235);
        assert_eq!(cpu.de(), 0x5679);
        assert_eq!(cpu.hl(), 0x9abd);
        assert_eq!(cpu.sp, 0xffff);
        assert!(cpu.f.z() && cpu.f.n() && cpu.f.h() && cpu.f.c());
    }

    #[test]
    fn dec_rr_decrements_pairs_and_preserves_flags() {
        let mut cpu = Cpu::new();
        let mut mem = Memory::new();
        cpu.set_bc(0x1234);
        cpu.set_de(0x5678);
        cpu.set_hl(0x9abc);
        cpu.sp = 0xfffe;
        cpu.f.set_z(true);
        cpu.f.set_n(true);
        cpu.f.set_h(true);
        cpu.f.set_c(true);
        mem.load(0x0000, &[0x0b, 0x1b, 0x2b, 0x3b]); // DEC BC/DE/HL/SP

        for _ in 0..4 {
            assert_eq!(cpu.step(&mut mem), 8);
        }

        assert_eq!(cpu.bc(), 0x1233);
        assert_eq!(cpu.de(), 0x5677);
        assert_eq!(cpu.hl(), 0x9abb);
        assert_eq!(cpu.sp, 0xfffd);
        assert!(cpu.f.z() && cpu.f.n() && cpu.f.h() && cpu.f.c());
    }

    #[test]
    fn alu_immediate_all_opcodes_dispatch_correctly() {
        // Each opcode fetches its immediate and delegates to the matching ALU helper.
        for (opcode, imm, expected_a) in [
            (0xc6_u8, 0x05_u8, 0x15_u8), // ADD A, 0x05
            (0xce, 0x05, 0x15),          // ADC A, 0x05 (no carry-in)
            (0xd6, 0x05, 0x0b),          // SUB A, 0x05
            (0xde, 0x05, 0x0b),          // SBC A, 0x05 (no borrow-in)
            (0xe6, 0x0f, 0x00),          // AND A, 0x0F
            (0xee, 0x05, 0x15),          // XOR A, 0x05
            (0xf6, 0x05, 0x15),          // OR  A, 0x05
        ] {
            let mut cpu = Cpu::new();
            let mut mem = Memory::new();
            cpu.a = 0x10;
            mem.load(0x0000, &[opcode, imm]);

            let cycles = cpu.step(&mut mem);

            assert_eq!(cpu.a, expected_a, "opcode {:#04x}", opcode);
            assert_eq!(cycles, 8, "opcode {:#04x}", opcode);
            assert_eq!(cpu.pc, 0x0002, "opcode {:#04x}", opcode);
        }
    }

    #[test]
    fn cp_a_n_does_not_modify_a() {
        let mut cpu = Cpu::new();
        let mut mem = Memory::new();
        cpu.a = 0x42;
        mem.load(0x0000, &[0xfe, 0x40]); // CP A, 0x40

        let cycles = cpu.step(&mut mem);

        assert_eq!(cpu.a, 0x42);
        assert!(cpu.f.n());
        assert!(!cpu.f.z());
        assert!(!cpu.f.c());
        assert_eq!(cycles, 8);
    }

    #[test]
    fn adc_a_n_uses_carry_in() {
        let mut cpu = Cpu::new();
        let mut mem = Memory::new();
        cpu.a = 0x10;
        cpu.f.set_c(true);
        mem.load(0x0000, &[0xce, 0x20]); // ADC A, 0x20

        cpu.step(&mut mem);

        assert_eq!(cpu.a, 0x31);
    }

    #[test]
    fn sbc_a_n_uses_borrow_in() {
        let mut cpu = Cpu::new();
        let mut mem = Memory::new();
        cpu.a = 0x10;
        cpu.f.set_c(true);
        mem.load(0x0000, &[0xde, 0x01]); // SBC A, 0x01

        cpu.step(&mut mem);

        assert_eq!(cpu.a, 0x0e); // 0x10 - 0x01 - 0x01 = 0x0e
        assert!(cpu.f.n());
    }

    #[test]
    fn rlca_keeps_z_clear_even_when_result_is_zero() {
        let mut cpu = Cpu::new();
        let mut mem = Memory::new();
        cpu.a = 0x80;
        mem.load(0x0000, &[0x07]); // RLCA

        let cycles = cpu.step(&mut mem);

        assert_eq!(cpu.a, 0x01);
        assert!(!cpu.f.z()); // crucial: Z must be 0 (CB-prefix RLC A would set Z here)
        assert!(cpu.f.c());
        assert_eq!(cycles, 4);
    }

    #[test]
    fn rrca_rotates_and_clears_z() {
        let mut cpu = Cpu::new();
        let mut mem = Memory::new();
        cpu.a = 0x01;
        mem.load(0x0000, &[0x0f]); // RRCA

        cpu.step(&mut mem);

        assert_eq!(cpu.a, 0x80);
        assert!(!cpu.f.z());
        assert!(cpu.f.c());
    }

    #[test]
    fn rla_uses_carry_in_and_clears_z() {
        let mut cpu = Cpu::new();
        let mut mem = Memory::new();
        cpu.a = 0x80;
        cpu.f.set_c(false);
        mem.load(0x0000, &[0x17]); // RLA

        cpu.step(&mut mem);

        assert_eq!(cpu.a, 0x00); // 0x80 << 1 with carry-in 0
        assert!(!cpu.f.z()); // Z must be 0 even when result is 0
        assert!(cpu.f.c()); // old bit 7
    }

    #[test]
    fn rra_uses_carry_in_and_clears_z() {
        let mut cpu = Cpu::new();
        let mut mem = Memory::new();
        cpu.a = 0x01;
        cpu.f.set_c(true);
        mem.load(0x0000, &[0x1f]); // RRA

        cpu.step(&mut mem);

        assert_eq!(cpu.a, 0x80); // carry-in 1 to bit 7
        assert!(!cpu.f.z());
        assert!(cpu.f.c()); // old bit 0
    }

    #[test]
    fn daa_after_bcd_add() {
        let mut cpu = Cpu::new();
        let mut mem = Memory::new();
        cpu.a = 0x45;
        cpu.b = 0x38;
        mem.load(0x0000, &[0x80, 0x27]); // ADD A, B; DAA

        cpu.step(&mut mem); // ADD: A = 0x7D
        assert_eq!(cpu.a, 0x7d);

        cpu.step(&mut mem); // DAA: should adjust to 0x83 (45 + 38 = 83 in BCD)
        assert_eq!(cpu.a, 0x83);
        assert!(!cpu.f.c());
        assert!(!cpu.f.h());
    }

    #[test]
    fn daa_after_bcd_sub() {
        let mut cpu = Cpu::new();
        let mut mem = Memory::new();
        cpu.a = 0x47;
        cpu.b = 0x28;
        mem.load(0x0000, &[0x90, 0x27]); // SUB B; DAA

        cpu.step(&mut mem); // SUB: A = 0x1F, N=1, H=1
        cpu.step(&mut mem); // DAA: 47 - 28 = 19 in BCD

        assert_eq!(cpu.a, 0x19);
        assert!(cpu.f.n());
    }

    #[test]
    fn cpl_inverts_a_and_sets_n_h() {
        let mut cpu = Cpu::new();
        let mut mem = Memory::new();
        cpu.a = 0xaa;
        mem.load(0x0000, &[0x2f]); // CPL

        cpu.step(&mut mem);

        assert_eq!(cpu.a, 0x55);
        assert!(cpu.f.n());
        assert!(cpu.f.h());
    }

    #[test]
    fn scf_sets_carry_and_clears_n_h() {
        let mut cpu = Cpu::new();
        let mut mem = Memory::new();
        cpu.f.set_z(true); // Z should be preserved
        cpu.f.set_n(true);
        cpu.f.set_h(true);
        mem.load(0x0000, &[0x37]); // SCF

        cpu.step(&mut mem);

        assert!(cpu.f.z()); // preserved
        assert!(!cpu.f.n());
        assert!(!cpu.f.h());
        assert!(cpu.f.c());
    }

    #[test]
    fn ccf_toggles_carry() {
        let mut cpu = Cpu::new();
        let mut mem = Memory::new();
        cpu.f.set_c(true);
        cpu.f.set_n(true);
        cpu.f.set_h(true);
        mem.load(0x0000, &[0x3f, 0x3f]); // CCF; CCF

        cpu.step(&mut mem);
        assert!(!cpu.f.c());
        assert!(!cpu.f.n());
        assert!(!cpu.f.h());

        cpu.step(&mut mem);
        assert!(cpu.f.c());
    }

    #[test]
    fn ld_rr_nn_loads_all_pairs() {
        let mut cpu = Cpu::new();
        let mut mem = Memory::new();
        mem.load(
            0x0000,
            &[
                0x01, 0x34, 0x12, // LD BC, 0x1234
                0x11, 0x78, 0x56, // LD DE, 0x5678
                0x21, 0xbc, 0x9a, // LD HL, 0x9ABC
                0x31, 0xfe, 0xff, // LD SP, 0xFFFE
            ],
        );

        for _ in 0..4 {
            assert_eq!(cpu.step(&mut mem), 12);
        }

        assert_eq!(cpu.bc(), 0x1234);
        assert_eq!(cpu.de(), 0x5678);
        assert_eq!(cpu.hl(), 0x9abc);
        assert_eq!(cpu.sp, 0xfffe);
        assert_eq!(cpu.pc, 0x000c);
    }

    #[test]
    fn push_then_pop_roundtrips_register_pair() {
        let mut cpu = Cpu::new();
        let mut mem = Memory::new();
        cpu.sp = 0xfffe;
        cpu.set_bc(0xabcd);
        cpu.set_de(0x1234);
        mem.load(0x0000, &[0xc5, 0xd1]); // PUSH BC; POP DE

        assert_eq!(cpu.step(&mut mem), 16); // PUSH BC
        assert_eq!(cpu.sp, 0xfffc);
        assert_eq!(mem.read8(0xfffc), 0xcd); // low at SP
        assert_eq!(mem.read8(0xfffd), 0xab); // high at SP+1

        assert_eq!(cpu.step(&mut mem), 12); // POP DE
        assert_eq!(cpu.sp, 0xfffe);
        assert_eq!(cpu.de(), 0xabcd);
    }

    #[test]
    fn pop_af_masks_lower_nibble_of_f() {
        let mut cpu = Cpu::new();
        let mut mem = Memory::new();
        cpu.sp = 0xfffc;
        mem.write8(0xfffc, 0xff); // F = 0xFF on the stack
        mem.write8(0xfffd, 0x42); // A = 0x42
        mem.load(0x0000, &[0xf1]); // POP AF

        cpu.step(&mut mem);

        assert_eq!(cpu.a, 0x42);
        assert_eq!(cpu.f.bits(), 0xf0); // low nibble masked
        assert_eq!(cpu.sp, 0xfffe);
    }

    #[test]
    fn push_af_writes_f_with_low_nibble_zero() {
        let mut cpu = Cpu::new();
        let mut mem = Memory::new();
        cpu.sp = 0xfffe;
        cpu.a = 0x12;
        cpu.f.set_z(true);
        cpu.f.set_c(true);
        mem.load(0x0000, &[0xf5]); // PUSH AF

        cpu.step(&mut mem);

        assert_eq!(cpu.sp, 0xfffc);
        assert_eq!(mem.read8(0xfffc), 0x90); // F = Z+C = 0x90
        assert_eq!(mem.read8(0xfffd), 0x12); // A
    }

    #[test]
    fn halt_bug_repeats_next_opcode_byte() {
        let mut cpu = Cpu::new();
        let mut mem = Memory::new();
        cpu.pc = 0x1000;
        cpu.ime = false;
        mem.write8(0xff0f, 0x01); // pending, but IME=0
        mem.write8(0xffff, 0x01);
        mem.write8(0x1000, 0x76); // HALT
        mem.write8(0x1001, 0x00); // NOP

        // HALT triggers the bug: not halted, halt_bug latched, PC advanced past HALT
        let cycles = cpu.step(&mut mem);
        assert!(!cpu.halted);
        assert!(cpu.halt_bug);
        assert_eq!(cpu.pc, 0x1001);
        assert_eq!(cycles, 4);

        // Next fetch reads opcode at 0x1001 but PC stays (bug consumed)
        cpu.step(&mut mem);
        assert!(!cpu.halt_bug);
        assert_eq!(cpu.pc, 0x1001);

        // Subsequent step advances normally
        cpu.step(&mut mem);
        assert_eq!(cpu.pc, 0x1002);
    }
}
