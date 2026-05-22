//! DIV / TIMA / TMA / TAC at 0xFF04-0xFF07.
//!
//! A single 16-bit counter increments every T-cycle and DIV (0xFF04)
//! is its high byte, so any write to DIV clears the whole counter.
//! TIMA increments on a falling edge of a selected counter bit (bit
//! 9 / 3 / 5 / 7 for TAC clock 00 / 01 / 10 / 11 — i.e. every 1024
//! / 16 / 64 / 256 T-cycles). When TIMA overflows it reloads from
//! TMA and tick() returns true so the MMU can raise IF bit 2.

pub struct Timer {
    counter: u16, // internal 16-bit counter; DIV is the high byte
    tima: u8,
    tma: u8,
    tac: u8,
}

impl Timer {
    pub fn new() -> Self {
        Self {
            counter: 0,
            tima: 0,
            tma: 0,
            tac: 0,
        }
    }

    pub fn read(&self, addr: u16) -> u8 {
        match addr {
            0xff04 => (self.counter >> 8) as u8,
            0xff05 => self.tima,
            0xff06 => self.tma,
            0xff07 => self.tac | 0xf8, // upper 5 bits always read as 1
            _ => panic!("Timer: unmapped read at {:#06x}", addr),
        }
    }

    pub fn write(&mut self, addr: u16, value: u8) {
        match addr {
            0xff04 => self.counter = 0, // any write to DIV clears the internal counter
            0xff05 => self.tima = value,
            0xff06 => self.tma = value,
            0xff07 => self.tac = value & 0x07,
            _ => panic!("Timer: unmapped write at {:#06x}", addr),
        }
    }

    /// Tick the timer by `cycles` T-cycles. Returns true if the Timer
    /// interrupt was raised during this tick.
    pub fn tick(&mut self, cycles: u8) -> bool {
        let mut interrupt = false;
        for _ in 0..cycles {
            let old_counter = self.counter;
            self.counter = self.counter.wrapping_add(1);

            if self.tac & 0x04 != 0 {
                let bit = match self.tac & 0x03 {
                    0 => 9, // 4096 Hz
                    1 => 3, // 262144 Hz
                    2 => 5, // 65536 Hz
                    3 => 7, // 16384 Hz
                    _ => unreachable!(),
                };
                let mask = 1_u16 << bit;
                // Falling edge on the selected bit increments TIMA
                if (old_counter & mask) != 0 && (self.counter & mask) == 0 {
                    let (new_tima, overflow) = self.tima.overflowing_add(1);
                    self.tima = if overflow { self.tma } else { new_tima };
                    if overflow {
                        interrupt = true;
                    }
                }
            }
        }
        interrupt
    }
}

impl Default for Timer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn div_increments_every_256_cycles() {
        let mut timer = Timer::new();

        timer.tick(255);
        assert_eq!(timer.read(0xff04), 0);

        timer.tick(1);
        assert_eq!(timer.read(0xff04), 1);

        timer.tick(255);
        assert_eq!(timer.read(0xff04), 1);

        timer.tick(1);
        assert_eq!(timer.read(0xff04), 2);
    }

    #[test]
    fn div_write_resets_internal_counter() {
        let mut timer = Timer::new();
        timer.tick(0xff);

        timer.write(0xff04, 0x42); // value ignored, counter reset

        assert_eq!(timer.read(0xff04), 0);
    }

    #[test]
    fn tima_does_not_increment_when_disabled() {
        let mut timer = Timer::new();
        timer.write(0xff07, 0x00); // disabled, clock select 00

        for _ in 0..16 {
            timer.tick(255); // plenty of cycles
        }

        assert_eq!(timer.read(0xff05), 0);
    }

    #[test]
    fn tima_increments_at_each_clock_rate() {
        // (TAC bits 1-0, expected cycles per TIMA increment)
        let rates = [(0b00, 1024_u32), (0b01, 16), (0b10, 64), (0b11, 256)];
        for (clock_sel, cycles_per_inc) in rates {
            let mut timer = Timer::new();
            timer.write(0xff07, 0x04 | clock_sel as u8);

            // Run for exactly one period: TIMA should advance by 1.
            for _ in 0..cycles_per_inc {
                timer.tick(1);
            }

            assert_eq!(
                timer.read(0xff05),
                1,
                "clock_sel {:02b}: TIMA should be 1 after {} cycles",
                clock_sel,
                cycles_per_inc
            );
        }
    }

    #[test]
    fn tima_overflow_reloads_tma_and_raises_interrupt() {
        let mut timer = Timer::new();
        timer.write(0xff05, 0xff); // TIMA on the brink
        timer.write(0xff06, 0x42); // TMA reload value
        timer.write(0xff07, 0x05); // enable, clock select 01 (every 16 cycles)

        // First 15 cycles: no increment yet
        let mut interrupt = false;
        for _ in 0..15 {
            interrupt |= timer.tick(1);
        }
        assert!(!interrupt);
        assert_eq!(timer.read(0xff05), 0xff);

        // 16th cycle: TIMA overflows, reloads TMA, raises interrupt
        interrupt = timer.tick(1);
        assert!(interrupt);
        assert_eq!(timer.read(0xff05), 0x42);
    }

    #[test]
    fn tac_read_returns_upper_bits_set() {
        let mut timer = Timer::new();
        timer.write(0xff07, 0x05); // enable, clock 01

        // Stored as 0x05, but read returns upper bits set as 1
        assert_eq!(timer.read(0xff07), 0xfd); // 0xf8 | 0x05
    }
}
