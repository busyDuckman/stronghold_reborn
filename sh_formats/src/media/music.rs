// --------------------------------------------------------------------------------------------------------------------
// A piece of music: messages on one timeline, with nothing of the game's encoding left on them.
//
// The timeline is MIDI's, ticks of a division the score states, and the tempo rides in the
// messages the way a standard MIDI file carries it. Image is RGBA because that is what a
// modern consumer of a picture wants; a Score is MIDI for the same reason.
//
// Coming from XMI is two jobs. A note on carries how long it sounds rather than being answered
// by a note off, so every one schedules its own. And an XMI interval is 1/120 second of real
// time whatever the tempo says, so ticks are worked out against the tempo in force: that keeps
// the composer's tempo and bar lines in the file while the playing time stays the game's.

use crate::error::{FormatError, Result};
use crate::formats::music::{XmiSequence, CHANNEL_PRESSURE, CONTROLLER, META_END_OF_TRACK, META_STATUS, META_TEMPO, NOTE_OFF, NOTE_ON, PITCH_BEND, POLY_PRESSURE, PROGRAM_CHANGE, SYSEX_CONTINUE, SYSEX_START, XMI_TICKS_PER_SECOND};
use crate::media::export::Exporter;
use std::io::Write;

/// Ticks a quarter note is divided into: sixteen to an XMI interval at 120 BPM, and divisible
/// by 3, 4, 5 and 8, so triplets and the rest land on whole ticks.
pub const TICKS_PER_QUARTER: u16 = 960;

/// What a quarter note lasts until a tempo message says otherwise. MIDI's own default, 120 BPM.
pub const DEFAULT_TEMPO_US: u32 = 500_000;

/// The velocity a scheduled note off carries. A duration says when a note stops, not how hard.
const NOTE_OFF_VELOCITY: u8 = 0;

/// One MIDI message, with the channel lifted out of the status byte.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Message {
    NoteOff { channel: u8, note: u8, velocity: u8 },
    NoteOn { channel: u8, note: u8, velocity: u8 },
    PolyPressure { channel: u8, note: u8, pressure: u8 },
    Controller { channel: u8, controller: u8, value: u8 },
    ProgramChange { channel: u8, program: u8 },
    ChannelPressure { channel: u8, pressure: u8 },
    PitchBend { channel: u8, value: u16 },
    /// The type byte and the payload after it: tempo is 0x51, time signature 0x58.
    Meta { meta_type: u8, data: Vec<u8> },
    /// 0xf0 or 0xf7, and the payload that followed the length.
    SysEx { status: u8, data: Vec<u8> },
}

impl Message {
    /// Rebuild a message from a status byte and the bytes that followed it in the file.
    pub fn from_wire(status: u8, data: &[u8]) -> Result<Message> {
        let channel = status & 0x0f;
        let short = |want: usize| FormatError::malformed(format!("a {status:#04x} message carries {} bytes, not {want}", data.len()));
        match status {
            META_STATUS => match data.split_first() {
                Some((meta_type, payload)) => Ok(Message::Meta { meta_type: *meta_type, data: payload.to_vec() }),
                None => Err(short(1)),
            },
            SYSEX_START | SYSEX_CONTINUE => Ok(Message::SysEx { status, data: data.to_vec() }),
            _ => match (status & 0xf0, data) {
                (NOTE_OFF, [note, velocity]) => Ok(Message::NoteOff { channel, note: *note, velocity: *velocity }),
                (NOTE_ON, [note, velocity]) => Ok(Message::NoteOn { channel, note: *note, velocity: *velocity }),
                (POLY_PRESSURE, [note, pressure]) => Ok(Message::PolyPressure { channel, note: *note, pressure: *pressure }),
                (CONTROLLER, [controller, value]) => Ok(Message::Controller { channel, controller: *controller, value: *value }),
                (PROGRAM_CHANGE, [program]) => Ok(Message::ProgramChange { channel, program: *program }),
                (CHANNEL_PRESSURE, [pressure]) => Ok(Message::ChannelPressure { channel, pressure: *pressure }),
                (PITCH_BEND, [low, high]) => Ok(Message::PitchBend { channel, value: (*low as u16) | ((*high as u16) << 7) }),
                (NOTE_OFF | NOTE_ON | POLY_PRESSURE | CONTROLLER | PITCH_BEND, _) => Err(short(2)),
                (PROGRAM_CHANGE | CHANNEL_PRESSURE, _) => Err(short(1)),
                _ => Err(FormatError::malformed(format!("{status:#04x} is not a message a score may carry"))),
            },
        }
    }

