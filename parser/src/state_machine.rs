//! Hand-written state machine parser for the binary message protocol.

use protocol::{ESCAPE, Message, START_SEQUENCE, compute_checksum};

use crate::{ParseError, ParseResult, Parser};

const START_FIRST: u8 = START_SEQUENCE[0];
const START_SECOND: u8 = START_SEQUENCE[1];
const _: () = assert!(
    ESCAPE == START_FIRST,
    "ESCAPE should be equal to START_FIRST"
);

/// States for the state machine parser.
#[derive(Debug)]
enum State {
    /// Seeking start sequence [0xFF, 0x00], managing a gap byte count during the search.
    SeekingStart { gap_count: usize },

    /// Reading the address byte.
    Address,

    /// Reading the destination byte.
    Destination { address: u8 },

    /// Reading the data length byte.
    DataLength { address: u8, destination: u8 },

    /// Reading the data payload.
    Data {
        address: u8,
        destination: u8,
        data_length: u8,
        data: Vec<u8>,
        remaining: usize,
    },

    /// Reading the checksum byte.
    Checksum {
        address: u8,
        destination: u8,
        data_length: u8,
        data: Vec<u8>,
    },
}

/// State machine parser implementation.
#[derive(Debug, Default)]
pub struct StateMachineParser {
    // An escape byte read from the input stream before reaching its end.
    escape_byte: Option<u8>,
    // Current state of the parser.
    state: Option<State>,
}

impl StateMachineParser {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Parser for StateMachineParser {
    fn feed(&mut self, input: &[u8]) -> Vec<ParseResult> {
        let mut input = ParserInput::new(input, self);
        let mut results = Vec::new();

        loop {
            let (should_continue, maybe_result) = input.advance_state();
            if let Some(result) = maybe_result {
                results.push(result);
            }
            if !should_continue {
                break;
            }
        }

        results
    }
}

struct ParserInput<'a> {
    buffer_pos: usize,
    input: &'a [u8],
    escape_byte: Option<u8>,
    parser: &'a mut StateMachineParser,
}

impl<'a> ParserInput<'a> {
    fn new(input: &'a [u8], parser: &'a mut StateMachineParser) -> Self {
        Self {
            buffer_pos: 0,
            input,
            escape_byte: parser.escape_byte.take(),
            parser,
        }
    }

