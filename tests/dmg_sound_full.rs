//! Headless regression for Blargg's combined `dmg_sound.gb`.

use formula::bus::Bus;
use formula::cartridge::load_cartridge;
use formula::cpu::Cpu;
use formula::flags::Flags;
use formula::mmu::Mmu;
use std::path::Path;

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
fn dmg_sound_full_reaches_pass_result() {
    let rom_path = Path::new("test-rom/gb-test-roms/dmg_sound/dmg_sound.gb");
    if !rom_path.exists() {
        eprintln!(
            "skipping combined dmg_sound regression: drop the ROM at {} to enable it",
            rom_path.display()
        );
        return;
    }

    let rom = std::fs::read(rom_path).expect("read dmg_sound.gb");
    let mut mmu = Mmu::new(load_cartridge(rom));
    let mut cpu = post_boot_cpu();
    mmu.write8(0xff40, 0x91);
    mmu.write8(0xff47, 0xfc);

    for _ in 0..48_000_000usize {
        cpu.step(&mut mmu);
        let signature_ready =
            mmu.read8(0xa001) == 0xde && mmu.read8(0xa002) == 0xb0 && mmu.read8(0xa003) == 0x61;
        if signature_ready && mmu.read8(0xa000) != 0x80 {
            break;
        }
    }

    assert_eq!(mmu.read8(0xa001), 0xde, "Blargg signature byte 0");
    assert_eq!(mmu.read8(0xa002), 0xb0, "Blargg signature byte 1");
    assert_eq!(mmu.read8(0xa003), 0x61, "Blargg signature byte 2");
    assert_eq!(mmu.read8(0xa000), 0x00, "final result should be PASS");
}
