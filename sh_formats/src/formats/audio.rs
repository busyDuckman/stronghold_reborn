// --------------------------------------------------------------------------------------------------------------------
// The kind 3 audio header: 13 bytes wrapping somebody else's sound file.
//
// From about_assets.md: u8 leading byte = 3, u16 data_size at +5, u16 sample rate in Hz at +11,
// and the bytes between them a far pointer left over from serialising a resident structure.
// The rate is zero for music and non-zero for samples. What follows the header is one of three
// things and only its content says which: bare unsigned 8-bit PCM, an IFF FORM, or an IFF
// FORM with the sample sign flip applied to every byte so FORM only shows after XOR 0x80.

use crate::error::{FormatError, Result};
use crate::file_format::{FileFormat, SourceFileType};

pub const AUDIO_TYPE_BYTE: u8 = 3;
pub const AUDIO_HEADER_LEN: usize = 13;

/// What is behind the header.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum AudioType {
    /// Raw unsigned 8-bit samples, no container.
    Pcm,
    /// AIL/Miles XMI music, an IFF FORM.
    Xmi,
    /// IFF 8SVX audio, stored with every byte XOR 0x80.
    Iff8svxFlipped,
}

impl AudioType {
    pub fn extension(self) -> &'static str {
        match self {
            AudioType::Pcm => "pcm",
            AudioType::Xmi => "xmi",
            AudioType::Iff8svxFlipped => "8svx",
        }
    }
}

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct AudioHeader {
    pub audio_type: AudioType,
    /// Sample rate in Hz; zero for music.
    pub sample_rate: u16,
    /// Bytes of audio after the header.
    pub data_size: usize,
}

impl AudioHeader {
    /// The sound bytes behind the header, as stored: 8SVX still has its sign flip on.
    pub fn body_of<'a>(&self, payload: &'a [u8]) -> &'a [u8] {
        &payload[AUDIO_HEADER_LEN.min(payload.len())..]
    }
}

impl FileFormat for AudioHeader {
    const DESCRIPTION: &'static str = "a sound";

    fn read(bytes: &[u8]) -> Result<AudioHeader> {
        if bytes.first() != Some(&AUDIO_TYPE_BYTE) {
            return Err(FormatError::malformed("leading byte is not 3"));
        }
        if bytes.len() <= AUDIO_HEADER_LEN {
            return Err(FormatError::malformed(format!("{} bytes is no more than the {AUDIO_HEADER_LEN}-byte header", bytes.len())));
        }
        let data_size = u16::from_le_bytes([bytes[5], bytes[6]]) as usize;
        if AUDIO_HEADER_LEN + data_size != bytes.len() {
            return Err(FormatError::malformed(format!("declared data size {data_size} plus the header is not the payload size {}", bytes.len())));
        }
        let sample_rate = u16::from_le_bytes([bytes[11], bytes[12]]);
        let inner = &bytes[AUDIO_HEADER_LEN..];
        let audio_type = if inner.starts_with(b"FORM") {
            AudioType::Xmi
        } else if inner.len() >= 4 && inner[..4].iter().zip(b"FORM").all(|(b, f)| b ^ 0x80 == *f) {
            AudioType::Iff8svxFlipped
        } else {
            AudioType::Pcm
        };
        Ok(AudioHeader { audio_type, sample_rate, data_size })
    }

    fn source_type(&self) -> SourceFileType {
        SourceFileType::Audio(*self)
    }
}

// --------------------------------------------------------------------------------------------------------------------
// --------------------------------------------------------------------------------------------------------------------
// --------------------------------------------------------------------------------------------------------------------
// Tests

#[cfg(test)]
mod tests {
    use super::*;

    fn wrapped(inner: &[u8], rate: u16) -> Vec<u8> {
        let mut payload = vec![AUDIO_TYPE_BYTE, 0, 0, 0, 0];
        payload.extend_from_slice(&(inner.len() as u16).to_le_bytes());
        payload.extend_from_slice(&[0x3c, 0x00, 0x07, 0xb8]);
        payload.extend_from_slice(&rate.to_le_bytes());
        payload.extend_from_slice(inner);
        payload
    }

    #[test]
    fn the_three_things_under_the_header_are_told_apart() {
        let payload = wrapped(&[128, 129, 127, 128], 8000);
        let pcm = AudioHeader::read(&payload).expect("pcm");
        assert_eq!((pcm.audio_type, pcm.sample_rate, pcm.data_size), (AudioType::Pcm, 8000, 4));
        assert_eq!(pcm.body_of(&payload), &[128, 129, 127, 128]);
        let xmi = AudioHeader::read(&wrapped(b"FORMxxxx", 0)).expect("xmi");
        assert_eq!((xmi.audio_type, xmi.source_type()), (AudioType::Xmi, SourceFileType::Audio(xmi)));
        let flipped: Vec<u8> = b"FORM8SVX".iter().map(|b| b ^ 0x80).collect();
        assert_eq!(AudioHeader::read(&wrapped(&flipped, 11111)).expect("8svx").audio_type, AudioType::Iff8svxFlipped);
    }

    #[test]
    fn a_header_that_does_not_add_up_is_refused() {
        let mut bad = wrapped(&[1, 2, 3], 8000);
        bad[5] = 9;
        assert!(AudioHeader::read(&bad).is_err());
        assert!(AudioHeader::read(&[3; 13]).is_err());
        assert!(AudioHeader::read(&[1; 20]).is_err());
    }
}
