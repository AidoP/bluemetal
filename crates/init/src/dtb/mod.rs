use device_tree::DeviceTree;
use serial::sifive_uart;

/// Parse the device tree and initialise any supported systems in the process.
pub fn init(dt: DeviceTree<'static>) {
    ::serial::set_global(sifive_uart(0));
    //::serial::set_global(::serial::uart16550(0));
    let mut depth = 0;
    let mut tokens = dt.tokens();
    while let Some(token) = tokens.next() {
        use device_tree::Token as T;
        ::serial::println!("token: {token:?}");
        match (depth, token) {
            (1, T::NodeBegin { name: "serial" }) => {
  //              init_serial(tokens.clone());
            },
            (_, T::Property { .. }) => {},
            (_, T::NodeEnd) => depth -= 1,
            (_, T::NodeBegin { .. }) => depth += 1,
        }
    }
}
/*
pub fn init_serial(mut tokens: device_tree::TokenIter) {
    use device_tree::Token as T;
    let mut compatible = None;
    let mut reg = None;
    while let Some(T::Property { name, data }) = tokens.next() {
        match name.split_once('@').map(|(name, _)| name).unwrap_or(name) {
            "compatible" => {
                compatible = Some(data.split(0));
            },
            "reg" => {
                reg = Some(data);
            },
            _ => (),
        }
    }
    let Some(compatible) = compatible else {
        return;
    };
    for compatible in compatible {
        match compatible {
            b"ns16550" | b"ns16550a" => {
                let Some(reg) = reg else {
                    continue;
                };
                //let addr = 
                ::serial::set_global(::serial::uart16550(0));
            },
        }
    }
    //#[cfg(target_device = "sifive_uart")]
}
*/
