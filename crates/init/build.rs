fn main() {
    let machine = configure::Machine::configure();
    machine.features();
    machine.build_lib("rt", &[
        &format!("src/init/{}", machine.init_name()),
        "src/trap.s",
    ]);
}
