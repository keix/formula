//! `formula` runner — drives the emulator core against a minifb window.
//!
//! Loads a ROM from argv, primes the CPU and MMU to the DMG post-boot-
//! ROM state, then runs `cpu.step` / `mmu.tick` in a loop. The runner
//! drains serial bytes to stdout, blits the framebuffer to the window
//! once per VBlank, and feeds the host keyboard state into the joypad
//! at the same cadence. A tight self-jump (`PC` unchanged with the
//! CPU not halted) parks the loop so test ROMs that print their
//! result and spin can exit cleanly.
//!
//! Press `D` while the window is focused to dump OAM + the LCDC /
//! STAT / palette / IE / IF state to stderr — useful for triaging
//! ROMs that misbehave silently.

use formula::bus::Bus;
use formula::cartridge::load_cartridge;
use formula::cpu::Cpu;
use formula::flags::Flags;
use formula::joypad::{
    BUTTON_A, BUTTON_B, BUTTON_DOWN, BUTTON_LEFT, BUTTON_RIGHT, BUTTON_SELECT, BUTTON_START,
    BUTTON_UP,
};
use formula::mmu::Mmu;
use formula::ppu::{HEIGHT, WIDTH};
use minifb::{Key, KeyRepeat, Scale, Window, WindowOptions};
use std::env;
use std::io::Write;
use std::process::{Child, ChildStdin, Command, ExitCode, Stdio};
use std::thread;
use std::time::{Duration, Instant};

// Safety net so a runaway ROM eventually returns control instead of hanging
// the terminal. Generous enough for Blargg's cpu_instrs to finish even
// without the spin-loop terminator below.
const MAX_CYCLES: u64 = 2_000_000_000;
const AUDIO_BATCH_SAMPLES: usize = 2048;
const GB_FRAME_DURATION: Duration = Duration::from_nanos(16_742_706);

// DMG-style green palette, lightest -> darkest.
const PALETTE: [u32; 4] = [0x9bbc0f, 0x8bac0f, 0x306230, 0x0f380f];

struct AudioSink {
    _child: Child,
    stdin: ChildStdin,
    pending: Vec<i16>,
}

