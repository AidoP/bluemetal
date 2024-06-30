extern "C" {
    static EH_FRAME: CallFrameInfo;
}

#[repr(C)]
#[derive(Clone, Copy)]
struct CallFrameInfo {
    ptr: *const u8,
    len: usize,
}
impl CallFrameInfo {
    fn new() -> Self {
        unsafe { EH_FRAME }
    }
    fn length(self) -> u32 {
        unsafe {
            self.ptr.cast::<u32>().read()
        }
    }
    fn cie_id(self) -> u32 {
        unsafe {
            self.ptr.add(4).cast::<u32>().read()
        }
    }
    fn version(self) -> u8 {
        unsafe {
            self.ptr.add(8).cast::<u8>().read()
        }
    }
}
impl core::fmt::Debug for CallFrameInfo {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("CallFrameInfo")
            .field("length", &self.length())
            .field("cie_id", &self.cie_id())
            .field("version", &self.version())
            .finish()
    }
}