    /// Append this message to a standard MIDI track, status byte first.
    pub fn write_to(&self, out: &mut Vec<u8>) -> Result<()> {
        match self {
            Message::NoteOff { channel, note, velocity } => out.extend_from_slice(&[NOTE_OFF | channel, *note, *velocity]),
            Message::NoteOn { channel, note, velocity } => out.extend_from_slice(&[NOTE_ON | channel, *note, *velocity]),
            Message::PolyPressure { channel, note, pressure } => out.extend_from_slice(&[POLY_PRESSURE | channel, *note, *pressure]),
            Message::Controller { channel, controller, value } => out.extend_from_slice(&[CONTROLLER | channel, *controller, *value]),
            Message::ProgramChange { channel, program } => out.extend_from_slice(&[PROGRAM_CHANGE | channel, *program]),
            Message::ChannelPressure { channel, pressure } => out.extend_from_slice(&[CHANNEL_PRESSURE | channel, *pressure]),
            Message::PitchBend { channel, value } => out.extend_from_slice(&[PITCH_BEND | channel, (value & 0x7f) as u8, ((value >> 7) & 0x7f) as u8]),
            Message::Meta { meta_type, data } => {
                out.extend_from_slice(&[META_STATUS, *meta_type]);
                write_vlq(out, data.len() as u32)?;
                out.extend_from_slice(data);
            }
            Message::SysEx { status, data } => {
                out.push(*status);
                write_vlq(out, data.len() as u32)?;
                out.extend_from_slice(data);
            }
        }
        Ok(())
    }

    /// The microseconds a quarter note lasts, if this is a tempo message.
    pub fn tempo_us(&self) -> Option<u32> {
        match self {
            Message::Meta { meta_type, data } if *meta_type == META_TEMPO && data.len() == 3 => Some(u32::from_be_bytes([0, data[0], data[1], data[2]])),
            _ => None,
        }
    }
}

/// One message and where it falls, in ticks from the start of the score.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct TimedMessage {
    pub at: u32,
    pub message: Message,
}

/// A piece of music: messages on one timeline, earliest first.
#[derive(Clone, PartialEq, Eq)]
pub struct Score {
    ticks_per_quarter: u16,
    messages: Vec<TimedMessage>,
}

impl std::fmt::Debug for Score {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Score {} messages, {} ticks at {} a quarter, {:.1}s", self.messages.len(), self.duration_ticks(), self.ticks_per_quarter, self.duration_seconds())
    }
}

impl Score {
    /// Messages are sorted into time order; a stable sort keeps what was built first, first.
    pub fn new(ticks_per_quarter: u16, mut messages: Vec<TimedMessage>) -> Score {
        messages.sort_by_key(|m| m.at);
        Score { ticks_per_quarter, messages }
    }

