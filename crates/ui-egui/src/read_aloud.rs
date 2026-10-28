//! View ▸ Read aloud: the document read from the current page to the end with the system's
//! voices (Windows), page after page, the spoken word highlighted and the view following it.
//!
//! A page's text comes from its text layer (scanned pages need Recognize text first). It is read
//! as running prose: line breaks become spaces and a word hyphenated across a line break is joined,
//! so the voice doesn't pause or spell the halves. Each character of what is spoken remembers its
//! glyph, which is how the spoken word (UTF-16 offsets from the voice) finds its place on the page.

use pdfcraft_platform::speech::{SpeechState, VoiceInfo};
use pdfcraft_render::PageText;

use crate::PdfCraftApp;
use crate::theme::Tokens;
use crate::widgets;

/// What a page's text is spoken as, and for each UTF-16 unit of it the glyph it came from.
pub fn spoken_text(text: &PageText) -> (String, Vec<usize>) {
    let mut out = String::new();
    let mut units: Vec<usize> = Vec::new();
    let push = |out: &mut String, units: &mut Vec<usize>, c: char, glyph: usize| {
        out.push(c);
        units.extend(std::iter::repeat_n(glyph, c.len_utf16()));
    };
    let n = text.glyphs.len();
    for i in 0..n {
        let Some(g) = text.glyphs.get(i) else { break };
        let line = text.line_of.get(i).copied();
        let prev_line = i.checked_sub(1).and_then(|p| text.line_of.get(p).copied());
        if i > 0 {
            if prev_line != line {
                // A word hyphenated at the end of the line continues on the next: drop the hyphen
                // (when the next line goes on in lower case) instead of adding a space.
                let joined = out.ends_with('-') && g.text.chars().next().is_some_and(char::is_lowercase);
                if joined {
                    out.pop();
                    units.pop();
                } else if !out.ends_with(' ') {
                    push(&mut out, &mut units, ' ', i);
                }
            } else if text.space_before.get(i).copied().unwrap_or(false) && !out.ends_with(' ') {
                push(&mut out, &mut units, ' ', i);
            }
        }
        for c in g.text.chars() {
            push(&mut out, &mut units, c, i);
        }
    }
    (out, units)
}

/// The glyphs of the spoken word `[start, end)` (UTF-16 offsets into the spoken text).
pub fn word_glyphs(units: &[usize], start: usize, end: usize) -> Option<std::ops::Range<usize>> {
    let first = *units.get(start)?;
    let last = *units.get(end.saturating_sub(1).max(start))?;
    (last >= first).then_some(first..last + 1)
}

/// A reading in progress.
pub struct Reading {
    /// The document being read (by identity: tabs move when others close).
    pub doc: pdfcraft_engine::DocId,
    pub page: usize,
    pub(crate) units: Vec<usize>,
    pub(crate) utterance: u64,
    pub(crate) speaker: pdfcraft_platform::speech::Speaker,
    /// Waiting for the page's text layer.
    pub(crate) waiting: bool,
}

/// Read aloud settings, kept for the session.
#[derive(Clone, Debug, PartialEq)]
pub struct ReadAloudPrefs {
    /// A voice id; `None`: the system's default voice.
    pub voice: Option<String>,
    pub rate: f64,
}

impl Default for ReadAloudPrefs {
    fn default() -> Self {
        Self { voice: None, rate: 1.0 }
    }
}

impl PdfCraftApp {
    /// Start reading the active document at its current page (or stop, if it is being read).
    pub fn toggle_read_aloud(&mut self) {
        if self.reading.is_some() {
            self.stop_read_aloud();
            return;
        }
        let Some(view) = self.active else { return };
        let Some((doc, page)) = self.views.get(view).map(|v| (v.id, v.current)) else { return };
        let Some(speaker) = pdfcraft_platform::speech::Speaker::new() else {
            self.notify_tr("Read aloud uses the voices of Windows.");
            return;
        };
        if self.read_prefs.voice.is_none() {
            self.read_prefs.voice = default_voice(&pdfcraft_platform::speech::Speaker::voices(), crate::i18n::current().code());
        }
        self.reading = Some(Reading { doc, page, units: Vec::new(), utterance: 0, speaker, waiting: true });
        if let Some(v) = self.views.get_mut(view) {
            v.read_page = Some(page);
        }
    }

