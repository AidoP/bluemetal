# Bluemetal

## Building

The build system is a single-crate binary at `dev/main.rs` with a shebang to
compile itself as `bs`.

After bootstrapping the build system will recompile itself as needed.

```sh
# Bootstrap
./dev/main.rs

# Build
./bs -g rv64
```
