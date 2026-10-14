//! Integration tests for `target/configure`.
//!
//! Each test writes a project to a temporary directory, runs `configure` on its `Build` file from that directory, and
//! checks the diagnostics and the generated `build.ninja`.
//!
//! The `configure` binary is found with the `CONFIGURE` environment variable, or at `target/configure` relative to the
//! working directory.

use std::path::{Path, PathBuf};
use std::process::Command;

/// A project in a temporary directory, removed when dropped.
struct Project {
    dir: PathBuf,
}
impl Project {
    /// Create a project from `(path, contents)` pairs. The first file should be the root `Build`.
    fn new(name: &str, files: &[(&str, &str)]) -> Self {
        let dir = std::env::temp_dir().join(format!("bluemetal-configure-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for (path, contents) in files {
            let path = dir.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, contents).unwrap();
        }
        Self { dir }
    }
    fn configure(&self) -> Output {
        let configure = match std::env::var_os("CONFIGURE") {
            Some(path) => PathBuf::from(path),
            None => std::env::current_dir().unwrap().join("target/configure"),
        };
        let output = Command::new(&configure)
            .arg("Build")
            .current_dir(&self.dir)
            .output()
            .unwrap_or_else(|error| panic!("failed to run {configure:?}: {error}"));
        Output {
            success: output.status.success(),
            stderr: String::from_utf8(output.stderr).unwrap(),
            ninja: std::fs::read_to_string(self.dir.join("build.ninja")).unwrap_or_default(),
        }
    }
    /// The host triple, as seen by `rustc` from the project directory.
    fn host(&self) -> String {
        let output = Command::new("rustc").arg("--print=host-tuple").current_dir(&self.dir).output().unwrap();
        String::from_utf8(output.stdout).unwrap().trim().into()
    }
}
impl Drop for Project {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

struct Output {
    success: bool,
    stderr: String,
    ninja: String,
}
impl Output {
    /// Assert that configuration succeeded, returning the output for further checks.
    #[track_caller]
    fn ok(self) -> Self {
        assert!(self.success, "configure failed:\n{}", self.stderr);
        self
    }
    /// Assert that configuration failed, returning the output for further checks.
    #[track_caller]
    fn failed(self) -> Self {
        assert!(!self.success, "configure succeeded:\n{}", self.stderr);
        self
    }
    /// Assert that stderr contains `expected`.
    #[track_caller]
    fn stderr(&self, expected: &str) -> &Self {
        assert!(self.stderr.contains(expected), "stderr does not contain {expected:?}:\n{}", self.stderr);
        self
    }
    /// Returns the `build` statement for an output, including its variables.
    #[track_caller]
    fn build(&self, output: &str) -> &str {
        let prefix = format!("build {output}:");
        let Some(start) = self.ninja.find(&prefix) else {
            panic!("no build statement for {output:?}:\n{}", self.ninja);
        };
        let rest = &self.ninja[start..];
        &rest[..rest.find("\n\n").unwrap_or(rest.len())]
    }
    /// Returns the outputs of all `build` statements.
    fn outputs(&self) -> Vec<&str> {
        self.ninja.lines().filter_map(|line| line.strip_prefix("build ")?.split(':').next()).collect()
    }
}

#[track_caller]
fn assert_contains(haystack: &str, needle: &str) {
    assert!(haystack.contains(needle), "{needle:?} not found in:\n{haystack}");
}

#[test]
fn log_evaluates_expressions() {
    let project = Project::new("log", &[("Build", "\
declare pages
    type 'unsigned'
    default 4
log note 'size {pages * 0x1000} {{literal}}'
log warning 'careful'
")]);
    project.configure().ok().stderr("note: size 16384 {literal}").stderr("warning: careful");
}

#[test]
fn target_defaults_to_host() {
    let project = Project::new("target-default", &[("Build", "crate 'app' 'app.rs'\n")]);
    let host = project.host();
    let output = project.configure().ok();
    let build = output.build(&format!("target/{host}/libapp.rlib"));
    assert_contains(build, "crate-name = app");
    assert_contains(build, &format!("--target={host}"));
    assert_contains(build, &format!("-L dependency=target/{host}"));
}

#[test]
fn target_is_resolved_at_declaration() {
    let project = Project::new("target-resolve", &[("Build", "\
crate 'before' 'x.rs'
set target 'riscv64gc-unknown-none-elf'
crate 'after' 'x.rs'
crate 'attribute' 'x.rs'
    target 'aarch64-unknown-none'
crate 'host' 'x.rs'
    target host-target
log note 'host-target is {host-target}'
")]);
    let host = project.host();
    let output = project.configure().ok();
    output.stderr(&format!("note: host-target is {host}"));
    output.build(&format!("target/{host}/libbefore.rlib"));
    output.build("target/riscv64gc-unknown-none-elf/libafter.rlib");
    output.build("target/aarch64-unknown-none/libattribute.rlib");
    output.build(&format!("target/{host}/libhost.rlib"));
}

#[test]
fn crate_default_path() {
    let project = Project::new("crate-path", &[
        ("Build", "use 'lib'\n"),
        ("lib/Build", "declare suffix\n    type 'str'\n    default '2'\ncrate 'util{suffix}'\n"),
    ]);
    let host = project.host();
    let output = project.configure().ok();
    assert_contains(output.build(&format!("target/{host}/libutil2.rlib")), ": rustc lib/util2/lib.rs");
}

#[test]
fn crates_are_keyed_by_target() {
    let project = Project::new("crate-key", &[("Build", "\
crate 'core' 'core.rs'
crate 'core' 'core.rs'
    target 'riscv64gc-unknown-none-elf'
")]);
    let host = project.host();
    let output = project.configure().ok();
    let mut outputs = output.outputs();
    outputs.sort();
    assert_eq!(outputs, [
        "target/riscv64gc-unknown-none-elf/libcore.rlib".to_string(),
        format!("target/{host}/libcore.rlib"),
    ]);
}

#[test]
fn crate_redeclaration() {
    let project = Project::new("crate-redeclare", &[("Build", "\
crate 'dup' 'x.rs'
crate 'dup' 'y.rs'
")]);
    project.configure().failed()
        .stderr("error: redeclaration of crate \"dup\"")
        .stderr(" --> Build:2:7")
        .stderr("note: previously declared here\n --> Build:1:7");
}

#[test]
fn builtin_crates_are_extern_by_default() {
    let project = Project::new("builtin", &[("Build", "\
crate 'core' 'core.rs'
    builtin
crate 'compiler_builtins' 'cb.rs'
    builtin
    use 'core'
crate 'app' 'app.rs'
crate 'kernel' 'kernel.rs'
    target 'riscv64gc-unknown-none-elf'
")]);
    let host = project.host();
    let output = project.configure().ok();
    let core = format!("target/{host}/libcore.rlib");
    let compiler_builtins = format!("target/{host}/libcompiler_builtins.rlib");

    let app = output.build(&format!("target/{host}/libapp.rlib"));
    assert_contains(app, &format!("| {compiler_builtins} {core}\n"));
    assert_contains(app, &format!("--extern compiler_builtins={compiler_builtins} --extern core={core}"));

    // builtins only receive the crates they use
    let core_build = output.build(&core);
    assert!(!core_build.contains("--extern"), "{core_build}");
    assert_contains(output.build(&compiler_builtins), &format!("--extern core={core}"));

    // builtins are only passed to crates for the same target
    let kernel = output.build("target/riscv64gc-unknown-none-elf/libkernel.rlib");
    assert!(!kernel.contains("--extern"), "{kernel}");
}

#[test]
fn use_adds_dependency() {
    let project = Project::new("use", &[
        ("Build", "crate 'app' 'app.rs'\n    use 'util'\nuse 'util'\n"),
        ("util/Build", "crate 'util'\n"),
    ]);
    let host = project.host();
    let output = project.configure().ok();
    let util = format!("target/{host}/libutil.rlib");
    let app = output.build(&format!("target/{host}/libapp.rlib"));
    assert_contains(app, &format!("| {util}\n"));
    assert_contains(app, &format!("--extern util={util}"));
}

#[test]
fn use_unknown_crate() {
    let project = Project::new("use-unknown", &[("Build", "\
crate 'util' 'u.rs'
crate 'kernel' 'k.rs'
    target 'riscv64gc-unknown-none-elf'
    use 'util'
crate 'bad' 'b.rs'
    use 5
")]);
    project.configure().failed()
        .stderr("error: no crate \"util\" for target \"riscv64gc-unknown-none-elf\"\n --> Build:4:9")
        .stderr("error: unsigned value used where str is required\n --> Build:6:9");
}

#[test]
fn unknown_target() {
    let project = Project::new("target-unknown", &[("Build", "\
crate 'a' 'x.rs'
    target 'riscv64gc-unknown-none'
crate 'b' 'x.rs'
    target 'nope'
")]);
    let output = project.configure().failed();
    output
        .stderr("error: unknown target \"riscv64gc-unknown-none\", did you mean \"riscv64gc-unknown-none-elf\"?\n --> Build:2:12")
        .stderr("error: unknown target \"nope\"\n --> Build:4:12")
        .stderr("note: available targets are:\n    ")
        .stderr("\n    riscv64gc-unknown-none-elf\n");
    assert_eq!(output.stderr.matches("available targets are").count(), 1, "{}", output.stderr);
}

#[test]
fn invalid_crate_name() {
    let project = Project::new("crate-name", &[("Build", "\
declare suffix
    type 'str'
    default 'x'
crate 'host-tool' 'x.rs'
crate 'a-{suffix}' 'x.rs'
crate 'Upper' 'x.rs'
crate '2fast' 'x.rs'
crate 5 'x.rs'
crate 'compiler_builtins2' 'x.rs'
")]);
    project.configure().failed()
        // a string literal points at the character, a template at the whole name
        .stderr("error: invalid character '-' in crate name \"host-tool\"\n --> Build:4:12")
        .stderr("error: invalid character '-' in crate name \"a-x\"\n --> Build:5:7")
        .stderr("error: invalid character 'U' in crate name \"Upper\"")
        .stderr("error: invalid character '2' in crate name \"2fast\"")
        .stderr("error: unsigned value used where str is required\n --> Build:8:7")
        .stderr("configuration failed with 5 errors");
}

#[test]
fn invalid_identifier() {
    let project = Project::new("ident", &[("Build", "\
declare my_prop
declare prop2
    type 'str'
    default 'v'
    Default 'x'
Set prop2 'y'
log note '{Prop}'
declare 2go
")]);
    project.configure().failed()
        .stderr("error: invalid character '_' in identifier \"my_prop\"\n --> Build:1:11")
        .stderr("error: invalid character 'D' in identifier \"Default\"\n --> Build:5:5")
        .stderr("error: invalid character 'S' in identifier \"Set\"\n --> Build:6:1")
        .stderr("error: invalid character 'P' in identifier \"Prop\"\n --> Build:7:12")
        .stderr("error: invalid character '2' in identifier \"2go\"\n --> Build:8:9")
        .stderr("configuration failed with 5 errors");
}

#[test]
fn writes_depfile() {
    let project = Project::new("depfile", &[
        ("Build", "use 'a'\n"),
        ("a/Build", "crate 'a' 'a.rs'\n"),
    ]);
    project.configure().ok();
    let depfile = std::fs::read_to_string(Path::new(&project.dir).join("build.ninja.d")).unwrap();
    assert_eq!(depfile, "build.ninja: Build a/Build\n");
}

#[test]
fn read_only_property() {
    let project = Project::new("read-only", &[("Build", "\
declare version
    type 'unsigned'
    default 3
    read-only
declare free
    default 1
set free 2
declare after
    read-only
set after 'a'
set before 'b'
declare before
    read-only
log note '{version} {free} {after} {before}'
")]);
    // a read-only property may be assigned once, by a default or by a `set` either side of the declaration
    project.configure().ok().stderr("note: 3 2 a b");
}

#[test]
fn set_read_only_property() {
    let project = Project::new("read-only-set", &[("Build", "\
set host-target 'x'
declare version
    default 3
    read-only
set version 4
set early 'a'
set early 'b'
declare early
    read-only
set defaulted 'a'
declare defaulted
    default 'd'
    read-only
log note 'early is {early}'
")]);
    let output = project.configure().failed();
    output
        .stderr("error: cannot set read-only property \"host-target\"\n --> Build:1:5")
        .stderr("note: this builtin property is read-only")
        .stderr("error: cannot set read-only property \"version\"\n --> Build:5:5")
        .stderr("note: value assigned here\n --> Build:3:5")
        .stderr("note: declared read-only here\n --> Build:2:9")
        // only the first `set` before the declaration is permitted, and its value is kept
        .stderr("error: cannot set read-only property \"early\"\n --> Build:7:5")
        .stderr("note: value assigned here\n --> Build:6:5")
        .stderr("note: early is a")
        // a default leaves no room for an earlier `set`
        .stderr("error: cannot set read-only property \"defaulted\"\n  --> Build:10:5")
        .stderr("note: value assigned here\n  --> Build:12:5")
        .stderr("configuration failed with 4 errors");
}

#[test]
fn duplicate_declare() {
    let project = Project::new("declare-duplicate", &[("Build", "\
set later 1
declare later
declare size
    type 'unsigned'
    default 1
declare size
    type 'str'
    default 'x'
declare host-target
    default 'y'
log note 'size is {size}'
")]);
    project.configure().failed()
        .stderr("error: redeclaration of property \"size\"\n --> Build:6:9")
        .stderr("note: previously declared here\n --> Build:3:9")
        .stderr("error: redeclaration of property \"host-target\"\n --> Build:9:9")
        .stderr("note: this is a builtin property")
        // the attributes of a duplicate are ignored
        .stderr("note: size is 1")
        .stderr("configuration failed with 2 errors");
}

#[test]
fn meta_if() {
    let project = Project::new("meta-if", &[("Build", "\
declare debug
    default false
declare level
    default 2
@if debug
set level 9
@if !debug
log note 'release'
@if level > 1
@if {'true'}
crate 'kept' 'x.rs'
    target host-target
@if false
crate 'skipped' 'x.rs'
    target host-target
@if false
@if missing
log note 'not evaluated'
log note 'level is {level}'
")]);
    let host = project.host();
    let output = project.configure().ok();
    output.stderr("note: release").stderr("note: level is 2");
    assert!(!output.stderr.contains("not evaluated"), "{}", output.stderr);
    // the attributes of a skipped statement are skipped with it
    assert_eq!(output.outputs(), [format!("target/{host}/libkept.rlib")]);
}

#[test]
fn meta_if_errors() {
    let project = Project::new("meta-if-errors", &[("Build", "\
@if 1
log note 'unsigned condition'
@if true

log note 'after blank line'
@if true
# comment
log note 'after comment'
@elif true
log note 'unknown'
@if true
")]);
    let output = project.configure().failed();
    output
        .stderr("error: unsigned value used where bool is required\n --> Build:1:5")
        .stderr("error: expected statement on the line after a meta-statement\n --> Build:3:1")
        .stderr("error: expected statement on the line after a meta-statement\n --> Build:6:1")
        .stderr("error: unknown meta-statement \"@elif\"\n --> Build:9:1")
        .stderr("error: expected statement on the line after a meta-statement\n  --> Build:11:1")
        .stderr("configuration failed with 5 errors");
    // an invalid condition skips the statement
    assert!(!output.stderr.contains("note: unsigned condition"), "{}", output.stderr);
    assert!(!output.stderr.contains("note: unknown"), "{}", output.stderr);
}

#[test]
fn meta_else() {
    let project = Project::new("meta-else", &[("Build", "\
declare level
    default 2
@if level == 1
log note 'one'
@else
@if level == 2
log note 'two'
@else
log note 'other'

# blank lines and comments may separate the statements of a chain
@if level == 2
log note 'if taken'

@else
log note 'else not taken'
@if false
log note 'x'
@else
@if false
log note 'y'
@else
log note 'final else'
@if false
@if missing
log note 'skipped'
@else
crate 'fallback' 'x.rs'
    target host-target
")]);
    let host = project.host();
    let output = project.configure().ok();
    output.stderr("note: two").stderr("note: if taken").stderr("note: final else");
    for skipped in ["one", "other", "else not taken", "note: x", "note: y", "skipped"] {
        assert!(!output.stderr.contains(skipped), "{skipped:?} was not skipped:\n{}", output.stderr);
    }
    assert_eq!(output.outputs(), [format!("target/{host}/libfallback.rlib")]);
}

#[test]
fn meta_else_errors() {
    let project = Project::new("meta-else-errors", &[("Build", "\
@else
log note 'no if'
@if true
log note 'a'
log note 'unconditional'
@else
log note 'chain ended'
@if true
@else
log note 'not first'
@if 1
log note 'bad condition'
@else
log note 'else after bad condition'
@else x
log note 'trailing'
")]);
    let output = project.configure().failed();
    output
        .stderr("error: \"@else\" must follow a conditional statement\n --> Build:1:1")
        // an unconditional statement ends the chain
        .stderr("error: \"@else\" must follow a conditional statement\n --> Build:6:1")
        .stderr("error: \"@else\" must be the first meta-statement of a statement\n --> Build:9:1")
        .stderr("error: unsigned value used where bool is required\n  --> Build:11:5")
        .stderr("error: expected \"\\n\"\n  --> Build:15:6")
        .stderr("configuration failed with 5 errors");
    // an invalid chain processes none of its statements
    for skipped in ["no if", "chain ended", "not first", "bad condition", "else after bad condition", "trailing"] {
        assert!(!output.stderr.contains(skipped), "{skipped:?} was not skipped:\n{}", output.stderr);
    }
}

#[test]
fn meta_else_if() {
    let project = Project::new("meta-else-if", &[("Build", "\
declare level
    default 2
@if level == 1
log note 'one'
@else-if level == 2
log note 'two'
@else-if missing
log note 'not evaluated'
@else
log note 'other'
@if false
log note 'x'
@else-if {'false'}
log note 'y'
@else-if true
@if level > 1
log note 'nested condition'
")]);
    let output = project.configure().ok();
    output.stderr("note: two").stderr("note: nested condition");
    for skipped in ["one", "not evaluated", "other", "note: x", "note: y"] {
        assert!(!output.stderr.contains(skipped), "{skipped:?} was not skipped:\n{}", output.stderr);
    }
}

#[test]
fn meta_else_if_errors() {
    let project = Project::new("meta-else-if-errors", &[("Build", "\
@else-if true
log note 'no if'
@if true
@else-if true
log note 'not first'
@if false
log note 'a'
@else-if 1
log note 'bad condition'
@else
log note 'else after bad condition'
")]);
    let output = project.configure().failed();
    output
        .stderr("error: \"@else-if\" must follow a conditional statement\n --> Build:1:1")
        .stderr("error: \"@else-if\" must be the first meta-statement of a statement\n --> Build:4:1")
        .stderr("error: unsigned value used where bool is required\n --> Build:8:10")
        .stderr("configuration failed with 3 errors");
    for skipped in ["no if", "not first", "bad condition", "else after bad condition"] {
        assert!(!output.stderr.contains(skipped), "{skipped:?} was not skipped:\n{}", output.stderr);
    }
}
