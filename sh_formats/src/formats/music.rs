// --------------------------------------------------------------------------------------------------------------------
// XMI: the AIL/Miles music score, an IFF FORM. The 43 music entries carry one behind a kind 3
// audio header, which is why the header's sample rate is zero on every one of them.
//
// The shape, checked against all 43 in the CD build: FORM XDIR with a two byte INFO sequence
// count, then a CAT XMID beside it holding one FORM XMID per song, each an optional TIMB
// naming the instruments and a mandatory EVNT holding the music.
//
// An EVNT event is the run of bytes below 0x80 in front of it, summed into an interval, then a
// status byte. Running status is not used and there are no note off events: a note on carries
// how long it sounds, as a variable length quantity after the velocity.
//
// An interval is 1/120 second of real time and does not move with the tempo. The AIL manual
// says so in the rules for the For and Next loop controllers, which require "more than 1/120
// second (8.4 milliseconds) of real time" between them, that being one quantization interval.

use crate::error::{FormatError, Result};
use crate::file_format::{FileFormat, SourceFileType};
use crate::formats::audio::{AudioHeader, AudioType};

/// Intervals a second, fixed, whatever a tempo event in the stream says.
pub const XMI_TICKS_PER_SECOND: u32 = 120;

/// The status byte a meta event opens with; its type byte follows.
pub const META_STATUS: u8 = 0xff;
/// Meta type: the last event of a track.
pub const META_END_OF_TRACK: u8 = 0x2f;
/// Meta type: microseconds a quarter note lasts, three bytes, most significant first.
pub const META_TEMPO: u8 = 0x51;
/// A sysex, and the continuation form of one.
pub const SYSEX_START: u8 = 0xf0;
pub const SYSEX_CONTINUE: u8 = 0xf7;

/// Status nibbles, channel in the low four bits.
pub const NOTE_OFF: u8 = 0x80;
pub const NOTE_ON: u8 = 0x90;
pub const POLY_PRESSURE: u8 = 0xa0;
pub const CONTROLLER: u8 = 0xb0;
pub const PROGRAM_CHANGE: u8 = 0xc0;
pub const CHANNEL_PRESSURE: u8 = 0xd0;
pub const PITCH_BEND: u8 = 0xe0;

/// An instrument the sequence asks the driver to load before it plays.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct Timbre {
    pub patch: u8,
    /// 0 is the melodic bank and 127 the percussion one, both indexing midpak.ad, entry 1109.
    pub bank: u8,
}

/// One event of an EVNT chunk, as it sits on the wire.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct XmiEvent {
    /// Ticks of the 120 Hz clock between the previous event and this one.
    pub interval: u32,
    /// The status byte. XMI states one on every event; running status is not used.
    pub status: u8,
    /// What follows the status: the data bytes of a channel message, the type byte then the
    /// payload for a meta, the payload alone for a sysex.
    pub data: Vec<u8>,
    /// How long a note on sounds, in the same ticks. None on every other status.
    pub duration: Option<u32>,
}

/// One song: the instruments it wants and the events that play it.
#[derive(Clone, PartialEq, Eq)]
pub struct XmiSequence {
    pub timbres: Vec<Timbre>,
    pub events: Vec<XmiEvent>,
}

impl std::fmt::Debug for XmiSequence {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "XmiSequence {} events, {} timbres, {} ticks", self.events.len(), self.timbres.len(), self.tick_count())
    }
}

impl XmiSequence {
    /// How long the song runs, in 120 Hz ticks.
    pub fn tick_count(&self) -> u32 {
        self.events.iter().map(|e| e.interval).sum()
    }

    pub fn duration_seconds(&self) -> f64 {
        self.tick_count() as f64 / XMI_TICKS_PER_SECOND as f64
    }
}

/// An XMI file: the catalogue of songs in it.
#[derive(Clone, PartialEq, Eq)]
pub struct XmiScore {
    pub sequences: Vec<XmiSequence>,
}

impl std::fmt::Debug for XmiScore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "XmiScore {} sequences", self.sequences.len())
    }
}

