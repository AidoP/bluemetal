use std::process::ExitCode;
use std::ffi::OsStr;
use std::ffi::OsString;
use std::io::IsTerminal;
use std::io::Write;
use std::ops::ControlFlow;

struct Options {
    /// The name of the program that was executed.
    program: OsString,
    /// Use ANSI escape sequences.
    ansi: bool,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            program: "bs".into(),
            ansi: false,
        }
    }
}

enum ArgError {
    Unexpected(OsString),
    MissingValue,
}
impl core::fmt::Display for ArgError {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        match self {
            Self::Unexpected(arg) => write!(f, "unexpected {arg:?}"),
            Self::MissingValue => write!(f, "missing value"),
        }
    }
}

struct Arg {
    name: &'static str,
    long: Option<&'static str>,
    short: Option<char>,
    /// The number of times the argument can be used as a positional argument.
    count: usize,
    help: &'static str,
    info: Option<&'static str>,
    process: fn(Option<&OsStr>, &mut Options) -> Result<ControlFlow<ExitCode>, ArgError>,
}

const ARGS: &[Arg] = &[
    Arg {
        name: "color",
        long: Some("color"),
        short: None,
        count: 0,
        help: "Enable emitting ANSI escape character.
The default (auto) uses escape characters if the output device is a terminal.",
        info: Some("auto,always,never"),
        process: |_, options| {
            help(&mut std::io::stdout(), options);
            Ok(ControlFlow::Break(ExitCode::SUCCESS))
        },
    },
    Arg {
        name: "help",
        long: Some("help"),
        short: Some('h'),
        count: 0,
        help: "Print this help text.",
        info: None,
        process: |_, options| {
            help(&mut std::io::stdout(), options);
            Ok(ControlFlow::Break(ExitCode::SUCCESS))
        },
    },
];

fn main() -> ExitCode {
    let mut options = Options::default();
    let mut args = std::env::args_os();
    let mut stderr = std::io::stderr();
    if let Some(program) = args.next() {
        options.program = program;
    }

    if stderr.is_terminal() {
        options.ansi = true;
    }

    match parse_args(args, &mut options) {
        Err(e) => {
            eprintln!("invalid argument: {e}");
            help(&mut stderr, &options);
            return ExitCode::FAILURE;
        },
        Ok(ControlFlow::Break(e)) => return e,
        Ok(ControlFlow::Continue(())) => (),
    };

    ExitCode::SUCCESS
}

fn parse_args(mut args: std::env::ArgsOs, options: &mut Options) -> Result<ControlFlow<ExitCode>, ArgError> {
    let mut pos = 0;
    let mut count = 0;

    'args: for arg in args {
        let arg_bytes = arg.as_encoded_bytes();
        unsafe fn split_arg(s: &[u8]) -> (&OsStr, Option<&OsStr>) {
            let mut i = 0;
            while i < s.len() {
                if s[i] == b'=' {
                    unsafe {
                        return (
                            OsStr::from_encoded_bytes_unchecked(&s[..i]),
                            Some(OsStr::from_encoded_bytes_unchecked(&s[i+1..]))
                        );
                    }
                }
                i += 1;
            }
            (OsStr::from_encoded_bytes_unchecked(s), None)
        }
        if arg_bytes.starts_with(b"--") {
            let (key, value) = unsafe { split_arg(&arg_bytes[2..]) };
            for arg in ARGS {
                let Some(name) = arg.long else {
                    continue;
                };
                if name.as_bytes() == key.as_encoded_bytes() {
                    match (arg.process)(value, options) {
                        Ok(ControlFlow::Continue(())) => continue 'args,
                        Ok(c) => return Ok(c),
                        Err(error) => break,
                    }
                }
            }
        } else if arg_bytes.starts_with(b"-") {
            let (key, value) = unsafe { split_arg(&arg_bytes[1..]) };
        } else {
            for a in &ARGS[pos..] {
                count += 1;
                if count > a.count {
                    pos += 1;
                    count = 0;
                    continue;
                }
                match (a.process)(Some(&arg), options) {
                    Ok(ControlFlow::Continue(())) => continue 'args,
                    Ok(c) => return Ok(c),
                    Err(error) => break,
                }
            }
        };
        return Err(ArgError::Unexpected(arg));
    }
    Ok(ControlFlow::Continue(()))
}

fn help(f: &mut dyn Write, options: &Options) {
    writeln!(f, "Usage: {program} [OPTION]...", program=options.program.display());
    for arg in ARGS {
        let mut seperator = "\n    ";
        if arg.count > 0 {
            write!(f, "{seperator}{}", arg.name.to_ascii_uppercase());
            seperator = ", ";
        }
        if let Some(name) = arg.short {
            write!(f, "{seperator}-{name}");
            seperator = ", ";
        }
        if let Some(name) = arg.long {
            write!(f, "{seperator}--{name}");
        }
        if let Some(info) = arg.info {
            write!(f, "={info}");
        }
        writeln!(f);
        for line in arg.help.lines() {
            writeln!(f, "        {}", line);
        }
    }
}