    /// An XMI sequence as a score, playing for exactly as long as the game played it.
    ///   - Every note on schedules the note off its duration asks for.
    ///   - Ticks are worked out against the tempo in force, re-anchored at each tempo change.
    ///   - The end of track marker belongs to the file format, so it is dropped here and the
    ///     exporter writes it back.
    pub fn from_xmi(sequence: &XmiSequence) -> Result<Score> {
        let mut timed: Vec<(u64, Message)> = Vec::with_capacity(sequence.events.len() * 2);
        let mut now: u64 = 0;
        for event in &sequence.events {
            now += event.interval as u64;
            let message = Message::from_wire(event.status, &event.data)?;
            if matches!(&message, Message::Meta { meta_type, .. } if *meta_type == META_END_OF_TRACK) {
                continue;
            }
            // A note on of no velocity is already a note off and must not be answered with another.
            let off = match (&message, event.duration) {
                (Message::NoteOn { channel, note, velocity }, Some(duration)) if *velocity > 0 => {
                    Some((now + duration as u64, Message::NoteOff { channel: *channel, note: *note, velocity: NOTE_OFF_VELOCITY }))
                }
                _ => None,
            };
            timed.push((now, message));
            timed.extend(off);
        }
        timed.sort_by_key(|(at, _)| *at);

        let mut messages = Vec::with_capacity(timed.len());
        let mut tempo_us = DEFAULT_TEMPO_US as u64;
        let (mut anchor_xmi, mut anchor_score) = (0u64, 0u64);
        for (xmi_at, message) in timed {
            let at = score_tick(xmi_at, anchor_xmi, anchor_score, tempo_us)?;
            if let Some(new_tempo) = message.tempo_us() {
                if new_tempo == 0 {
                    return Err(FormatError::malformed("a tempo of zero microseconds a quarter note"));
                }
                // Re-anchor so rounding cannot drift across a tempo change.
                (anchor_xmi, anchor_score, tempo_us) = (xmi_at, at, new_tempo as u64);
            }
            let at = u32::try_from(at).map_err(|_| FormatError::malformed(format!("the score runs to {at} ticks, past what a MIDI file holds")))?;
            messages.push(TimedMessage { at, message });
        }
        Ok(Score { ticks_per_quarter: TICKS_PER_QUARTER, messages })
    }

    pub fn ticks_per_quarter(&self) -> u16 {
        self.ticks_per_quarter
    }

    pub fn messages(&self) -> &[TimedMessage] {
        &self.messages
    }

    pub fn len(&self) -> usize {
        self.messages.len()
    }

    pub fn is_empty(&self) -> bool {
        self.messages.is_empty()
    }

    /// Where the last message falls.
    pub fn duration_ticks(&self) -> u32 {
        self.messages.last().map(|m| m.at).unwrap_or(0)
    }

    /// How long the score plays, walking its own tempo map.
    pub fn duration_seconds(&self) -> f64 {
        if self.ticks_per_quarter == 0 {
            return 0.0;
        }
        let mut seconds = 0.0;
        let mut tempo_us = DEFAULT_TEMPO_US as f64;
        let mut previous = 0u32;
        for timed in &self.messages {
            seconds += (timed.at - previous) as f64 * tempo_us / (self.ticks_per_quarter as f64 * 1_000_000.0);
            previous = timed.at;
            if let Some(tempo) = timed.message.tempo_us() {
                tempo_us = tempo as f64;
            }
        }
        seconds
    }

    /// Every channel the score touches, lowest first. Channel 9 is the percussion one.
    pub fn channels(&self) -> Vec<u8> {
        let mut seen = [false; 16];
        for timed in &self.messages {
            if let Some(channel) = timed.message.channel() {
                seen[channel as usize & 0x0f] = true;
            }
        }
        (0..16u8).filter(|c| seen[*c as usize]).collect()
    }
}

impl Message {
    /// The channel a channel message speaks on; None for meta and sysex.
    pub fn channel(&self) -> Option<u8> {
        match self {
            Message::NoteOff { channel, .. }
            | Message::NoteOn { channel, .. }
            | Message::PolyPressure { channel, .. }
            | Message::Controller { channel, .. }
            | Message::ProgramChange { channel, .. }
            | Message::ChannelPressure { channel, .. }
            | Message::PitchBend { channel, .. } => Some(*channel),
            Message::Meta { .. } | Message::SysEx { .. } => None,
        }
    }
}

