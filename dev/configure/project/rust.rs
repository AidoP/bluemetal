use std::path::PathBuf;
use super::Span;

pub struct Crate {
    pub name: String,
    pub name_span: Span,
    pub path: PathBuf,
    pub path_span: Span,
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
    /// The names of crates for the same target which are passed to this crate with `--extern`.
    pub uses: Vec<(String, Span)>,
}

#[derive(Clone, Copy, PartialEq)]
pub enum Kind {
    Lib,
    Bin,
    /// Only a test harness.
    Test,
}

pub struct Rule {

}

impl Crate {
    /// The `--crate-type` of the crate.
    pub fn crate_type(&self) -> &'static str {
        match self.kind {
            Kind::Lib | Kind::Test => "lib",
            Kind::Bin => "bin",
        }
    }
    /// The library or binary built from the crate, which a crate which is only a test harness does not have.
    pub fn output(&self) -> Option<PathBuf> {
        match self.kind {
            _ if self.output.is_some() => self.output.clone(),
            Kind::Lib => Some(PathBuf::from(format!("target/{}/lib{}.rlib", self.target, self.name))),
            Kind::Bin => Some(PathBuf::from(format!("target/{}/{}", self.target, self.name))),
            Kind::Test => None,
        }
    }
    /// The test harness of the crate, built with `--test`.
    pub fn test_harness(&self) -> PathBuf {
        PathBuf::from(format!("target/{}/test/{}", self.target, self.name))
    }
}
