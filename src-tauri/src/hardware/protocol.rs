//! IOTL v1: one complete RGB value, network byte order, no per-frame ACKs.
pub const PACKET_LEN: usize = 29;
pub const VERSION: u8 = 1;
pub const TIMEOUT_MS: u32 = 1_000;
pub type Session = [u8; 16];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Frame {
    pub session: Session,
    pub sequence: u32,
    pub rgb: [u8; 3],
}
impl Frame {
    pub fn encode(self) -> [u8; PACKET_LEN] {
        let mut bytes = [0; PACKET_LEN];
        bytes[..4].copy_from_slice(b"IOTL");
        bytes[4] = VERSION;
        bytes[6..22].copy_from_slice(&self.session);
        bytes[22..26].copy_from_slice(&self.sequence.to_be_bytes());
        bytes[26..].copy_from_slice(&self.rgb);
        bytes
    }
    pub fn decode(bytes: &[u8]) -> Option<Self> {
        if bytes.len() != PACKET_LEN
            || &bytes[..4] != b"IOTL"
            || bytes[4] != VERSION
            || bytes[5] != 0
        {
            return None;
        }
        Some(Self {
            session: bytes[6..22].try_into().ok()?,
            sequence: u32::from_be_bytes(bytes[22..26].try_into().ok()?),
            rgb: bytes[26..].try_into().ok()?,
        })
    }
}

/// RFC 1982 serial arithmetic. Exactly half the range is ambiguous and rejected.
pub fn newer(sequence: u32, previous: u32) -> bool {
    let distance = sequence.wrapping_sub(previous);
    distance != 0 && distance < 0x8000_0000
}
pub fn token() -> Result<Session, String> {
    let mut token = [0; 16];
    getrandom::fill(&mut token).map_err(|e| e.to_string())?;
    Ok(token)
}
pub fn hex(token: &Session) -> String {
    token.iter().map(|b| format!("{b:02x}")).collect()
}
pub fn parse_token(value: &str) -> Option<Session> {
    if value.len() != 32
        || !value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return None;
    }
    let mut result = [0; 16];
    for (i, byte) in result.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&value[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn packet_matches_firmware_golden_vector() {
        let frame = Frame {
            session: [0x11; 16],
            sequence: 0xffff_fffe,
            rgb: [1, 128, 255],
        };
        let golden = include_str!("../../../tests/fixtures/udp-v1.hex").trim();
        assert_eq!(
            frame
                .encode()
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>(),
            golden
        );
        assert_eq!(Frame::decode(&frame.encode()), Some(frame));
        for index in [0, 4, 5] {
            let mut invalid = frame.encode();
            invalid[index] ^= 1;
            assert!(Frame::decode(&invalid).is_none());
        }
        assert!(Frame::decode(&frame.encode()[..28]).is_none());
        assert!(Frame::decode(&[0; 30]).is_none());
    }
    #[test]
    fn wrapping_sequences_and_session_tokens() {
        assert!(newer(0, u32::MAX));
        assert!(!newer(u32::MAX, 0));
        assert!(!newer(12, 12));
        assert!(!newer(0x8000_0000, 0));
        assert_eq!(parse_token(&hex(&[255; 16])), Some([255; 16]));
        assert!(parse_token("aa").is_none());
        assert!(parse_token(&"x".repeat(32)).is_none());
    }
}
