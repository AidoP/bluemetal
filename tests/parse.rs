use bluemetal::{Unit, ParseError};

#[test]
fn test_parse() -> Result<(), ParseError> {
    let unit = Unit::parse("tests", "syscall")?;
    panic!("{:?}", unit["syscall"].functions["other"]);
    Ok(())
}