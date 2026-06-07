#![allow(unused)]
use std::sync::atomic::{AtomicUsize, Ordering};

static FORMATTED: AtomicUsize = AtomicUsize::new(0);
pub fn is_formatted() -> bool {
    FORMATTED.load(Ordering::Relaxed) != 0
}
pub fn set_formatted(value: bool) {
    FORMATTED.store(value as usize, Ordering::Relaxed)
}
pub fn reset() -> &'static str {
    if is_formatted() { "\x1b[0m" } else { "" }
}
pub fn red() -> &'static str{
    if is_formatted() { "\x1b[31m" } else { "" }
}
pub fn yellow() -> &'static str{
    if is_formatted() { "\x1b[33m" } else { "" }
}
pub fn green() -> &'static str{
    if is_formatted() { "\x1b[32m" } else { "" }
}
pub fn bright_green() -> &'static str{
    if is_formatted() { "\x1b[92m" } else { "" }
}
pub fn bright_blue() -> &'static str {
    if is_formatted() { "\x1b[94m" } else { "" }
}
pub fn bright_magenta() -> &'static str {
    if is_formatted() { "\x1b[95m" } else { "" }
}
pub fn bright_cyan() -> &'static str {
    if is_formatted() { "\x1b[96m" } else { "" }
}

#[allow(unused)]
#[macro_export]
macro_rules! fmtprintln {
    ($, $hl:ident; $msg:literal, $fmt:literal $($tt:tt)*) => {
        ::std::eprintln!(
            ::core::concat!($msg, $fmt),
            reset=$crate::fmt::reset(),
            red=$crate::fmt::red(),
            yellow=$crate::fmt::yellow(),
            green=$crate::fmt::green(),
            bright_green=$crate::fmt::bright_green(),
            bright_blue=$crate::fmt::bright_blue(),
            bright_magenta=$crate::fmt::bright_magenta(),
            bright_cyan=$crate::fmt::bright_cyan() $($tt)*
        )
    };
}

#[allow(unused)]
#[macro_export]
macro_rules! error {
    ($fmt:literal $($tt:tt)*) => {
        $crate::fmtprintln!("{red}error:{reset} ", $fmt, $($tt)*)
    };
}
#[allow(unused)]
#[macro_export]
macro_rules! warning {
    ($fmt:literal $($tt:tt)*) => {
        $crate::fmtprintln!("{yellow}warning:{reset} ", $fmt, $($tt)*)
    };
}
#[allow(unused)]
#[macro_export]
macro_rules! hint {
    ($fmt:literal $($tt:tt)*) => {
        $crate::fmtprintln!("{bright_blue}hint:{reset} ", $fmt, $($tt)*)
    };
}
#[allow(unused)]
#[macro_export]
macro_rules! debug {
    ($fmt:literal $(,$tt:tt)*) => {
        ::std::eprintln!(::core::concat!("{bright_magenta}debug:{reset} ", $fmt, "\n{}"), $crate::fmt::bright_magenta(), $crate::fmt::reset() $(,$tt)*, std::backtrace::Backtrace::force_capture())
    };
}
