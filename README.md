## Building

Create the main `Build` file to configure the build system.

Typically, this will set some values then include a the build definition for a particular platform.

```
include arch/riscv/virt
```

Now the build system can be bootstrapped.

```
ninja -f bootstrap.ninja
```

From now on just build with `ninja`.


### Statements

- `use <path>`: start processing the build configuration file at `<path>`, unless it has already been processed. A file which uses itself, directly or through other files, raises a warning
- `declare <name>`: define a property that can be used for substitution
- `set <name> <value>`: assign a value to a property
- `crate <name> [<path>]`: introduce a Rust library crate named `<name>`, with its root at `<path>`, or at
  `<name>/lib.rs` relative to the file without one
- `log <severity> <expression>`: report the value of an expression as an `error`, `warning` or `note`

A `<name>` is an identifier: ASCII letters, digits, `-` and `_`, not starting with a digit. Crate names cannot
contain `-`, as rustc does not accept it. A name can instead be computed with an expression in `{}`, whose value must
be a valid name.

```
crate compiler_builtins
set {'heap-{arch}'} 0x1000
```


### Meta-statements

A meta-statement is a line starting with `@` which applies to the statement on the next line. There cannot be a blank
line or a comment between them, but several meta-statements can apply to the same statement.

- `@if <expression>`: skip the statement, including its attributes, unless the expression is `true`. The expression
  must be a `bool`, or coerced to one with `{}`. Once a statement is skipped, the conditions of later meta-statements
  are not evaluated

```
@if {device-tree}
@if target == 'riscv64gc-unknown-none-elf'
use 'dtb'
```

- `@else`: process the statement only if no statement has been processed in the chain of conditional statements before
  it. A chain starts with a statement with `@if` and is continued by each statement with `@else`, so `@else` followed
  by `@if` is an else-if. Blank lines and comments may separate the statements of a chain, but another statement ends
  it. `@else` must be the first meta-statement of its statement
- `@else-if <expression>`: the same as `@else` followed by `@if <expression>`

```
@if level == 1
log note 'one'
@else-if level == 2
log note 'two'
@else
log note 'many'
```


### Expressions

An expression is a string, `true`, `false`, an integer or the name of a property.

Integers and integer properties can be combined with `+`, `-`, `*`, `/` and `%`, with a single space on either side
of the operator, and grouped with `(` and `)`. Both operands must have the same type, `unsigned` or `signed`.

Integers can be compared with `<`, `<=`, `>` and `>=`, and any two values of the same type with `==` and `!=`.
A `bool` can be negated with a leading `!` and combined with `&&` and `||`. From lowest to highest, the precedence is
`||`, `&&`, comparisons, `+` and `-`, then `*`, `/` and `%`.

An expression written in `{}` is coerced to the type the operator requires, or to the type of the other operand.
This converts between `unsigned` and `signed`, parses a `str` as an integer or as `true` or `false`, treats a `bool`
as `0` or `1`, and treats an integer as `true` unless it is zero. Integer literals are only coerced between
`unsigned` and `signed`.

```
set size {pages} * 0x1000 - 1
set large size >= 0x10_0000 && !{small}
```

Functions are called with their arguments in `()`, separated by `, `:

- `upper(str)` and `lower(str)`: convert a `str` to upper or lower case
- `starts-with(str, prefix)` and `ends-with(str, suffix)`: whether a `str` starts or ends with another, as a `bool`
- `select(name, ...)`: the value of the first of the named properties which has a value. Each property must be
  declared, or set before it is declared, so that a misspelt name is an error rather than ignored
- `env(name)`: the value of an environment variable, which must be set. The build is not reconfigured when it changes
- `path(str)`: a path relative to the directory of the file containing the expression, as a `str`. An absolute path
  is not made relative. `.` and `..` are resolved without following symbolic links
- `read(path)`: the contents of a file, without trailing newlines, where the path is relative to the file containing
  the expression. The build is reconfigured when the file changes
- `exists(path)`: whether a file or directory exists, as a `bool`, where the path is relative to the file containing
  the expression. The build is reconfigured when an existing path changes, but not when a missing one is created
- `try(expression)`: whether the expression is evaluated without errors, as a `bool`. Its errors are reported as
  warnings instead, and its value is discarded

```
@if try(env('CROSS_COMPILE'))
set prefix env('CROSS_COMPILE')
```

- `which(name)`: the path of the first executable file with the given name in the directories of the `PATH` environment
  variable. The build is reconfigured when one of the directories searched before it changes
- `run(program, argument, ..., NAME: value, ...)`: run a program and return its output, without trailing newlines.
  As in `sh`, a program containing a `/` is a path, relative to the file containing the expression, while any other
  program is found in `PATH`. It inherits the environment, with any variables given as `NAME: value` after the
  arguments, where `NAME` is a name like those of properties, so it can be computed with `{}`. The configuration fails
  if the program exits unsuccessfully. The build is reconfigured when a program given as a path changes, but not when
  its output would change otherwise

```
set board select(board, default-board)
@if starts-with(target, 'riscv')
use 'arch/riscv'
```

Within a string, an expression in `{}` is replaced with its value, and `{{` and `}}` with a single brace.
The expression cannot start with `{`, but can contain strings, which are templates themselves.

```
log note 'size is {size} with {size / 0x1000} pages'
log note 'home is {env('HOME')}'
```

A `declare` statement may be followed by indented attributes:

- `type <type>`: the type of the property, one of `'str'`, `'bool'`, `'unsigned'` or `'signed'`
- `default <value>`: the initial value of the property
- `info <string>`: a description of the property, included in diagnostics about it
- `read-only`: the property can only be assigned once, by its default or by a single `set` before or after the declaration

A `crate` statement may be followed by indented attributes:

- `target <string>`: the target triple to compile the crate for, which must be one supported by rustc. Without it, the
  `target` property at the point of the declaration is used. A crate can be declared once for each target
- `use <name>`: a crate for the same target which the crate depends on, passed to rustc with `--extern`. It can be
  repeated, and the crate can be declared later or in another file
- `builtin`: pass the crate with `--extern` to every crate for the same target which is not builtin, like the standard
  library. Builtin crates are not passed to each other, so a builtin crate must `use` the ones it depends on

```
crate core '{rust-src}/library/core/src/lib.rs'
    builtin

crate compiler_builtins
    builtin
    use core

crate build_tool
    target host-target
```

### Builtin properties

- `sysroot`: the sysroot of the Rust toolchain, as printed by `rustc --print=sysroot`
- `target`: the target triple for crates without a `target` attribute, initially the host
- `host-target`: the target triple of the host, as printed by `rustc --print=host-tuple`, which is read-only
