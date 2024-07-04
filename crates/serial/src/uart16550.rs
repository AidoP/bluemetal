use crate::Serial;

const UART0: Uart = unsafe { Uart::at_address(0x1000_0000) };

pub fn uart16550(num: usize) -> Option<&'static dyn Serial> {
    match num {
        0 => Some(unsafe { UART0.init() }),
        _ => None,
    }
}

struct Uart(*mut u8);
impl Uart {
    #[inline]
    const unsafe fn at_address(address: usize) -> Self {
        Uart(address as *mut u8)
    }
    unsafe fn init(&self) -> &Self {
        // set LCR - line control register
        let lcr = 0b0000_0011;
        self.0.add(3).write_volatile(lcr);
        // set FCR - FIFO control register
        self.0.add(2).write_volatile(0b0000_0001);
        // set IER - interrupt enable register
        self.0.add(1).write_volatile(0b0000_0001);

        // set divisor latch access
        self.0.add(3).write_volatile(lcr | 0b1000_0000);

        let [div_low, div_high] = 592u16.to_ne_bytes();
        // write divisor
        self.0.add(0).write_volatile(div_low);
        self.0.add(1).write_volatile(div_high);

        // unset divisor latch access
        self.0.add(3).write_volatile(lcr);

        self
    }
}
impl Serial for Uart {
    fn read_byte(&self) -> Result<u8, crate::Error> {
        unsafe {
            // data ready bit unset
            if self.0.add(5).read_volatile() & 1 == 0 {
                return Err(crate::Error::Busy);
            }
            Ok(self.0.add(0).read_volatile() as u8)
        }
    }
    fn write_byte(&self, byte: u8) -> Result<(), crate::Error> {
        unsafe {
            self.0.add(0).write_volatile(byte);
            Ok(())
        }
    }
}

