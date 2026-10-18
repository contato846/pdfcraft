//! Read aloud with the operating system's voices.
//!
//! On Windows this uses Windows' own speech synthesis (`Windows.Media.SpeechSynthesis`) and media
//! player: the voices installed with Windows (Portuguese, English…), a speaking rate, pause and
//! resume, and the word being spoken (from the synthesiser's word boundaries), so the reader can
//! show it. Elsewhere [`Speaker::new`] returns `None`.
//!
//! A [`Speaker`] owns a worker thread that holds the synthesiser and the player; commands go to
//! it through a channel and its state comes back through [`Speaker::status`], so the interface
//! never waits on speech.

use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};

/// A voice the system can speak with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VoiceInfo {
    pub id: String,
    pub name: String,
    /// BCP-47 tag, such as "pt-BR".
    pub language: String,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SpeechState {
    #[default]
    Idle,
    /// Turning the text into speech.
    Preparing,
    Speaking,
    Paused,
    /// The last text was spoken to its end.
    Finished,
}

/// What the speaker is doing.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SpeechStatus {
    pub state: SpeechState,
    /// The word being spoken, as UTF-16 offsets `[start, end)` into the text given to
    /// [`Speaker::speak`].
    pub word: Option<(usize, usize)>,
    /// Which [`Speaker::speak`] call this status is about (they count from 1).
    pub utterance: u64,
    pub error: Option<String>,
}

#[cfg_attr(not(windows), allow(dead_code))]
enum Command {
    Speak { text: String, voice: Option<String>, rate: f64, utterance: u64 },
    Pause,
    Resume,
    Stop,
}

/// A speaking voice and its worker thread (stopped when this is dropped).
pub struct Speaker {
    tx: Sender<Command>,
    status: Arc<Mutex<SpeechStatus>>,
    utterances: u64,
}

/// The slowest and fastest speaking rates (1.0 is the voice's normal pace).
pub const RATES: std::ops::RangeInclusive<f64> = 0.5..=3.0;

impl Speaker {
    /// A speaker, or `None` where the system has no speech this can use.
    pub fn new() -> Option<Speaker> {
        #[cfg(windows)]
        {
            let (tx, rx) = std::sync::mpsc::channel();
            let status = Arc::new(Mutex::new(SpeechStatus::default()));
            let s = status.clone();
            std::thread::Builder::new().name("pdfcraft-speech".into()).spawn(move || windows_speech::run(rx, s)).ok()?;
            Some(Speaker { tx, status, utterances: 0 })
        }
        #[cfg(not(windows))]
        None
    }

    /// The installed voices (empty where there are none).
    pub fn voices() -> Vec<VoiceInfo> {
        #[cfg(windows)]
        return windows_speech::voices();
        #[cfg(not(windows))]
        Vec::new()
    }

    /// Speak `text` (replacing whatever is being spoken) with the voice `voice` (an id from
    /// [`Self::voices`]; the system's default when `None` or unknown) at `rate`.
    pub fn speak(&mut self, text: String, voice: Option<String>, rate: f64) -> u64 {
        self.utterances = self.utterances.saturating_add(1);
        let rate = if rate.is_finite() { rate.clamp(*RATES.start(), *RATES.end()) } else { 1.0 };
        if let Ok(mut s) = self.status.lock() {
            *s = SpeechStatus { state: SpeechState::Preparing, word: None, utterance: self.utterances, error: None };
        }
        let _ = self.tx.send(Command::Speak { text, voice, rate, utterance: self.utterances });
        self.utterances
    }

    pub fn pause(&self) {
        let _ = self.tx.send(Command::Pause);
    }

    pub fn resume(&self) {
        let _ = self.tx.send(Command::Resume);
    }

    pub fn stop(&self) {
        let _ = self.tx.send(Command::Stop);
    }

    pub fn status(&self) -> SpeechStatus {
        self.status.lock().map(|s| s.clone()).unwrap_or_default()
    }
}

#[cfg(windows)]
mod windows_speech {
    use std::sync::mpsc::{Receiver, RecvTimeoutError};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use windows::Media::Core::{MediaSource, SpeechCue};
    use windows::Media::Playback::{MediaPlaybackState, MediaPlayer};
    use windows::Media::SpeechSynthesis::SpeechSynthesizer;
    use windows::core::{HSTRING, Interface};

    use super::{Command, SpeechState, SpeechStatus, VoiceInfo};

    /// How often the worker looks at the player (the highlighted word follows at this pace).
    const TICK: Duration = Duration::from_millis(40);

    pub fn voices() -> Vec<VoiceInfo> {
        let Ok(all) = SpeechSynthesizer::AllVoices() else { return Vec::new() };
        all.into_iter()
            .filter_map(|v| {
                Some(VoiceInfo { id: v.Id().ok()?.to_string(), name: v.DisplayName().ok()?.to_string(), language: v.Language().ok()?.to_string() })
            })
            .collect()
    }