/// Where an XMI tick lands on the score's timeline, measured from the last tempo change so
/// rounding cannot accumulate over a song.
fn score_tick(xmi_at: u64, anchor_xmi: u64, anchor_score: u64, tempo_us: u64) -> Result<u64> {
    let divisor = XMI_TICKS_PER_SECOND as u64 * tempo_us;
    let numerator = (xmi_at - anchor_xmi)
        .checked_mul(TICKS_PER_QUARTER as u64)
        .and_then(|n| n.checked_mul(1_000_000))
        .ok_or_else(|| FormatError::malformed("the score is too long to time in ticks"))?;
    Ok(anchor_score + (numerator + divisor / 2) / divisor)
}

/// Append a standard MIDI variable length quantity: 7 bits a byte, most significant first.
pub fn write_vlq(out: &mut Vec<u8>, value: u32) -> Result<()> {
    if value > 0x0fff_ffff {
        return Err(FormatError::Export(format!("{value} does not fit the four bytes a MIDI quantity has")));
    }
    let mut seven_bits = [0u8; 4];
    let mut count = 0;
    let mut left = value;
    loop {
        seven_bits[count] = (left & 0x7f) as u8;
        count += 1;
        left >>= 7;
        if left == 0 {
            break;
        }
    }
    while count > 0 {
        count -= 1;
        out.push(seven_bits[count] | if count > 0 { 0x80 } else { 0 });
    }
    Ok(())
}

// --------------------------------------------------------------------------------------------------------------------
// Out as a file

/// Standard MIDI file, format 0: the score's messages on the one track, deltas in its own ticks.
pub struct Midi;

impl Exporter<Score> for Midi {
    const EXTENSION: &'static str = "mid";

    fn write(&self, score: &Score, out: &mut dyn Write) -> Result<()> {
        let mut track = Vec::new();
        let mut previous = 0u32;
        for timed in score.messages() {
            write_vlq(&mut track, timed.at - previous)?;
            previous = timed.at;
            timed.message.write_to(&mut track)?;
        }
        // The end of track marker belongs to the file, not the music, so a Score never holds one.
        write_vlq(&mut track, 0)?;
        track.extend_from_slice(&[META_STATUS, META_END_OF_TRACK, 0]);
        let track_len = u32::try_from(track.len()).map_err(|_| FormatError::Export("mid: the track is longer than a chunk length holds".into()))?;
        let mut header = Vec::with_capacity(22);
        header.extend_from_slice(b"MThd");
        header.extend_from_slice(&6u32.to_be_bytes());
        header.extend_from_slice(&0u16.to_be_bytes());
        header.extend_from_slice(&1u16.to_be_bytes());
        header.extend_from_slice(&score.ticks_per_quarter().to_be_bytes());
        header.extend_from_slice(b"MTrk");
        header.extend_from_slice(&track_len.to_be_bytes());
        let io = |e| FormatError::Export(format!("mid: {e}"));
        out.write_all(&header).map_err(io)?;
        out.write_all(&track).map_err(io)
    }
}

// --------------------------------------------------------------------------------------------------------------------
// --------------------------------------------------------------------------------------------------------------------
// --------------------------------------------------------------------------------------------------------------------
// Tests

#[cfg(test)]
mod tests {
    use super::*;
    use crate::formats::music::XmiEvent;

    fn note_on(interval: u32, note: u8, duration: u32) -> XmiEvent {
        XmiEvent { interval, status: 0x90, data: vec![note, 100], duration: Some(duration) }
    }

    fn tempo(us: u32) -> XmiEvent {
        let bytes = us.to_be_bytes();
        XmiEvent { interval: 0, status: 0xff, data: vec![META_TEMPO, bytes[1], bytes[2], bytes[3]], duration: None }
    }

    fn sequence(events: Vec<XmiEvent>) -> XmiSequence {
        XmiSequence { timbres: Vec::new(), events }
    }

