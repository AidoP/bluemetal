fn main() {
    let machine = configure::Machine::configure();
    machine.features();
    machine.build_lib("debug", &[
        "src/debug.s",
    ]);
}
