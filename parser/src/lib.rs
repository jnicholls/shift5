//! Parser crate: types and one parser implementation for the binary message protocol.
//!
//! Additional parser implementations could be implemented using, for example, parser combinator
//! libraries.

mod state_machine;

pub use protocol::Message;
pub use state_machine::StateMachineParser;

/// Errors that can occur while parsing the message stream.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ParseError {
    /// The calculated checksum did not match the parsed checksum byte.
    ChecksumMismatch { expected: u8, calculated: u8 },
    /// An invalid escape sequence was encountered (0xFF followed by a byte that is not 0x00 at start or 0xFF in body).
    InvalidEscapeSequence {
        /// Offset into the input slice where the invalid sequence started.
        offset: usize,
    },
    /// Bytes were encountered before the start of a message that were not a start sequence (gap).
    Gap(usize),
    /// A start sequence was encountered before a previous message was fully received.
    UnexpectedStartSequence {
        /// Offset into the input slice where the unexpected start sequence started.
        offset: usize,
    },
}

/// A single result from the parser: either a complete message, a partial (incomplete) message indicator, or an error.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ParseResult {
    Complete(Message),
    Partial,
    Error(ParseError),
}

impl From<Message> for ParseResult {
    fn from(message: Message) -> Self {
        Self::Complete(message)
    }
}

impl From<ParseError> for ParseResult {
    fn from(error: ParseError) -> Self {
        Self::Error(error)
    }
}

/// Parser trait: stateful consumption of byte slices, producing a list of results per feed.
pub trait Parser {
    /// Process the given bytes and return any complete messages, partial indicator, or errors.
    /// Partial is pushed when input ends mid-message; the next feed continues from that state.
    fn feed(&mut self, input: &[u8]) -> Vec<ParseResult>;
}
