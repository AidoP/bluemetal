use std::path::PathBuf;
use super::Span;

pub struct Crate {
    pub name: String,
    pub name_span: Span,
    pub path: PathBuf,
    pub path_span: Span,
    pub builtin: bool,
}

pub struct Rule {

}
