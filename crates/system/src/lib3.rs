#![no_std]
use core::{fmt::Debug, mem::MaybeUninit};


macro_rules! impl_unsized_from_raw_parts {
    ($ty:ty) => {
        impl $ty {
            #[inline]
            const unsafe fn from_raw_parts<'a>(data: *const u8, len: usize) -> &'a Self {
                (::core::ptr::slice_from_raw_parts(data, len) as *const Self).as_ref().unwrap_unchecked()
            }
            #[inline]
            const unsafe fn from_raw_parts_mut<'a>(data: *mut u8, len: usize) -> &'a mut Self {
                (::core::ptr::slice_from_raw_parts_mut(data, len) as *mut Self).as_mut().unwrap_unchecked()
            }
        }
    };
}

/// Suitably aligned storage for a system description table.
///
/// Typically `include!("path")` would be used instead.
#[derive(Debug)]
#[repr(C, align(16))]
pub struct DynStorage<T: ?Sized>(pub T);
impl DynStorage<[u8]> {
    pub const fn len(&self) -> usize {
        self.0.len()
    }
}
pub type Storage = DynStorage<[u8]>;

#[macro_export]
macro_rules! include {
    ($file:expr $(,)?) => {
        unsafe {
            const STORAGE: &$crate::Storage = &$crate::DynStorage(*::core::include_bytes!($file));
            $crate::System::from_storage(STORAGE)
        }
    };
}

#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(u64)]
pub enum Endian {
    Big = u64::from_be_bytes(*b"\xc2\xb5system"),
    Little = u64::from_le_bytes(*b"\xc2\xb5system"),
}
impl Endian {
    #[cfg(target_endian = "big")]
    pub const NATIVE: Self = Self::Big;
    #[cfg(target_endian = "little")]
    pub const NATIVE: Self = Self::Little;
}
impl Debug for Endian {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Big => write!(f, "Endian::Big"),
            Self::Little => write!(f, "Endian::Little"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PtrSize {
   Bits16 = 2,
   Bits32 = 4,
   Bits64 = 8,
   Bits128 = 16,
}
impl PtrSize {
    #[cfg(target_pointer_width = "16")]
    pub const NATIVE: Self = Self::Bits16;
    #[cfg(target_pointer_width = "32")]
    pub const NATIVE: Self = Self::Bits32;
    #[cfg(target_pointer_width = "64")]
    pub const NATIVE: Self = Self::Bits64;
}

#[derive(Debug)]
#[repr(C, align(16))]
pub struct System {
    endian: Endian,
    /// The version of this structure.
    /// Specifies the presence of extension areas.
    version: u16,
    ptr_size: PtrSize,
    _reserved1: u8,
    /// Total size of the system description table.
    ///
    /// As such,
    /// ```ignore
    /// (&mut raw self.magic).cast::<u8>().add(self.capacity).write(1)
    /// ```
    /// would be an invalid write.
    len: u32,
    _reserved2: [u64; 4],

    // Version 0
    memory: Section,

    // Future Versions
    // _reserved: [u8],
    data: [u8],
}
impl System {
    pub const fn new(DynStorage(storage): &mut DynStorage<[MaybeUninit<u8>]>) -> Option<&mut Self> {
        assert!(align_of_val(storage) == align_of::<Self>());
        if storage.len() < size_of::<Self>() {
            return None;
        }
        unsafe {
            storage.as_mut_ptr().cast::<Self>().write(Self {
                endian: Endian::NATIVE,
                version: 0,
                ptr_size: PtrSize::NATIVE,
                len: 1,
                memory: Section {
                    offset: 0,
                    len: 0,
                    _reserved: 0,
                },
                
                _reserved1: 0,
                _reserved2: [0; _],
            });
        }
    }
    pub const fn from_storage(storage: &Storage) -> Option<&Self> {
        
        if storage.len() < size_of::<Self>() {
            return None;
        }
        let magic: u64 = unsafe { *storage.0.as_ptr().cast() };
        if magic != Endian::Big as u64 && magic != Endian::Little as u64 {
            return None;
        }
        let header = unsafe {
            &*(storage as *const _ as *const Self)
        };
        let PtrSize::NATIVE = header.ptr_size else {
            return None;
        };
        if header.len > usize::MAX as u32 || header.len as usize > storage.len() {
            return None;
        }
        Some(header)
    }
}

#[derive(Clone, Copy, Debug)]
#[repr(C, align(8))]
pub struct Section {
    offset: u32,
    len: u32,
    _reserved: u64,
}

#[derive(Debug)]
#[repr(C)]
pub struct Memory {
    len: u16,
    _reserved1: [u16; 3],
    _reserved: [u64; 7],
    regions: [MemoryRegion],
}
impl_unsized_from_raw_parts!(Memory);
#[derive(Debug)]
#[repr(C)]
pub struct MemoryRegion {
    ty: MemoryType,
}
#[derive(Clone,Copy, Debug, PartialEq, Eq)]
#[repr(transparent)]
pub struct MemoryType(u16);
impl MemoryType {
    pub const NONE: Self = Self(0);
    pub const STANDARD: Self = Self(1);
}
