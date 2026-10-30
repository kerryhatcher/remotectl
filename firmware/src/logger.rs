//! In-RAM log buffer.
//!
//! The USB port is a HID keyboard, so there is no USB serial console. Log lines go into
//! a ring buffer instead and can be read with `GET /logs`.

use core::cell::RefCell;
use core::fmt::Write;

use alloc::string::String;
use embassy_sync::blocking_mutex::Mutex;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_time::Instant;
use heapless::Deque;
use log::{LevelFilter, Log, Metadata, Record};

const CAPACITY: usize = 4096;

static BUFFER: Mutex<CriticalSectionRawMutex, RefCell<Deque<u8, CAPACITY>>> =
    Mutex::new(RefCell::new(Deque::new()));

struct RingLogger;

static LOGGER: RingLogger = RingLogger;

/// Formats into the ring buffer, dropping the oldest bytes when it is full.
struct RingWriter<'a>(&'a mut Deque<u8, CAPACITY>);

impl Write for RingWriter<'_> {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        for byte in s.bytes() {
            if self.0.is_full() {
                self.0.pop_front();
            }
            let _ = self.0.push_back(byte);
        }
        Ok(())
    }
}

impl Log for RingLogger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        metadata.level() <= log::max_level()
    }

    fn log(&self, record: &Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let ms = Instant::now().as_millis();
        BUFFER.lock(|cell| {
            let mut buffer = cell.borrow_mut();
            let _ = writeln!(
                RingWriter(&mut buffer),
                "[{:>6}.{:03}] {:<5} {}",
                ms / 1000,
                ms % 1000,
                record.level(),
                record.args()
            );
        });
    }

    fn flush(&self) {}
}

pub fn init(level: LevelFilter) {
    if log::set_logger(&LOGGER).is_ok() {
        log::set_max_level(level);
    }
}

/// Everything currently in the buffer, oldest line first.
pub fn contents() -> String {
    BUFFER.lock(|cell| {
        let buffer = cell.borrow();
        let (a, b) = buffer.as_slices();
        let mut bytes = alloc::vec::Vec::with_capacity(buffer.len());
        bytes.extend_from_slice(a);
        bytes.extend_from_slice(b);
        String::from_utf8_lossy(&bytes).into_owned()
    })
}
