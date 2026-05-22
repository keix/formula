use formula::cartridge::load_cartridge;
use formula::cpu::Cpu;
use formula::flags::Flags;
use formula::joypad::{
    BUTTON_A, BUTTON_B, BUTTON_DOWN, BUTTON_LEFT, BUTTON_RIGHT, BUTTON_SELECT, BUTTON_START,
    BUTTON_UP,
};
use formula::mmu::Mmu;
use formula::ppu::{HEIGHT, WIDTH};
use minifb::{Key, Scale, Window, WindowOptions};
use std::env;
use std::io::Write;
use std::process::ExitCode;

// Safety net so a runaway ROM eventually returns control instead of hanging
// the terminal. Generous enough for Blargg's cpu_instrs to finish even
// without the spin-loop terminator below.
const MAX_CYCLES: u64 = 2_000_000_000;

// DMG-style green palette, lightest -> darkest.
const PALETTE: [u32; 4] = [0x9bbc0f, 0x8bac0f, 0x306230, 0x0f380f];

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
    mmu_write_post_boot_io(&mut mmu);

    // minifb's X11 backend calls XOpenIM, which fails if XMODIFIERS points
    // at an X input method daemon that isn't reachable (e.g. @im=ibus
    // inside a Nix shell with no ibus running). We don't take text input,
    // so suppress IM unconditionally. Safe because main() is still
    // single-threaded and no other env reader has run.
    unsafe {
        std::env::set_var("XMODIFIERS", "@im=none");
    }

    let mut window = match Window::new(
        "formula",
        WIDTH,
        HEIGHT,
        WindowOptions {
            resize: false,
            scale: Scale::X4,
            ..WindowOptions::default()
        },
    ) {
        Ok(w) => w,
        Err(e) => {
            eprintln!("failed to open window: {e:?}");
            return ExitCode::from(1);
        }
    };
    // Block update_with_buffer to ~60 Hz so the emulator paces itself
    // close to GB frame rate (one update per VBlank).
    window.set_target_fps(60);

    let stdout = std::io::stdout();
    let mut stdout = stdout.lock();
    let mut pixel_buffer: Vec<u32> = vec![0; WIDTH * HEIGHT];
    let mut total_cycles: u64 = 0;
    let mut parked = false;

    while window.is_open() && !window.is_key_down(Key::Escape) {
        if parked || cpu.locked || total_cycles >= MAX_CYCLES {
            // Hold the last frame visible while the user decides to close.
            // Re-blitting the cached buffer keeps the fps limiter active so
            // this loop doesn't spin a core.
            let _ = window.update_with_buffer(&pixel_buffer, WIDTH, HEIGHT);
            continue;
        }

        let pre_pc = cpu.pc;
        let cycles = cpu.step(&mut mmu);
        mmu.tick(cycles);
        total_cycles += u64::from(cycles);

        let out = mmu.drain_serial_output();
        if !out.is_empty() {
            let _ = stdout.write_all(&out);
            let _ = stdout.flush();
        }

        if mmu.take_frame_ready() {
            blit_framebuffer(&mut pixel_buffer, mmu.framebuffer().as_slice());
            let _ = window.update_with_buffer(&pixel_buffer, WIDTH, HEIGHT);
            mmu.set_buttons(read_joypad(&window));
        }

        // Blargg test ROMs (and many homebrew) park themselves in a tight
        // `JR -2` after printing the result. Detect that and switch to
        // idle mode so the user can see the final frame.
        if !cpu.halted && cpu.pc == pre_pc {
            parked = true;
        }
    }

    let _ = stdout.flush();
    if cpu.locked {
        eprintln!("\n[CPU locked on illegal opcode at PC={:#06x}]", cpu.pc);
    }
    ExitCode::SUCCESS
}

fn blit_framebuffer(buffer: &mut [u32], framebuffer: &[u8]) {
    for (dst, &shade) in buffer.iter_mut().zip(framebuffer.iter()) {
        *dst = PALETTE[(shade & 0b11) as usize];
    }
}

/// Map the host keyboard state onto the GB's eight buttons.
///
/// D-pad: arrow keys. A: Z. B: X. Start: Enter. Select: Backspace.
fn read_joypad(window: &Window) -> u8 {
    let mut state = 0u8;
    if window.is_key_down(Key::Up) {
        state |= BUTTON_UP;
    }
    if window.is_key_down(Key::Down) {
        state |= BUTTON_DOWN;
    }
    if window.is_key_down(Key::Left) {
        state |= BUTTON_LEFT;
    }
    if window.is_key_down(Key::Right) {
        state |= BUTTON_RIGHT;
    }
    if window.is_key_down(Key::Z) {
        state |= BUTTON_A;
    }
    if window.is_key_down(Key::X) {
        state |= BUTTON_B;
    }
    if window.is_key_down(Key::Enter) {
        state |= BUTTON_START;
    }
    if window.is_key_down(Key::Backspace) {
        state |= BUTTON_SELECT;
    }
    state
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
