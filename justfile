configure profile="default" *args="":
    cargo run --bin=configure_cli --profile=configure -- '{{profile}}' {{args}}
configure-tui profile="default":
    cargo run --bin=configure_tui --profile=configure '{{profile}}'

build profile="default":
    cargo run --bin=configure_cli --profile=configure -- '{{profile}}' build
run profile="default":
    cargo run --bin=configure_cli --profile=configure -- '{{profile}}' run
cargo-runner path:
    cargo run --bin=configure_cli --profile=configure -- "${BLUEMETAL_PROFILE}" cargo-runner '{{path}}'
