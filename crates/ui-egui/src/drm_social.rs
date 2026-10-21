//! House tools ▸ DRM Social: a licensed copy of the open document for one person.
//!
//! It is "social" protection: it does not stop a determined person, it makes the copy traceable
//! and states the terms on it. Every page gets the licensee's name diagonally (very faint) and a
//! footer line naming them; each page also carries an invisible code (an invisible text layer)
//! that the document's metadata repeats; a licence page closes the document; and the copy is
//! encrypted (AES-256) with printing allowed and copying, extraction and changes refused.
//!
//! The permissions password is kept beside the saved copy, in `<file>_DRM_social_senha.txt`
//! (only the password, no label and no line break), so the owner can lift the protection later.

use pdfcraft_engine::{AddedText, Changes, Edit, HeaderFooter, Printing, Protection, Watermark};
use pdfcraft_render::DocInfo;

use crate::PdfCraftApp;
use crate::theme::Tokens;
use crate::widgets;

/// The DRM Social dialog.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DrmDraft {
    /// Who the copy is licensed to (required).
    pub name: String,
    /// The document's title, as the licence page names it.
    pub title: String,
    /// Optional: who grants the licence (a publisher, a course, an author).
    pub issuer: String,
    /// Optional: the tracking code; made from the name when empty.
    pub code: String,
    /// DD/MM/YYYY, today by default.
    pub date: String,
    /// The permissions password: 8 letters and digits made when the dialog opens; editable.
    pub password: String,
    /// The colour of the marks (the name across the page, the footer, the licence title): red
    /// unless changed.
    pub color: [f64; 3],
}

/// The marks' colour until changed: a dark red that reads on paper and on screen.
pub const DEFAULT_COLOR: [f64; 3] = [0.78, 0.09, 0.09];

/// The password of a licensed copy waiting to be saved: written beside the file each time the
/// document is saved.
#[derive(Clone, PartialEq)]
pub struct PendingPassword {
    pub doc: pdfcraft_engine::DocId,
    pub password: String,
}

impl std::fmt::Debug for PendingPassword {
    // Never print the password.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PendingPassword").field("doc", &self.doc).field("password", &"…").finish()
    }
}

/// A random permissions password from the system's random source: 8 letters and digits, with
/// at least one of each, from an alphabet without look-alikes (no 0/O, 1/l/I). About 46 bits:
/// enough to stop guessing by hand, and the owner keeps it in the file beside the copy anyway.
pub fn random_password() -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz23456789";
    let mut bytes = [0u8; 8];
    if getrandom::fill(&mut bytes).is_err() {
        // No system randomness: fall back to the clock, which is still unguessable in practice.
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for (i, b) in bytes.iter_mut().enumerate() {
            h = (h ^ (nanos as u64).rotate_left(i as u32 % 64) ^ u64::from(std::process::id())).wrapping_mul(0x0100_0000_01b3);
            *b = (h >> 24) as u8;
        }
    }
    let password: String = bytes.iter().map(|b| char::from(ALPHABET[usize::from(*b) % ALPHABET.len()])).collect();
    // Both kinds, or a mostly-letters password would be weaker than it looks.
    if password.chars().any(|c| c.is_ascii_digit()) && password.chars().any(|c| c.is_ascii_alphabetic()) { password } else { random_password() }
}

/// The tracking code for `name`: `DRM-<FIRST NAME>-<4 digits>`, ASCII only.
pub fn code_for(name: &str) -> String {
    let first: String = name
        .split_whitespace()
        .next()
        .unwrap_or("")
        .chars()
        .filter_map(|c| {
            let c = fold_ascii(c);
            c.is_ascii_alphanumeric().then(|| c.to_ascii_uppercase())
        })
        .take(16)
        .collect();
    let mut n = [0u8; 2];
    let number = if getrandom::fill(&mut n).is_ok() { u16::from_le_bytes(n) % 10_000 } else { 1 };
    if first.is_empty() { format!("DRM-{number:04}") } else { format!("DRM-{first}-{number:04}") }
}

