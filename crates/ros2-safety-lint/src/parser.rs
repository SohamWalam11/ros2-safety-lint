use roxmltree::Document;
use std::ops::Range;

/// A wrapper struct that holds a parsed value along with its exact byte range
/// in the original source file.
#[derive(Debug, Clone, PartialEq)]
pub struct Spanning<T> {
    pub node: T,
    pub range: Range<usize>,
}

impl<T> Spanning<T> {
    pub fn new(node: T, range: Range<usize>) -> Self {
        Self { node, range }
    }
}

/// Parses an XML string and returns the roxmltree Document.
pub fn parse_xml(xml: &str) -> Result<Document<'_>, roxmltree::Error> {
    Document::parse(xml)
}
