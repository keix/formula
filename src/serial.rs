pub struct Serial {
    sb: u8,
    sc: u8,
    output: Vec<u8>,
    // Latched when a transfer is started; consumed by tick() to set IF bit 3.
    pending_interrupt: bool,
}

impl Serial {
    pub fn new() -> Self {
        Self {
            sb: 0,
            sc: 0,
            output: Vec::new(),
            pending_interrupt: false,
        }
    }

    pub fn read(&self, addr: u16) -> u8 {
        match addr {
            0xff01 => self.sb,
            // Bits 6..1 are unused on DMG and read as 1.
            0xff02 => self.sc | 0x7e,
            _ => panic!("Serial: unmapped read at {:#06x}", addr),
        }
    }

    pub fn write(&mut self, addr: u16, value: u8) {
        match addr {
            0xff01 => self.sb = value,
            0xff02 => {
                // Only bits 7 (transfer start) and 0 (clock select) are
                // writable on DMG; the rest are wired high on read.
                self.sc = value & 0x81;
                if value & 0x80 != 0 {
                    // Treat the transfer as instantaneous: capture the byte,
                    // clear the start flag, and queue the completion IRQ.
                    // This is sufficient for Blargg-style serial logging,
                    // which polls SC bit 7 to wait for the byte to drain.
                    self.output.push(self.sb);
                    self.sc &= 0x7f;
                    self.pending_interrupt = true;
                }
            }
            _ => panic!("Serial: unmapped write at {:#06x}", addr),
        }
    }

    /// Tick the serial port. Returns true if the Serial interrupt should be
    /// raised during this tick.
    pub fn tick(&mut self, _cycles: u8) -> bool {
        let raise = self.pending_interrupt;
        self.pending_interrupt = false;
        raise
    }

    pub fn drain_output(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.output)
    }
}

impl Default for Serial {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writing_sc_with_start_bit_captures_sb() {
        let mut serial = Serial::new();
        serial.write(0xff01, 0x41); // 'A'
        serial.write(0xff02, 0x81);

        assert_eq!(serial.drain_output(), b"A");
    }

    #[test]
    fn writing_sc_without_start_bit_does_not_capture() {
        let mut serial = Serial::new();
        serial.write(0xff01, 0x41);
        serial.write(0xff02, 0x01); // clock select only, no start

        assert!(serial.drain_output().is_empty());
    }

    #[test]
    fn transfer_clears_start_flag_on_read() {
        let mut serial = Serial::new();
        serial.write(0xff02, 0x81);

        // bit 7 must read as 0 after the (instantaneous) transfer completes
        assert_eq!(serial.read(0xff02) & 0x80, 0);
    }

    #[test]
    fn sc_read_returns_unused_bits_set() {
        let serial = Serial::new();
        assert_eq!(serial.read(0xff02), 0x7e);
    }

    #[test]
    fn transfer_raises_interrupt_once() {
        let mut serial = Serial::new();
        serial.write(0xff02, 0x81);

        assert!(serial.tick(4));
        assert!(!serial.tick(4), "interrupt must not refire");
    }

    #[test]
    fn tick_without_transfer_stays_quiet() {
        let mut serial = Serial::new();
        assert!(!serial.tick(255));
    }

    #[test]
    fn drain_output_is_destructive() {
        let mut serial = Serial::new();
        serial.write(0xff01, 0x41);
        serial.write(0xff02, 0x81);
        serial.write(0xff01, 0x42);
        serial.write(0xff02, 0x81);

        assert_eq!(serial.drain_output(), b"AB");
        assert!(serial.drain_output().is_empty());
    }
}
