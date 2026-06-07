use std::{marker::PhantomData, mem::MaybeUninit, ptr::NonNull};


#[derive(Clone, Copy)]
#[repr(transparent)]
struct Metadata(u8);
impl Metadata {
    const EMPTY: u8 = 0xFF;
    const DELETED: u8 = 0x80;
    pub const fn empty() -> Self {
        Self(Self::EMPTY)
    }
    const fn is_empty(self) -> bool {
        self.0 == Self::EMPTY
    }
    const fn is_deleted(self) -> bool {
        self.0 == Self::DELETED
    }
    const fn is_populated(self) -> bool {
        self.0 & 0x80 == 0
    }
    const fn hash_prefix(self) -> u64 {
        self.0 as u64 & 0x7F
    }
}

type GroupWord = cfg_select!{
    any(
        target_pointer_width = "64",
        target_arch = "x86_64",
        target_arch = "s390x",
        target_arch = "riscv64",
    ) => u64,
    _ => u32,
};

#[repr(C)]
struct Group {
    _align: [GroupWord; 0],
    data: [Metadata; size_of::<GroupWord>()],
}
impl Group {
    pub const fn new() -> Group {
        Group {
            _align: [],
            data: [Metadata::empty(); _],
        }
    }
}

struct RawDict {
    mask: usize,
    len: usize,
    remaining: usize,
    /// Points to the metadata, which immediately follows the data.
    metadata: NonNull<u8>,
}
impl RawDict {
    pub const fn new() -> Self {
        const METADATA: Group = Group::new();
        Self {
            mask: 0,
            len: 0,
            remaining: 0,
            metadata: NonNull::from_ref(&METADATA).cast(),
        }
    }
    // pub unsafe fn new(data: *mut u8, mask: usize) -> Self {
    // }

    // pub const fn search() -> Option<&> {
    //
    // }
}

// struct Dict<K, V, H>([u8]);

/// A deterministic, const-compatible implementation of [Fold Hash](https://github.com/orlp/foldhash).
struct Hasher {
    acc: u64,
    sponge: u128,
    sponge_len: u8,
}
impl Hasher {
    const SEEDS: [u64; 6] = [
        0x3c3951d02ade9cc5,
        0x2fae55d6d452088b,
        0xe89866d6276ce026,
        0x1c0f6aaee63ea5ad,
        0xe05a4a16ed699292,
        0x4c23461141849d02,
    ];
    pub const fn new() -> Self {
        Self {
            acc: 0x537d2c054560385c,
            sponge: 0,
            sponge_len: 0,
        }
    }
}
pub const fn hash(data: &[u8]) -> u64 {
    /// Roughly rotate a value.
    /// On platforms that do not support 64-bit operations, this can be approximated.
    const fn rough_rotate_right(value: u64, w: u32) -> u64 {
        value.rotate_right(w)
    }
    const fn folded_multiply(lhs: u64, rhs: u64) -> u64 {
        let full = (lhs as u128).wrapping_mul(rhs as u128);
        let low = full as u64;
        let high = (full >> 64) as u64;
        low ^ high
    }
    let mut state = Hasher::new();
    state.acc = rough_rotate_right(state.acc, data.len() as u32);
    if data.len() < 16 {

    } else {

    }
    if state.sponge_len > 0 {
        let low = state.sponge as u64;
        let high = (state.sponge >> 64) as u64;
        folded_multiply(low ^ state.acc, high ^ Hasher::SEEDS[0])
    } else {
        state.acc
    }
}

struct Swiss<'a, T, const N: usize> {
    metadata: [Metadata; N],
    _key: PhantomData<&'a str>,
    key_len: [MaybeUninit<u8>; N],
    key_ptr: [MaybeUninit<*const u8>; N],
    values: [MaybeUninit<T>; N],
    remaining: usize,
    len: usize,
}
impl<'a, T, const N: usize> Swiss<'a, T, N> {
    const MASK: usize = N - 1;
    pub const fn new() -> Self {
        Self {
            metadata: [Metadata::empty(); _],
            _key: PhantomData,
            key_len: unsafe { MaybeUninit::uninit().assume_init() },
            key_ptr: unsafe { MaybeUninit::uninit().assume_init() },
            values: unsafe { MaybeUninit::uninit().assume_init() },
            remaining: 0,
            len: 0,
        }
    }
    pub const fn search() -> Option<&T> {

    }
}
