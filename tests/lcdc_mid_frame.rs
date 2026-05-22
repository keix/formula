//! End-to-end check that dmg-acid2's signature trick works on us:
//! a STAT/LYC interrupt fires, the CPU services it, the ISR rewrites
//! LCDC, and the very next scanline the PPU samples reflects the new
//! value. If anything in the LYC->IF->CPU->ISR->LCDC->render chain
//! is off by even a single scanline, the framebuffer diverges visibly
//! and the assertions below catch it.

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

/// Build a tiny MBC0 ROM that:
/// - fills tile 0 in VRAM with solid color 3 pixels,
/// - enables STAT's LYC=LY source with LYC = `lyc`,
/// - kicks the LCD on with BG enabled,
/// - HALTs forever,
/// - has the STAT ISR overwrite LCDC with `new_lcdc`.
fn build_rom(lyc: u8, new_lcdc: u8) -> Vec<u8> {
    let mut rom = vec![0u8; 0x8000];
    rom[0x0147] = 0x00; // MBC0

    // Entry at $0100.
    rom[0x0100] = 0x00; // NOP
    rom[0x0101] = 0xc3; // JP $0150
    rom[0x0102] = 0x50;
    rom[0x0103] = 0x01;

    // STAT vector at $0048 jumps to the ISR at $0200.
    rom[0x0048] = 0xc3;
    rom[0x0049] = 0x00;
    rom[0x004a] = 0x02;

    // ISR at $0200: write new_lcdc to LCDC, then RETI.
    rom[0x0200] = 0x3e; // LD A, n
    rom[0x0201] = new_lcdc;
    rom[0x0202] = 0xe0; // LDH (n), A   ; LCDC = $FF40
    rom[0x0203] = 0x40;
    rom[0x0204] = 0xd9; // RETI

    // Main program at $0150.
    let program: &[u8] = &[
        0xf3, // DI
        0x31, 0xff, 0xcf, // LD SP, $CFFF
        // --- Fill tile 0 ($8000-$800F) with 16 bytes of 0xFF ---
        0x21, 0x00, 0x80, // LD HL, $8000
        0x3e, 0xff, // LD A, $FF
        0x06, 0x10, // LD B, 16
        0x22, // .loop: LD [HL+], A
        0x05, //        DEC B
        0x20, 0xfc, //        JR NZ, .loop  (back -4)
        // --- STAT setup: bit 6 enables LYC=LY interrupts ---
        0x3e, 0x40, // LD A, $40
        0xe0, 0x41, // LDH ($41), A   ; STAT
        // --- LYC ---
        0x3e, lyc, // LD A, lyc
        0xe0, 0x45, // LDH ($45), A
        // --- IE: STAT only ---
        0x3e, 0x02, // LD A, $02
        0xe0, 0xff, // LDH ($FF), A   ; IE
        // --- LCDC: LCD on, tile data unsigned ($8000), BG on ---
        0x3e, 0x91, // LD A, $91
        0xe0, 0x40, // LDH ($40), A
        // --- BGP: identity ---
        0x3e, 0xe4, // LD A, $E4
        0xe0, 0x47, // LDH ($47), A
        0xfb, // EI
        0x76, // .forever: HALT
        0x18, 0xfd, //           JR .forever (back -3)
    ];
    rom[0x0150..0x0150 + program.len()].copy_from_slice(program);
    rom
}

fn run_one_frame(rom: Vec<u8>) -> Mmu {
    let mut mmu = Mmu::new(Box::new(Mbc0::new(rom)));
    let mut cpu = post_boot_cpu();
    for _ in 0..200_000 {
        let cycles = cpu.step(&mut mmu);
        mmu.tick(cycles);
        if mmu.take_frame_ready() {
            return mmu;
        }
    }
    panic!("frame never completed");
}

#[test]
fn stat_lyc_isr_disables_bg_starting_at_target_line() {
    // LYC = 80. ISR writes LCDC = $90 (LCD on, BG off).
    let mmu = run_one_frame(build_rom(80, 0x90));
    let fb = mmu.framebuffer();

    // Before LYC: BG on -> shade 3.
    assert_eq!(fb.pixel(0, 0), 3, "line 0");
    assert_eq!(fb.pixel(0, 79), 3, "line 79 (just before LYC)");

    // At and after LYC: ISR has cleared LCDC.0 -> shade 0.
    assert_eq!(
        fb.pixel(0, 80),
        0,
        "line 80 (LYC line itself sees the change)"
    );
    assert_eq!(fb.pixel(0, 100), 0, "line 100 stays off");
    assert_eq!(fb.pixel(0, 143), 0, "last visible line stays off");
}

#[test]
fn stat_lyc_isr_re_enabling_bg_kicks_in_on_target_line() {
    // Two-step ISR pattern: first frame disables BG at LYC=20, then on the
    // *next* frame the test reprograms LYC to fire later and re-enable BG.
    // Easier to verify with two separate ROMs that share the build_rom
    // skeleton: this test just confirms re-enabling at LYC works too.

    // LYC = 30, ISR writes LCDC = $91 (LCD on, BG on) — but we boot with BG
    // off, so the ISR is what turns it on.
    let mut rom = build_rom(30, 0x91);
    // Override the LCDC setup near the end of main() to start with BG off.
    // The byte at $0150 + 33 = $0171 is the LCDC literal $91 in the main
    // program; flip it to $90 so BG is off until the ISR fires.
    // (Counting: 1 di + 3 ldsp + 3 ldhl + 2 lda + 2 ldb + 1 ldhlinc + 1 dec
    //  + 2 jrnz + 2 lda + 2 ldhstat + 2 lda + 2 ldhlyc + 2 lda + 2 ldhie
    //  + 2 lda = 29 bytes before the LCDC LDH. The LCDC value byte is
    //  the one right after the 0x3E inside the LCDC-setup pair: offset 28.)
    rom[0x0150 + 28] = 0x90;
    let mmu = run_one_frame(rom);
    let fb = mmu.framebuffer();

    // Before LYC: BG off (never enabled) -> shade 0.
    assert_eq!(fb.pixel(0, 0), 0, "line 0 starts off");
    assert_eq!(fb.pixel(0, 29), 0, "line 29 still off");

    // At and after LYC: ISR turned BG on -> shade 3.
    assert_eq!(fb.pixel(0, 30), 3, "line 30 sees BG enabled");
    assert_eq!(fb.pixel(0, 143), 3, "stays on through end of frame");
}
