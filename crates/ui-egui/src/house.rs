//! House tools of this personal build: the Editora Conjecturas and IPBB routines, gathered in the
//! "House tools" group so they are one click from the mode bar.
//!
//! Finish Conjecturas file sets the house's document information and removes what the layout
//! applications left in the file, in one undoable step: InDesign's XMP history and document IDs,
//! Photoshop's image XMP, and the private data (`/PieceInfo`) of InDesign and Illustrator.
//!
//! Proof for the author marks every page "PROOF – name – date" and protects the copy against
//! changes and copying (commenting and printing stay allowed, so the author can review it), in one
//! undoable step. The original is untouched until it is saved: the dialog says to Save As.
//!
//! Initial every page puts the saved initials at the same place on each page (a corner or the
//! right margin, a set distance from the edge, as the page is displayed), in one undoable step,
//! and can then take the user to the last page with the signature tool ready.

use pdfcraft_engine::{Changes, Edit, Hidden, Navigation, Protection, Shape, Watermark};
use pdfcraft_render::PageInfo;

use crate::theme::Tokens;
use crate::{PdfCraftApp, widgets};

/// The application name the house writes into every finished file.
pub const HOUSE_CREATOR: &str = "Editora Conjecturas";

/// The Finish Conjecturas file dialog.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FinishDraft {
    pub title: String,
    pub author: String,
    pub creator: String,
    pub producer: String,
    /// Kept as they are, but carried here so removing the metadata doesn't lose them.
    pub subject: String,
    pub keywords: String,
    /// Remove every XMP packet (InDesign history, document IDs, image metadata).
    pub strip_xmp: bool,
    /// Remove the private data of other applications.
    pub strip_private: bool,
    /// Open with the bookmarks panel (offered when the file has bookmarks).
    pub show_bookmarks: bool,
    pub has_bookmarks: bool,
}

impl PdfCraftApp {
    pub fn open_finish_conjecturas(&mut self) {
        let Some((_, id)) = self.active_ids() else { return };
        let Some(doc) = self.session.get(id) else { return };
        let info = |k: &str| doc.info_value(k).unwrap_or_default();
        let found = doc.hidden_info();
        let has_bookmarks = found.iter().any(|(h, n)| *h == Hidden::Bookmarks && *n > 0);
        self.finish_draft = FinishDraft {
            title: info("Title"),
            author: info("Author"),
            creator: HOUSE_CREATOR.to_owned(),
            producer: String::new(),
            subject: info("Subject"),
            keywords: info("Keywords"),
            strip_xmp: true,
            strip_private: true,
            show_bookmarks: has_bookmarks,
            has_bookmarks,
        };
        self.dialog = Some(crate::Dialog::FinishConjecturas);
    }

    /// The single undoable step the dialog's Apply makes, or `None` when there is no document.
    pub(crate) fn finish_conjecturas_edit(&self) -> Option<Edit> {
        let (_, id) = self.active_ids()?;
        let doc = self.session.get(id)?;
        let d = &self.finish_draft;
        let found = doc.hidden_info();
        let any = |h: Hidden| found.iter().any(|(k, n)| *k == h && *n > 0);
        let mut edits = Vec::new();
        // Removing the metadata also removes the document information; it is written again below.
        let which: Vec<Hidden> = [(d.strip_xmp, Hidden::Metadata), (d.strip_private, Hidden::PrivateData)]
            .into_iter()
            .filter(|(on, h)| *on && any(*h))
            .map(|(_, h)| h)
            .collect();
        if !which.is_empty() {
            edits.push(Edit::RemoveHidden { which });
        }
        for (key, value) in [
            ("Title", &d.title),
            ("Author", &d.author),
            ("Subject", &d.subject),
            ("Keywords", &d.keywords),
            ("Creator", &d.creator),
            ("Producer", &d.producer),
        ] {
            edits.push(Edit::SetInfo { key: key.to_owned(), value: value.trim().to_owned() });
        }
        if d.has_bookmarks && d.show_bookmarks {
            let mut view = doc.initial_view();
            if view.navigation != Navigation::Bookmarks {
                view.navigation = Navigation::Bookmarks;
                edits.push(Edit::SetInitialView(Box::new(view)));
            }
        }
        Some(Edit::Batch { label: "Finish Conjecturas file".into(), edits })
    }
}

