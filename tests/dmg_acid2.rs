//! Visual regression test: run dmg-acid2 to its settled frame and compare
//! every pixel against the canonical reference image.
//!
//! The ROM and the reference live under `test-rom/dmg-acid2/`. If either
//! file is missing locally we skip the test with a hint so a fresh clone
//! without the assets still has a green test suite.

use formula::bus::Bus;
use formula::cartridge::load_cartridge;
use formula::cpu::Cpu;
use formula::flags::Flags;
use formula::mmu::Mmu;
use formula::ppu::{HEIGHT, WIDTH};
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

/// Map a 4-color reference pixel onto one of our 2-bit shades by
/// quantising the RGB sum. The reference image uses a grayscale-ish
/// palette where lightness monotonically tracks the shade index.
fn shade_of_rgb(rgb: &[u8; 3]) -> u8 {
    let lum = rgb[0] as u32 + rgb[1] as u32 + rgb[2] as u32;
    if lum > 600 {
        0
    } else if lum > 400 {
        1
    } else if lum > 200 {
        2
    } else {
        3
    }
}

#[test]
fn dmg_acid2_matches_reference_image() {
    let rom_path = Path::new("test-rom/dmg-acid2/dmg-acid2.gb");
    let ref_path = Path::new("test-rom/dmg-acid2/img/reference-dmg.png");
    if !rom_path.exists() || !ref_path.exists() {
        eprintln!(
            "skipping dmg-acid2 visual regression: drop the ROM at {} and \
             the reference at {} to enable it",
            rom_path.display(),
            ref_path.display()
        );
        return;
    }

    let rom = std::fs::read(rom_path).expect("read dmg-acid2.gb");
    let mut mmu = Mmu::new(load_cartridge(rom));
    let mut cpu = post_boot_cpu();
    // Match the post-boot-ROM IO state the binary writes.
    mmu.write8(0xff40, 0x91);
    mmu.write8(0xff47, 0xfc);

    // dmg-acid2's main loop reaches its `ld b, b` source-code breakpoint
    // 10 frames in. Run 12 frames so we sample well past that boundary.
    let mut frames = 0;
    while frames < 12 {
        let cycles = cpu.step(&mut mmu);
        mmu.tick(cycles);
        if mmu.take_frame_ready() {
            frames += 1;
        }
    }

    let fb = mmu.framebuffer().as_slice();
    let reference = image::open(ref_path).expect("decode reference").to_rgb8();
    assert_eq!(reference.width() as usize, WIDTH, "reference width");
    assert_eq!(reference.height() as usize, HEIGHT, "reference height");

    let mut mismatches = Vec::new();
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            let px = reference.get_pixel(x as u32, y as u32);
            let expected = shade_of_rgb(&px.0);
            let actual = fb[y * WIDTH + x];
            if expected != actual {
                mismatches.push((x, y, expected, actual));
            }
        }
    }

    if !mismatches.is_empty() {
        for (x, y, expected, actual) in mismatches.iter().take(10) {
            eprintln!("({x:3}, {y:3}): expected shade {expected}, got {actual}");
        }
        panic!(
            "dmg-acid2 framebuffer disagrees with the reference at {} pixel(s)",
            mismatches.len()
        );
    }
}