/// The ASCII letter a Latin accented letter is written with ("é" → "e"); other characters as they are.
fn fold_ascii(c: char) -> char {
    match c {
        'á' | 'à' | 'â' | 'ã' | 'ä' | 'å' | 'Á' | 'À' | 'Â' | 'Ã' | 'Ä' | 'Å' => 'a',
        'é' | 'è' | 'ê' | 'ë' | 'É' | 'È' | 'Ê' | 'Ë' => 'e',
        'í' | 'ì' | 'î' | 'ï' | 'Í' | 'Ì' | 'Î' | 'Ï' => 'i',
        'ó' | 'ò' | 'ô' | 'õ' | 'ö' | 'Ó' | 'Ò' | 'Ô' | 'Õ' | 'Ö' => 'o',
        'ú' | 'ù' | 'û' | 'ü' | 'Ú' | 'Ù' | 'Û' | 'Ü' => 'u',
        'ç' | 'Ç' => 'c',
        'ñ' | 'Ñ' => 'n',
        other => other,
    }
}

/// Where the password of a copy saved at `pdf` is kept: `<folder>/<file name without .pdf>_DRM_social_senha.txt`.
pub fn password_file(pdf: &std::path::Path) -> std::path::PathBuf {
    let stem = pdf.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "documento".into());
    pdf.with_file_name(format!("{stem}_DRM_social_senha.txt"))
}