/// Returns (apply, cancel).
pub(crate) fn finish_body(ui: &mut egui::Ui, d: &mut FinishDraft, t: &Tokens) -> (bool, bool) {
    ui.set_width(520.0);
    ui.label(egui::RichText::new(tl!("Finish Conjecturas file")).font(crate::theme::semibold(18.0)));
    ui.add_space(4.0);
    ui.label(
        egui::RichText::new(tl!("Sets the house's document information and removes what other applications left in the file.")).color(t.text_muted),
    );
    ui.add_space(10.0);
    egui::Grid::new("finish-conjecturas").num_columns(2).spacing([18.0, 8.0]).min_col_width(120.0).show(ui, |ui| {
        for (label, value, salt) in [
            ("Title", &mut d.title, "title"),
            ("Author", &mut d.author, "author"),
            ("Application", &mut d.creator, "creator"),
            ("PDF producer", &mut d.producer, "producer"),
        ] {
            let l = ui.label(egui::RichText::new(tl!(label)).color(t.text_muted));
            ui.add(widgets::line(value).desired_width(360.0).background_color(t.field).id_salt(("finish", salt))).labelled_by(l.id);
            ui.end_row();
        }
    });
    ui.add_space(10.0);
    ui.add(widgets::checkbox(
        &mut d.strip_xmp,
        tl!("Remove the XMP metadata of other applications (InDesign history, document IDs, image metadata)"),
    ));
    ui.add(widgets::checkbox(&mut d.strip_private, tl!("Remove the private data of other applications")));
    ui.add_enabled_ui(d.has_bookmarks, |ui| {
        ui.add(widgets::checkbox(&mut d.show_bookmarks, tl!("Open with the bookmarks panel")));
    });
    ui.add_space(6.0);
    ui.label(
        egui::RichText::new(tl!("Save the document afterwards; saving rewrites the whole file so nothing removed stays in it."))
            .small()
            .color(t.text_faint),
    );
    ui.add_space(12.0);
    let (mut apply, mut cancel) = (false, false);
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        if widgets::pill_button(ui, tl!("Apply"), true).clicked() {
            apply = true;
        }
        if widgets::pill_button(ui, tl!("Cancel"), false).clicked() {
            cancel = true;
        }
    });
    (apply, cancel)
}

/// The Proof for the author dialog.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ProofDraft {
    /// Who receives the proof (written into the watermark).
    pub name: String,
    /// DD/MM/YYYY, today by default.
    pub date: String,
    /// Optional; when empty a random one is made (the proof is a copy, the original stays open).
    pub password: String,
    /// Let the author comment on the proof.
    pub allow_comments: bool,
}

impl ProofDraft {
    /// The watermark's text.
    pub fn mark(&self) -> String {
        [tl!("PROOF").to_string(), self.name.trim().to_owned(), self.date.trim().to_owned()]
            .into_iter()
            .filter(|p| !p.is_empty())
            .collect::<Vec<_>>()
            .join(" – ")
    }
}

/// A permissions password nobody needs to know: the proof is a copy and the original stays editable.
fn random_password() -> String {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    // FNV-1a over the clock and the process id; the password only has to be unguessable in practice.
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in nanos.to_le_bytes().into_iter().chain(std::process::id().to_le_bytes()) {
        h = (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3);
    }
    let mut g = h ^ 0x9e37_79b9_7f4a_7c15;
    g = g.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    format!("{h:016x}{g:016x}")
}

impl PdfCraftApp {
    pub fn open_author_proof(&mut self) {
        if self.active_ids().is_none() {
            return;
        }
        let (y, m, d) = self.session.today();
        self.proof_draft = ProofDraft { date: format!("{d:02}/{m:02}/{y}"), allow_comments: true, ..ProofDraft::default() };
        self.dialog = Some(crate::Dialog::AuthorProof);
    }

    /// The single undoable step the dialog's Apply makes: the watermark, then the protection.
    pub(crate) fn author_proof_edit(&self) -> Option<Edit> {
        let (_, id) = self.active_ids()?;
        let doc = self.session.get(id)?;
        let d = &self.proof_draft;
        let pages: Vec<usize> = (0..doc.info.pages.len()).collect();
        let settings = Watermark { text: d.mark(), opacity: 0.18, color: [0.55, 0.55, 0.55], rotation: 45.0, ..Watermark::default() };
        let password = if d.password.trim().is_empty() { random_password() } else { d.password.trim().to_owned() };
        let protection = Protection {
            permissions_password: Some(password),
            changes: if d.allow_comments { Changes::CommentFillSign } else { Changes::None },
            copy: false,
            ..Protection::default()
        };
        Some(Edit::Batch {
            label: "Proof for the author".into(),
            edits: vec![Edit::AddWatermark { pages, settings, replace: false, file: None }, Edit::Protect(protection)],
        })
    }
}

