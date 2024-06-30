//! The globally accessible serial device.
//! Accessed by `println!()`.
//!
//! Has a buffer and maintains a lock over the serial device.

#![allow(dead_code)]

use core::{cell::UnsafeCell, fmt, mem::MaybeUninit};
use crate::Serial;

/// The global serial device.
static GLOBAL: Global = Global::new();
pub fn global() -> &'static Global {
    &GLOBAL
}

/// Initialise the global serial device.
///
/// Required for the [`print!`] and [`println!`] macros to work correctly.
pub fn init() {
    use crate::sifive_uart;
    let device = sifive_uart(0)
        .or_else(|| sifive_uart(1));
    let global = GLOBAL.lock();
    global.0.device = device;
}

struct CircularBuffer {
    buffer: [MaybeUninit<u8>; 4096],
    /// Buffer head index. The next byte to read.
    head: u16,
    /// BUffer tail index. The last byte that can be read.
    tail: u16,
}
impl CircularBuffer {
    const fn new() -> Self {
        Self {
            buffer: unsafe { MaybeUninit::uninit().assume_init() },
            head: 0,
            tail: 0,
        }
    }
}

pub struct Global(UnsafeCell<GlobalInner>);
impl Global {
    const fn new() -> Self {
        Self(UnsafeCell::new(GlobalInner {
            device: None,
            input: CircularBuffer::new(),
            output: CircularBuffer::new(),
        }))
    }
    pub fn lock(&self) -> GlobalGuard {
        // no locking for now
        unsafe {
            GlobalGuard(&mut *self.0.get())
        }
    }
    pub fn try_lock(&self) -> Option<GlobalGuard> {
        // no locking for now
        Some(self.lock())
    }
}
unsafe impl Sync for Global {}
struct GlobalInner {
    // lock: Spinlock,
    device: Option<&'static dyn Serial>,
    input: CircularBuffer,
    output: CircularBuffer,
}
pub struct GlobalGuard<'a>(&'a mut GlobalInner);
impl<'a> GlobalGuard<'a> {
    #[inline]
    pub fn device(&self) -> Option<&'static dyn Serial> {
        self.0.device
    }
    #[inline]
    pub fn write_fmt(&mut self, args: fmt::Arguments<'_>) -> fmt::Result {
        <Self as fmt::Write>::write_fmt(self, args)
    }
}
impl<'a> fmt::Write for GlobalGuard<'a> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        let Some(serial) = self.0.device else {
            return Err(fmt::Error)
        };
        for byte in s.as_bytes() {
            if let Err(_) = serial.write_byte(*byte) {
                return Err(fmt::Error);
            };
        }
        Ok(())
    }
}

pub unsafe trait AsUninitBuffer {
    fn as_uninit_buffer(&mut self) -> (*mut u8, usize);
}
unsafe impl<const N: usize> AsUninitBuffer for [u8; N] {
    fn as_uninit_buffer(&mut self) -> (*mut u8, usize) {
        (self.as_mut_ptr(), self.len())
    }
}
unsafe impl AsUninitBuffer for [u8] {
    fn as_uninit_buffer(&mut self) -> (*mut u8, usize) {
        (self.as_mut_ptr(), self.len())
    }
}
unsafe impl AsUninitBuffer for [MaybeUninit<u8>] {
    fn as_uninit_buffer(&mut self) -> (*mut u8, usize) {
        (self.as_mut_ptr().cast(), self.len())
    }
}

pub fn print_fmt(args: fmt::Arguments<'_>) {
    let _ = GLOBAL.lock().write_fmt(args);
}