impl AudioSink {
    fn spawn(sample_rate: u32) -> Option<Self> {
        let mut child = Command::new("aplay")
            .args([
                "-q",
                "-t",
                "raw",
                "-f",
                "S16_LE",
                "-c",
                "2",
                "-r",
                &sample_rate.to_string(),
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .ok()?;
        let stdin = child.stdin.take()?;
        Some(Self {
            _child: child,
            stdin,
            pending: Vec::with_capacity(AUDIO_BATCH_SAMPLES * 2),
        })
    }

    fn queue_samples(&mut self, samples: &[i16]) {
        if samples.is_empty() {
            return;
        }

        self.pending.extend_from_slice(samples);
        if self.pending.len() >= AUDIO_BATCH_SAMPLES {
            self.flush();
        }
    }

    fn flush(&mut self) {
        if self.pending.is_empty() {
            return;
        }

        let mut bytes = Vec::with_capacity(self.pending.len() * 2);
        for sample in self.pending.drain(..) {
            bytes.extend_from_slice(&sample.to_le_bytes());
        }
        let _ = self.stdin.write_all(&bytes);
    }
}

impl Drop for AudioSink {
    fn drop(&mut self) {
        self.flush();
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = env::args().collect();
    if args.len() != 2 {
        eprintln!(
            "usage: {} <rom.gb>",
            args.first().map(String::as_str).unwrap_or("formula")
        );
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
    // Pace explicitly to the DMG's real frame cadence instead of a coarse
    // 60.0 Hz host approximation. 70224 T-cycles at 4_194_304 Hz is
    // ~16.742706 ms per frame (~59.7275 Hz).
    window.set_target_fps(0);

    let stdout = std::io::stdout();
    let mut stdout = stdout.lock();
    let mut pixel_buffer: Vec<u32> = vec![0; WIDTH * HEIGHT];
    let mut audio = AudioSink::spawn(mmu.audio_sample_rate());
    let mut total_cycles: u64 = 0;
    let mut parked = false;
    let mut same_pc_count: u32 = 0;
    let mut next_frame_deadline = Instant::now();

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
        total_cycles += u64::from(cycles);

        let out = mmu.drain_serial_output();
        if !out.is_empty() {
            let _ = stdout.write_all(&out);
            let _ = stdout.flush();
        }
        if let Some(audio) = audio.as_mut() {
            let samples = mmu.drain_audio_samples();
            audio.queue_samples(&samples);
        }

        if mmu.take_frame_ready() {
            let now = Instant::now();
            if now < next_frame_deadline {
                thread::sleep(next_frame_deadline - now);
            } else if now.duration_since(next_frame_deadline) > GB_FRAME_DURATION {
                next_frame_deadline = now;
            }
            next_frame_deadline += GB_FRAME_DURATION;

            blit_framebuffer(&mut pixel_buffer, mmu.framebuffer().as_slice());
            let _ = window.update_with_buffer(&pixel_buffer, WIDTH, HEIGHT);
            mmu.set_buttons(read_joypad(&window));
            if let Some(audio) = audio.as_mut() {
                audio.flush();
            }

            // Press D to snapshot OAM + the palette / LCDC registers to
            // stderr. Useful when sprites are visibly missing — proves
            // whether the issue is upstream (no OAM data) or downstream
            // (data is there but the renderer skips it).
            if window.is_key_pressed(Key::D, KeyRepeat::No) {
                dump_state(&mut mmu);
            }
        }

        // Some ROMs transiently sit in a tight self-jump while waiting for an
        // interrupt or LCD state change. Only treat a frozen PC as "parked"
        // after it has stayed unchanged for a long stretch with the CPU still
        // running. This preserves the convenience for finished test ROMs
        // without prematurely idling active wait loops.
        if !cpu.halted && cpu.pc == pre_pc {
            same_pc_count = same_pc_count.saturating_add(1);
            if same_pc_count >= 1_000_000 {
                parked = true;
            }
        } else {
            same_pc_count = 0;
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

fn dump_state(mmu: &mut Mmu) {
    eprintln!("---");
    eprintln!(
        "LCDC={:#04x}  STAT={:#04x}  LY={:3}  LYC={:3}",
        mmu.read8(0xff40),
        mmu.read8(0xff41),
        mmu.read8(0xff44),
        mmu.read8(0xff45),
    );
    eprintln!(
        "BGP={:#04x}  OBP0={:#04x}  OBP1={:#04x}  SCX={:3} SCY={:3}  WX={:3} WY={:3}",
        mmu.read8(0xff47),
        mmu.read8(0xff48),
        mmu.read8(0xff49),
        mmu.read8(0xff43),
        mmu.read8(0xff42),
        mmu.read8(0xff4b),
        mmu.read8(0xff4a),
    );
    eprintln!(
        "IE={:#04x}  IF={:#04x}",
        mmu.read8(0xffff),
        mmu.read8(0xff0f)
    );
    let mut non_zero = 0;
    for i in 0..40 {
        let base = 0xfe00 + i * 4;
        let y = mmu.read8(base);
        let x = mmu.read8(base + 1);
        let tile = mmu.read8(base + 2);
        let attr = mmu.read8(base + 3);
        if y == 0 && x == 0 && tile == 0 && attr == 0 {
            continue;
        }
        eprintln!(
            "OAM[{i:02}] Y={y:3}({:+}) X={x:3}({:+}) tile={tile:02X} attr={attr:02X}",
            y as i16 - 16,
            x as i16 - 8,
        );
        non_zero += 1;
    }
    if non_zero == 0 {
        eprintln!("OAM: all zero");
    }
}

fn mmu_write_post_boot_io(mmu: &mut Mmu) {
    // Enabling the LCD here lets PPU-driven interrupts (VBlank/STAT) reach
    // the CPU once Blargg-style ROMs unmask IE. The palette defaults mirror
    // what the DMG boot ROM leaves behind — Tetris (and many other DMG
    // games) never write OBP0/OBP1 themselves and rely on this state, so
    // skipping them maps every sprite shade to 0 and hides every sprite.
    mmu.write8(0xff40, 0x91); // LCDC
    mmu.write8(0xff47, 0xfc); // BGP
    mmu.write8(0xff48, 0xff); // OBP0
    mmu.write8(0xff49, 0xff); // OBP1
}
