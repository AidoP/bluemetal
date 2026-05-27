#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Sev {
    Fatal,
    Error,
    Warning,
    Info,
    Allow,
}
impl Sev {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Fatal => "error",
            Self::Error => "error",
            Self::Warning => "warning",
            Self::Allow | Self::Info => "info",
        }
    }
    pub const fn accent(&self) -> &'static str {
        match self {
            Self::Error | Self::Fatal => "\x1b[31m",
            Self::Warning => "\x1b[33m",
            Self::Allow | Self::Info => "\x1b[94m",
        }
    }
    pub const fn is_logged(&self) -> bool {
        match self {
            Self::Allow => false,
            _ => true,
        }
    }
    pub const fn is_error(&self) -> bool {
        match self {
            Self::Fatal | Self::Error => true,
            _ => false,
        }
    }
}
