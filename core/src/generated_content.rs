#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GeneratedTextSource {
    Literal(String),
    Attribute(String),
}
