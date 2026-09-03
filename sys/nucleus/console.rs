use core::sync::atomic::AtomicPtr;

/// A console monitoring interface.
///
/// Provides input and/or output for the system console.
pub trait Monitor {

}

/// A log of the console messages.
struct Log {

}

struct Message {
    size: u16,
    category: u16,
    id: u32,
    // data: impl ([u8] + Thin),
}

struct Console {
    // TODO: make linked list to support multiple
    monitors: Option<&'static mut dyn Monitor>,
    buffer: AtomicPtr<Message>,
    buffer_size: usize,
}

static CONSOLE: Console = Console {
    monitors: None,
    buffer: AtomicPtr::new(core::ptr::null_mut()),
    buffer_size: 0,
};
