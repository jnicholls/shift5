use std::io;

use derive_builder::Builder;

/// Start sequence that denotes the beginning of a message.
pub const START_SEQUENCE: [u8; 2] = [0xFF, 0x00];

/// Escape byte: when followed by 0xFF, represents a literal 0xFF in the message stream.
pub const ESCAPE: u8 = 0xFF;

/// A message in the binary protocol.
#[derive(Builder, Clone, Debug, Eq, PartialEq)]
#[builder(setter(into), build_fn(skip), pattern = "owned")]
pub struct Message {
    pub address: u8,
    pub destination: u8,
    #[builder(setter(skip))]
    pub data_length: u8,
    pub data: Vec<u8>,
    #[builder(setter(skip))]
    pub checksum: u8,
}

impl Message {
    /// Create a new builder for constructing a Message.
    pub fn builder() -> MessageBuilder {
        MessageBuilder::default()
    }
}

impl MessageBuilder {
    /// Build the Message, computing the data_length and checksum from the other fields.
    pub fn build(self) -> Result<Message, MessageBuilderError> {
        let address = self
            .address
            .ok_or(MessageBuilderError::UninitializedField("address"))?;
        let destination = self
            .destination
            .ok_or(MessageBuilderError::UninitializedField("destination"))?;
        let data = self
            .data
            .ok_or(MessageBuilderError::UninitializedField("data"))?;

        if data.len() > u8::MAX as usize {
            return Err(MessageBuilderError::ValidationError(format!(
                "data length cannot exceed {}",
                u8::MAX
            )));
        }

        let data_length = data.len() as u8;
        let checksum = compute_checksum(address, destination, data_length, &data);

        Ok(Message {
            address,
            destination,
            data_length,
            data,
            checksum,
        })
    }
}

impl Message {
    /// Serialize this message to its binary wire format into a new Vec<u8>.
    /// Emits start sequence [0xFF, 0x00], then each logical byte (address, destination,
    /// data_length, data, checksum) with 0xFF escaped as [0xFF, 0xFF].
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        self.write_bytes(&mut out)
            .expect("failed to write bytes to output buffer");
        out
    }

    /// Serialize this message to its binary wire format into the given writer.
    /// Emits start sequence [0xFF, 0x00], then each logical byte (address, destination,
    /// data_length, data, checksum) with 0xFF escaped as [0xFF, 0xFF].
    pub fn write_bytes(&self, out: &mut impl io::Write) -> io::Result<()> {
        out.write_all(&START_SEQUENCE)?;
        emit_escaped_byte(self.address, out)?;
        emit_escaped_byte(self.destination, out)?;
        emit_escaped_byte(self.data_length, out)?;

        for b in self.data.iter().copied() {
            emit_escaped_byte(b, out)?;
        }

        emit_escaped_byte(self.checksum, out)?;
        Ok(())
    }
}

/// Compute the checksum for address, destination, data_length, and data.
pub fn compute_checksum(address: u8, destination: u8, data_length: u8, data: &[u8]) -> u8 {
    let data_sum = data.iter().copied().fold(0u8, |sum, b| sum.wrapping_add(b));
    address
        .wrapping_add(destination)
        .wrapping_add(data_length)
        .wrapping_add(data_sum)
}

fn emit_escaped_byte(b: u8, out: &mut impl io::Write) -> io::Result<()> {
    if b == ESCAPE {
        out.write_all(&[ESCAPE, ESCAPE])?;
    } else {
        out.write_all(&[b])?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    mod builder {
        use super::*;

        #[test]
        fn error_when_data_length_too_long() {
            let data = vec![0; u8::MAX as usize + 1];
            let msg = Message::builder()
                .address(1)
                .destination(2)
                .data(data)
                .build();
            assert!(
                matches!(msg, Err(MessageBuilderError::ValidationError(_))),
                "should return a validation error"
            );
        }

        #[test]
        fn error_when_address_is_not_set() {
            let msg = Message::builder().destination(2).data([1, 2, 3]).build();
            assert!(
                matches!(msg, Err(MessageBuilderError::UninitializedField("address"))),
                "should return an uninitialized field error for address"
            );
        }

        #[test]
        fn error_when_destination_is_not_set() {
            let msg = Message::builder().address(1).data([1, 2, 3]).build();
            assert!(
                matches!(
                    msg,
                    Err(MessageBuilderError::UninitializedField("destination"))
                ),
                "should return an uninitialized field error for destination"
            );
        }

        #[test]
        fn error_when_data_is_not_set() {
            let msg = Message::builder().address(1).destination(2).build();
            assert!(
                matches!(msg, Err(MessageBuilderError::UninitializedField("data"))),
                "should return an uninitialized field error for data"
            );
        }
    }

    mod to_bytes {
        use super::*;

        #[test]
        fn roundtrip_simple() {
            let data = [10, 20, 30];
            let msg = Message::builder()
                .address(1)
                .destination(2)
                .data(data)
                .build()
                .unwrap();
            let sum = compute_checksum(msg.address, msg.destination, msg.data_length, &msg.data);
            let bytes = msg.to_bytes();
            assert_eq!(bytes[0..2], START_SEQUENCE);
            assert_eq!(bytes[2], 1);
            assert_eq!(bytes[3], 2);
            assert_eq!(bytes[4], data.len() as u8);
            assert_eq!(bytes[5..8], [10, 20, 30]);
            assert_eq!(bytes[8], sum);
        }

        #[test]
        fn escape_ff() {
            let msg = Message::builder()
                .address(ESCAPE)
                .destination(0)
                .data([ESCAPE])
                .build()
                .unwrap();
            let sum = compute_checksum(msg.address, msg.destination, msg.data_length, &msg.data);
            let bytes = msg.to_bytes();
            assert_eq!(bytes[0..2], START_SEQUENCE);
            assert_eq!(bytes[2..4], [ESCAPE, ESCAPE], "address should be escaped");
            assert_eq!(bytes[4], 0);
            assert_eq!(bytes[5], 1);
            assert_eq!(bytes[6..8], [ESCAPE, ESCAPE], "data should be escaped");
            assert_eq!(
                sum, ESCAPE,
                "checksum should be equal to 0xFF and need to be escaped"
            );
            assert_eq!(bytes[8..10], [ESCAPE, ESCAPE], "checksum should be escaped");
        }
    }
}