    #[test]
    fn a_note_on_duration_becomes_a_note_off() {
        let score = Score::from_xmi(&sequence(vec![note_on(0, 60, 60)])).expect("converts");
        assert_eq!(score.len(), 2);
        assert_eq!(score.messages()[0], TimedMessage { at: 0, message: Message::NoteOn { channel: 0, note: 60, velocity: 100 } });
        // 60 intervals is half a second, which at 120 BPM is a quarter note.
        assert_eq!(score.messages()[1], TimedMessage { at: 960, message: Message::NoteOff { channel: 0, note: 60, velocity: 0 } });
        assert!((score.duration_seconds() - 0.5).abs() < 1e-9);
    }

    #[test]
    fn a_note_off_sorts_ahead_of_the_retrigger_it_shares_a_tick_with() {
        let score = Score::from_xmi(&sequence(vec![note_on(0, 60, 10), note_on(10, 60, 10)])).expect("converts");
        let kinds: Vec<&Message> = score.messages().iter().map(|m| &m.message).collect();
        assert!(matches!(kinds[0], Message::NoteOn { .. }));
        assert!(matches!(kinds[1], Message::NoteOff { .. }), "the first note stops before it starts again");
        assert!(matches!(kinds[2], Message::NoteOn { .. }));
        assert_eq!((score.messages()[1].at, score.messages()[2].at), (160, 160));
    }

    #[test]
    fn a_zero_velocity_note_on_is_left_alone() {
        let quiet = XmiEvent { interval: 0, status: 0x90, data: vec![60, 0], duration: Some(60) };
        assert_eq!(Score::from_xmi(&sequence(vec![quiet])).expect("converts").len(), 1);
    }

    #[test]
    fn the_playing_time_is_the_interval_count_whatever_the_tempo_says() {
        // 100 BPM over 1200 intervals: still ten seconds, because an interval is 1/120 second.
        let events = vec![tempo(600_000), note_on(0, 60, 10), note_on(1200, 62, 10)];
        let score = Score::from_xmi(&sequence(events)).expect("converts");
        assert!((score.duration_seconds() - 1210.0 / 120.0).abs() < 1e-3);
        assert_eq!(score.messages()[0].message.tempo_us(), Some(600_000), "the composer's tempo stays in the file");
    }

    #[test]
    fn rounding_does_not_drift_over_a_long_song() {
        // 13.333 score ticks an interval at 100 BPM: rounding each gap on its own would drift.
        let mut events = vec![tempo(600_000)];
        events.extend((0..3000).map(|n| note_on(1, 60 + (n % 12) as u8, 1)));
        let score = Score::from_xmi(&sequence(events)).expect("converts");
        let wanted = (3000.0f64 * 960.0 * 1_000_000.0 / (120.0 * 600_000.0)).round() as u32;
        let last_note_on = score.messages().iter().filter(|m| matches!(m.message, Message::NoteOn { .. })).last().expect("notes");
        assert_eq!(last_note_on.at, wanted, "the last note is where real time puts it");
        assert!((score.duration_seconds() - 3001.0 / 120.0).abs() < 1e-3);
    }

    #[test]
    fn the_end_of_track_marker_is_left_to_the_exporter() {
        let end = XmiEvent { interval: 10, status: 0xff, data: vec![META_END_OF_TRACK], duration: None };
        let score = Score::from_xmi(&sequence(vec![note_on(0, 60, 5), end])).expect("converts");
        assert!(!score.messages().iter().any(|m| matches!(&m.message, Message::Meta { meta_type, .. } if *meta_type == META_END_OF_TRACK)));
    }

