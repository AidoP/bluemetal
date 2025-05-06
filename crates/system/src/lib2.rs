#![no_std]
//! # System Description Structure
//!
//! A description of the system, such as the available devices and the memory
//! map.

//#[cfg(feature = "native")]
//pub mod native;

use core::mem::MaybeUninit;

struct Area {
    offset: u32,
    len: u16,
    capacity: u16,
}

const MAGIC: [u8; 8] = *b"\xc2\xb5system";
pub const MAGIC_NATIVE: u64 = u64::from_ne_bytes(MAGIC);
pub const MAGIC_BE: u64 = u64::from_be_bytes(MAGIC);
pub const MAGIC_LE: u64 = u64::from_le_bytes(MAGIC);

#[repr(align(16))]
#[repr(C)]
pub struct Header {
    pub magic: u64,
    capacity: u32,
    pub version: u16,
    addr_size: u8,
    _reserved: u8,
    memory: Area,
    devices: Area,
}

macro_rules! non_exhaustive_enum {
    (
        $(#[$meta:meta])*
        $vis:vis enum $name:ident($ty:ty) {
            $(
                $(#[$variant_meta:meta])*
                $variant_vis:vis $variant:ident = $value:expr,
            )*
        }
    ) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        #[repr(transparent)]
        $vis struct $name($ty);
        impl $name {
            #[inline]
            pub const fn from_raw(raw: $ty) -> Self {
                Self(raw)
            }
            #[inline]
            pub const fn to_raw(self) -> $ty {
                self.0
            }
            $(
                $(#[$variant_meta])*
                $variant_vis const $variant: Self = Self($value);
            )*
        }
        impl From<$ty> for $name {
            fn from(value: $ty) -> Self {
                Self::from_raw(value)
            }
        }
        impl From<$name> for $ty {
            fn from(value: $name) -> Self {
                value.to_raw()
            }
        }
    };
}

non_exhaustive_enum!{
    pub enum Protocol(u32) {
        pub SIFIVE_UART = 0x0000_0001,
    }
}
non_exhaustive_enum!{
    pub enum MemoryType(u32) {
        pub STANDARD = 0x0000_0001,
    }
}




#[repr(C, align(16))]
pub struct System([MaybeUninit<u8>]);
impl System {
    pub fn new(storage: &mut [MaybeUninit<u8>]) -> &mut Self {
        
    }
}
