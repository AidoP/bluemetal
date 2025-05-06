#![allow(dead_code)]
use super::{Endian, sealed};

pub enum Error {
    InvalidPtrSize,
    UnsupportedEndian,
    UnsupportedPtrSize,
}

macro_rules! impl_int {
    (type $name:ident = $int:ty) => {
        #[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
        #[repr(transparent)]
        pub struct $name<Target: sealed::Target>($int, core::marker::PhantomData<Target>);
        impl<Target: sealed::Target> $name<Target> {
            #[inline]
            pub const fn read(self, endian: Endian) -> $int {
                if Target::NATIVE {
                    return self.0;
                }
                match endian {
                    Endian::Big => <$int>::from_be(self.0),
                    Endian::Little => <$int>::from_le(self.0),
                }
            }
            #[inline]
            pub const fn write(&mut self, endian: Endian, value: $int) {
                if Target::NATIVE {
                    self.0 = value;
                    return;
                }
                match endian {
                    Endian::Big => self.0 = value.to_be(),
                    Endian::Little => self.0 = value.to_le(),
                }
            }
        }
        impl<Target: sealed::Target> Clone for $name<Target> {
            fn clone(&self) -> Self {
                Self(self.0, self.1)
            }
        }
        impl<Target: sealed::Target> Copy for $name<Target> {}
    };
}

impl_int!(type Ux16 = u16);
impl_int!(type Ix16 = i16);
impl_int!(type Ux32 = u32);
impl_int!(type Ix32 = i32);
impl_int!(type Ux64 = u64);
impl_int!(type Ix64 = i64);


#[cfg(target_pointer_width = "16")]
type UxSize<Target> = Ux16<Target>;
#[cfg(target_pointer_width = "16")]
type IxSize<Target> = Ix16<Target>;

#[cfg(target_pointer_width = "32")]
type UxSize<Target> = Ux32<Target>;
#[cfg(target_pointer_width = "32")]
type IxSize<Target> = Ix32<Target>;

#[cfg(target_pointer_width = "64")]
type UxSize<Target> = Ux64<Target>;
#[cfg(target_pointer_width = "64")]
type IxSize<Target> = Ix64<Target>;
