//! Joypad register at 0xFF00.
//!
//! Tracks which of the eight buttons the host considers pressed and
//! re-creates the active-low DMG wire format on read by ANDing the
//! pressed-button bitmask against whichever of the two scan rows the
//! CPU has selected (bits 5..4 of the register, written by the game).
//! The internal interrupt latch raises IF bit 4 on every released ->
//! pressed transition; holding a button does not refire it.

// Button bitmask. The values are arbitrary — the joypad layout on the wire
// is reconstructed in Joypad::read by combining state with the active
// selection lines.
pub const BUTTON_A: u8 = 1 << 0;
pub const BUTTON_B: u8 = 1 << 1;
pub const BUTTON_SELECT: u8 = 1 << 2;
pub const BUTTON_START: u8 = 1 << 3;
pub const BUTTON_RIGHT: u8 = 1 << 4;
pub const BUTTON_LEFT: u8 = 1 << 5;
pub const BUTTON_UP: u8 = 1 << 6;
pub const BUTTON_DOWN: u8 = 1 << 7;

pub struct Joypad {
    // Bitmask of currently-pressed buttons (1 = pressed).
    pressed: u8,
    // Bits 5..4 of the joypad register, written by the CPU to pick which
    // row to scan: bit 5 (P15) = 0 selects action buttons, bit 4 (P14)
    // = 0 selects the d-pad. Other bits ignored on write.
    select: u8,
    // Set when any button transitions from released to pressed; consumed
    // by take_interrupt() so the MMU can route it into IF bit 4.
    interrupt_pending: bool,
}

impl Joypad {
    pub fn new() -> Self {
        Self {
            pressed: 0,
            // Default: both selection lines high (no row selected); bits
            // 7..6 are unused and read as 1 on DMG.
            select: 0x30,
            interrupt_pending: false,
        }
    }

    /// Synthesise the joypad register byte for `addr` (must be 0xFF00).
    /// Bits 7..6 are hardwired high, 5..4 mirror the last CPU write,
    /// and 3..0 are active-low for the buttons on the selected row(s).
    pub fn read(&self, addr: u16) -> u8 {
        match addr {
            0xff00 => {
                // Bits 7..6 always 1, bits 5..4 mirror what the CPU wrote,
                // bits 3..0 are 0 for any pressed button on the selected row.
                let mut value = 0xc0 | (self.select & 0x30) | 0x0f;
                let dpad_selected = (self.select & 0x10) == 0;
                let action_selected = (self.select & 0x20) == 0;
                if dpad_selected {
                    if self.pressed & BUTTON_RIGHT != 0 {
                        value &= !0x01;
                    }
                    if self.pressed & BUTTON_LEFT != 0 {
                        value &= !0x02;
                    }
                    if self.pressed & BUTTON_UP != 0 {
                        value &= !0x04;
                    }
                    if self.pressed & BUTTON_DOWN != 0 {
                        value &= !0x08;
                    }
                }
                if action_selected {
                    if self.pressed & BUTTON_A != 0 {
                        value &= !0x01;
                    }
                    if self.pressed & BUTTON_B != 0 {
                        value &= !0x02;
                    }
                    if self.pressed & BUTTON_SELECT != 0 {
                        value &= !0x04;
                    }
                    if self.pressed & BUTTON_START != 0 {
                        value &= !0x08;
                    }
                }
                value
            }
            _ => panic!("Joypad: unmapped read at {:#06x}", addr),
        }
    }

    /// Latch the row-select bits (5..4) at `addr` (must be 0xFF00).
    /// Other bits are ignored — they're hardware-driven on read.
    pub fn write(&mut self, addr: u16, value: u8) {
        match addr {
            // Only bits 5..4 are writable; the rest are read-only on DMG.
            0xff00 => self.select = (self.select & !0x30) | (value & 0x30),
            _ => panic!("Joypad: unmapped write at {:#06x}", addr),
        }
    }

    /// Replace the pressed-buttons bitmask. Returns nothing directly, but
    /// any newly-pressed button (released -> pressed transition) latches
    /// an interrupt that take_interrupt() will surface to the MMU.
    pub fn set_pressed(&mut self, new_state: u8) {
        let newly_pressed = new_state & !self.pressed;
        if newly_pressed != 0 {
            self.interrupt_pending = true;
        }
        self.pressed = new_state;
    }

