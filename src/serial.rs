pub struct Serial {
    sb: u8,
    sc: u8,
    output: Vec<u8>,
    transfer_cycles_remaining: u16,
    // Latched when a transfer is started; consumed by tick() to set IF bit 3.
    pending_interrupt: bool,
}

const SC_START: u8 = 0x80;
const SC_INTERNAL_CLOCK: u8 = 0x01;
const SERIAL_TRANSFER_CYCLES: u16 = 8 * 512;

impl Serial {
    pub fn new() -> Self {
        Self {
            sb: 0,
            sc: 0,
            output: Vec::new(),
            transfer_cycles_remaining: 0,
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
                self.sc = value & (SC_START | SC_INTERNAL_CLOCK);
                self.transfer_cycles_remaining = 0;

                // With no emulated link partner, only the internal-clock
                // mode can make progress. External-clock transfers keep the
                // start bit set and never complete on their own.
                if self.sc == (SC_START | SC_INTERNAL_CLOCK) {
                    self.transfer_cycles_remaining = SERIAL_TRANSFER_CYCLES;
                }
            }
            _ => panic!("Serial: unmapped write at {:#06x}", addr),
        }
    }

    /// Tick the serial port. Returns true if the Serial interrupt should be
    /// raised during this tick.
    pub fn tick(&mut self, cycles: u8) -> bool {
        if self.transfer_cycles_remaining != 0 {
            self.transfer_cycles_remaining = self
                .transfer_cycles_remaining
                .saturating_sub(u16::from(cycles));
            if self.transfer_cycles_remaining == 0 {
                self.output.push(self.sb);
                self.sb = 0xff;
                self.sc &= !SC_START;
                self.pending_interrupt = true;
            }
        }

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

    fn run_transfer(serial: &mut Serial) -> bool {
        let mut remaining = SERIAL_TRANSFER_CYCLES;
        let mut irq = false;
        while remaining > 0 {
            let chunk = remaining.min(u16::from(u8::MAX)) as u8;
            irq |= serial.tick(chunk);
            remaining -= u16::from(chunk);
        }
        irq
    }

    #[test]
    fn writing_sc_with_start_bit_captures_sb() {
        let mut serial = Serial::new();
        serial.write(0xff01, 0x41); // 'A'
        serial.write(0xff02, 0x81);

        assert!(serial.drain_output().is_empty());
        assert_eq!(serial.read(0xff02) & 0x80, 0x80);

        assert!(run_transfer(&mut serial));
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
        run_transfer(&mut serial);

        // bit 7 must read as 0 after the transfer completes
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

        assert!(run_transfer(&mut serial));
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
        run_transfer(&mut serial);
        serial.write(0xff01, 0x42);
        serial.write(0xff02, 0x81);
        run_transfer(&mut serial);

        assert_eq!(serial.drain_output(), b"AB");
        assert!(serial.drain_output().is_empty());
    }

    #[test]
    fn external_clock_transfer_never_completes_without_partner() {
        let mut serial = Serial::new();
        serial.write(0xff01, 0x41);
        serial.write(0xff02, 0x80);

        for _ in 0..100 {
            assert!(!serial.tick(255));
        }
        assert!(serial.drain_output().is_empty());
        assert_eq!(serial.read(0xff02) & 0x80, 0x80);
    }
}
