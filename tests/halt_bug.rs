//! Headless regression for Blargg's `halt_bug.gb`.
//!
//! The ROM copies itself to WRAM, runs the full matrix, and leaves a small
//! result block behind. Locking that block gives us a deterministic CI check
//! without depending on a window server or screenshot diffing.

use formula::bus::Bus;
use formula::cartridge::load_cartridge;
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
fn blargg_halt_bug_rom_reaches_pass_signature() {
    let rom = std::fs::read("test-rom/gb-test-roms/halt_bug.gb").expect("read halt_bug.gb");
    let mut mmu = Mmu::new(load_cartridge(rom));
    let mut cpu = post_boot_cpu();

    // Match the same post-boot IO state as the runner.
    mmu.write8(0xff40, 0x91);
    mmu.write8(0xff47, 0xfc);

    for _ in 0..2_000_000usize {
        cpu.step(&mut mmu);
    }

    assert_eq!(mmu.read8(0xd800), 0x01, "ROM should report success");
    assert_eq!(mmu.read8(0xd880), 0xff, "result block sentinel");
    assert_eq!(mmu.read8(0xd881), 0x1d, "result text pointer low byte");
    assert_eq!(mmu.read8(0xd882), 0xc2, "result text pointer high byte");
}