    // Advances the state of the parser. This could result in a new or partial message, or an error.
    // We return a Result<T, E> for better semantics, but ultimately a ParseError is a type of
    // ParseResult.
    fn advance_state(&mut self) -> (bool, Option<ParseResult>) {
        use State::*;

        match self.take_state() {
            // Kick things off by starting in the seeking start state.
            None => {
                self.set_state(SeekingStart { gap_count: 0 });
                (true, None)
            }

            // We're seeking the start sequence [0xFF, 0x00]. We'll do this until we find it or we
            // hit the end of the input stream.
            Some(SeekingStart { gap_count }) => {
                let (found_start, gap_count) = self.seek_start(gap_count);

                if found_start {
                    // If we found the start sequence, we're ready to read the address byte.
                    self.set_state(Address);
                    // If there were any bytes before the start sequence, we'll return a gap error.
                    (
                        true,
                        (gap_count > 0).then_some(ParseError::Gap(gap_count).into()),
                    )
                } else {
                    // If we didn't find the start sequence, we'll continue seeking on the next
                    // feed, maintaining our gap count.
                    self.set_state(SeekingStart { gap_count });
                    (false, None)
                }
            }

            // We're reading the address byte.
            Some(Address) => match self.next_escaped_byte() {
                Ok(Some(address)) => {
                    self.set_state(Destination { address });
                    (true, None)
                }
                Ok(None) => {
                    self.set_state(Address);
                    (false, Some(ParseResult::Partial))
                }
                Err(error) => (true, Some(error.into())),
            },

            // We're reading the destination byte.
            Some(Destination { address }) => match self.next_escaped_byte() {
                Ok(Some(destination)) => {
                    self.set_state(DataLength {
                        address,
                        destination,
                    });
                    (true, None)
                }
                Ok(None) => {
                    self.set_state(Destination { address });
                    (false, Some(ParseResult::Partial))
                }
                Err(error) => (true, Some(error.into())),
            },

            // We're reading the data length byte.
            Some(DataLength {
                address,
                destination,
            }) => match self.next_escaped_byte() {
                Ok(Some(data_length)) => {
                    self.set_state(Data {
                        address,
                        destination,
                        data_length,
                        data: Vec::new(),
                        remaining: data_length as usize,
                    });
                    (true, None)
                }
                Ok(None) => {
                    self.set_state(DataLength {
                        address,
                        destination,
                    });
                    (false, Some(ParseResult::Partial))
                }
                Err(error) => (true, Some(error.into())),
            },

            // We're reading the data payload.
            Some(Data {
                address,
                destination,
                data_length,
                mut data,
                remaining,
            }) => {
                if remaining == 0 {
                    self.set_state(Checksum {
                        address,
                        destination,
                        data_length,
                        data,
                    });
                    (true, None)
                } else {
                    match self.next_escaped_byte() {
                        Ok(Some(byte)) => {
                            data.push(byte);

                            self.set_state(Data {
                                address,
                                destination,
                                data_length,
                                data,
                                remaining: remaining - 1,
                            });
                            (true, None)
                        }
                        Ok(None) => {
                            self.set_state(Data {
                                address,
                                destination,
                                data_length,
                                data,
                                remaining,
                            });
                            (false, Some(ParseResult::Partial))
                        }
                        Err(error) => (true, Some(error.into())),
                    }
                }
            }

            // We're reading the checksum byte.
            Some(Checksum {
                address,
                destination,
                data_length,
                data,
            }) => match self.next_escaped_byte() {
                Ok(Some(checksum)) => {
                    let calculated = compute_checksum(address, destination, data_length, &data);

                    if checksum == calculated {
                        let result = Message {
                            address,
                            destination,
                            data_length,
                            data,
                            checksum,
                        }
                        .into();
                        (true, Some(result))
                    } else {
                        let result = ParseError::ChecksumMismatch {
                            expected: checksum,
                            calculated,
                        }
                        .into();
                        (true, Some(result))
                    }
                }
                Ok(None) => {
                    self.set_state(Checksum {
                        address,
                        destination,
                        data_length,
                        data,
                    });
                    (false, Some(ParseResult::Partial))
                }
                Err(error) => (true, Some(error.into())),
            },
        }
    }

    fn current_offset(&self) -> usize {
        self.buffer_pos
    }

    fn next_byte(&mut self) -> Option<u8> {
        self.escape_byte.take().or_else(|| {
            let byte = self.peek_next_byte();
            self.buffer_pos += 1;
            byte
        })
    }

    // Reads the next byte that takes into account the escape sequence for 0xFF.
    fn next_escaped_byte(&mut self) -> Result<Option<u8>, ParseError> {
        // The offset into the input stream before we begin reading bytes.
        let offset = self.current_offset();

        match self.next_byte() {
            // We encountered the start of an escape sequence for 0xFF.
            Some(ESCAPE) => match self.next_byte() {
                // Next byte was also 0xFF, we're good.
                Some(ESCAPE) => Ok(Some(ESCAPE)),

                // Next byte was the second half of a start sequence. We're not expecting that in
                // this context.
                Some(START_SECOND) => {
                    self.set_state(State::Address);
                    Err(ParseError::UnexpectedStartSequence { offset })
                }

                // Next byte was anything else but 0x0 or 0xFF, which is invalid.
                Some(_) => Err(ParseError::InvalidEscapeSequence { offset }),

                // No more bytes in the input stream.
                // We'll hold onto the escape byte and consider it on the next feed.
                None => {
                    self.escape_byte = Some(ESCAPE);
                    Ok(None)
                }
            },

            // We encountered a non-escape byte, so return that.
            Some(byte) => Ok(Some(byte)),

            // No more bytes in the input stream.
            None => Ok(None),
        }
    }

    fn peek_next_byte(&mut self) -> Option<u8> {
        self.input.get(self.buffer_pos).copied()
    }

