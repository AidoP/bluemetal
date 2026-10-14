use std::path::PathBuf;
use super::Span;

pub struct Crate {
    pub name: String,
    pub name_span: Span,
    pub path: PathBuf,
    pub path_span: Span,
    /// The target triple the crate is compiled for.
    pub target: String,
    pub builtin: bool,
}

pub struct Rule {

}
