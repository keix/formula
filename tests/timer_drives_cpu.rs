//! End-to-end test: Timer fires across MMU, CPU services the interrupt.
//!
//! This is the smallest "system" that drives all three pieces together
//! (CPU instruction stream, MMU dispatch, Timer subsystem) without a
//! top-level GameBoy struct. Useful to lock the contract between them.
//!
//! TODO items to revisit when a top-level driver lands:
//! - Add a RETI variant: ISR ends with RETI and the CPU resumes the NOP run
//!   at the pushed return address with IME re-enabled. This exercises the
//!   full call/return shape, not just the "arrive at vector" half.
//! - Cover all four TAC clock-select rates through the system, not only 01.
//! - Tighten the TIMA assertion once we either disable the timer inside the
//!   ISR or compute the exact post-service value.
//! - Mirror this test for Joypad / LCD-STAT / Serial / VBlank once those
//!   subsystems can raise IF bits.

use formula::bus::Bus;
use formula::cartridge::Mbc0;
use formula::cpu::Cpu;
use formula::mmu::Mmu;

fn build_mmu_with_program(setup: impl FnOnce(&mut [u8])) -> Mmu {
    let mut rom = vec![0x00_u8; 0x8000]; // NOPs everywhere
    rom[0x0147] = 0x00; // MBC0
    setup(&mut rom);
    Mmu::new(Box::new(Mbc0::new(rom)))
}

#[test]
fn timer_overflow_jumps_cpu_to_vector_0x50() {
    // ROM: NOPs in the main flow, HALT at the Timer vector so we can detect arrival.
    let mut mmu = build_mmu_with_program(|rom| {
        rom[0x0050] = 0x76; // HALT inside the Timer ISR
    });

    // Pre-boot CPU state: stack ready, interrupts armed.
    let mut cpu = Cpu::new();
    cpu.sp = 0xfffe;
    cpu.ime = true;

    // Arm the Timer to overflow after 16 T-cycles (TAC clock select 01).
    mmu.write8(0xffff, 0x04); // IE: Timer enabled
    mmu.write8(0xff05, 0xff); // TIMA on the brink
    mmu.write8(0xff06, 0x00); // TMA
    mmu.write8(0xff07, 0x05); // TAC: enable, every 16 T-cycles

    // Drive the system until the ISR halts the CPU. The 1024-cycle ceiling
    // is a safety net; the contract is that HALT inside the ISR is reached
    // long before this. Replace with a deterministic bound once a top-level
    // driver tracks total elapsed cycles.
    for _ in 0..1024 {
        cpu.step(&mut mmu);
        if cpu.halted {
            break;
        }
    }

    // The HALT at 0x0050 should have stopped us inside the ISR.
    assert!(cpu.halted, "CPU never halted inside the ISR");
    assert_eq!(cpu.pc, 0x0051, "PC should be just past HALT inside the ISR");

    // TIMA overflows at 16 T-cycles, then spends 4 more T-cycles at 0x00
    // before the MMU raises IF. That lets the CPU complete one more NOP
    // before servicing the interrupt.
    assert_eq!(cpu.sp, 0xfffc);
    assert_eq!(mmu.read8(0xfffc), 0x05, "low byte of return PC on stack");
    assert_eq!(mmu.read8(0xfffd), 0x00, "high byte of return PC on stack");

    // Service cleared the Timer IF bit; IME was forced off.
    assert_eq!(mmu.read8(0xff0f) & 0x04, 0x00);
    assert!(!cpu.ime);

    // Timer kept ticking during service, so TIMA is no longer 0xff (it overflowed
    // and may have advanced past TMA). We don't assert an exact value; the
    // interrupt arrival above is the contract this test locks.
    assert_ne!(mmu.read8(0xff05), 0xff);
}
