fn main() {
    configure::Config::load()
        .cfg()
        .library("debug", &[
            "src/debug.s",
        ]);
}