impl FileFormat for XmiScore {
    const DESCRIPTION: &'static str = "XMI music";

    /// Reads the IFF as stored, with or without the kind 3 audio header in front of it.
    fn read(bytes: &[u8]) -> Result<XmiScore> {
        let iff = match AudioHeader::read(bytes) {
            Ok(header) if header.audio_type == AudioType::Xmi => header.body_of(bytes),
            _ => bytes,
        };
        let first = chunk_at(iff, 0)?;
        let sequences = match (first.tag, group_type(&first)) {
            (b"FORM", Some(b"XDIR")) => {
                let declared = declared_count(&first)?;
                let found = sequences_in(&chunk_at(iff, first.stride)?)?;
                // The events are the authority; refusing here would lose music over a stale count.
                if found.len() != declared {
                    log::warn!("XMI directory declares {declared} sequences, the catalogue holds {}", found.len());
                }
                found
            }
            (b"CAT ", Some(b"XMID")) => sequences_in(&first)?,
            (b"FORM", Some(b"XMID")) => vec![sequence_of(&first)?],
            _ => return Err(FormatError::malformed(format!("{} is not how an XMI starts", tag_name(first.tag)))),
        };
        if sequences.is_empty() {
            return Err(FormatError::malformed("the catalogue holds no sequences"));
        }
        Ok(XmiScore { sequences })
    }

    fn source_type(&self) -> SourceFileType {
        SourceFileType::Xmi
    }
}

// --------------------------------------------------------------------------------------------------------------------
// Walking the IFF container

/// One IFF chunk: the tag, the body after the big endian length, and how far to the next one.
struct Chunk<'a> {
    tag: &'a [u8],
    body: &'a [u8],
    /// Header plus body plus the pad byte an odd length carries.
    stride: usize,
}

fn chunk_at(bytes: &[u8], at: usize) -> Result<Chunk<'_>> {
    if at + 8 > bytes.len() {
        return Err(FormatError::malformed(format!("an IFF chunk header wants 8 bytes and {} are left", bytes.len().saturating_sub(at))));
    }
    let len = u32::from_be_bytes([bytes[at + 4], bytes[at + 5], bytes[at + 6], bytes[at + 7]]) as usize;
    let end = at + 8 + len;
    if end > bytes.len() {
        return Err(FormatError::malformed(format!("{} at {at} declares {len} bytes and {} are left", tag_name(&bytes[at..at + 4]), bytes.len() - at - 8)));
    }
    Ok(Chunk { tag: &bytes[at..at + 4], body: &bytes[at + 8..end], stride: 8 + len + (len & 1) })
}

/// The four character type a FORM, CAT or LIST names in the first bytes of its body.
fn group_type<'a>(chunk: &Chunk<'a>) -> Option<&'a [u8]> {
    match chunk.tag {
        b"FORM" | b"CAT " | b"LIST" if chunk.body.len() >= 4 => Some(&chunk.body[..4]),
        _ => None,
    }
}

fn tag_name(tag: &[u8]) -> String {
    String::from_utf8_lossy(tag).escape_debug().to_string()
}

/// The XDIR's INFO chunk: how many sequences the catalogue beside it should hold.
fn declared_count(xdir: &Chunk<'_>) -> Result<usize> {
    let info = chunk_at(xdir.body, 4)?;
    if info.tag != b"INFO" || info.body.len() < 2 {
        return Err(FormatError::malformed("the XDIR has no two byte INFO count"));
    }
    Ok(u16::from_le_bytes([info.body[0], info.body[1]]) as usize)
}

/// Every FORM XMID in a catalogue, in order.
fn sequences_in(cat: &Chunk<'_>) -> Result<Vec<XmiSequence>> {
    if cat.tag != b"CAT " || group_type(cat) != Some(b"XMID") {
        return Err(FormatError::malformed(format!("{} is where a CAT XMID belongs", tag_name(cat.tag))));
    }
    let mut out = Vec::new();
    let mut at = 4;
    while at < cat.body.len() {
        let form = chunk_at(cat.body, at)?;
        out.push(sequence_of(&form)?);
        at += form.stride;
    }
    Ok(out)
}

