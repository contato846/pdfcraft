//! House tools ▸ Table of contents to links: the dialog over [`pdfcraft_engine::toc`]. It reads the
//! entries of the table of contents on the pages the user names (the current page by default),
//! shows where each goes, and in one undoable step puts a link over each entry and nested
//! bookmarks after the existing ones.

use pdfcraft_engine::toc::{TocEntry, TocScan};
use pdfcraft_engine::{Edit, LinkAction, LinkStyle};

use crate::theme::Tokens;
use crate::{PdfCraftApp, widgets};

/// The dialog's state.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TocDraft {
    /// The pages of the table of contents, 1-based, as typed.
    pub from: String,
    pub to: String,
    /// The file page (1-based) of the book's page 1, as typed; empty finds it.
    pub page_one: String,
    pub links: bool,
    pub bookmarks: bool,
    /// The last reading of the pages.
    pub scan: Option<TocScan>,
}

impl TocDraft {
    /// The table of contents pages, 0-based, or `None` when the range isn't valid for `n` pages.
    pub fn pages(&self, n: usize) -> Option<Vec<usize>> {
        let from: usize = self.from.trim().parse().ok()?;
        let to: usize = if self.to.trim().is_empty() { from } else { self.to.trim().parse().ok()? };
        (from >= 1 && to >= from && to <= n && to - from < 50).then(|| (from - 1..to).collect())
    }

    /// The offset the typed page number implies: printed 1 is file page `page_one`.
    pub fn offset(&self) -> Option<i64> {
        let p: i64 = self.page_one.trim().parse().ok()?;
        (p >= 1).then(|| p - 2)
    }
}

/// The links and bookmarks for `entries` as one step; bookmarks go after the `top_level`
/// existing top-level ones.
pub fn toc_edit(entries: &[TocEntry], links: bool, bookmarks: bool, top_level: usize) -> Option<Edit> {
    let mut edits = Vec::new();
    let placed: Vec<&TocEntry> = entries.iter().filter(|e| e.target.is_some()).collect();
    if links {
        for e in &placed {
            if let Some(target) = e.target {
                edits.push(Edit::AddLink { page: e.page, rect: e.rect, action: LinkAction::Page(target), style: LinkStyle::default() });
            }
        }
    }
    if bookmarks {
        // (level, path, children so far)
        let mut stack: Vec<(usize, Vec<usize>, usize)> = Vec::new();
        let mut top = top_level;
        for e in &placed {
            let Some(target) = e.target else { continue };
            while stack.last().is_some_and(|(level, _, _)| *level >= e.level) {
                stack.pop();
            }
            let (parent, index) = match stack.last_mut() {
                Some((_, path, children)) => {
                    *children += 1;
                    (path.clone(), *children - 1)
                }
                None => {
                    top += 1;
                    (Vec::new(), top - 1)
                }
            };
            let mut path = parent.clone();
            path.push(index);
            edits.push(Edit::AddBookmark { parent, index, title: e.title.clone(), page: target });
            stack.push((e.level, path, 0));
        }
    }
    (!edits.is_empty()).then(|| Edit::Batch { label: "Table of contents to links".into(), edits })
}

impl PdfCraftApp {
    pub fn open_toc_links(&mut self) {
        let Some((index, _)) = self.active_ids() else { return };
        let current = self.views.get(index).map_or(0, |v| v.current) + 1;
        self.toc_draft = TocDraft { from: current.to_string(), links: true, bookmarks: true, ..TocDraft::default() };
        self.scan_toc();
        self.dialog = Some(crate::Dialog::TocLinks);
    }

    /// Read the table of contents again with what the dialog says.
    pub(crate) fn scan_toc(&mut self) {
        let Some((_, id)) = self.active_ids() else { return };
        let Some(doc) = self.session.get(id) else { return };
        let scan = self.toc_draft.pages(doc.info.pages.len()).map(|pages| doc.table_of_contents(&pages, self.toc_draft.offset()));
        if let Some(o) = scan.as_ref().and_then(|s| s.offset)
            && self.toc_draft.page_one.trim().is_empty()
        {
            self.toc_draft.page_one = (o + 2).to_string();
        }
        self.toc_draft.scan = scan;
    }

    pub(crate) fn apply_toc_links(&mut self) {
        let Some((_, id)) = self.active_ids() else { return };
        let Some(scan) = self.toc_draft.scan.as_ref() else { return };
        let top = self.session.get(id).map_or(0, |d| d.info.outline.len());
        let placed = scan.entries.iter().filter(|e| e.target.is_some()).count();
        if let Some(edit) = toc_edit(&scan.entries, self.toc_draft.links, self.toc_draft.bookmarks, top)
            && self.apply_edit(edit)
        {
            self.notify_fmt("Linked {n} entries of the table of contents.", &[("n", &placed.to_string())]);
        }
    }
}

/// What the dialog asks for.
pub(crate) enum TocAction {
    Rescan,
    Apply,
    Cancel,
}

