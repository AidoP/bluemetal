//! Build dependencies inside of a workspace getting cross compiled seem to
//! be bugged and built as a normal dependency as well.
//!
//! Once fixed this file can be removed.
#![cfg_attr(not(feature = "fix-build-dep-bug"), no_std)]
#![allow(special_module_name)]

#[cfg(feature = "fix-build-dep-bug")]
mod lib;
#[cfg(feature = "fix-build-dep-bug")]
pub use lib::*;
