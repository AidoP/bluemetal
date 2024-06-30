use crate::Serial;

// Safety: this module is only enabled if the `sifive_uart` device is enabled,
// which always has the UART at these addresses.
const UART0: Uart = unsafe { Uart::at_address(0x10010000) };
const UART1: Uart = unsafe { Uart::at_address(0x10011000) };

pub fn sifive_uart(num: usize) -> Option<&'static dyn Serial> {
    // Safety: the target machine has been configured to have a compatible
    // UART at the correct addresses.
    match num {
        0 => Some(&UART0),
        1 => Some(&UART1),
        _ => None,
    }
}

struct Uart(*mut u32);
impl Uart {
    #[inline]
    const unsafe fn at_address(address: usize) -> Self {
        Uart(address as *mut u32)
    }
}
impl Serial for Uart {
    fn read_byte(&self) -> Result<u8, crate::Error> {
        unsafe {
            let rxdata = self.0.add(1);
            let data = rxdata.read_volatile();
            if data & 0x8000_0000 != 0 {
                return Err(crate::Error::Busy);
            }
            Ok(data as u8)
        }
    }
    fn write_byte(&self, byte: u8) -> Result<(), crate::Error> {
        unsafe {
            let txdata = self.0;
            if txdata.read_volatile() & 0x8000_0000 != 0 {
                return Err(crate::Error::Busy);
            }
            txdata.write_volatile(byte as u32);
            Ok(())
        }
    }
}
