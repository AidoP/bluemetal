//! The globally accessible serial device.
//! Accessed by `println!()`.
//!
//! Has a buffer and maintains a lock over the serial device.

macro_rules! println {
    () => {};
    ($($arg:tt)*) => {};
}

/// The global serial device.
static mut SERIAL: Global;

struct CircularBuffer {
    buffer: [u8; 4096],
    /// Buffer head index. The next byte to read.
    head: u16,
    /// BUffer tail index. The last byte that can be read.
    tail: u16,
}

struct Global {
    lock: Spinlock,
    input: CircularBuffer,
    output: CircularBuffer,
}
impl Global {

}
pub struct GlobalGuard;
impl core::fmt::Write for GlobalGuard {

}
