//! End-to-end test: PPU enters VBlank, CPU services the interrupt at 0x40.
//!
//! Companion to `timer_drives_cpu.rs`. Where the Timer test triggers in
//! ~16 cycles, this one has to wait a full pre-VBlank stretch
//! (144 scanlines × 456 dots), so the CPU runs a tight `JR -2` loop at
//! 0x0000 to avoid reaching the ISR address by normal flow.
//!
//! TODO when more PPU interrupts land:
//! - Mirror this test for STAT (IF bit 1) once LY=LYC / mode-change
//!   interrupts can fire from the PPU.

use formula::bus::Bus;
use formula::cartridge::Mbc0;
use formula::cpu::Cpu;
use formula::mmu::Mmu;

fn build_mmu_with_program(setup: impl FnOnce(&mut [u8])) -> Mmu {
    let mut rom = vec![0x00_u8; 0x8000];
    rom[0x0147] = 0x00; // MBC0
    setup(&mut rom);
    Mmu::new(Box::new(Mbc0::new(rom)))
}

#[test]
fn vblank_jumps_cpu_to_vector_0x40() {
    let mut mmu = build_mmu_with_program(|rom| {
        // JR -2 at 0x0000: infinite tight loop, PC never advances past 0x0001.
        rom[0x0000] = 0x18;
        rom[0x0001] = 0xfe;
        // HALT at the VBlank vector — the only way the CPU reaches here is
        // via interrupt servicing.
        rom[0x0040] = 0x76;
    });

    let mut cpu = Cpu::new();
    cpu.sp = 0xfffe;
    cpu.ime = true;

    // Arm VBlank only.
    mmu.write8(0xffff, 0x01);

    // Drive until the ISR halts the CPU. Need ~5500 JR iterations
    // (12 cycles each) to cross 65664 PPU dots; 10000 is a safety net.
    for _ in 0..10000 {
        let cycles = cpu.step(&mut mmu);
        mmu.tick(cycles);
        if cpu.halted {
            break;
        }
    }

    assert!(cpu.halted, "CPU never halted inside the ISR");
    assert_eq!(cpu.pc, 0x0041, "PC should be just past HALT inside the ISR");

    // Service pushed the return address from inside the JR loop.
    assert_eq!(cpu.sp, 0xfffc);
    assert_eq!(mmu.read8(0xfffc), 0x00, "low byte of return PC");
    assert_eq!(mmu.read8(0xfffd), 0x00, "high byte of return PC");

    // Service cleared VBlank IF; IME was forced off.
    assert_eq!(mmu.read8(0xff0f) & 0x01, 0x00);
    assert!(!cpu.ime);

    // PPU should be sitting inside VBlank now (LY in 144..=153).
    let ly = mmu.read8(0xff44);
    assert!(
        (144..=153).contains(&ly),
        "expected LY in VBlank range, got {ly}"
    );
}
