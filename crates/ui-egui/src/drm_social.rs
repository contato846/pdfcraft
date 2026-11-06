//! House tools ▸ DRM Social: a licensed copy of the open document for one person.
//!
//! It is "social" protection: it does not stop a determined person, it makes the copy traceable
//! and states the terms on it. Every page gets, as chosen, the licensee's name diagonally (very
//! faint), a footer line naming them and the same line as a header; the lines sit on the first
//! blank band from the page edge, clear of a letterhead's bar or logo. Each page also carries an
//! invisible code (an invisible text layer) that the document's metadata repeats; a licence page
//! closes the document; and the copy is encrypted (AES-256), copying, extraction and changes
//! refused, printing allowed unless unticked.
//!
//! The permissions password is kept beside the saved copy, in `<file>_DRM_social_senha.txt`
//! (only the password, no label and no line break), so the owner can lift the protection later.

use pdfcraft_engine::{AddedText, Changes, Edit, HeaderFooter, Printing, Protection, Watermark};
use pdfcraft_render::DocInfo;

use crate::PdfCraftApp;
use crate::theme::Tokens;
use crate::widgets;

/// The DRM Social dialog.
#[derive(Clone, Debug, PartialEq)]
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
    /// Where the licensee's line goes: the top of each page, the bottom, and the name across the
    /// page, in any combination (at least one). Footer and diagonal unless changed.
    pub in_header: bool,
    pub in_footer: bool,
    pub diagonal: bool,
    /// The protection lets the copy be printed (on unless unticked).
    pub allow_printing: bool,
    /// The distance of the header line from the top edge and of the footer line from the bottom
    /// edge, in points, measured from the pages ([`clear_offset`]) before applying; `None` uses
    /// the default distance.
    pub header_top: Option<f64>,
    pub footer_bottom: Option<f64>,
}

impl Default for DrmDraft {
    fn default() -> Self {
        Self {
            name: String::new(),
            title: String::new(),
            issuer: String::new(),
            code: String::new(),
            date: String::new(),
            password: String::new(),
            color: DEFAULT_COLOR,
            in_header: false,
            in_footer: true,
            diagonal: true,
            allow_printing: true,
            header_top: None,
            footer_bottom: None,
        }
    }
}

/// A rendered page: premultiplied RGBA, `width` × `height` pixels, `scale` pixels per point.
#[derive(Clone, Copy)]
pub struct Raster<'a> {
    pub rgba: &'a [u8],
    pub width: usize,
    pub height: usize,
    pub scale: f64,
}

