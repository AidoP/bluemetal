# Bluemetal

## Building

First, the build system itself needs to be compiled. It is a standalone Rust
program.

```sh
rustc -o bs dev/main.rs
```

Now configure a build environment.

```sh
./bs -p rv64
```
