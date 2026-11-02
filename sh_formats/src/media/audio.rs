// --------------------------------------------------------------------------------------------------------------------
// A sound: samples at a rate, with nothing of the game's wrapper left on it.

use crate::error::{FormatError, Result};
use crate::formats::audio::{AudioHeader, AudioType};
use crate::media::export::Exporter;
use std::io::Write;

/// Mono, unsigned 8-bit samples, as the game's DAC took them.
#[derive(Clone, PartialEq, Eq)]
pub struct AudioClip {
    sample_rate: u32,
    samples: Vec<u8>,
}

impl std::fmt::Debug for AudioClip {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "AudioClip {} samples at {} Hz", self.samples.len(), self.sample_rate)
    }
}

impl AudioClip {
    pub fn new(sample_rate: u32, samples: Vec<u8>) -> AudioClip {
        AudioClip { sample_rate, samples }
    }

    /// The samples behind a kind 3 header. Only bare PCM is a clip: music is a score, not
    /// samples, and the 8SVX payloads are not decoded yet.
    pub fn from_dat(header: &AudioHeader, payload: &[u8]) -> Result<AudioClip> {
        match header.audio_type {
            AudioType::Pcm => Ok(AudioClip::new(header.sample_rate as u32, header.body_of(payload).to_vec())),
            AudioType::Xmi => Err(FormatError::malformed("XMI is music, not samples")),
            AudioType::Iff8svxFlipped => Err(FormatError::malformed("8SVX payloads are not decoded yet")),
        }
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub fn samples(&self) -> &[u8] {
        &self.samples
    }

    pub fn len(&self) -> usize {
        self.samples.len()
    }

    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    pub fn duration_seconds(&self) -> f64 {
        if self.sample_rate == 0 { 0.0 } else { self.samples.len() as f64 / self.sample_rate as f64 }
    }
}

// --------------------------------------------------------------------------------------------------------------------
// Out as a file

/// WAV, mono unsigned 8-bit PCM: the samples as they are, with the 44-byte header in front.
/// Small enough to write by hand rather than take a crate for.
pub struct Wav;

impl Exporter<AudioClip> for Wav {
    const EXTENSION: &'static str = "wav";

    fn write(&self, clip: &AudioClip, out: &mut dyn Write) -> Result<()> {
        let data_len = clip.len() as u32;
        let mut header = Vec::with_capacity(44);
        header.extend_from_slice(b"RIFF");
        header.extend_from_slice(&(36 + data_len).to_le_bytes());
        header.extend_from_slice(b"WAVEfmt ");
        header.extend_from_slice(&16u32.to_le_bytes());
        header.extend_from_slice(&1u16.to_le_bytes());
        header.extend_from_slice(&1u16.to_le_bytes());
        header.extend_from_slice(&clip.sample_rate().to_le_bytes());
        header.extend_from_slice(&clip.sample_rate().to_le_bytes());
        header.extend_from_slice(&1u16.to_le_bytes());
        header.extend_from_slice(&8u16.to_le_bytes());
        header.extend_from_slice(b"data");
        header.extend_from_slice(&data_len.to_le_bytes());
        let io = |e| FormatError::Export(format!("wav: {e}"));
        out.write_all(&header).map_err(io)?;
        out.write_all(clip.samples()).map_err(io)
    }
}

// --------------------------------------------------------------------------------------------------------------------
// --------------------------------------------------------------------------------------------------------------------
// --------------------------------------------------------------------------------------------------------------------
// Tests

#[cfg(test)]
mod tests {
    use super::*;
    use crate::file_format::FileFormat;

    fn wrapped(inner: &[u8], rate: u16) -> Vec<u8> {
        let mut payload = vec![3u8, 0, 0, 0, 0];
        payload.extend_from_slice(&(inner.len() as u16).to_le_bytes());
        payload.extend_from_slice(&[0x3c, 0x00, 0x07, 0xb8]);
        payload.extend_from_slice(&rate.to_le_bytes());
        payload.extend_from_slice(inner);
        payload
    }

    #[test]
    fn pcm_becomes_a_clip_and_the_rest_says_why_not() {
        let payload = wrapped(&[128, 129, 127, 128], 8000);
        let clip = AudioClip::from_dat(&AudioHeader::read(&payload).expect("header"), &payload).expect("clip");
        assert_eq!((clip.sample_rate(), clip.samples()), (8000, &[128u8, 129, 127, 128][..]));
        assert_eq!(clip.duration_seconds(), 0.0005);
        let music = wrapped(b"FORMxxxx", 0);
        assert!(AudioClip::from_dat(&AudioHeader::read(&music).expect("header"), &music).is_err());
    }

    #[test]
    fn wav_has_the_header_the_samples_deserve() {
        let bytes = Wav.to_bytes(&AudioClip::new(8000, vec![128, 200, 50])).expect("encodes");
        assert_eq!(bytes.len(), 47);
        assert_eq!(&bytes[..4], b"RIFF");
        assert_eq!(u32::from_le_bytes([bytes[24], bytes[25], bytes[26], bytes[27]]), 8000);
        assert_eq!(u16::from_le_bytes([bytes[34], bytes[35]]), 8, "8 bits a sample");
        assert_eq!(&bytes[44..], &[128, 200, 50]);
        assert_eq!(<Wav as Exporter<AudioClip>>::EXTENSION, "wav");
    }
}