/// How far from a page edge (in points) a line of `height` points can sit on blank paper: the
/// first offset from `start` up to `limit` where the band it takes, with a little air around it,
/// is white in the page. Measured from the bottom edge, or the top one when `from_top`. `start`
/// when nothing in reach is blank, so a full-bleed page keeps the default position.
pub fn clear_offset(page: Raster<'_>, from_top: bool, start: f64, height: f64, limit: f64) -> f64 {
    let Raster { rgba, width, height: height_px, scale } = page;
    // About a millimetre of paper between the line and whatever is drawn next to it.
    const AIR: f64 = 3.0;
    let blank_row = |y: usize| {
        rgba.get(y.saturating_mul(width).saturating_mul(4)..(y + 1).saturating_mul(width).saturating_mul(4)).is_none_or(|row| {
            row.chunks_exact(4).all(|p| {
                let a = p.get(3).copied().unwrap_or(0);
                // Premultiplied: transparent is the paper; otherwise every channel near full.
                a < 16 || p.iter().take(3).all(|c| u32::from(*c) * 255 >= 236 * u32::from(a))
            })
        })
    };
    let rows = |off: f64| {
        let (near, far) = ((off - AIR).max(0.0), off + height + AIR);
        let (a, b) = ((near * scale).floor() as usize, (far * scale).ceil() as usize);
        (a..b.min(height_px)).map(move |r| if from_top { r } else { height_px.saturating_sub(1).saturating_sub(r) })
    };
    if !(scale.is_finite() && scale > 0.0) || width == 0 || height_px == 0 {
        return start;
    }
    let mut off = start;
    while off <= limit {
        if rows(off).all(blank_row) {
            return off;
        }
        off += 0.5;
    }
    start
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
            if self.allow_printing {
                doc_tl("The licence is personal and non-transferable. Printing for one's own use is permitted.").to_string()
            } else {
                doc_tl("The licence is personal and non-transferable. Printing is not permitted.").to_string()
            },
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
        if self.diagonal {
            edits.push(Edit::AddWatermark { pages: pages.clone(), settings: mark, replace: false, file: None });
        }
        // 6.5 pt, smaller when the line would not fit across the page.
        let line = self.footer();
        let at = |size: f64| pdfcraft_engine::added_text_width(&AddedText { text: line.clone(), size, ..Default::default() });
        let room = (w - 2.0 * m).max(36.0);
        let size = if at(6.5) > room { (6.5 * room / at(6.5).max(1.0)).max(3.0) } else { 6.5 };
        let edge = (m / 2.0).max(14.0);
        let margins = [self.header_top.unwrap_or(edge), self.footer_bottom.unwrap_or(edge), m, m];
        let mut lines = HeaderFooter { font_size: size, color: self.color, margins, ..HeaderFooter::default() };
        if self.in_header {
            lines.text[1] = line.clone();
        }
        if self.in_footer {
            lines.text[4] = line;
        }
        if self.in_header || self.in_footer {
            edits.push(Edit::AddHeaderFooter { pages, settings: lines, replace: false });
        }
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
                confidence: None,
                source: "drm-social".into(),
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
            printing: if self.allow_printing { Printing::High } else { Printing::None },
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
        self.drm_draft = DrmDraft { date: format!("{d:02}/{m:02}/{y}"), title, password: random_password(), ..DrmDraft::default() };
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
        self.place_drm_lines(id);
        let Some(edit) = self.session.get(id).and_then(|doc| self.drm_draft.edit(&doc.info, &password)) else { return };
        if !self.apply_edit(edit) {
            return;
        }
        self.drm_password = Some(PendingPassword { doc: id, password });
        self.save_view(index, crate::editing::SaveTarget::As);
    }

    /// Where the header and footer lines go: the first blank band from each edge, so they don't
    /// run into a letterhead's bar or logo. Measured on the first pages (a template repeats).
    fn place_drm_lines(&mut self, id: pdfcraft_engine::DocId) {
        let d = &mut self.drm_draft;
        (d.header_top, d.footer_bottom) = (None, None);
        let Some(doc) = self.session.get(id) else { return };
        let Some(first) = doc.info.pages.first() else { return };
        let (w, h) = (f64::from(first.width.max(72.0)), f64::from(first.height.max(72.0)));
        let m = (w.min(h) * 0.12).clamp(18.0, 72.0);
        let edge = (m / 2.0).max(14.0);
        // The header and footer are placed by their baseline; the band the line takes reaches a
        // descender below it and a little over the cap height above it, at the usual 6.5 pt.
        let descent = 6.5 * 0.25;
        let line = 6.5 * 1.3;
        let limit = h * 0.22;
        let config = pdfcraft_render::RenderConfig { password: doc.password.as_deref().map(std::sync::Arc::from), ..Default::default() };
        let mut renderer = pdfcraft_render::PageRenderer::new(doc.display.clone(), config);
        let scale = 2.0_f32;
        let (mut top, mut bottom) = (edge, edge);
        for page in 0..doc.info.pages.len().min(3) {
            let out = renderer.render(pdfcraft_render::RenderRequest { page, scale, ..Default::default() });
            if out.error.is_some() {
                continue;
            }
            let (pw, ph) = (out.width as usize, out.height as usize);
            let s = f64::from(scale);
            if d.in_header {
                top = top.max(clear_offset(Raster { rgba: &out.rgba, width: pw, height: ph, scale: s }, true, edge, line, limit));
            }
            if d.in_footer {
                bottom = bottom
                    .max(clear_offset(Raster { rgba: &out.rgba, width: pw, height: ph, scale: s }, false, edge - descent, line, limit) + descent);
            }
        }
        (d.header_top, d.footer_bottom) = (Some(top), Some(bottom));
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
            "A licensed copy for one person: their name on the pages, an invisible code, a licence page at the end, and protection against copying and changes."
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
        ui.label(egui::RichText::new(tl!("Marks")).color(t.text_muted));
        ui.horizontal(|ui| {
            ui.add(widgets::checkbox(&mut d.in_header, tl!("Header line")));
            ui.add(widgets::checkbox(&mut d.in_footer, tl!("Footer line")));
            ui.add(widgets::checkbox(&mut d.diagonal, tl!("Name across the page")));
        });
        ui.end_row();
        ui.label("");
        ui.add(widgets::checkbox(&mut d.allow_printing, tl!("Allow printing")));
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
    let marked = d.in_header || d.in_footer || d.diagonal;
    if !marked {
        ui.add_space(4.0);
        ui.label(egui::RichText::new(tl!("Choose at least one mark.")).small().color(egui::Color32::from_rgb(0xC6, 0x28, 0x28)));
    }
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
        let ready = !d.name.trim().is_empty() && undrawable.is_none() && marked;
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

    /// A white page, `w` × `h` points at 1 px per point, with a dark band from `from` to `to`
    /// points above the bottom edge (a letterhead's bar).
    fn page_with_bar(w: usize, h: usize, from: usize, to: usize) -> Vec<u8> {
        let mut px = vec![255u8; w * h * 4];
        for y in (h - to)..(h - from) {
            for x in 10..w - 10 {
                let i = (y * w + x) * 4;
                px[i..i + 3].copy_from_slice(&[70, 120, 200]);
            }
        }
        px
    }

    #[test]
    fn the_footer_line_rises_just_clear_of_a_bar_at_the_foot_of_the_page() {
        let (w, h) = (120, 300);
        let line = 6.5 * 1.3;
        // A bar from 20 to 50 pt: the line's band (with 3 pt of air) starts above it.
        let px = page_with_bar(w, h, 20, 50);
        let off = clear_offset(Raster { rgba: &px, width: w, height: h, scale: 1.0 }, false, 18.0, line, 66.0);
        assert!((53.0..=53.5).contains(&off), "{off}");
        // The same bar at the top, read from the top edge.
        let mut flipped = vec![0u8; px.len()];
        for y in 0..h {
            flipped[y * w * 4..(y + 1) * w * 4].copy_from_slice(&px[(h - 1 - y) * w * 4..(h - y) * w * 4]);
        }
        assert!((53.0..=53.5).contains(&clear_offset(Raster { rgba: &flipped, width: w, height: h, scale: 1.0 }, true, 18.0, line, 66.0)));
        // A blank foot keeps the usual place; a bar out of reach does too.
        assert_eq!(clear_offset(Raster { rgba: &vec![255u8; w * h * 4], width: w, height: h, scale: 1.0 }, false, 18.0, line, 66.0), 18.0);
        assert_eq!(clear_offset(Raster { rgba: &page_with_bar(w, h, 0, 200), width: w, height: h, scale: 1.0 }, false, 18.0, line, 66.0), 18.0);
        // Transparent paper counts as blank; nothing to read gives the start.
        assert_eq!(clear_offset(Raster { rgba: &vec![0u8; w * h * 4], width: w, height: h, scale: 1.0 }, false, 18.0, line, 66.0), 18.0);
        assert_eq!(clear_offset(Raster { rgba: &[], width: 0, height: 0, scale: 1.0 }, false, 18.0, line, 66.0), 18.0);
        assert_eq!(clear_offset(Raster { rgba: &px, width: w, height: h, scale: f64::NAN }, false, 18.0, line, 66.0), 18.0);
    }

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
