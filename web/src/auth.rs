#[derive(Clone, Debug)]
pub enum Auth {
    None,
    Fixed(String),
    FromHeader(String),
}
