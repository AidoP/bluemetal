use system::{DynStorage, System};

#[test]
fn test_storage() {
    /*
    let mut storage = SystemStorage::<4096>::new();
    let mut system = SystemMut::with_storage(&mut storage);
    assert_eq!(system.header().magic, system::MAGIC_LE);
    */
    let storage = system::uninit_storage!(4096);
    let mut system = System::new(storage, 8).expect("failed to initialise");
    println!("{:?}", system.header());
    println!("{:#?}", system.section_table());
}