    /// A word boundary: when it starts (100 ns units) and where it is in the text (UTF-16).
    struct Cue {
        at: i64,
        start: usize,
        end: usize,
    }

    struct Worker {
        synth: SpeechSynthesizer,
        player: MediaPlayer,
        cues: Vec<Cue>,
        speaking: bool,
        status: Arc<Mutex<SpeechStatus>>,
    }

    impl Worker {
        fn set(&self, f: impl FnOnce(&mut SpeechStatus)) {
            if let Ok(mut s) = self.status.lock() {
                f(&mut s);
            }
        }

        fn speak(&mut self, text: &str, voice: Option<&str>, rate: f64) -> windows::core::Result<()> {
            self.player.Pause()?;
            self.speaking = false;
            if let Some(id) = voice
                && let Ok(all) = SpeechSynthesizer::AllVoices()
                && let Some(v) = all.into_iter().find(|v| v.Id().is_ok_and(|i| i == id))
            {
                self.synth.SetVoice(&v)?;
            }
            let options = self.synth.Options()?;
            options.SetSpeakingRate(rate)?;
            options.SetIncludeWordBoundaryMetadata(true)?;
            let stream = self.synth.SynthesizeTextToStreamAsync(&HSTRING::from(text))?.join()?;
            self.cues.clear();
            for track in stream.TimedMetadataTracks()? {
                for cue in track.Cues()? {
                    let Ok(cue) = cue.cast::<SpeechCue>() else { continue };
                    let (Ok(at), Ok(start), Ok(end)) = (cue.StartTime(), cue.StartPositionInInput(), cue.EndPositionInInput()) else { continue };
                    let (Ok(start), Ok(end)) = (start.Value(), end.Value()) else { continue };
                    let (Ok(start), Ok(end)) = (usize::try_from(start), usize::try_from(end)) else { continue };
                    // The end position is the word's last character.
                    self.cues.push(Cue { at: at.Duration, start, end: end.saturating_add(1).max(start) });
                }
            }
            self.cues.sort_by_key(|c| c.at);
            let source = MediaSource::CreateFromStream(&stream, &stream.ContentType()?)?;
            self.player.SetSource(&source)?;
            self.player.Play()?;
            self.speaking = true;
            Ok(())
        }

        /// Follow the player: the word being spoken, and the end.
        fn tick(&mut self) {
            if !self.speaking {
                return;
            }
            let Ok(session) = self.player.PlaybackSession() else { return };
            let position = session.Position().map(|t| t.Duration).unwrap_or(0);
            let duration = session.NaturalDuration().map(|t| t.Duration).unwrap_or(0);
            let state = session.PlaybackState().unwrap_or(MediaPlaybackState::None);
            let word = self.cues.iter().rev().find(|c| c.at <= position).map(|c| (c.start, c.end));
            // The player stops itself at the end of the stream.
            let ended = duration > 0 && position >= duration && state != MediaPlaybackState::Playing;
            if ended {
                self.speaking = false;
            }
            self.set(|s| {
                s.word = word;
                if ended {
                    s.state = SpeechState::Finished;
                } else if s.state == SpeechState::Preparing && state == MediaPlaybackState::Playing {
                    s.state = SpeechState::Speaking;
                }
            });
        }
    }

    pub fn run(rx: Receiver<Command>, status: Arc<Mutex<SpeechStatus>>) {
        let fail = |status: &Arc<Mutex<SpeechStatus>>, e: windows::core::Error| {
            if let Ok(mut s) = status.lock() {
                s.state = SpeechState::Idle;
                s.error = Some(e.message());
            }
        };
        let (synth, player) = match (SpeechSynthesizer::new(), MediaPlayer::new()) {
            (Ok(s), Ok(p)) => (s, p),
            (Err(e), _) | (_, Err(e)) => return fail(&status, e),
        };
        let mut w = Worker { synth, player, cues: Vec::new(), speaking: false, status: status.clone() };
        loop {
            match rx.recv_timeout(TICK) {
                Ok(Command::Speak { text, voice, rate, utterance }) => {
                    if let Err(e) = w.speak(&text, voice.as_deref(), rate) {
                        fail(&status, e);
                    } else {
                        w.set(|s| {
                            if s.utterance == utterance {
                                s.state = SpeechState::Speaking;
                            }
                        });
                    }
                }
                Ok(Command::Pause) => {
                    if w.player.Pause().is_ok() {
                        w.set(|s| s.state = SpeechState::Paused);
                    }
                }
                Ok(Command::Resume) => {
                    if w.player.Play().is_ok() {
                        w.set(|s| s.state = SpeechState::Speaking);
                    }
                }
                Ok(Command::Stop) => {
                    let _ = w.player.Pause();
                    w.speaking = false;
                    w.set(|s| {
                        s.state = SpeechState::Idle;
                        s.word = None;
                    });
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
            w.tick();
        }
        let _ = w.player.Pause();
        let _ = w.player.Close();
    }
}
