#![feature(ptr_metadata)]

use std::{mem::MaybeUninit, num::{NonZero, NonZeroU32}, ops::{Deref, Sub}, ptr::NonNull, u16};

// FIXME: This module uses unsized types extensively where thin unsized types
// would be preferable once available in the language.

/// Suitably aligned storage for a system description table.
///
/// Allows for unsizing coercions for aligned byte slices.
#[derive(Debug)]
#[repr(C, align(16))]
pub struct DynStorage<T: ?Sized>(pub T);
impl Storage {
    pub const fn len(&self) -> usize {
        self.0.len()
    }
}
pub type Storage = DynStorage<[u8]>;
pub type MaybeUninitStorage = DynStorage<[core::mem::MaybeUninit::<u8>]>;

#[macro_export]
macro_rules! include {
    ($file:expr $(,)?) => {
        unsafe {
            const STORAGE: &$crate::Storage = &$crate::DynStorage(*::core::include_bytes!($file));
            $crate::System::from_storage(STORAGE)
        }
    };
}
#[macro_export]
macro_rules! uninit_storage {
    ($size:literal) => {
        &mut $crate::DynStorage::<[::core::mem::MaybeUninit::<::core::primitive::u8>; $size]>(
            unsafe { ::core::mem::MaybeUninit::uninit().assume_init() }
        )
    };
}

/// A buffer containg a system description table.
///
// Safety: Writing `MaybeUninit::uninit()` to the slice is undefined behaviour.
// TODO: Adopt an unsized type with thin pointers once available.
#[repr(transparent)]
pub struct System([MaybeUninit<u8>]);
impl System {
    pub fn new(DynStorage(storage): &mut MaybeUninitStorage, sections: u16) -> Option<&mut Self> {
        if storage.len() < size_of::<Header>() {
            return None;
        }
        let capacity = storage.len() as u32;
        let system = unsafe { &mut *(storage as *mut _ as *mut Self) };
        const SECTIONS_OFFSET: usize = size_of::<Header>();
        unsafe {
            let sections = SectionTable::init(&mut system.as_mut_bytes()[SECTIONS_OFFSET..], sections)?;
            let len = (SECTIONS_OFFSET + sections.len()) as u32;
            system.cast_mut::<Header>().write(Header {
                endian: Endian::NATIVE,
                version: Header::LATEST_VERSION,
                ptr_size: PtrSize::NATIVE,
                capacity,
                len,
                // immediately following the header
                sections: SECTIONS_OFFSET as u32,
                // no memory section
                memory: u16::MAX,

                _reserved1: Default::default(),
                _reserved2: Default::default(),
                _reserved3: Default::default(),
                _reserved4: Default::default(),
            });
        };
        Some(system)
    }

    pub unsafe fn from_storage(DynStorage(storage): &mut MaybeUninitStorage) {

    }

    pub fn len(&self) -> usize {
        self.header().capacity as usize
    }

    #[inline]
    fn as_bytes(&self) -> &[MaybeUninit<u8>] {
        &self.0
    }
    #[inline]
    fn as_mut_bytes(&mut self) -> &mut [MaybeUninit<u8>] {
        &mut self.0
    }

    /// Reinterpret the `offset` from the start of the system description table
    /// as `&T`.
    ///
    /// # Safety
    /// It is undefined behaviour for the memory at the specified offset to not
    /// be aligned for `T` or not be valid for a read of `T`.
    unsafe fn cast<T>(&self, offset: u32) -> &T {
        unsafe { &*self.0.as_ptr().add(offset as usize).cast::<T>() }
    }
    fn cast_mut<T>(&mut self) -> NonNull<T> {
        unsafe { NonNull::new_unchecked(&mut self.0) }.cast()
    }

    /// Returns a reference to the header describing the system structure.
    pub fn header(&self) -> &Header {
        unsafe { self.cast(0) }
    }
    /// Returns a mutable reference to the header describing the system structure.
    pub fn header_mut(&mut self) -> &mut Header {
        unsafe { self.cast_mut().as_mut() }
    }

    pub fn section_table(&self) -> &SectionTable {
        unsafe {
            SectionTable::from_ptr(
                self.as_bytes().as_ptr().add((*self).sections as usize)
            )
        }
    }
    pub fn section_table_mut(&mut self) -> &mut SectionTable {
        unsafe {
            SectionTable::from_mut_ptr(
                self.as_mut_bytes().as_mut_ptr().add((*self).sections as usize)
            )
        }
    }
}