/// Returns (apply, cancel).
pub(crate) fn proof_body(ui: &mut egui::Ui, d: &mut ProofDraft, t: &Tokens) -> (bool, bool) {
    ui.set_width(520.0);
    ui.label(egui::RichText::new(tl!("Proof for the author")).font(crate::theme::semibold(18.0)));
    ui.add_space(4.0);
    ui.label(egui::RichText::new(tl!("Marks every page with a watermark and protects the copy against changes and copying.")).color(t.text_muted));
    ui.add_space(10.0);
    egui::Grid::new("author-proof").num_columns(2).spacing([18.0, 8.0]).min_col_width(120.0).show(ui, |ui| {
        for (label, value, salt) in [("Name", &mut d.name, "name"), ("Date", &mut d.date, "date")] {
            let l = ui.label(egui::RichText::new(tl!(label)).color(t.text_muted));
            ui.add(widgets::line(value).desired_width(360.0).background_color(t.field).id_salt(("proof", salt))).labelled_by(l.id);
            ui.end_row();
        }
        let l = ui.label(egui::RichText::new(tl!("Permissions password")).color(t.text_muted));
        ui.add(
            widgets::line(&mut d.password)
                .password(true)
                .hint_text(tl!("Optional"))
                .desired_width(360.0)
                .background_color(t.field)
                .id_salt(("proof", "password")),
        )
        .labelled_by(l.id);
        ui.end_row();
    });
    ui.add_space(8.0);
    ui.label(egui::RichText::new(d.mark()).font(crate::theme::semibold(13.0)).color(t.text_muted));
    ui.add_space(8.0);
    ui.add(widgets::checkbox(&mut d.allow_comments, tl!("Let the author comment on the proof")));
    ui.add_space(6.0);
    ui.label(egui::RichText::new(tl!("Use Save As for the proof, so the original stays unprotected.")).small().color(t.text_faint));
    ui.add_space(12.0);
    let (mut apply, mut cancel) = (false, false);
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        if ui.add_enabled_ui(!d.name.trim().is_empty(), |ui| widgets::pill_button(ui, tl!("Apply"), true)).inner.clicked() {
            apply = true;
        }
        if widgets::pill_button(ui, tl!("Cancel"), false).clicked() {
            cancel = true;
        }
    });
    (apply, cancel)
}

/// Where the initials go on each page, as the page is displayed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum InitialsSpot {
    #[default]
    BottomRight,
    BottomLeft,
    RightMiddle,
}

impl InitialsSpot {
    pub const ALL: [InitialsSpot; 3] = [InitialsSpot::BottomRight, InitialsSpot::BottomLeft, InitialsSpot::RightMiddle];

    pub fn label(self) -> &'static str {
        match self {
            InitialsSpot::BottomRight => "Bottom right corner",
            InitialsSpot::BottomLeft => "Bottom left corner",
            InitialsSpot::RightMiddle => "Right margin, middle",
        }
    }
}

/// The Initial every page dialog.
#[derive(Clone, Debug, PartialEq)]
pub struct InitialsDraft {
    pub spot: InitialsSpot,
    /// Distance from the page edges, in millimetres.
    pub margin_mm: f32,
    /// Leave the last page out (it is the one signed).
    pub skip_last: bool,
    /// Afterwards, go to the last page with the signature tool.
    pub sign_last: bool,
}

impl Default for InitialsDraft {
    fn default() -> Self {
        Self { spot: InitialsSpot::BottomRight, margin_mm: 10.0, skip_last: true, sign_last: true }
    }
}

/// The user-space bounds of what a placement edit draws.
fn placed_bounds(edit: &Edit) -> Option<[f64; 4]> {
    let union = |a: Option<[f64; 4]>, b: [f64; 4]| Some(a.map_or(b, |a| [a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])]));
    match edit {
        Edit::AddAnnotation(a) => match &a.shape {
            Shape::Signature { strokes } | Shape::Ink { strokes, .. } => {
                strokes.iter().flatten().fold(None, |acc, p| union(acc, [p[0], p[1], p[0], p[1]]))
            }
            Shape::TypedSignature { rect, .. } => Some(*rect),
            _ => None,
        },
        Edit::AddCustomStamp { rect, .. } => Some(*rect),
        Edit::Batch { edits, .. } => edits.iter().filter_map(placed_bounds).fold(None, union),
        _ => None,
    }
}