    pub fn stop_read_aloud(&mut self) {
        if let Some(r) = self.reading.take() {
            r.speaker.stop();
            if let Some(v) = self.views.iter_mut().find(|v| v.id == r.doc) {
                v.read_page = None;
                v.read_word = None;
            }
        }
    }

    pub fn pause_read_aloud(&mut self) {
        if let Some(r) = &self.reading {
            match r.speaker.status().state {
                SpeechState::Paused => r.speaker.resume(),
                SpeechState::Speaking | SpeechState::Preparing => r.speaker.pause(),
                _ => {}
            }
        }
    }

    /// Each frame: start the next page when one is spoken, follow the spoken word.
    pub(crate) fn poll_read_aloud(&mut self, ctx: &egui::Context) {
        let Some(r) = self.reading.as_mut() else { return };
        // The document was closed, or another one took its place.
        let Some(v) = self.views.iter_mut().find(|v| v.id == r.doc) else {
            self.stop_read_aloud();
            return;
        };
        ctx.request_repaint_after(std::time::Duration::from_millis(60));
        let pages = v.page_count();
        if r.waiting {
            let Some(text) = v.page_text(r.page) else { return };
            let (spoken, units) = spoken_text(&text);
            if spoken.trim().is_empty() {
                // A page without text (a picture, a blank page): on to the next.
                r.page += 1;
                if r.page >= pages {
                    self.stop_read_aloud();
                    self.notify_tr("Finished reading aloud.");
                    return;
                }
                v.read_page = Some(r.page);
                return;
            }
            r.units = units;
            r.utterance = r.speaker.speak(spoken, self.read_prefs.voice.clone(), self.read_prefs.rate);
            r.waiting = false;
            if v.current != r.page {
                v.go_to_page(r.page);
            }
            return;
        }
        let status = r.speaker.status();
        if status.utterance != r.utterance {
            return;
        }
        if let Some(e) = status.error {
            self.stop_read_aloud();
            self.notify(e);
            return;
        }
        v.read_word = status.word.and_then(|(s, e)| word_glyphs(&r.units, s, e)).map(|g| (r.page, g));
        if status.state == SpeechState::Finished {
            r.page += 1;
            if r.page >= pages {
                self.stop_read_aloud();
                self.notify_tr("Finished reading aloud.");
                return;
            }
            r.waiting = true;
            v.read_page = Some(r.page);
            v.read_word = None;
        }
    }
}

/// The voice for `ui_language` (the interface's language code), or the first voice.
pub fn default_voice(voices: &[VoiceInfo], ui_language: &str) -> Option<String> {
    let base = |l: &str| l.split(['-', '_']).next().unwrap_or("").to_ascii_lowercase();
    voices
        .iter()
        .find(|v| v.language.eq_ignore_ascii_case(ui_language))
        .or_else(|| voices.iter().find(|v| base(&v.language) == base(ui_language)))
        .or(voices.first())
        .map(|v| v.id.clone())
}