pub(crate) fn toc_body(ui: &mut egui::Ui, d: &mut TocDraft, t: &Tokens, pages: &[String]) -> Option<TocAction> {
    let mut action = None;
    ui.set_width(600.0);
    ui.label(egui::RichText::new(tl!("Table of contents to links")).font(crate::theme::semibold(18.0)));
    ui.add_space(4.0);
    ui.label(
        egui::RichText::new(tl!(
            "Reads the entries of the table of contents (a scan needs Recognize text first), finds the page of each one, and puts a link over each entry and a bookmark for it."
        ))
        .color(t.text_muted),
    );
    ui.add_space(10.0);
    egui::Grid::new("toc-links").num_columns(2).spacing([18.0, 8.0]).min_col_width(170.0).show(ui, |ui| {
        let l = ui.label(egui::RichText::new(tl!("Table of contents on pages")).color(t.text_muted));
        ui.horizontal(|ui| {
            let a = ui.add(widgets::line(&mut d.from).desired_width(60.0).background_color(t.field).id_salt(("toc", "from"))).labelled_by(l.id);
            ui.label(tl!("to"));
            let b = ui.add(widgets::line(&mut d.to).desired_width(60.0).background_color(t.field).id_salt(("toc", "to")));
            if a.lost_focus() || b.lost_focus() {
                action = Some(TocAction::Rescan);
            }
        });
        ui.end_row();
        let l = ui.label(egui::RichText::new(tl!("The book's page 1 is page")).color(t.text_muted));
        let r = ui.add(widgets::line(&mut d.page_one).desired_width(60.0).background_color(t.field).id_salt(("toc", "one"))).labelled_by(l.id);
        if r.lost_focus() {
            action = Some(TocAction::Rescan);
        }
        ui.end_row();
    });
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.add(widgets::checkbox(&mut d.links, tl!("Links over the entries")));
        ui.add(widgets::checkbox(&mut d.bookmarks, tl!("Bookmarks")));
        if ui.button(tl!("Read again")).clicked() {
            action = Some(TocAction::Rescan);
        }
    });
    ui.add_space(8.0);
    let entries = d.scan.as_ref().map(|s| s.entries.as_slice()).unwrap_or_default();
    if entries.is_empty() {
        ui.label(egui::RichText::new(tl!("No entries found on these pages.")).color(t.text_faint));
    } else {
        egui::ScrollArea::vertical().max_height(300.0).auto_shrink([false, true]).show(ui, |ui| {
            egui::Grid::new("toc-entries").num_columns(3).striped(true).spacing([12.0, 4.0]).show(ui, |ui| {
                for e in entries {
                    ui.horizontal(|ui| {
                        ui.add_space(e.level as f32 * 14.0);
                        ui.label(&e.title);
                    });
                    ui.label(egui::RichText::new(&e.printed).color(t.text_muted));
                    match e.target {
                        Some(p) => ui.label(format!("→ {}", pages.get(p).cloned().unwrap_or_else(|| (p + 1).to_string()))),
                        None => ui.label(egui::RichText::new(tl!("not found")).color(t.text_faint)),
                    };
                    ui.end_row();
                }
            });
        });
        let placed = entries.iter().filter(|e| e.target.is_some()).count();
        ui.label(
            egui::RichText::new(crate::i18n::fmt(
                tl!("{placed} of {n} entries have a page."),
                &[("placed", &placed.to_string()), ("n", &entries.len().to_string())],
            ))
            .small()
            .color(t.text_muted),
        );
    }
    ui.add_space(12.0);
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        let ready = entries.iter().any(|e| e.target.is_some()) && (d.links || d.bookmarks);
        if ui.add_enabled_ui(ready, |ui| widgets::pill_button(ui, tl!("Apply"), true)).inner.clicked() {
            action = Some(TocAction::Apply);
        }
        if widgets::pill_button(ui, tl!("Cancel"), false).clicked() {
            action = Some(TocAction::Cancel);
        }
    });
    action
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(title: &str, level: usize, target: Option<usize>) -> TocEntry {
        TocEntry { page: 1, rect: [10.0, 10.0, 100.0, 20.0], title: title.into(), printed: "1".into(), level, target }
    }

    #[test]
    fn bookmarks_nest_by_level_after_the_existing_ones() {
        let entries = [
            entry("A", 0, Some(3)),
            entry("A.1", 1, Some(4)),
            entry("A.2", 1, Some(5)),
            entry("Lost", 0, None),
            entry("B", 0, Some(9)),
            entry("B.1", 1, Some(9)),
        ];
        let Some(Edit::Batch { edits, .. }) = toc_edit(&entries, true, true, 2) else { panic!("an edit") };
        let marks: Vec<(Vec<usize>, usize, String, usize)> = edits
            .iter()
            .filter_map(|e| match e {
                Edit::AddBookmark { parent, index, title, page } => Some((parent.clone(), *index, title.clone(), *page)),
                _ => None,
            })
            .collect();
        assert_eq!(
            marks,
            [
                (vec![], 2, "A".into(), 3),
                (vec![2], 0, "A.1".into(), 4),
                (vec![2], 1, "A.2".into(), 5),
                (vec![], 3, "B".into(), 9),
                (vec![3], 0, "B.1".into(), 9),
            ]
        );
        assert_eq!(edits.iter().filter(|e| matches!(e, Edit::AddLink { .. })).count(), 5, "no link for an entry without a page");
        assert!(toc_edit(&entries, false, false, 0).is_none());
    }

    #[test]
    fn the_typed_page_one_gives_the_offset() {
        let d = TocDraft { from: "3".into(), to: "".into(), page_one: "9".into(), ..TocDraft::default() };
        assert_eq!(d.pages(10), Some(vec![2]));
        assert_eq!(d.pages(2), None);
        assert_eq!(d.offset(), Some(7), "printed 1 on file page 9 (0-based 8): offset 7");
    }
}
