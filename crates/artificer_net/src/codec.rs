//! Versioned wire codec: 2-byte little-endian protocol version prefix +
//! postcard-encoded payload. Version mismatches are detected before any
//! payload parsing so old/new clients fail loudly and cheaply.

use serde::{de::DeserializeOwned, Serialize};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodecError {
    /// The message advertised a different protocol version.
    VersionMismatch { got: u16, expected: u16 },
    /// Too short to even carry the version prefix.
    Truncated,
    /// Payload failed to parse.
    Corrupt(String),
}

impl std::fmt::Display for CodecError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CodecError::VersionMismatch { got, expected } => {
                write!(
                    f,
                    "protocol version mismatch: got {got}, expected {expected}"
                )
            }
            CodecError::Truncated => write!(f, "message truncated"),
            CodecError::Corrupt(e) => write!(f, "corrupt payload: {e}"),
        }
    }
}

impl std::error::Error for CodecError {}

pub fn encode_versioned<T: Serialize>(protocol: u16, value: &T) -> Vec<u8> {
    let mut out = protocol.to_le_bytes().to_vec();
    let payload = postcard::to_stdvec(value).expect("serializable message");
    out.extend_from_slice(&payload);
    out
}

pub fn decode_versioned<T: DeserializeOwned>(
    expected_protocol: u16,
    bytes: &[u8],
) -> Result<T, CodecError> {
    if bytes.len() < 2 {
        return Err(CodecError::Truncated);
    }
    let got = u16::from_le_bytes([bytes[0], bytes[1]]);
    if got != expected_protocol {
        return Err(CodecError::VersionMismatch {
            got,
            expected: expected_protocol,
        });
    }
    postcard::from_bytes(&bytes[2..]).map_err(|e| CodecError::Corrupt(e.to_string()))
}

/// Peek the protocol version without parsing the payload (server-side
/// rejection with a friendly message).
pub fn peek_version(bytes: &[u8]) -> Option<u16> {
    if bytes.len() < 2 {
        return None;
    }
    Some(u16::from_le_bytes([bytes[0], bytes[1]]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct Msg {
        a: u32,
        b: String,
        v: [f32; 3],
    }

    #[test]
    fn round_trip() {
        let m = Msg {
            a: 7,
            b: "hello".into(),
            v: [1.0, -2.5, 3.25],
        };
        let bytes = encode_versioned(3, &m);
        let back: Msg = decode_versioned(3, &bytes).unwrap();
        assert_eq!(m, back);
    }

    #[test]
    fn version_mismatch_rejected() {
        let m = Msg {
            a: 1,
            b: "x".into(),
            v: [0.0; 3],
        };
        let bytes = encode_versioned(2, &m);
        let err = decode_versioned::<Msg>(3, &bytes).unwrap_err();
        assert_eq!(
            err,
            CodecError::VersionMismatch {
                got: 2,
                expected: 3
            }
        );
        assert_eq!(peek_version(&bytes), Some(2));
    }

    #[test]
    fn truncated_and_corrupt_rejected() {
        assert_eq!(
            decode_versioned::<Msg>(1, &[5]).unwrap_err(),
            CodecError::Truncated
        );
        let mut bytes = encode_versioned(
            1,
            &Msg {
                a: 1,
                b: "y".into(),
                v: [0.0; 3],
            },
        );
        bytes.truncate(4);
        assert!(matches!(
            decode_versioned::<Msg>(1, &bytes).unwrap_err(),
            CodecError::Corrupt(_)
        ));
    }
}