impl core::ops::Deref for System {
    type Target = Header;
    fn deref(&self) -> &Self::Target {
        self.header()
    }
}
impl core::ops::DerefMut for System {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.header_mut()
    }
}

#[derive(Debug)]
#[repr(C, align(16))]
pub struct Header {
    endian: Endian,
    /// The version of this structure.
    /// Specifies the presence of extension areas.
    version: u16,
    ptr_size: PtrSize,
    _reserved1: u8,
    /// Total size of the system description table including reserved memory in
    /// bytes.
    ///
    /// May be ignored for files at rest and set arbitrarily larger when loaded.
    capacity: u32,
    /// Offset to the unallocated memory reserved for the system description
    /// table.
    len: u32,
    /// Offset to the section table.
    sections: u32,
    _reserved2: u32,
    /// Index of the memory section.
    memory: u16,
    _reserved3: u16,
    _reserved4: [u64; 3],
}
impl Header {
    pub const LATEST_VERSION: u16 = 0;
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
impl core::fmt::Debug for Endian {
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
struct SectionTableHeader {
    /// The length of `sections`.
    len: u16,
    _reserved1: [u16; 3],
    _reserved2: [u64; 3],
}
#[derive(Debug)]
#[repr(C)]
pub struct SectionTable {
    header: SectionTableHeader,
    // Intended to be thin unsized. Do not use the pointer metadata.
    pub sections: [Section],
}
impl SectionTable {
    /// Initialise the section table with a maximum length of `len`.
    /// Returns the complete section table, or [`None`] if the buffer is too
    /// small.
    ///
    /// # Panics
    /// Panics if `buffer` is not aligned suitably.
    pub unsafe fn init(buffer: &mut [MaybeUninit<u8>], len: u16) -> Option<&mut SectionTable> {
        // https://github.com/rust-lang/rust/issues/96284
        assert!(buffer.as_mut_ptr().addr() & (align_of::<SectionTableHeader>() - 1) == 0);
        if buffer.len() < size_of::<SectionTableHeader>() + usize::from(len) {
            return None;
        }
        let header = buffer.as_mut_ptr().cast::<SectionTableHeader>();
        unsafe {
            header.write(SectionTableHeader {
                len,
                _reserved1: Default::default(),
                _reserved2: Default::default(),
            });
        }
        let mut section = unsafe { header.add(1) }.cast::<Section>();
        let section_end = unsafe { section.add(len as usize) };
        while section != section_end {
            unsafe {
                section.write(Section::default());
                section = section.add(1);
            }
        }
        unsafe {
            // Safety: The structure is fully initialized.
            Some(Self::from_mut_ptr(buffer.as_mut_ptr()))
        }
    }
    unsafe fn from_ptr<'a>(buffer: *const MaybeUninit<u8>) -> &'a Self {
        // FIXME: The length here is unnecessary - should be a thin pointer.
        let len = unsafe { &*buffer.cast::<SectionTableHeader>() }.len;
        unsafe {
            &*core::ptr::from_raw_parts(
                buffer,
                len as usize
            )
        }
    }
    unsafe fn from_mut_ptr<'a>(buffer: *mut MaybeUninit<u8>) -> &'a mut Self {
        // FIXME: The length here is unnecessary - should be a thin pointer.
        let len = unsafe { &*buffer.cast::<SectionTableHeader>() }.len;
        unsafe {
            &mut *core::ptr::from_raw_parts_mut(
                buffer,
                len as usize
            )
        }
    }
    /// The length of the section table in bytes.
    pub fn len(&self) -> usize {
        size_of::<SectionTableHeader>() + (usize::from(self.header.len) * size_of::<Section>())
    }
}

#[derive(Debug)]
#[repr(transparent)]
pub struct SectionType(u32);
impl SectionType {
    /// Unused section entry, does not point at anything.
    pub const NONE: Self = Self(0);
}

#[derive(Debug)]
#[repr(C)]
pub struct Section {
    ty: SectionType,
    offset: u32,
    _reserved: [u64; 3],
}
impl Default for Section {
    fn default() -> Self {
        Self {
            ty: SectionType::NONE,
            offset: Default::default(),
            _reserved: Default::default(),
        }
    }
}
