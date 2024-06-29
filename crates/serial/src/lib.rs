#![no_std]

use core::mem::MaybeUninit;

pub enum Error {
    /// The serial device is not ready to send or recieve more data.
    Busy,
}

trait Serial {
    fn read_byte(&self) -> Result<u8, Error>;

    /// Read bytes from the device in to `buffer[i]` while `i` is less than
    /// `len`.
    ///
    /// Returns the error and number of bytes read into `buffer`.
    ///
    /// # Safety
    /// `ptr` must be valid for writes for up to `len` bytes.
    #[inline]
    unsafe fn read(&self, ptr: *mut u8, len: usize) -> Result<usize, (Error, usize)> {
        let mut i = 0;
        while i != len {
            match self.read_byte() {
                Err(Error::Busy) => return Ok(i),
                Ok(b) => ptr.add(i).write(b),
            };
            i += 1;
        }
        Ok(i)
    }

    fn write_byte(&self, byte: u8) -> Result<(), Error>;
    /// Write all of `bytes` to the device.
    ///
    /// Returns the error and number of bytes written on failure.
    #[inline]
    fn write(&self, bytes: &[u8]) -> Result<(), (Error, usize)> {
        for (i, &b) in bytes.into_iter().enumerate() {
            self.write_byte(b).map_err(|e| (e, i))?;
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

/// A serial I/O device, typically a UART.
#[derive(Clone, Copy)]
pub struct Device(&'static dyn Serial);
impl Device {
    const fn new(device: &'static dyn Serial) -> Self {
        Self(device)
    }
    #[inline]
    pub fn read_byte(self) -> Result<u8, Error> {
        self.0.read_byte()
    }
    #[inline]
    pub fn read<T: AsUninitBuffer>(self, buffer: &mut T) -> Result<usize, (Error, usize)> {
        let (ptr, len) = buffer.as_uninit_buffer();
        // Safety: `AsUninitBytes::as_uninit_buffer()` guarantees a valid `ptr`
        // and `len` pair.
        unsafe { self.0.read(ptr, len) }
    }
    #[inline]
    pub fn write(self, bytes: &[u8]) -> Result<(), (Error, usize)> {
        self.0.write(bytes)
    }
    #[inline]
    pub fn write_byte(self, byte: u8) -> Result<(), Error> {
        self.0.write_byte(byte)
    }
    #[inline]
    pub fn write_fmt(mut self, args: core::fmt::Arguments<'_>) -> core::fmt::Result {
        <Self as core::fmt::Write>::write_fmt(&mut self, args)
    }
}
impl core::fmt::Write for Device {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        let mut bytes = s.as_bytes();
        loop {
            match self.write(s.as_bytes()) {
                Ok(()) => break Ok(()),
                Err((Error::Busy, written)) => {
                    bytes = &bytes[written..];
                    continue
                },
            }
        }
    }
}

pub fn default() -> Option<Device> {
    sifive_uart(0)
        .or_else(|| sifive_uart(1))
}

#[cfg(target_device = "sifive_uart")]
pub mod sifive_uart;
#[cfg(target_device = "sifive_uart")]
pub use sifive_uart::sifive_uart;
#[cfg(not(target_device = "sifive_uart"))]
pub fn sifive_uart(_: usize) -> Option<Device> { None }
