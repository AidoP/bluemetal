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
    /// The names of crates for the same target which are passed to this crate with `--extern`.
    pub uses: Vec<(String, Span)>,
}

pub struct Rule {

}
