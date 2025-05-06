pub struct Target {
    ptr_size: PtrSize,
    endian: Endian,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(transparent)]
pub struct PtrSize(u8);
impl PtrSize {
    const BITS8: Self = Self(1);
    const BITS16: Self = Self(2);
    const BITS32: Self = Self(4);
    const BITS64: Self = Self(8);
    const BITS128: Self = Self(16);

    #[cfg(target_pointer_width = "16")]
    pub const NATIVE: Self = Self::BITS16;
    #[cfg(target_pointer_width = "32")]
    pub const NATIVE: Self = Self::BITS32;
    #[cfg(target_pointer_width = "64")]
    pub const NATIVE: Self = Self::BITS64;
}

pub enum Endian {
    Big,
    Little,
}

pub struct System {
    target: Target,
}
impl System {
    pub fn new(target: Target) -> Self {
        Self {
            target,
        }
    }
}

pub struct Memory {

}