/// `s` in the interface language for the document's marks, or in English when the standard
/// fonts the marks are drawn in can't show that language (Greek, Cyrillic, Chinese…).
fn doc_tl(s: &'static str) -> &'static str {
    let t = tl!(s);
    if pdfcraft_fonts::first_non_win_ansi(t).is_some() { s } else { t }
}

impl DrmDraft {
    /// The footer line on every page.
    pub fn footer(&self) -> String {
        let mut parts = vec![
            crate::i18n::fmt(doc_tl("Licensed to {name}"), &[("name", self.name.trim())]),
            doc_tl("personal, non-transferable use").to_string(),
            crate::i18n::fmt(doc_tl("issued on {date}"), &[("date", self.date.trim())]),
        ];
        if !self.issuer.trim().is_empty() {
            parts.push(self.issuer.trim().to_owned());
        }
        parts.join(" · ")
    }

    /// The licence page's paragraphs.
    pub fn licence(&self) -> Vec<String> {
        let title = format!("“{}”", self.title.trim());
        let first = if self.title.trim().is_empty() {
            if self.issuer.trim().is_empty() {
                crate::i18n::fmt(doc_tl("This document was issued exclusively for {name}."), &[("name", self.name.trim())])
            } else {
                crate::i18n::fmt(
                    doc_tl("This document is material of {issuer}, issued exclusively for {name}."),
                    &[("issuer", self.issuer.trim()), ("name", self.name.trim())],
                )
            }
        } else if self.issuer.trim().is_empty() {
            crate::i18n::fmt(doc_tl("This document, {title}, was issued exclusively for {name}."), &[("title", &title), ("name", self.name.trim())])
        } else {
            crate::i18n::fmt(
                doc_tl("This document, {title}, is material of {issuer}, issued exclusively for {name}."),
                &[("title", &title), ("issuer", self.issuer.trim()), ("name", self.name.trim())],
            )
        };
        vec![
            first,
            doc_tl("The licence is personal and non-transferable. Printing for one's own use is permitted.").to_string(),
            doc_tl("It is forbidden to reproduce, share, distribute, publish or hand this document over to others, in whole or in part, by any means, and to upload it, in whole or in part, to artificial intelligence tools.").to_string(),
            doc_tl("Each copy carries a visible and an invisible identification of the licensee that traces its origin.").to_string(),
            doc_tl("Infringement is subject to the penalties of copyright law, without prejudice to other measures.").to_string(),
            crate::i18n::fmt(doc_tl("Issued on {date}. Code {code}."), &[("date", self.date.trim()), ("code", self.code.trim())]),
        ]
    }

    /// The first character the standard fonts can't draw in the marks (name, footer, licence),
    /// which applying would refuse.
    pub fn first_undrawable(&self) -> Option<char> {
        let mut texts = vec![self.name.clone(), self.footer()];
        texts.extend(self.licence());
        texts.push(doc_tl("Licence").to_string());
        texts.iter().flat_map(|t| t.lines().map(str::to_owned).collect::<Vec<_>>()).find_map(|t| pdfcraft_fonts::first_non_win_ansi(&t))
    }

    /// The single undoable step: a licence page at the end, then on every page the name, the
    /// footer and the invisible code; the metadata; and the protection with `password`.
    pub fn edit(&self, info: &DocInfo, password: &str) -> Option<Edit> {
        let first = info.pages.first()?;
        let (w, h) = (f64::from(first.width.max(72.0)), f64::from(first.height.max(72.0)));
        let n = info.pages.len();
        // An inch of margin on a book page, less on a small one.
        let m = (w.min(h) * 0.12).clamp(18.0, 72.0);
        let mut edits = vec![Edit::InsertBlankPage { at: n, width: w, height: h }];
        let title = AddedText {
            rect: [m, h - m - 40.0, w - m, h - m - 10.0],
            text: doc_tl("Licence").to_string(),
            bold: true,
            size: 16.0,
            color: self.color,
            ..Default::default()
        };
        edits.push(Edit::AddText { page: n, text: title });
        // 10.5 pt, smaller until the text fits between the title and the footer.
        let room = (h - m - 56.0) - (m + 12.0);
        let mut body = AddedText {
            rect: [m, m + 12.0, w - m, h - m - 56.0],
            text: self.licence().join("\n\n"),
            size: 10.5,
            color: [0.15, 0.15, 0.15],
            ..Default::default()
        };
        while body.size > 5.0 {
            let r = pdfcraft_engine::added_text_rect(&body);
            if r[3] - r[1] <= room {
                break;
            }
            body.size -= 0.5;
        }
        edits.push(Edit::AddText { page: n, text: body });
        let pages: Vec<usize> = (0..=n).collect();
        let mark = Watermark {
            text: self.name.trim().to_owned(),
            font_size: 30.0,
            color: self.color,
            opacity: 0.07,
            rotation: 45.0,
            ..Watermark::default()
        };
        edits.push(Edit::AddWatermark { pages: pages.clone(), settings: mark, replace: false, file: None });
        // 6.5 pt, smaller when the line would not fit across the page.
        let line = self.footer();
        let at = |size: f64| pdfcraft_engine::added_text_width(&AddedText { text: line.clone(), size, ..Default::default() });
        let room = (w - 2.0 * m).max(36.0);
        let size = if at(6.5) > room { (6.5 * room / at(6.5).max(1.0)).max(3.0) } else { 6.5 };
        let mut footer = HeaderFooter { font_size: size, color: self.color, margins: [m, (m / 2.0).max(14.0), m, m], ..HeaderFooter::default() };
        footer.text[4] = line;
        edits.push(Edit::AddHeaderFooter { pages, settings: footer, replace: false });
        // The invisible code, in each page's own user space (the new page has the first page's size).
        let hidden = format!("{} {}", self.code.trim(), self.date.trim());
        for page in 0..=n {
            let (x0, y0, ph) = info.pages.get(page).map_or((0.0, 0.0, h), |p| {
                (f64::from(p.crop[0].min(p.crop[2])), f64::from(p.crop[1].min(p.crop[3])), f64::from((p.crop[3] - p.crop[1]).abs()))
            });
            let words = vec![pdfcraft_engine::ocr::PlacedWord {
                text: hidden.clone(),
                origin: [x0 + 20.0, y0 + ph / 2.0],
                across: [hidden.chars().count() as f64 * 2.0, 0.0],
                up: [0.0, 4.0],
            }];
            edits.push(Edit::AddOcrText { page, words });
        }
        if !self.title.trim().is_empty() {
            edits.push(Edit::SetInfo { key: "Title".into(), value: self.title.trim().to_owned() });
        }
        if !self.issuer.trim().is_empty() {
            edits.push(Edit::SetInfo { key: "Author".into(), value: self.issuer.trim().to_owned() });
        }
        edits.push(Edit::SetInfo { key: "Subject".into(), value: crate::i18n::fmt(doc_tl("Licensed to {name}"), &[("name", self.name.trim())]) });
        edits.push(Edit::SetInfo { key: "Keywords".into(), value: self.code.trim().to_owned() });
        edits.push(Edit::Protect(Protection {
            permissions_password: Some(password.to_owned()),
            printing: Printing::High,
            changes: Changes::None,
            copy: false,
            accessibility: true,
            ..Protection::default()
        }));
        Some(Edit::Batch { label: "DRM Social".into(), edits })
    }
}

impl PdfCraftApp {
    pub fn open_drm_social(&mut self) {
        let Some((_, id)) = self.active_ids() else { return };
        let (y, m, d) = self.session.today();
        let title = self.session.get(id).and_then(|doc| doc.info_value("Title")).unwrap_or_default();
        self.drm_draft =
            DrmDraft { date: format!("{d:02}/{m:02}/{y}"), title, password: random_password(), color: DEFAULT_COLOR, ..DrmDraft::default() };
        self.dialog = Some(crate::Dialog::DrmSocial);
    }

    /// The dialog's Create: apply the protections as one step, then ask where to save the copy;
    /// the password goes beside it.
    pub(crate) fn apply_drm_social(&mut self) {
        let Some((index, id)) = self.active_ids() else { return };
        if self.drm_draft.code.trim().is_empty() {
            self.drm_draft.code = code_for(&self.drm_draft.name);
        }
        let password = if self.drm_draft.password.trim().is_empty() { random_password() } else { self.drm_draft.password.trim().to_owned() };
        let Some(edit) = self.session.get(id).and_then(|doc| self.drm_draft.edit(&doc.info, &password)) else { return };
        if !self.apply_edit(edit) {
            return;
        }
        self.drm_password = Some(PendingPassword { doc: id, password });
        self.save_view(index, crate::editing::SaveTarget::As);
    }

    /// After document `id` was written to `dest`: keep the password of its licensed copy beside it.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn keep_drm_password(&mut self, id: pdfcraft_engine::DocId, dest: &str) {
        let Some(pending) = self.drm_password.as_ref().filter(|p| p.doc == id) else { return };
        let file = password_file(std::path::Path::new(dest));
        match std::fs::write(&file, pending.password.as_bytes()) {
            Ok(()) => {
                let name = file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                self.notify_fmt("The password is in {file}, beside the copy.", &[("file", &name)]);
            }
            Err(e) => self.notify_fmt("Couldn't write the password file: {e}", &[("e", &e.to_string())]),
        }
    }
}