/// One FORM XMID: the timbres it names and the events it plays.
fn sequence_of(form: &Chunk<'_>) -> Result<XmiSequence> {
    if form.tag != b"FORM" || group_type(form) != Some(b"XMID") {
        return Err(FormatError::malformed(format!("{} is where a FORM XMID belongs", tag_name(form.tag))));
    }
    let (mut timbres, mut events) = (Vec::new(), None);
    let mut at = 4;
    while at < form.body.len() {
        let chunk = chunk_at(form.body, at)?;
        match chunk.tag {
            b"TIMB" => timbres = timbres_in(chunk.body)?,
            b"EVNT" => events = Some(events_in(chunk.body)?),
            // RBRN is the branch point table, for sequences that jump; nothing here has one.
            b"RBRN" => (),
            other => return Err(FormatError::malformed(format!("{} is not a chunk of an XMI sequence", tag_name(other)))),
        }
        at += chunk.stride;
    }
    match events {
        Some(events) => Ok(XmiSequence { timbres, events }),
        None => Err(FormatError::malformed("a sequence carries no EVNT chunk")),
    }
}

fn timbres_in(body: &[u8]) -> Result<Vec<Timbre>> {
    if body.len() < 2 {
        return Err(FormatError::malformed("a TIMB chunk is shorter than its own count"));
    }
    let count = u16::from_le_bytes([body[0], body[1]]) as usize;
    if 2 + count * 2 > body.len() {
        return Err(FormatError::malformed(format!("a TIMB chunk claims {count} timbres and carries {} bytes", body.len() - 2)));
    }
    Ok((0..count).map(|n| Timbre { patch: body[2 + n * 2], bank: body[3 + n * 2] }).collect())
}

// --------------------------------------------------------------------------------------------------------------------
// The event stream

/// Intervals are summed runs of bytes below 0x80; every event then states its own status.
fn events_in(body: &[u8]) -> Result<Vec<XmiEvent>> {
    let mut out = Vec::new();
    let mut at = 0;
    while at < body.len() {
        let mut interval = 0u32;
        while at < body.len() && body[at] < 0x80 {
            interval += body[at] as u32;
            at += 1;
        }
        let status = *body.get(at).ok_or_else(|| FormatError::malformed("the event stream ends inside an interval"))?;
        at += 1;
        let (data, duration) = event_body(body, &mut at, status)?;
        let done = status == META_STATUS && data.first() == Some(&META_END_OF_TRACK);
        out.push(XmiEvent { interval, status, data, duration });
        // Whatever pads the chunk after the end of track marker is not music.
        if done {
            break;
        }
    }
    Ok(out)
}

/// What follows one status byte, and the duration if it is a note on.
fn event_body(body: &[u8], at: &mut usize, status: u8) -> Result<(Vec<u8>, Option<u32>)> {
    let take = |at: &mut usize, count: usize| -> Result<Vec<u8>> {
        let end = *at + count;
        if end > body.len() {
            return Err(FormatError::malformed(format!("a {status:#04x} event wants {count} bytes and {} are left", body.len() - *at)));
        }
        let taken = body[*at..end].to_vec();
        *at = end;
        Ok(taken)
    };
    match status {
        META_STATUS => {
            let meta_type = take(at, 1)?[0];
            let len = read_vlq(body, at)? as usize;
            let mut data = vec![meta_type];
            data.extend_from_slice(&take(at, len)?);
            Ok((data, None))
        }
        SYSEX_START | SYSEX_CONTINUE => {
            let len = read_vlq(body, at)? as usize;
            Ok((take(at, len)?, None))
        }
        _ => match status & 0xf0 {
            NOTE_ON => {
                let data = take(at, 2)?;
                Ok((data, Some(read_vlq(body, at)?)))
            }
            NOTE_OFF | POLY_PRESSURE | CONTROLLER | PITCH_BEND => Ok((take(at, 2)?, None)),
            PROGRAM_CHANGE | CHANNEL_PRESSURE => Ok((take(at, 1)?, None)),
            _ => Err(FormatError::malformed(format!("{status:#04x} is not a status byte an XMI may carry"))),
        },
    }
}

