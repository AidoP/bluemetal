#![no_std]
#![allow(internal_features)]
#![feature(allow_internal_unstable)]

mod global;
pub use global::{global, init, print_fmt};

pub mod prelude {
    pub use crate::{print, println};
}

#[allow_internal_unstable(print_internals)]
#[macro_export]
macro_rules! print {
    ($($arg:tt)*) => {{
        $crate::print_fmt(::core::format_args!($($arg)*));
    }};
}
#[allow_internal_unstable(print_internals, format_args_nl)]
#[macro_export]
macro_rules! println {
    () => {
        $crate::print!("\n")
    };
    ($($arg:tt)*) => {{
        $crate::print_fmt(::core::format_args_nl!($($arg)*));
    }};
}

pub enum Error {
    /// The serial device is not ready to send or recieve more data.
    Busy,
}

pub trait Serial {
    fn read_byte(&self) -> Result<u8, Error>;

    /// Read bytes from the device in to `buffer[i]` while `i` is less than
    /// `len`.
    ///
    /// Returns the error and number of bytes read into `buffer`.
    ///
    /// # Safety
    /// `ptr` must be valid for writes for up to `len` bytes.
    #[inline]
    unsafe fn read(&self, ptr: *mut u8, len: usize) -> (usize, Result<(), Error>) {
        let mut i = 0;
        while i != len {
            match self.read_byte() {
                Err(e) => return (i, Err(e)),
                Ok(b) => ptr.add(i).write(b),
            };
            i += 1;
        }
        (i, Ok(()))
    }

    fn write_byte(&self, byte: u8) -> Result<(), Error>;
    /// Write all of `bytes` to the device.
    ///
    /// Returns the error and number of bytes written on failure.
    #[inline]
    fn write(&self, bytes: &[u8]) -> (usize, Result<(), Error>) {
        let mut i = 0;
        while i < bytes.len() {
            if let Err(e) = self.write_byte(unsafe { *bytes.get_unchecked(i) }) {
                return (i, Err(e));
            };
            i += 1;
        }
        (i, Ok(()))
    }
}

#[cfg(target_device = "sifive_uart")]
pub mod sifive_uart;
#[cfg(target_device = "sifive_uart")]
pub use sifive_uart::sifive_uart;
#[cfg(not(target_device = "sifive_uart"))]
pub fn sifive_uart(_: usize) -> Option<Device> { None }
