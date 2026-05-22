//! End-to-end smoke test for the runner's serial path: a tiny MBC0 ROM
//! ships two bytes out via SB/SC, and we observe them through
//! `Mmu::drain_serial_output`. This is the M1a疎通 — proves the binary's
//! step loop and serial stub work before we add MBC1 + Blargg.

use formula::cartridge::Mbc0;
use formula::cpu::Cpu;
use formula::flags::Flags;
use formula::mmu::Mmu;

fn post_boot_cpu() -> Cpu {
    let mut cpu = Cpu::new();
    cpu.a = 0x01;
    cpu.f = Flags::from_bits(0xb0);
    cpu.b = 0x00;
    cpu.c = 0x13;
    cpu.d = 0x00;
    cpu.e = 0xd8;
    cpu.h = 0x01;
    cpu.l = 0x4d;
    cpu.sp = 0xfffe;
    cpu.pc = 0x0100;
    cpu
}

#[test]
fn rom_can_print_bytes_through_serial_link() {
    let mut rom = vec![0x00_u8; 0x8000];
    rom[0x0147] = 0x00; // MBC0

    // Program at 0x0100: ship 'A' then 'B' via SB/SC, then spin.
    let program: &[u8] = &[
        0x3e, 0x41, // LD A, 'A'
        0xe0, 0x01, // LDH ($FF01), A   ; SB = 'A'
        0x3e, 0x81, // LD A, $81
        0xe0, 0x02, // LDH ($FF02), A   ; SC = transfer start
        0x3e, 0x42, // LD A, 'B'
        0xe0, 0x01, // LDH ($FF01), A
        0x3e, 0x81, // LD A, $81
        0xe0, 0x02, // LDH ($FF02), A
        0x18, 0xfe, // JR -2            ; spin
    ];
    rom[0x0100..0x0100 + program.len()].copy_from_slice(program);

    let mut mmu = Mmu::new(Box::new(Mbc0::new(rom)));
    let mut cpu = post_boot_cpu();

    let mut output: Vec<u8> = Vec::new();
    for _ in 0..10_000 {
        let cycles = cpu.step(&mut mmu);
        mmu.tick(cycles);
        output.extend(mmu.drain_serial_output());
        if output == b"AB" {
            return;
        }
    }
    panic!("serial output never reached \"AB\", got {output:?}");
}