/// The saved initials on `page`, at `spot`, `margin` points from the edges as displayed. The
/// initials are placed once in the middle of the page to measure them, then moved: placing is
/// the same at any point, so the move is exact for drawn, typed and picture initials alike.
pub fn initials_on(page: usize, info: &PageInfo, sig: &crate::fill_sign::SavedSig, spot: InitialsSpot, margin: f32, author: &str) -> Option<Edit> {
    let (w, h) = (info.width, info.height);
    if !(w > 0.0 && h > 0.0) {
        return None;
    }
    let probe_view = [w / 2.0, h / 2.0];
    let probe_user = info.view_to_user(probe_view[0], probe_view[1]);
    let probe = crate::fill_sign::place(page, info, [f64::from(probe_user[0]), f64::from(probe_user[1])], sig, true, author)?;
    let b = placed_bounds(&probe)?;
    let corners = [[b[0], b[1]], [b[2], b[1]], [b[0], b[3]], [b[2], b[3]]].map(|c| info.user_to_view(c[0] as f32, c[1] as f32));
    let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for c in corners {
        (x0, y0, x1, y1) = (x0.min(c[0]), y0.min(c[1]), x1.max(c[0]), y1.max(c[1]));
    }
    // View space runs y down: the bottom edge is the largest y.
    let (dx, dy) = match spot {
        InitialsSpot::BottomRight => (w - margin - x1, h - margin - y1),
        InitialsSpot::BottomLeft => (margin - x0, h - margin - y1),
        InitialsSpot::RightMiddle => (w - margin - x1, h / 2.0 - (y0 + y1) / 2.0),
    };
    let to = info.view_to_user(probe_view[0] + dx, probe_view[1] + dy);
    crate::fill_sign::place(page, info, [f64::from(to[0]), f64::from(to[1])], sig, true, author)
}

impl PdfCraftApp {
    pub fn open_initial_every_page(&mut self) {
        if self.active_ids().is_none() {
            return;
        }
        self.dialog = Some(crate::Dialog::InitialEveryPage);
    }

    /// The pages the dialog initials.
    pub(crate) fn initials_pages(&self) -> Vec<usize> {
        let Some((_, id)) = self.active_ids() else { return Vec::new() };
        let Some(doc) = self.session.get(id) else { return Vec::new() };
        let n = doc.info.pages.len();
        let end = if self.initials_draft.skip_last { n.saturating_sub(1) } else { n };
        (0..end).collect()
    }

    /// The single undoable step the dialog's Apply makes.
    pub(crate) fn initial_every_page_edit(&self) -> Option<Edit> {
        let (_, id) = self.active_ids()?;
        let doc = self.session.get(id)?;
        let sig = self.initials.as_ref()?;
        let d = &self.initials_draft;
        let margin = d.margin_mm.clamp(0.0, 100.0) * 72.0 / 25.4;
        let author = self.comment_prefs.author.clone();
        let edits: Vec<Edit> =
            self.initials_pages().into_iter().filter_map(|p| initials_on(p, doc.info.pages.get(p)?, sig, d.spot, margin, &author)).collect();
        (!edits.is_empty()).then(|| Edit::Batch { label: "Initial every page".into(), edits })
    }

    /// After initialling: the last page, with the signature tool ready (or its creation dialog).
    pub(crate) fn sign_last_page(&mut self) {
        let Some(i) = self.active else { return };
        let Some((_, id)) = self.active_ids() else { return };
        let Some(n) = self.session.get(id).map(|d| d.info.pages.len()) else { return };
        if let Some(v) = self.views.get_mut(i) {
            v.go_to_page(n.saturating_sub(1));
        }
        self.run_command("sign.fill.signature");
    }
}