/// The bar shown while reading: pause or resume, stop, voice and speed.
pub(crate) fn bar(ctx: &egui::Context, app: &mut PdfCraftApp) {
    let Some(r) = &app.reading else { return };
    let t = Tokens::get(ctx);
    let state = r.speaker.status().state;
    let page = r.page;
    let (mut pause, mut stop) = (false, false);
    let voices = pdfcraft_platform::speech::Speaker::voices();
    egui::Area::new(egui::Id::unique("read-aloud-bar"))
        .order(egui::Order::Foreground)
        .anchor(egui::Align2::CENTER_BOTTOM, egui::vec2(0.0, -24.0))
        .show(ctx, |ui| {
            egui::Frame::popup(ui.style()).corner_radius(egui::CornerRadius::same(8)).inner_margin(egui::Margin::symmetric(12, 8)).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(crate::i18n::fmt(tl!("Reading page {n}"), &[("n", &(page + 1).to_string())])).color(t.text));
                    ui.add_space(8.0);
                    let label = if state == SpeechState::Paused { tl!("Resume") } else { tl!("Pause") };
                    if widgets::pill_button(ui, label, true).clicked() {
                        pause = true;
                    }
                    if widgets::pill_button(ui, tl!("Stop"), false).clicked() {
                        stop = true;
                    }
                    ui.add_space(8.0);
                    let current = app
                        .read_prefs
                        .voice
                        .as_deref()
                        .and_then(|id| voices.iter().find(|v| v.id == id))
                        .map_or_else(|| tl!("Default voice").to_string(), |v| v.name.clone());
                    craft_ui::ComboBox::from_id_salt("read-aloud-voice").selected_text(current).width(220.0).show_ui(ui, |ui| {
                        for v in &voices {
                            ui.selectable_value(&mut app.read_prefs.voice, Some(v.id.clone()), format!("{} ({})", v.name, v.language));
                        }
                    });
                    ui.label(tl!("Speed"));
                    ui.add(egui::Slider::new(&mut app.read_prefs.rate, pdfcraft_platform::speech::RATES).step_by(0.1).suffix("×"));
                });
            });
        });
    if pause {
        app.pause_read_aloud();
    }
    if stop {
        app.stop_read_aloud();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pdfcraft_render::TextGlyph;

    fn page(lines: &[&str]) -> PageText {
        let mut t = PageText::default();
        for (l, line) in lines.iter().enumerate() {
            for (w, word) in line.split(' ').enumerate() {
                for (k, c) in word.chars().enumerate() {
                    t.glyphs.push(TextGlyph { text: c.to_string(), rect: [0.0; 4], quad: [[0.0; 2]; 4], direction: [1.0, 0.0] });
                    t.line_of.push(l as u32);
                    t.space_before.push(w > 0 && k == 0);
                }
            }
        }
        t
    }

    #[test]
    fn lines_run_on_and_hyphenated_words_are_joined() {
        let (s, units) = spoken_text(&page(&["O processo consti-", "tucional é", "Democrático"]));
        assert_eq!(s, "O processo constitucional é Democrático");
        assert_eq!(units.len(), s.encode_utf16().count(), "one glyph per UTF-16 unit");
        // "tucional" starts at the glyph after the dropped hyphen.
        let at = s.find("tucional").unwrap();
        let glyph = units[s[..at].encode_utf16().count()];
        assert_eq!(page(&["O processo consti-", "tucional é", "Democrático"]).glyphs[glyph].text, "t");
        // A capital after a line-ending hyphen is not a broken word.
        assert_eq!(spoken_text(&page(&["Rio-", "Grande"])).0, "Rio- Grande");
    }

    #[test]
    fn a_spoken_word_finds_its_glyphs() {
        let t = page(&["Eficiência e processo"]);
        let (s, units) = spoken_text(&t);
        let start = s.encode_utf16().take_while(|_| true).count() - "processo".encode_utf16().count();
        let g = word_glyphs(&units, start, start + "processo".encode_utf16().count()).unwrap();
        let word: String = t.glyphs[g].iter().map(|g| g.text.as_str()).collect();
        assert_eq!(word, "processo");
        assert!(word_glyphs(&units, 999, 1000).is_none(), "out of range");
        assert!(word_glyphs(&[], 0, 0).is_none());
    }

    #[test]
    fn the_default_voice_follows_the_interface_language() {
        let v = |id: &str, language: &str| VoiceInfo { id: id.into(), name: id.into(), language: language.into() };
        let voices = [v("david", "en-US"), v("maria", "pt-BR"), v("helia", "pt-PT")];
        assert_eq!(default_voice(&voices, "pt-BR").as_deref(), Some("maria"));
        assert_eq!(default_voice(&voices, "pt-br").as_deref(), Some("maria"));
        assert_eq!(default_voice(&voices, "en").as_deref(), Some("david"));
        assert_eq!(default_voice(&voices, "ja").as_deref(), Some("david"), "the first voice otherwise");
        assert_eq!(default_voice(&[], "pt-BR"), None);
    }
}
