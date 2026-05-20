# Design

Decisions that shape how the emulator grows. Anything not covered here is deferred until the relevant opcode forces a choice.

## 1. `Cpu::step` returns elapsed T-cycles

### Unit

| Unit    | Per-instruction range | Notes                                           |
| ------- | --------------------- | ----------------------------------------------- |
| T-cycle | 4 / 8 / 12 / 16 / 20 / 24 | Native clock; PPU mode transitions are here. |
| M-cycle | 1 / 2 / 3 / 4 / 5 / 6     | Bus-cycle granularity (T / 4).               |

**Decision: T-cycles, returned as `u8`.**

- Maximum cost of a single instruction is 24 T-cycles, so `u8` is sufficient.
- The PPU's mode timings (OAM scan = 80, drawing ≈ 172, H-blank, V-blank) are documented in T-cycles. Driving them directly from `step`'s return value avoids unit conversions.
- Callers that need to accumulate over a frame can widen at the call site: `total += cpu.step(bus) as u32`.

### Signature

```rust
pub fn step(&mut self, bus: &mut impl Bus) -> u8;
```

### HALT behavior

The hardware clock keeps running while the CPU is halted; the timer and PPU continue to advance, and an interrupt is what wakes the CPU. The current "step is a no-op when halted" therefore needs to change:

```rust
if self.halted {
    return 4; // one M-cycle elapsed
}
```

This makes "timer raises an interrupt → HALT exits" fall out naturally once interrupts are implemented.

### Where the cycle counts live

The cycle cost lives at the end of each match arm, next to the instruction that produced it:

```rust
0x00 => 4,                                  // NOP
0x06 => { self.b = self.fetch8(bus); 8 }    // LD B, n
0x76 => { self.halted = true; 4 }           // HALT
```

A separate `CYCLES: [u8; 256]` table was considered and rejected — keeping the cost next to the behavior avoids the two-place edit that table-based timing forces.

## 2. Opcode dispatch: match-based, with bit decoding for regular families

### Options considered

| Option                                             | Code size  | Greppability | Speed                            |
| -------------------------------------------------- | ---------- | ------------ | -------------------------------- |
| A. Flat `match` over all 256 opcodes               | ~700 lines | ◎            | ◎ (LLVM lowers to a jump table)  |
| B. Bit decoding throughout                         | ~100 lines | ✗            | ◎                                |
| C. `[fn(&mut Cpu, &mut Bus); 256]` dispatch table  | 256 fns    | △            | △ (function pointers block inlining) |
| D. Hybrid — match arms by default, decode the regular families | ~200 lines | ◯ | ◎                                |

**Decision: option D.**

The SM83 instruction set mixes two kinds of opcodes:

- **Regular families** — `LD r, r'`, ALU ops on `A, r` — where the operand is encoded in three bits of the opcode and every member of the family does the same thing.
- **Irregular instructions** — `CALL`, `JR cc`, `RET cc`, `EI` / `DI`, the CB prefix — that don't share structure with anything else.

Writing the regular families as 49 + 32 + 32 near-identical match arms adds nothing. Decoding the irregular instructions by bit pattern obscures what they do. Treating each category in its natural form is what option D buys.

### What the decoding looks like

The three exploitable bit patterns:

```text
0x40..=0x7F   LD r, r'  (0x76 = HALT is the hole)   01 ddd sss
0x80..=0xBF   ALU A, r                              10 ooo sss
some 0xC0..=0xFF arms   conditional control flow    11 ccc xxx
```

A pair of helpers indexes the eight slots `ddd` / `sss` can name (B, C, D, E, H, L, (HL), A):

```rust
fn read_r(&mut self, idx: u8, bus: &mut impl Bus) -> u8 {
    match idx & 7 {
        0 => self.b,
        1 => self.c,
        2 => self.d,
        3 => self.e,
        4 => self.h,
        5 => self.l,
        6 => bus.read8(self.hl()),
        7 => self.a,
        _ => unreachable!(),
    }
}

fn write_r(&mut self, idx: u8, value: u8, bus: &mut impl Bus);
```

With those, the entire `LD r, r'` family collapses to a single arm (HALT is matched separately first so the 0x76 hole doesn't leak in):

```rust
0x40..=0x7f => {
    let dst = (opcode >> 3) & 7;
    let src = opcode & 7;
    let value = self.read_r(src, bus);
    self.write_r(dst, value, bus);
    if dst == 6 || src == 6 { 8 } else { 4 }
}
```

The ALU family (`0x80..=0xBF`) decodes the same way.

### What comes with it

Bringing in `read_r` / `write_r` pulls in two adjacent pieces:

- **16-bit register-pair accessors** — `hl()`, `bc()`, `de()` (and their setters). `read_r` for index 6 needs `hl()` to dereference `(HL)`.
- **A `&mut impl Bus` plumbed through `read_r` / `write_r`** — index 6 reads from / writes to memory, so the helpers can't be pure register manipulation.

## Implementation order

1. Change `step` to return `u8`. Update existing arms to return their cycle counts. HALT-while-halted returns 4.
2. Add `hl()` / `bc()` / `de()` and their setters.
3. Add `read_r` / `write_r`.
4. Add the `LD r, r'` family as one decoded arm. HALT (0x76) stays a separate arm above it.

After that, the ALU family (`ADD / ADC / SUB / SBC / AND / XOR / OR / CP A, r`) is the next natural batch — same decode shape, plus the flag-register design decision that was deferred.
