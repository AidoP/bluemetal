#![no_std]

#[derive(Clone, Copy)]
struct bu32(u32);
impl bu32 {
    pub fn read(self) -> u32 {
        
    }
    pub fn write(&mut self, value: u32) {
        
    }
}
impl core::fmt::Debug for bu32 {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        core::fmt::Debug::fmt(self.read()
    }
}

#[repr(C)]
struct DeviceTree {
    magic: u32,
    total_size: u32,
    off_dt_struct: u32,
    off_dt_strings: u32,
    off_mem_rsvmap: u32,
    version: u32,
}