/// Returns (create, cancel).
pub(crate) fn drm_body(ui: &mut egui::Ui, d: &mut DrmDraft, t: &Tokens) -> (bool, bool) {
    ui.set_width(560.0);
    ui.label(egui::RichText::new(tl!("DRM Social")).font(crate::theme::semibold(18.0)));
    ui.add_space(4.0);
    ui.label(
        egui::RichText::new(tl!(
            "A licensed copy for one person: their name on every page, an invisible code, a licence page at the end, and protection that allows printing but not copying or changes."
        ))
        .color(t.text_muted),
    );
    ui.add_space(10.0);
    egui::Grid::new("drm-social").num_columns(2).spacing([18.0, 8.0]).min_col_width(130.0).show(ui, |ui| {
        let rows: [(&str, &mut String, &str, bool); 5] = [
            ("Licensed to", &mut d.name, "name", false),
            ("Title", &mut d.title, "title", false),
            ("Issued by", &mut d.issuer, "issuer", true),
            ("Code", &mut d.code, "code", true),
            ("Date", &mut d.date, "date", false),
        ];
        for (label, value, salt, optional) in rows {
            let l = ui.label(egui::RichText::new(tl!(label)).color(t.text_muted));
            let mut line = widgets::line(value).desired_width(380.0).background_color(t.field).id_salt(("drm", salt));
            if optional {
                line = line.hint_text(tl!("Optional"));
            }
            ui.add(line).labelled_by(l.id);
            ui.end_row();
        }
        let l = ui.label(egui::RichText::new(tl!("Permissions password")).color(t.text_muted));
        ui.horizontal(|ui| {
            ui.add(widgets::line(&mut d.password).desired_width(300.0).background_color(t.field).id_salt(("drm", "password"))).labelled_by(l.id);
            if ui.button(tl!("New password")).clicked() {
                d.password = random_password();
            }
        });
        ui.end_row();
        let l = ui.label(egui::RichText::new(tl!("Colour")).color(t.text_muted));
        ui.horizontal(|ui| {
            crate::marks_ui::color_button(ui, &mut d.color).labelled_by(l.id);
            if d.color != DEFAULT_COLOR && ui.button(tl!("Red")).clicked() {
                d.color = DEFAULT_COLOR;
            }
        });
        ui.end_row();
    });
    ui.add_space(8.0);
    ui.label(egui::RichText::new(d.footer()).small().color(t.text_muted));
    ui.add_space(6.0);
    ui.label(
        egui::RichText::new(tl!("You choose where to save the copy; the password is kept beside it, in a file ending in _DRM_social_senha.txt."))
            .small()
            .color(t.text_faint),
    );
    let undrawable = d.first_undrawable();
    if let Some(c) = undrawable {
        ui.add_space(4.0);
        ui.label(
            egui::RichText::new(crate::i18n::fmt(tl!("The character {c} can't be drawn in the marks."), &[("c", &c.to_string())]))
                .small()
                .color(egui::Color32::from_rgb(0xC6, 0x28, 0x28)),
        );
    }
    ui.add_space(12.0);
    let (mut create, mut cancel) = (false, false);
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        let ready = !d.name.trim().is_empty() && undrawable.is_none();
        if ui.add_enabled_ui(ready, |ui| widgets::pill_button(ui, tl!("Create licensed copy…"), true)).inner.clicked() {
            create = true;
        }
        if widgets::pill_button(ui, tl!("Cancel"), false).clicked() {
            cancel = true;
        }
    });
    (create, cancel)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_password_file_sits_beside_the_copy() {
        let p = password_file(std::path::Path::new("/tmp/livros/Manual - cópia.pdf"));
        assert_eq!(p, std::path::Path::new("/tmp/livros/Manual - cópia_DRM_social_senha.txt"));
    }

    #[test]
    fn codes_are_ascii_and_passwords_are_long() {
        let code = code_for("Ângela Souza");
        assert!(code.starts_with("DRM-ANGELA-") && code.len() == "DRM-ANGELA-0000".len(), "{code}");
        assert!(code_for("").starts_with("DRM-"));
        let (a, b) = (random_password(), random_password());
        assert_eq!(a.chars().count(), 8);
        assert!(a.chars().all(|c| c.is_ascii_alphanumeric() && !"0O1lI".contains(c)), "{a}");
        assert!(a.chars().any(|c| c.is_ascii_digit()) && a.chars().any(|c| c.is_ascii_alphabetic()), "{a}");
        assert_ne!(a, b);
    }
}
