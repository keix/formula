use formula::cartridge::load_cartridge;
use formula::cpu::Cpu;
use formula::flags::Flags;
use formula::mmu::Mmu;
use std::env;
use std::io::Write;
use std::process::ExitCode;

// Safety net so a runaway ROM eventually returns control instead of hanging
// the terminal. Generous enough for Blargg's cpu_instrs to finish even
// without the spin-loop terminator below.
const MAX_CYCLES: u64 = 2_000_000_000;

fn main() -> ExitCode {
    let args: Vec<String> = env::args().collect();
    if args.len() != 2 {
        eprintln!("usage: {} <rom.gb>", args.first().map(String::as_str).unwrap_or("formula"));
        return ExitCode::from(2);
    }

    let rom = match std::fs::read(&args[1]) {
        Ok(bytes) => bytes,
        Err(e) => {
            eprintln!("failed to read {}: {}", args[1], e);
            return ExitCode::from(1);
        }
    };

    let mut mmu = Mmu::new(load_cartridge(rom));
    let mut cpu = post_boot_cpu();
    // Match the post-boot-ROM hardware state for the bits ROMs actually rely on.
    mmu_write_post_boot_io(&mut mmu);

    let stdout = std::io::stdout();
    let mut stdout = stdout.lock();
    let mut total_cycles: u64 = 0;

    while !cpu.locked && total_cycles < MAX_CYCLES {
        let pre_pc = cpu.pc;
        let cycles = cpu.step(&mut mmu);
        mmu.tick(cycles);
        total_cycles += u64::from(cycles);

        let out = mmu.drain_serial_output();
        if !out.is_empty() {
            let _ = stdout.write_all(&out);
            let _ = stdout.flush();
        }

        // Blargg test ROMs (and many homebrew) park themselves in a tight
        // `JR -2` (or similar) after printing the result. Detect that the
        // CPU is making no forward progress and exit cleanly instead of
        // burning cycles to the safety net.
        if !cpu.halted && cpu.pc == pre_pc {
            let _ = stdout.flush();
            return ExitCode::SUCCESS;
        }
    }

    let _ = stdout.flush();
    if cpu.locked {
        eprintln!("\n[CPU locked on illegal opcode at PC={:#06x}]", cpu.pc);
        ExitCode::from(1)
    } else {
        eprintln!("\n[timeout after {total_cycles} cycles]");
        ExitCode::from(1)
    }
}

fn post_boot_cpu() -> Cpu {
    let mut cpu = Cpu::new();
    // DMG post-boot-ROM register file (Pan Docs).
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

fn mmu_write_post_boot_io(mmu: &mut Mmu) {
    use formula::bus::Bus;
    // Enabling the LCD here lets PPU-driven interrupts (VBlank/STAT) reach
    // the CPU once Blargg-style ROMs unmask IE.
    mmu.write8(0xff40, 0x91); // LCDC
    mmu.write8(0xff47, 0xfc); // BGP
}
