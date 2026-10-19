//! Pages ▸ Split book spreads: the dialog, and the search for the bindings on a worker thread
//! (a page at a time, with the progress shown), then the split as one undoable step.

use std::sync::{Arc, Mutex};

use pdfcraft_engine::DocId;
use pdfcraft_engine::spreads::{SplitAt, Spread};

use crate::theme::Tokens;
use crate::{PdfCraftApp, widgets};

#[derive(Default)]
pub struct SpreadProgress {
    pub done: usize,
    pub total: usize,
    pub result: Option<Vec<Spread>>,
}

pub struct SpreadRun {
    pub doc: DocId,
    pub progress: Arc<Mutex<SpreadProgress>>,
}

/// The Split book spreads dialog.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpreadDraft {
    pub at: SplitAt,
}

impl Default for SpreadDraft {
    fn default() -> Self {
        Self { at: SplitAt::Binding }
    }
}

impl PdfCraftApp {
    pub fn open_split_spreads(&mut self) {
        if self.active_ids().is_none() {
            return;
        }
        self.dialog = Some(crate::Dialog::SplitSpreads);
    }

    /// The spreads (pages wider than tall) of the active document.
    pub(crate) fn spread_pages(&self) -> usize {
        let Some((_, id)) = self.active_ids() else { return 0 };
        self.session.get(id).map_or(0, |d| d.info.pages.iter().filter(|p| pdfcraft_engine::spreads::is_spread(p)).count())
    }

    /// Look for the bindings on a worker thread; [`Self::poll_spreads`] splits when done.
    pub fn split_spreads(&mut self) {
        if self.spread_run.is_some() {
            return;
        }
        let Some((_, id)) = self.active_ids() else { return };
        let Some(job) = self.session.spread_job(id, &[], self.spread_draft.at) else { return };
        let progress = Arc::new(Mutex::new(SpreadProgress { total: job.pages.len(), ..Default::default() }));
        let p = progress.clone();
        let work = move || {
            let found = job.run(|done, total| {
                if let Ok(mut s) = p.lock() {
                    s.done = done;
                    s.total = total;
                }
                true
            });
            if let Ok(mut s) = p.lock() {
                s.result = Some(found);
            }
        };
        #[cfg(not(target_arch = "wasm32"))]
        if self.run_inline {
            work();
        } else {
            std::thread::Builder::new().name("pdfcraft-spreads".into()).spawn(work).ok();
        }
        #[cfg(target_arch = "wasm32")]
        work();
        self.spread_run = Some(SpreadRun { doc: id, progress });
        self.poll_spreads();
    }

    pub(crate) fn poll_spreads(&mut self) {
        let Some(run) = &self.spread_run else { return };
        let doc = run.doc;
        let Ok(mut s) = run.progress.lock() else {
            self.spread_run = None;
            self.progress_notice = None;
            return;
        };
        let Some(found) = s.result.take() else {
            let label = crate::i18n::fmt(
                tl!("Finding the binding… page {n} of {total}"),
                &[("n", &(s.done + 1).min(s.total.max(1)).to_string()), ("total", &s.total.to_string())],
            );
            let fraction = if s.total == 0 { 0.0 } else { s.done as f32 / s.total as f32 };
            self.progress_notice = Some(widgets::ProgressNotice { label, fraction, cancellable: false });
            drop(s);
            if let Some(ctx) = &self.ctx {
                ctx.request_repaint_after(std::time::Duration::from_millis(50));
            }
            return;
        };
        drop(s);
        self.spread_run = None;
        self.progress_notice = None;
        match self.session.apply_spreads(doc, &found) {
            Ok(0) => self.notify_tr("No page here is a book spread (wider than tall)."),
            Ok(n) => {
                if let Some(info) = self.session.get(doc).map(|d| d.info.clone())
                    && let Some(view) = self.views.iter_mut().find(|v| v.id == doc)
                {
                    view.document_changed(&info);
                }
                self.notify_fmt("Split {n} spreads into {pages} pages.", &[("n", &n.to_string()), ("pages", &(n * 2).to_string())]);
            }
            Err(e) => self.notify_error(e),
        }
    }
}

/// Returns (split, cancel).
pub(crate) fn body(ui: &mut egui::Ui, app: &mut PdfCraftApp, t: &Tokens) -> (bool, bool) {
    ui.set_width(480.0);
    ui.label(egui::RichText::new(tl!("Split book spreads")).font(crate::theme::semibold(18.0)));
    ui.add_space(4.0);
    ui.label(
        egui::RichText::new(tl!(
            "Each page scanned with the book open becomes two pages, the left one first. The scan itself is not cut or recompressed."
        ))
        .color(t.text_muted),
    );
    ui.add_space(10.0);
    let n = app.spread_pages();
    let d = &mut app.spread_draft;
    widgets::radio_value(ui, &mut d.at, SplitAt::Binding, tl!("At the binding (found on each page)"));
    widgets::radio_value(ui, &mut d.at, SplitAt::Middle, tl!("Exactly in the middle"));
    ui.add_space(8.0);
    ui.label(
        egui::RichText::new(crate::i18n::fmt(tl!("Spreads (pages wider than tall): {n}. The others stay as they are."), &[("n", &n.to_string())]))
            .small()
            .color(t.text_faint),
    );
    ui.add_space(12.0);
    let (mut go, mut cancel) = (false, false);
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        if ui.add_enabled_ui(n > 0, |ui| widgets::pill_button(ui, tl!("Split"), true)).inner.clicked() {
            go = true;
        }
        if widgets::pill_button(ui, tl!("Cancel"), false).clicked() {
            cancel = true;
        }
    });
    (go, cancel)
}