/// A standard MIDI variable length quantity: 7 bits a byte, most significant byte first.
pub fn read_vlq(body: &[u8], at: &mut usize) -> Result<u32> {
    let mut value: u32 = 0;
    for _ in 0..4 {
        let byte = *body.get(*at).ok_or_else(|| FormatError::malformed("a variable length quantity runs past the chunk"))?;
        *at += 1;
        value = (value << 7) | (byte & 0x7f) as u32;
        if byte & 0x80 == 0 {
            return Ok(value);
        }
    }
    Err(FormatError::malformed("a variable length quantity is longer than the four bytes MIDI allows"))
}

// --------------------------------------------------------------------------------------------------------------------
// --------------------------------------------------------------------------------------------------------------------
// --------------------------------------------------------------------------------------------------------------------
// Tests

#[cfg(test)]
mod tests {
    use super::*;

    fn chunk(tag: &[u8], body: &[u8]) -> Vec<u8> {
        let mut out = tag.to_vec();
        out.extend_from_slice(&(body.len() as u32).to_be_bytes());
        out.extend_from_slice(body);
        if body.len() & 1 == 1 {
            out.push(0);
        }
        out
    }

    /// One song in the shape every entry in the archive has.
    fn xmi(events: &[u8], timbres: &[u8]) -> Vec<u8> {
        let mut song = b"XMID".to_vec();
        song.extend_from_slice(&chunk(b"TIMB", timbres));
        song.extend_from_slice(&chunk(b"EVNT", events));
        let mut cat = b"XMID".to_vec();
        cat.extend_from_slice(&chunk(b"FORM", &song));
        let mut dir = b"XDIR".to_vec();
        dir.extend_from_slice(&chunk(b"INFO", &1u16.to_le_bytes()));
        let mut out = chunk(b"FORM", &dir);
        out.extend_from_slice(&chunk(b"CAT ", &cat));
        out
    }

    /// Note on, middle C, velocity 100, lasting 24 ticks, then end of track.
    const ONE_NOTE: &[u8] = &[0x90, 60, 100, 24, 0x0a, 0xff, 0x2f, 0x00];

    #[test]
    fn the_container_comes_apart_into_sequences() {
        let score = XmiScore::read(&xmi(ONE_NOTE, &[1u8, 0, 48, 0])).expect("reads");
        assert_eq!(score.sequences.len(), 1);
        let song = &score.sequences[0];
        assert_eq!(song.timbres, vec![Timbre { patch: 48, bank: 0 }]);
        assert_eq!(song.events.len(), 2);
        assert_eq!(song.events[0], XmiEvent { interval: 0, status: 0x90, data: vec![60, 100], duration: Some(24) });
        assert_eq!(song.events[1], XmiEvent { interval: 10, status: 0xff, data: vec![0x2f], duration: None });
        assert_eq!(song.tick_count(), 10);
        assert_eq!(score.source_type(), SourceFileType::Xmi);
    }

    #[test]
    fn the_kind_3_header_in_front_is_stepped_over() {
        let inner = xmi(ONE_NOTE, &[0u8, 0]);
        let mut payload = vec![3u8, 0, 0, 0, 0];
        payload.extend_from_slice(&(inner.len() as u16).to_le_bytes());
        payload.extend_from_slice(&[0x3c, 0x00, 0x07, 0xb8, 0x00, 0x00]);
        payload.extend_from_slice(&inner);
        assert_eq!(XmiScore::read(&payload).expect("reads").sequences.len(), 1);
    }

    #[test]
    fn a_bare_form_xmid_and_a_bare_catalogue_both_read() {
        let mut song = b"XMID".to_vec();
        song.extend_from_slice(&chunk(b"EVNT", ONE_NOTE));
        let bare = chunk(b"FORM", &song);
        assert_eq!(XmiScore::read(&bare).expect("bare form").sequences[0].events.len(), 2);
        let mut cat = b"XMID".to_vec();
        cat.extend_from_slice(&bare);
        cat.extend_from_slice(&bare);
        assert_eq!(XmiScore::read(&chunk(b"CAT ", &cat)).expect("bare cat").sequences.len(), 2);
    }