/// Returns (apply, cancel).
pub(crate) fn initials_body(ui: &mut egui::Ui, app: &mut PdfCraftApp, t: &Tokens) -> (bool, bool) {
    ui.set_width(480.0);
    ui.label(egui::RichText::new(tl!("Initial every page")).font(crate::theme::semibold(18.0)));
    ui.add_space(4.0);
    ui.label(egui::RichText::new(tl!("Puts your saved initials at the same place on each page.")).color(t.text_muted));
    ui.add_space(10.0);
    let has_initials = app.initials.is_some();
    if !has_initials {
        ui.label(egui::RichText::new(tl!("Create your initials first.")).color(t.text));
        if widgets::pill_button(ui, tl!("Create initials"), false).clicked() {
            app.dialog = None;
            app.run_command("sign.fill.initials");
            return (false, false);
        }
        ui.add_space(8.0);
    }
    let d = &mut app.initials_draft;
    for spot in InitialsSpot::ALL {
        widgets::radio_value(ui, &mut d.spot, spot, tl!(spot.label()));
    }
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.label(tl!("Distance from the edges"));
        ui.add(egui::DragValue::new(&mut d.margin_mm).range(0.0..=100.0).speed(0.5).suffix(" mm"));
    });
    ui.add(widgets::checkbox(&mut d.skip_last, tl!("Leave out the last page")));
    ui.add(widgets::checkbox(&mut d.sign_last, tl!("Then sign the last page")));
    let n = app.initials_pages().len();
    ui.add_space(6.0);
    ui.label(egui::RichText::new(crate::i18n::fmt(tl!("Pages to initial: {n}"), &[("n", &n.to_string())])).small().color(t.text_faint));
    ui.add_space(12.0);
    let (mut apply, mut cancel) = (false, false);
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        if ui.add_enabled_ui(has_initials && n > 0, |ui| widgets::pill_button(ui, tl!("Apply"), true)).inner.clicked() {
            apply = true;
        }
        if widgets::pill_button(ui, tl!("Cancel"), false).clicked() {
            cancel = true;
        }
    });
    (apply, cancel)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fill_sign::SavedSig;

    /// A 200 × 300 page shown turned `rotation` degrees clockwise.
    fn page(rotation: u16) -> PageInfo {
        let (width, height) = if rotation.is_multiple_of(180) { (200.0, 300.0) } else { (300.0, 200.0) };
        PageInfo { width, height, label: String::new(), crop: [0.0, 0.0, 200.0, 300.0], rotation }
    }

    /// The placed initials' box as displayed (view space, y down).
    fn shown(info: &PageInfo, edit: &Edit) -> [f32; 4] {
        let b = placed_bounds(edit).expect("bounds");
        let c = [[b[0], b[1]], [b[2], b[3]]].map(|c| info.user_to_view(c[0] as f32, c[1] as f32));
        [c[0][0].min(c[1][0]), c[0][1].min(c[1][1]), c[0][0].max(c[1][0]), c[0][1].max(c[1][1])]
    }

    #[test]
    fn initials_sit_at_the_chosen_spot_as_displayed() {
        let drawn = SavedSig::Drawn(vec![vec![[0.0, 0.0], [1.0, 0.3]]]);
        let typed = SavedSig::Typed("BB".into());
        for sig in [&drawn, &typed] {
            for rotation in [0, 90, 180, 270] {
                let info = page(rotation);
                let (w, h) = (info.width, info.height);
                let near = |a: f32, b: f32| (a - b).abs() < 0.5;
                let r = shown(&info, &initials_on(0, &info, sig, InitialsSpot::BottomRight, 20.0, "").unwrap());
                assert!(near(r[2], w - 20.0) && near(r[3], h - 20.0), "bottom right at {rotation}°: {r:?}");
                let r = shown(&info, &initials_on(0, &info, sig, InitialsSpot::BottomLeft, 20.0, "").unwrap());
                assert!(near(r[0], 20.0) && near(r[3], h - 20.0), "bottom left at {rotation}°: {r:?}");
                let r = shown(&info, &initials_on(0, &info, sig, InitialsSpot::RightMiddle, 20.0, "").unwrap());
                assert!(near(r[2], w - 20.0) && near((r[1] + r[3]) / 2.0, h / 2.0), "right middle at {rotation}°: {r:?}");
            }
        }
    }

    #[test]
    fn a_page_without_size_gets_no_initials() {
        let info = PageInfo { width: 0.0, height: 0.0, label: String::new(), crop: [0.0; 4], rotation: 0 };
        assert!(initials_on(0, &info, &SavedSig::Typed("BB".into()), InitialsSpot::BottomRight, 20.0, "").is_none());
    }
}