    /// Consume the pending-interrupt flag. Returns true once per
    /// released -> pressed transition; subsequent calls return false
    /// until the next fresh press.
    pub fn take_interrupt(&mut self) -> bool {
        let pending = self.interrupt_pending;
        self.interrupt_pending = false;
        pending
    }
}

impl Default for Joypad {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_read_returns_no_buttons_pressed() {
        let joypad = Joypad::new();
        // Both lines deselected -> nibble reads 0x0F.
        assert_eq!(joypad.read(0xff00), 0xff);
    }

    #[test]
    fn dpad_selection_shows_dpad_buttons() {
        let mut joypad = Joypad::new();
        joypad.write(0xff00, 0x20); // P14 = 0 (dpad), P15 = 1
        joypad.set_pressed(BUTTON_DOWN | BUTTON_RIGHT);
        // Bits: 7..6 = 1, 5..4 = select bits (0b10), 3..0 = ~(down|right)
        // Down is bit 3, Right is bit 0 -> low when pressed.
        let v = joypad.read(0xff00);
        assert_eq!(v & 0x08, 0, "down");
        assert_eq!(v & 0x04, 0x04, "up");
        assert_eq!(v & 0x02, 0x02, "left");
        assert_eq!(v & 0x01, 0, "right");
    }

    #[test]
    fn action_selection_shows_action_buttons_only() {
        let mut joypad = Joypad::new();
        joypad.write(0xff00, 0x10); // P15 = 0, P14 = 1
        joypad.set_pressed(BUTTON_A | BUTTON_START | BUTTON_DOWN);
        let v = joypad.read(0xff00);
        // D-pad is unselected: pressing Down must NOT pull bit 3 low.
        // Start is bit 3 of the action row.
        assert_eq!(v & 0x01, 0, "A pressed");
        assert_eq!(v & 0x08, 0, "Start pressed");
        assert_eq!(v & 0x02, 0x02, "B not pressed");
    }

    #[test]
    fn both_lines_selected_reads_union_of_pressed_buttons() {
        let mut joypad = Joypad::new();
        joypad.write(0xff00, 0x00); // both lines low
        joypad.set_pressed(BUTTON_A | BUTTON_DOWN);
        let v = joypad.read(0xff00);
        assert_eq!(v & 0x01, 0, "A pulls bit 0 low through the action row");
        assert_eq!(v & 0x08, 0, "Down pulls bit 3 low through the dpad row");
    }

    #[test]
    fn write_only_touches_selection_bits() {
        let mut joypad = Joypad::new();
        joypad.write(0xff00, 0xff);
        // Only bits 5..4 stored; rest of the byte comes from the read-side
        // synthesis (0xC0 + selection + nibble).
        let v = joypad.read(0xff00);
        assert_eq!(v & 0x30, 0x30);
        assert_eq!(v & 0xc0, 0xc0); // hardwired high bits
    }

    #[test]
    fn newly_pressed_button_raises_an_interrupt() {
        let mut joypad = Joypad::new();
        assert!(!joypad.take_interrupt());

        joypad.set_pressed(BUTTON_A);
        assert!(joypad.take_interrupt());
        assert!(!joypad.take_interrupt(), "one-shot");
    }

    #[test]
    fn holding_a_button_does_not_refire_the_interrupt() {
        let mut joypad = Joypad::new();
        joypad.set_pressed(BUTTON_A);
        let _ = joypad.take_interrupt();

        // Same state again -> no transition -> no new interrupt.
        joypad.set_pressed(BUTTON_A);
        assert!(!joypad.take_interrupt());
    }

    #[test]
    fn releasing_then_pressing_re_triggers_interrupt() {
        let mut joypad = Joypad::new();
        joypad.set_pressed(BUTTON_A);
        let _ = joypad.take_interrupt();

        joypad.set_pressed(0); // release
        assert!(!joypad.take_interrupt());

        joypad.set_pressed(BUTTON_A); // press again -> new transition
        assert!(joypad.take_interrupt());
    }
}