    #[test]
    fn an_interval_is_the_sum_of_the_bytes_before_the_event() {
        // 0x7f 0x7f 0x40 is 127 + 127 + 64, not a three byte concatenation.
        let events = [0x7f, 0x7f, 0x40, 0xb0, 7, 100, 0xff, 0x2f, 0x00];
        let score = XmiScore::read(&xmi(&events, &[0u8, 0])).expect("reads");
        assert_eq!(score.sequences[0].events[0].interval, 318);
        assert_eq!(score.sequences[0].tick_count(), 318);
    }

    #[test]
    fn every_status_an_xmi_may_carry_is_sized_right() {
        let events = [
            0x90, 60, 100, 0x81, 0x00, // note on, duration 128 as a two byte quantity
            0xa0, 60, 40, 0xb0, 7, 90, 0xc0, 48, 0xd0, 70, 0xe0, 0, 64,
            0xf0, 0x03, 0x7e, 0x7f, 0xf7, 0xff, 0x51, 0x03, 0x07, 0xa1, 0x20, 0xff, 0x2f, 0x00,
        ];
        let score = XmiScore::read(&xmi(&events, &[0u8, 0])).expect("reads");
        let got = &score.sequences[0].events;
        assert_eq!(got.len(), 9);
        assert_eq!(got[0].duration, Some(128));
        assert_eq!(got[6].status, 0xf0);
        assert_eq!(got[6].data, vec![0x7e, 0x7f, 0xf7]);
        assert_eq!(got[7].data, vec![0x51, 0x07, 0xa1, 0x20], "the meta type leads the payload");
    }

    #[test]
    fn bytes_after_the_end_of_track_are_not_read_as_music() {
        let mut events = ONE_NOTE.to_vec();
        events.extend_from_slice(&[0xcc, 0xcc, 0xcc]);
        assert_eq!(XmiScore::read(&xmi(&events, &[0u8, 0])).expect("reads").sequences[0].events.len(), 2);
    }

    #[test]
    fn what_does_not_add_up_is_refused_with_a_reason() {
        assert!(XmiScore::read(b"FORM").is_err(), "no room for a header");
        assert!(XmiScore::read(&chunk(b"FORM", b"ILBMxxxx")).is_err(), "not an XMI form");
        assert!(XmiScore::read(&xmi(&[0x90, 60, 100, 24], &[0u8, 0])).is_ok(), "no end of track is allowed");
        assert!(XmiScore::read(&xmi(&[0x90, 60], &[0u8, 0])).is_err(), "a note on cut short");
        assert!(XmiScore::read(&xmi(&[0x10, 0x10], &[0u8, 0])).is_err(), "an interval with no event after it");
        assert!(XmiScore::read(&xmi(ONE_NOTE, &[9u8, 0, 1, 0])).is_err(), "a TIMB count past its own bytes");
        let mut unknown = b"XMID".to_vec();
        unknown.extend_from_slice(&chunk(b"JUNK", b"xx"));
        assert!(XmiScore::read(&chunk(b"FORM", &unknown)).is_err(), "an unknown chunk is not skipped");
    }

    #[test]
    fn variable_length_quantities_read_as_midi_writes_them() {
        let mut at = 0;
        assert_eq!(read_vlq(&[0x00], &mut at).expect("zero"), 0);
        at = 0;
        assert_eq!(read_vlq(&[0xff, 0xff, 0xff, 0x7f], &mut at).expect("largest"), 0x0fff_ffff);
        assert_eq!(at, 4);
        at = 0;
        assert!(read_vlq(&[0xff, 0xff, 0xff, 0xff, 0x7f], &mut at).is_err(), "five bytes is not legal");
        at = 0;
        assert!(read_vlq(&[0x81], &mut at).is_err(), "runs past the end");
    }
}