    fn seek_start(&mut self, mut gap_count: usize) -> (bool, usize) {
        loop {
            match (self.next_byte(), self.peek_next_byte()) {
                // If we encounter the start sequence [0xFF, 0x00], we're done.
                (Some(START_FIRST), Some(START_SECOND)) => {
                    self.buffer_pos += 1;
                    break (true, gap_count);
                }
                // If we encounter any other byte, we're still seeking.
                (Some(_), Some(_)) => {
                    // Increment the gap count and continue seeking.
                    gap_count += 1;
                }
                // If we encounter the first part of the start sequence but are at the end of the input stream, we'll
                // return false and the current gap count.
                (Some(START_FIRST), None) => {
                    self.escape_byte = Some(START_FIRST);
                    break (false, gap_count);
                }
                // If we encounter the end of the input stream, we're done.
                (Some(_), None) => break (false, gap_count + 1),
                // No byte was consumed (buffer exhausted) so we do not increment gap_count.
                (None, _) => break (false, gap_count),
            }
        }
    }

    fn set_state(&mut self, state: impl Into<Option<State>>) {
        self.parser.state = state.into();
    }

    fn take_state(&mut self) -> Option<State> {
        self.parser.state.take()
    }
}

impl Drop for ParserInput<'_> {
    fn drop(&mut self) {
        // If we got to the end of the input stream while trying to read an escaped byte or a start
        // sequence, we'll hold onto that last 0xFF byte and consider it on the next feed.
        self.parser.escape_byte = self.escape_byte.take();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_input_returns_empty_results() {
        let mut p = StateMachineParser::new();
        let results = p.feed(&[]);
        assert!(results.is_empty(), "empty input should yield no results");
    }

    #[test]
    fn single_correct_message_returns_single_complete() {
        let msg = Message::builder()
            .address(1)
            .destination(2)
            .data([10u8, 20, 30])
            .build()
            .unwrap();
        let bytes = msg.to_bytes();

        let mut p = StateMachineParser::new();
        let results = p.feed(&bytes);
        assert_eq!(results, [ParseResult::Complete(msg)]);
    }

    #[test]
    fn multiple_correct_messages_return_same_messages_as_complete() {
        let m1 = Message::builder()
            .address(1)
            .destination(2)
            .data([1u8, 2])
            .build()
            .unwrap();
        let m2 = Message::builder()
            .address(3)
            .destination(4)
            .data([5u8, 6, 7])
            .build()
            .unwrap();

        let mut bytes = Vec::new();
        m1.write_bytes(&mut bytes).unwrap();
        m2.write_bytes(&mut bytes).unwrap();

        let mut p = StateMachineParser::new();
        let results = p.feed(&bytes);
        assert_eq!(
            results,
            [ParseResult::Complete(m1), ParseResult::Complete(m2)]
        );
    }

    #[test]
    fn message_with_escaped_value_parses_correctly() {
        let msg = Message::builder()
            .address(ESCAPE)
            .destination(0)
            .data([ESCAPE])
            .build()
            .unwrap();
        let bytes = msg.to_bytes();

        let mut p = StateMachineParser::new();
        let results = p.feed(&bytes);
        assert_eq!(results, [ParseResult::Complete(msg)]);
    }

    #[test]
    fn partial_at_address_then_complete() {
        let msg = Message::builder()
            .address(1)
            .destination(2)
            .data([])
            .build()
            .unwrap();
        let bytes = msg.to_bytes();
        let (first, second) = bytes.split_at(2);

        let mut p = StateMachineParser::new();
        let r1 = p.feed(first);
        let r2 = p.feed(second);

        assert_eq!(r1, [ParseResult::Partial]);
        assert_eq!(r2, [ParseResult::Complete(msg)]);
    }

    #[test]
    fn partial_at_destination_then_complete() {
        let msg = Message::builder()
            .address(1)
            .destination(2)
            .data([])
            .build()
            .unwrap();
        let bytes = msg.to_bytes();
        let (first, second) = bytes.split_at(3);

        let mut p = StateMachineParser::new();
        let r1 = p.feed(first);
        let r2 = p.feed(second);

        assert_eq!(r1, [ParseResult::Partial]);
        assert_eq!(r2, [ParseResult::Complete(msg)]);
    }

    #[test]
    fn partial_at_data_length_then_complete() {
        let msg = Message::builder()
            .address(1)
            .destination(2)
            .data([3u8])
            .build()
            .unwrap();
        let bytes = msg.to_bytes();
        let (first, second) = bytes.split_at(4);

        let mut p = StateMachineParser::new();
        let r1 = p.feed(first);
        let r2 = p.feed(second);

        assert_eq!(r1, [ParseResult::Partial]);
        assert_eq!(r2, [ParseResult::Complete(msg)]);
    }

    #[test]
    fn partial_at_data_then_complete() {
        let msg = Message::builder()
            .address(1)
            .destination(2)
            .data([10u8, 20])
            .build()
            .unwrap();
        let bytes = msg.to_bytes();
        let (first, second) = bytes.split_at(5);

        let mut p = StateMachineParser::new();
        let r1 = p.feed(first);
        let r2 = p.feed(second);

        assert_eq!(r1, [ParseResult::Partial]);
        assert_eq!(r2, [ParseResult::Complete(msg)]);
    }

    #[test]
    fn partial_at_checksum_then_complete() {
        let msg = Message::builder()
            .address(1)
            .destination(2)
            .data([7u8])
            .build()
            .unwrap();
        let bytes = msg.to_bytes();
        let split = bytes.len() - 1;
        let (first, second) = bytes.split_at(split);

        let mut p = StateMachineParser::new();
        let r1 = p.feed(first);
        let r2 = p.feed(second);

        assert_eq!(r1, [ParseResult::Partial]);
        assert_eq!(r2, [ParseResult::Complete(msg)]);
    }

    #[test]
    fn invalid_escape_at_address() {
        let mut bytes = START_SEQUENCE.to_vec();
        bytes.extend_from_slice(&[ESCAPE, 0x01]);

        let mut p = StateMachineParser::new();
        let results = p.feed(&bytes);

        assert_eq!(
            results,
            [ParseResult::Error(ParseError::InvalidEscapeSequence {
                offset: 2
            })]
        );
    }

    #[test]
    fn invalid_escape_at_destination() {
        let mut bytes = START_SEQUENCE.to_vec();
        bytes.extend_from_slice(&[0x01, ESCAPE, 0x01]);

        let mut p = StateMachineParser::new();
        let results = p.feed(&bytes);

        assert_eq!(
            results,
            [ParseResult::Error(ParseError::InvalidEscapeSequence {
                offset: 3
            })]
        );
    }

    #[test]
    fn invalid_escape_at_data_length() {
        let mut bytes = START_SEQUENCE.to_vec();
        bytes.extend_from_slice(&[0x01, 0x02, ESCAPE, 0x01]);

        let mut p = StateMachineParser::new();
        let results = p.feed(&bytes);

        assert_eq!(
            results,
            [ParseResult::Error(ParseError::InvalidEscapeSequence {
                offset: 4
            })]
        );
    }

    #[test]
    fn invalid_escape_in_data() {
        let mut bytes = START_SEQUENCE.to_vec();
        bytes.extend_from_slice(&[0x01, 0x02, 0x03, ESCAPE, 0x01]);

        let mut p = StateMachineParser::new();
        let results = p.feed(&bytes);

        assert_eq!(
            results,
            [ParseResult::Error(ParseError::InvalidEscapeSequence {
                offset: 5
            })]
        );
    }

    #[test]
    fn invalid_escape_at_checksum() {
        let msg = Message::builder()
            .address(1)
            .destination(2)
            .data([3u8, 4])
            .build()
            .unwrap();
        let mut bytes = msg.to_bytes();
        if let Some(last) = bytes.last_mut() {
            *last = ESCAPE;
        }
        bytes.push(0x01);

        let mut p = StateMachineParser::new();
        let results = p.feed(&bytes);

        assert_eq!(
            results,
            [ParseResult::Error(ParseError::InvalidEscapeSequence {
                offset: 7
            })]
        );
    }

    #[test]
    fn unexpected_start_at_address() {
        let mut bytes = START_SEQUENCE.to_vec();
        bytes.extend_from_slice(&START_SEQUENCE);

        let mut p = StateMachineParser::new();
        let results = p.feed(&bytes);

        assert_eq!(
            results,
            [
                ParseResult::Error(ParseError::UnexpectedStartSequence { offset: 2 }),
                ParseResult::Partial
            ]
        );
    }

    #[test]
    fn unexpected_start_at_destination() {
        let mut bytes = START_SEQUENCE.to_vec();
        bytes.push(0x01);
        bytes.extend_from_slice(&START_SEQUENCE);

        let mut p = StateMachineParser::new();
        let results = p.feed(&bytes);

        assert_eq!(
            results,
            [
                ParseResult::Error(ParseError::UnexpectedStartSequence { offset: 3 }),
                ParseResult::Partial
            ]
        );
    }

    #[test]
    fn unexpected_start_at_data_length() {
        let mut bytes = START_SEQUENCE.to_vec();
        bytes.extend_from_slice(&[0x01, 0x02]);
        bytes.extend_from_slice(&START_SEQUENCE);

        let mut p = StateMachineParser::new();
        let results = p.feed(&bytes);

        assert_eq!(
            results,
            [
                ParseResult::Error(ParseError::UnexpectedStartSequence { offset: 4 }),
                ParseResult::Partial
            ]
        );
    }

    #[test]
    fn unexpected_start_in_data() {
        let mut bytes = START_SEQUENCE.to_vec();
        bytes.extend_from_slice(&[0x01, 0x02, 0x03]);
        bytes.extend_from_slice(&START_SEQUENCE);

        let mut p = StateMachineParser::new();
        let results = p.feed(&bytes);

        assert_eq!(
            results,
            [
                ParseResult::Error(ParseError::UnexpectedStartSequence { offset: 5 }),
                ParseResult::Partial
            ]
        );
    }

    #[test]
    fn unexpected_start_at_checksum() {
        let msg = Message::builder()
            .address(1)
            .destination(2)
            .data([3u8, 4])
            .build()
            .unwrap();
        let mut bytes = msg.to_bytes();
        bytes.truncate(bytes.len() - 3);
        bytes.extend_from_slice(&START_SEQUENCE);

        let mut p = StateMachineParser::new();
        let results = p.feed(&bytes);

        assert_eq!(
            results,
            [
                ParseResult::Error(ParseError::UnexpectedStartSequence { offset: 5 }),
                ParseResult::Partial
            ]
        );
    }

    #[test]
    fn input_ends_on_first_ff_of_escape_then_resume() {
        let msg = Message::builder()
            .address(ESCAPE)
            .destination(0)
            .data([])
            .build()
            .unwrap();
        let bytes = msg.to_bytes();

        let (first, second) = bytes.split_at(3);
        assert_eq!(first, &[ESCAPE, 0x00, ESCAPE]);

        let mut p = StateMachineParser::new();
        let r1 = p.feed(first);
        let r2 = p.feed(second);

        assert_eq!(r1, [ParseResult::Partial]);
        assert_eq!(r2, [ParseResult::Complete(msg)]);
    }

    #[test]
    fn gap_between_two_messages() {
        let m1 = Message::builder()
            .address(1)
            .destination(2)
            .data([])
            .build()
            .unwrap();
        let m2 = Message::builder()
            .address(3)
            .destination(4)
            .data([])
            .build()
            .unwrap();
        let mut bytes = Vec::new();
        m1.write_bytes(&mut bytes).unwrap();
        bytes.extend_from_slice(&[0xAB, 0xCD]);
        m2.write_bytes(&mut bytes).unwrap();

        let mut p = StateMachineParser::new();
        let results = p.feed(&bytes);

        assert_eq!(
            results,
            [
                ParseResult::Complete(m1),
                ParseResult::Error(ParseError::Gap(2)),
                ParseResult::Complete(m2)
            ]
        );
    }

    #[test]
    fn initial_gap_before_first_message() {
        let msg = Message::builder()
            .address(1)
            .destination(2)
            .data([])
            .build()
            .unwrap();
        let mut bytes = vec![0x11, 0x22];
        msg.write_bytes(&mut bytes).unwrap();

        let mut p = StateMachineParser::new();
        let results = p.feed(&bytes);

        assert_eq!(
            results,
            [
                ParseResult::Error(ParseError::Gap(2)),
                ParseResult::Complete(msg)
            ]
        );
    }
}
