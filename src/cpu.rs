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

    fn fetch8(&mut self, bus: &mut impl Bus) -> u8 {
        let byte = bus.read8(self.pc);
        self.pc = self.pc.wrapping_add(1);
        byte
    }

    fn read_r(&self, idx: u8, bus: &impl Bus) -> u8 {
        match idx & 7 {
            0 => self.b,
            1 => self.c,
            2 => self.d,
            3 => self.e,
            4 => self.h,
            5 => self.l,
            6 => bus.read8(self.hl()),
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
            6 => bus.write8(self.hl(), value),
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
                if src == 6 { 16 } else { 8 }
            }
            0x40..=0x7f => {
                let bit = (opcode >> 3) & 7;
                self.f.set_z(value & (1 << bit) == 0);
                self.f.set_n(false);
                self.f.set_h(true);
                if src == 6 { 12 } else { 8 }
            }
            0x80..=0xbf => {
                let bit = (opcode >> 3) & 7;
                self.write_r(src, value & !(1 << bit), bus);
                if src == 6 { 16 } else { 8 }
            }
            0xc0..=0xff => {
                let bit = (opcode >> 3) & 7;
                self.write_r(src, value | (1 << bit), bus);
                if src == 6 { 16 } else { 8 }
            }
        }
    }

    pub fn step(&mut self, bus: &mut impl Bus) -> u8 {
        if self.halted {
            return 4;
        }

        let opcode = self.fetch8(bus);
        match opcode {
            0x00 => 4,
            0x06 => {
                self.b = self.fetch8(bus);
                8
            }
            0x0e => {
                self.c = self.fetch8(bus);
                8
            }
            0x16 => {
                self.d = self.fetch8(bus);
                8
            }
            0x1e => {
                self.e = self.fetch8(bus);
                8
            }
            0x26 => {
                self.h = self.fetch8(bus);
                8
            }
            0x2e => {
                self.l = self.fetch8(bus);
                8
            }
            0x3e => {
                self.a = self.fetch8(bus);
                8
            }
            0x76 => {
                self.halted = true;
                4
            }
            0x40..=0x7f => {
                let dst = (opcode >> 3) & 7;
                let src = opcode & 7;
                let value = self.read_r(src, bus);
                self.write_r(dst, value, bus);
                if dst == 6 || src == 6 { 8 } else { 4 }
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
                if src == 6 { 8 } else { 4 }
            }
            0xcb => self.step_cb(bus),
            _ => panic!("unimplemented opcode: {:#04x}", opcode),
        }
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
                0x76,       // HALT
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
    #[should_panic(expected = "unimplemented opcode")]
    fn unimplemented_opcode_panics() {
        let mut cpu = Cpu::new();
        let mut mem = Memory::new();
        mem.load(0x0000, &[0xff]);

        cpu.step(&mut mem);
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

        assert_eq!(cpu.read_r(0, &mem), 0xb0); // B
        assert_eq!(cpu.read_r(1, &mem), 0xc0); // C
        assert_eq!(cpu.read_r(2, &mem), 0xd0); // D
        assert_eq!(cpu.read_r(3, &mem), 0xe0); // E
        assert_eq!(cpu.read_r(4, &mem), 0x40); // H
        assert_eq!(cpu.read_r(5, &mem), 0x50); // L
        assert_eq!(cpu.read_r(6, &mem), 0x66); // (HL)
        assert_eq!(cpu.read_r(7, &mem), 0xa0); // A
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
}
