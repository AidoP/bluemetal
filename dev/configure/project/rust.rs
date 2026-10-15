use std::path::PathBuf;
use super::syntax::Span;

pub struct Crate {
    pub name: String,
    pub name_span: Span,
    pub path: PathBuf,
    /// The target the crate is compiled for, which is a builtin target or one declared by a `target` statement.
    pub target: String,
    pub target_span: Span,
    pub kind: Kind,
    /// The path of the output, instead of the one in the target's directory.
    pub output: Option<PathBuf>,
    pub builtin: bool,
    /// Whether the crate is part of the build system rather than declared by the configuration, so builtin crates are
    /// not passed to it.
    pub implicit: bool,
    /// Whether the crate is built by `ninja` without targets.
    pub default: bool,
    /// Whether the crate's tests are built as a test harness, and registered as a test named after the crate.
    pub test: bool,
    /// The crates for the same target which are passed to this crate with `--extern`.
    pub uses: Vec<Use>,
}

/// A crate used by another, from a `use` attribute.
#[derive(Clone)]
pub struct Use {
    /// The name of the crate, which is for the same target as the crate using it.
    pub name: String,
    pub span: Span,
}

#[derive(Clone, Copy, PartialEq)]
pub enum Kind {
    Lib,
    Bin,
    /// A procedural macro, which is a library for the host target, which rustc loads.
    ProcMacro,
    /// Only a test harness.
    Test,
}

impl Kind {
    /// The `--crate-type` of a crate of the kind.
    pub fn crate_type(self) -> &'static str {
        match self {
            Kind::Lib | Kind::Test => "lib",
            Kind::Bin => "bin",
            Kind::ProcMacro => "proc-macro",
        }
    }
}

impl Crate {
    /// The library or binary built from the crate, which a crate which is only a test harness does not have.
    pub fn output(&self) -> Option<PathBuf> {
        match self.kind {
            _ if self.output.is_some() => self.output.clone(),
            Kind::Lib => Some(PathBuf::from(format!("target/{}/lib{}.rlib", self.target, self.name))),
            Kind::Bin => Some(PathBuf::from(format!("target/{}/{}", self.target, self.name))),
            // a dynamic library for the host, which configure runs on
            Kind::ProcMacro => Some(PathBuf::from(format!(
                "target/{}/{}{}{}",
                self.target,
                std::env::consts::DLL_PREFIX,
                self.name,
                std::env::consts::DLL_SUFFIX,
            ))),
            Kind::Test => None,
        }
    }
    /// The test harness of the crate, built with `--test`.
    pub fn test_harness(&self) -> PathBuf {
        PathBuf::from(format!("target/{}/test/{}", self.target, self.name))
    }
}
