use crate::bus::Bus;

pub struct Cpu {
    pub a: u8,
    pub f: u8,
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
            f: 0,
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

    fn fetch8(&mut self, bus: &mut impl Bus) -> u8 {
        let byte = bus.read8(self.pc);
        self.pc = self.pc.wrapping_add(1);
        byte
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
}
