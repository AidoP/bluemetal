use crate::lex::Token;

enum Statement<'a> {
    Function {
        calling_convention: CallingConvention,
        identifier: Identifier<'a>,
        block: Option<Block<'a>>
    },
    Structure {
        calling_convention: CallingConvention,
        identifier: Identifier<'a>
    }
}

struct Identifier<'a> {
    name: &'a str
}

#[repr(transparent)]
struct Block<'a>(Vec<Expression<'a>>);

enum Expression<'a> {
    Constant(Value),
    Call(Binding<'a>),
}

enum Value {
    String(String),
    Integer(usize),
    Char(char)
}

struct Binding<'a>(Identifier<'a>);

enum CallingConvention {
    C
}