    #[test]
    fn messages_survive_the_round_trip_through_the_wire_form() {
        let cases = [
            (0x82u8, vec![60u8, 64]), (0x93, vec![60, 100]), (0xa4, vec![60, 20]), (0xb5, vec![7, 90]),
            (0xc6, vec![48]), (0xd7, vec![70]), (0xe8, vec![0x00, 0x40]), (0xff, vec![0x51, 7, 0xa1, 0x20]),
            (0xf0, vec![0x7e, 0xf7]),
        ];
        for (status, data) in cases {
            let message = Message::from_wire(status, &data).expect("reads");
            let mut out = Vec::new();
            message.write_to(&mut out).expect("writes");
            assert_eq!(out[0], status, "{status:#04x} keeps its status byte");
            if status != 0xff && status != 0xf0 {
                assert_eq!(&out[1..], &data[..], "{status:#04x} keeps its data");
            }
        }
        assert_eq!(Message::from_wire(0xb5, &[7, 90]).expect("reads").channel(), Some(5));
        assert_eq!(Message::from_wire(0xff, &[0x2f]).expect("reads").channel(), None);
    }

    #[test]
    fn a_message_that_does_not_carry_its_data_is_refused() {
        assert!(Message::from_wire(0x90, &[60]).is_err());
        assert!(Message::from_wire(0xc0, &[]).is_err());
        assert!(Message::from_wire(0xff, &[]).is_err());
        assert!(Message::from_wire(0xf8, &[]).is_err(), "a realtime byte belongs to a wire, not a file");
        let zero = XmiEvent { interval: 0, status: 0xff, data: vec![META_TEMPO, 0, 0, 0], duration: None };
        assert!(Score::from_xmi(&sequence(vec![zero])).is_err(), "a tempo of nothing");
    }

    #[test]
    fn quantities_are_written_the_way_midi_reads_them() {
        let mut out = Vec::new();
        write_vlq(&mut out, 0).expect("zero");
        write_vlq(&mut out, 127).expect("one byte");
        write_vlq(&mut out, 128).expect("two bytes");
        write_vlq(&mut out, 0x0fff_ffff).expect("largest");
        assert_eq!(out, vec![0x00, 0x7f, 0x81, 0x00, 0xff, 0xff, 0xff, 0x7f]);
        assert!(write_vlq(&mut out, 0x1000_0000).is_err());
    }

    #[test]
    fn channels_are_listed_once_each_in_order() {
        let events = vec![
            XmiEvent { interval: 0, status: 0x99, data: vec![36, 100], duration: Some(4) },
            XmiEvent { interval: 0, status: 0x91, data: vec![60, 100], duration: Some(4) },
            XmiEvent { interval: 0, status: 0xb1, data: vec![7, 90], duration: None },
        ];
        assert_eq!(Score::from_xmi(&sequence(events)).expect("converts").channels(), vec![1, 9]);
    }

    #[test]
    fn a_midi_file_has_the_chunks_and_the_terminator() {
        let messages = vec![
            TimedMessage { at: 0, message: Message::ProgramChange { channel: 0, program: 48 } },
            TimedMessage { at: 0, message: Message::NoteOn { channel: 0, note: 60, velocity: 100 } },
            TimedMessage { at: 960, message: Message::NoteOff { channel: 0, note: 60, velocity: 0 } },
        ];
        let bytes = Midi.to_bytes(&Score::new(960, messages)).expect("encodes");
        assert_eq!(&bytes[..4], b"MThd");
        assert_eq!(&bytes[8..14], &[0, 0, 0, 1, 0x03, 0xc0], "format 0, one track, 960 a quarter");
        assert_eq!(&bytes[14..18], b"MTrk");
        let track = &bytes[22..];
        assert_eq!(u32::from_be_bytes([bytes[18], bytes[19], bytes[20], bytes[21]]) as usize, track.len());
        assert_eq!(&track[..9], &[0x00, 0xc0, 48, 0x00, 0x90, 60, 100, 0x87, 0x40], "the third delta is 960 as two bytes");
        assert_eq!(&track[track.len() - 3..], &[0xff, 0x2f, 0x00], "every track ends with the marker");
        assert_eq!(<Midi as Exporter<Score>>::EXTENSION, "mid");
    }
}
