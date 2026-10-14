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
- `crate <name>`: introduces a crate named `<name>`


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

Within a string, an expression in `{}` is replaced with its value, and `{{` and `}}` with a single brace.
The expression cannot contain a string, and cannot start with `{`.

```
log note 'size is {size} with {size / 0x1000} pages'
```

A `declare` statement may be followed by indented attributes:

- `type <type>`: the type of the property, one of `'str'`, `'bool'`, `'unsigned'` or `'signed'`
- `default <value>`: the initial value of the property
- `read-only`: the property can only be assigned once, by its default or by a single `set` before or after the declaration

### Builtin properties

- `sysroot`: the sysroot of the Rust toolchain, as printed by `rustc --print=sysroot`
- `target`: the target triple for crates without a `target` attribute, initially the host
- `host-target`: the target triple of the host, as printed by `rustc --print=host-tuple`, which is read-only
