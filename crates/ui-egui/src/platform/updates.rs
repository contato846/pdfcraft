//! Help ▸ Check for updates (issue #28), and in this personal build the update itself.
//!
//! The desktop app supplies how to ask ([`PdfCraftApp::update_source`]), how to download and
//! stage a release ([`PdfCraftApp::update_installer`]) and how to swap the program for the staged
//! one and start it again ([`PdfCraftApp::update_apply`]), so this crate has no network or file
//! code of its own; without them (the web build, tests) the command opens the releases page.
//!
//! The portable build checks once when it starts: a release of another build than this one
//! opens the Updates dialog with an Update now button. Nothing is downloaded until that button
//! is clicked, and a failed check at start says nothing (the manual check still reports it).

use std::sync::Arc;

use egui::{Align, Layout};

use crate::{PdfCraftApp, theme, widgets};

/// Where every release of this build is listed.
pub const RELEASES_PAGE: &str = "https://github.com/contato846/pdfcraft/releases";

/// The latest published release.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Release {
    /// Its version tag, such as `v0.2.0` or `fontes-livres-c30002f`.
    pub version: String,
    /// Its page on [`RELEASES_PAGE`], where the downloads are.
    pub url: String,
    /// The portable build's download, when the release has one the app can install itself.
    pub asset: Option<String>,
}

/// Asks for the latest release (blocking; it runs on its own thread).
pub type UpdateSource = Arc<dyn Fn() -> Result<Release, String> + Send + Sync>;

/// Downloads `asset` and stages the new program beside the running one, reporting the bytes
/// received so far (blocking; it runs on its own thread).
pub type UpdateInstaller = Arc<dyn Fn(&str, &mut dyn FnMut(u64)) -> Result<(), String> + Send + Sync>;

/// Puts the staged program in the running one's place and starts it; the caller then quits.
pub type UpdateApply = Arc<dyn Fn() -> Result<(), String> + Send + Sync>;

/// Whether release `latest` (a tag such as `v0.2.0`) is newer than version `current` (`0.1.1`).
/// Pre-release and build suffixes are ignored; a version that doesn't parse is never newer.
pub fn is_newer(latest: &str, current: &str) -> bool {
    matches!((parse(latest), parse(current)), (Some(l), Some(c)) if l > c)
}

/// Whether release `tag` is another build than the running one, which was compiled from commit
/// `build_sha` (`fontes-livres-c30002f` against `c30002f0ac…`). Builds of this fork are told
/// apart by commit, not by version number, which upstream owns. Unknown on either side: not.
pub fn is_other_build(tag: &str, build_sha: Option<&str>) -> bool {
    let Some(sha) = build_sha.map(str::trim).filter(|s| s.len() >= 7 && s.chars().all(|c| c.is_ascii_hexdigit())) else { return false };
    let Some(tagged) = tag.trim().rsplit('-').next().filter(|s| s.len() >= 7 && s.chars().all(|c| c.is_ascii_hexdigit())) else {
        return false;
    };
    let n = tagged.len().min(sha.len());
    !tagged.get(..n).unwrap_or_default().eq_ignore_ascii_case(sha.get(..n).unwrap_or_default())
}

fn parse(v: &str) -> Option<(u64, u64, u64)> {
    let v = v.trim().trim_start_matches(['v', 'V']);
    let core = v.split(['-', '+']).next()?;
    let mut parts = core.split('.');
    let mut next = |required: bool| match parts.next() {
        Some(p) => p.parse::<u64>().ok(),
        None if required => None,
        None => Some(0),
    };
    let version = (next(true)?, next(false)?, next(false)?);
    parts.next().is_none().then_some(version)
}

/// Where a check is.
#[derive(Default)]
pub(crate) enum Check {
    #[default]
    Idle,
    #[cfg(not(target_arch = "wasm32"))]
    Running(std::sync::mpsc::Receiver<Result<Release, String>>),
    Done(Result<Release, String>),
}

/// Where the download and install are.
#[derive(Default)]
pub(crate) enum Install {
    #[default]
    Idle,
    /// Downloading: the bytes received so far arrive on the channel, then the outcome.
    #[cfg(not(target_arch = "wasm32"))]
    Running(std::sync::mpsc::Receiver<Progress>, u64),
    /// Staged: the new program waits beside this one.
    Staged,
    Failed(String),
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) enum Progress {
    Bytes(u64),
    Done(Result<(), String>),
}

#[derive(Default)]
pub(crate) struct Updates {
    pub(crate) check: Check,
    pub(crate) install: Install,
    /// The Updates dialog is showing.
    pub(crate) open: bool,
    /// The check at start ran (or was skipped).
    pub(crate) checked_at_start: bool,
    /// The check running now was started at start: a failure says nothing.
    pub(crate) quiet: bool,
}

impl Updates {
    /// A release of another build, with a download the app can install: what the Update now
    /// button needs.
    fn installable(&self, build_sha: Option<&str>) -> Option<&Release> {
        match &self.check {
            Check::Done(Ok(r)) if r.asset.is_some() && is_other_build(&r.version, build_sha) => Some(r),
            _ => None,
        }
    }
}

impl PdfCraftApp {
    /// Help ▸ Check for updates: ask for the latest release and show the outcome.
    pub fn check_for_updates(&mut self) {
        self.updates.quiet = false;
        self.start_update_check(true);
    }

    /// Once, when the app starts: ask quietly, and open the dialog only when a new build is there.
    pub(crate) fn check_for_updates_at_start(&mut self) {
        if self.updates.checked_at_start {
            return;
        }
        self.updates.checked_at_start = true;
        if self.update_installer.is_none() || self.build_sha.is_none() {
            return;
        }
        self.updates.quiet = true;
        self.start_update_check(false);
    }

    fn start_update_check(&mut self, open: bool) {
        let Some(source) = self.update_source.clone() else {
            self.open_url(RELEASES_PAGE);
            return;
        };
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.updates.open |= open;
            if matches!(self.updates.check, Check::Running(_)) {
                return;
            }
            let (tx, rx) = std::sync::mpsc::channel();
            let ctx = self.ctx.clone();
            std::thread::spawn(move || {
                // The receiver may be gone (the app quit): nothing to report to then.
                let _ = tx.send(source());
                if let Some(ctx) = ctx {
                    ctx.request_repaint();
                }
            });
            self.updates.check = Check::Running(rx);
        }
        #[cfg(target_arch = "wasm32")]
        {
            let _ = (source, open);
            self.open_url(RELEASES_PAGE);
        }
    }

    /// Pick up a finished check and a download's progress (each frame).
    pub(crate) fn poll_updates(&mut self) {
        #[cfg(not(target_arch = "wasm32"))]
        {
            if let Check::Running(rx) = &self.updates.check {
                let result = match rx.try_recv() {
                    Ok(r) => r,
                    Err(std::sync::mpsc::TryRecvError::Empty) => return,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => Err("the update check stopped unexpectedly".into()),
                };
                self.updates.check = Check::Done(result);
                // The check at start speaks only when there is something to install.
                if self.updates.quiet && self.updates.installable(self.build_sha).is_some() {
                    self.updates.open = true;
                }
            }
            if let Install::Running(rx, bytes) = &mut self.updates.install {
                loop {
                    match rx.try_recv() {
                        Ok(Progress::Bytes(n)) => *bytes = n,
                        Ok(Progress::Done(Ok(()))) => {
                            self.updates.install = Install::Staged;
                            break;
                        }
                        Ok(Progress::Done(Err(e))) => {
                            self.updates.install = Install::Failed(e);
                            break;
                        }
                        Err(std::sync::mpsc::TryRecvError::Empty) => break,
                        Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                            self.updates.install = Install::Failed("the download stopped unexpectedly".into());
                            break;
                        }
                    }
                }
            }
        }
    }

    /// Update now: download the release and stage it beside the running program.
    fn start_update_install(&mut self, asset: String) {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let Some(installer) = self.update_installer.clone() else { return };
            if matches!(self.updates.install, Install::Running(..)) {
                return;
            }
            let (tx, rx) = std::sync::mpsc::channel();
            let ctx = self.ctx.clone();
            std::thread::spawn(move || {
                let progress_tx = tx.clone();
                let repaint = ctx.clone();
                let mut progress = move |n: u64| {
                    let _ = progress_tx.send(Progress::Bytes(n));
                    if let Some(ctx) = &repaint {
                        ctx.request_repaint();
                    }
                };
                let outcome = installer(&asset, &mut progress);
                let _ = tx.send(Progress::Done(outcome));
                if let Some(ctx) = ctx {
                    ctx.request_repaint();
                }
            });
            self.updates.install = Install::Running(rx, 0);
        }
        #[cfg(target_arch = "wasm32")]
        {
            self.open_url(&asset);
        }
    }

    /// Restart into the staged program: the swap, then this one closes. Unsaved work stops it:
    /// once the new program is started, cancelling the close would leave two running.
    fn restart_into_update(&mut self, ctx: &egui::Context) {
        let Some(apply) = self.update_apply.clone() else { return };
        if (0..self.views.len()).any(|i| self.has_unsaved_work(i)) {
            self.updates.install = Install::Failed(tl!("save or close your documents first").to_string());
            return;
        }
        match apply() {
            Ok(()) => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            Err(e) => self.updates.install = Install::Failed(e),
        }
    }
}

/// The Updates dialog.
pub(crate) fn dialog(app: &mut PdfCraftApp, ctx: &egui::Context) {
    if !app.updates.open {
        return;
    }
    let t = theme::Tokens::get(ctx);
    let current = env!("CARGO_PKG_VERSION");
    let mut close = false;
    let mut download: Option<String> = None;
    let mut install: Option<String> = None;
    let mut restart = false;
    let installable = app.updates.installable(app.build_sha).cloned();
    let modal = crate::widgets::show_modal(ctx, "updates", |ui| {
        ui.set_width(440.0);
        ui.horizontal(|ui| {
            ui.add(crate::icons::image("cloud", 22.0, t.accent));
            ui.label(egui::RichText::new(tl!("Check for updates")).font(theme::semibold(16.0)));
        });
        ui.add_space(8.0);
        match (&app.updates.install, &installable) {
            #[cfg(not(target_arch = "wasm32"))]
            (Install::Running(_, bytes), _) => {
                ui.horizontal(|ui| {
                    ui.spinner();
                    let mb = format!("{:.1}", *bytes as f64 / 1_048_576.0);
                    ui.label(crate::i18n::fmt(tl!("Downloading the new version… {mb} MB"), &[("mb", &mb)]));
                });
            }
            (Install::Staged, _) => {
                ui.label(egui::RichText::new(tl!("The new version is ready. PdfCraft will close and open again as the new version.")).strong());
            }
            (Install::Failed(e), _) => {
                ui.label(crate::i18n::fmt(tl!("Couldn't update: {e}"), &[("e", e)]));
                ui.label(
                    egui::RichText::new(crate::i18n::fmt(tl!("All releases are listed at {page}."), &[("page", RELEASES_PAGE)])).color(t.text_muted),
                );
                if let Some(r) = &installable {
                    download = Some(r.url.clone());
                }
            }
            (Install::Idle, Some(r)) => {
                ui.label(egui::RichText::new(crate::i18n::fmt(tl!("A new version is available ({v})."), &[("v", &r.version)])).strong());
                ui.label(egui::RichText::new(tl!("Update now downloads it and swaps the program in place; your settings stay.")).color(t.text_muted));
                install = r.asset.clone();
                download = Some(r.url.clone());
            }
            (Install::Idle, None) => match &app.updates.check {
                Check::Idle => {
                    ui.label(tl!("No check has run yet."));
                }
                #[cfg(not(target_arch = "wasm32"))]
                Check::Running(_) => {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(tl!("Checking for a newer version…"));
                    });
                }
                Check::Done(Ok(r)) if is_newer(&r.version, current) || is_other_build(&r.version, app.build_sha) => {
                    let version = r.version.trim_start_matches(['v', 'V']);
                    ui.label(egui::RichText::new(crate::i18n::fmt(tl!("PdfCraft {v} is available."), &[("v", version)])).strong());
                    ui.label(
                        egui::RichText::new(crate::i18n::fmt(
                            tl!("You have version {c}. Download the new version from its release page."),
                            &[("c", current)],
                        ))
                        .color(t.text_muted),
                    );
                    download = Some(r.url.clone());
                }
                Check::Done(Ok(_)) => {
                    ui.label(crate::i18n::fmt(tl!("PdfCraft {c} is up to date."), &[("c", current)]));
                }
                Check::Done(Err(e)) => {
                    ui.label(crate::i18n::fmt(tl!("Couldn't check for updates: {e}"), &[("e", &e.to_string())]));
                    ui.label(
                        egui::RichText::new(crate::i18n::fmt(
                            tl!("You have version {c}. All releases are listed at {page}."),
                            &[("c", current), ("page", RELEASES_PAGE)],
                        ))
                        .color(t.text_muted),
                    );
                }
            },
        }
        ui.add_space(10.0);
        let note = if app.update_installer.is_some() {
            tl!("Checks GitHub for a new version when PdfCraft starts. Nothing is downloaded until you choose Update now.")
        } else {
            tl!("Asks GitHub for the latest release. Nothing is downloaded or installed automatically.")
        };
        ui.label(egui::RichText::new(note).color(t.text_muted).small());
        ui.add_space(12.0);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if matches!(app.updates.install, Install::Staged) {
                restart = widgets::pill_button(ui, tl!("Restart now"), true).clicked();
                close = widgets::pill_button(ui, tl!("Later"), false).clicked();
            } else if let Some(asset) = install.take() {
                let now = widgets::pill_button(ui, tl!("Update now"), true).clicked();
                let later = widgets::pill_button(ui, tl!("Later"), false).clicked();
                let page = widgets::pill_button(ui, tl!("Release page"), false).clicked();
                install = now.then_some(asset);
                download = if page { download.take() } else { None };
                close = page || later;
            } else if let Some(url) = download.take() {
                let get = widgets::pill_button(ui, tl!("Download"), true).clicked();
                let later = widgets::pill_button(ui, tl!("Later"), false).clicked();
                close = get || later;
                download = get.then_some(url);
            } else if widgets::pill_button(ui, tl!("Close"), true).clicked() {
                close = true;
            }
        });
    });
    if modal.should_close() {
        close = true;
        download = None;
    }
    if let Some(asset) = install {
        app.start_update_install(asset);
    }
    if restart {
        app.restart_into_update(ctx);
    }
    if close {
        app.updates.open = false;
        if let Some(url) = download {
            app.open_url(&url);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_are_told_apart_by_commit() {
        let sha = Some("c30002f0aca1ded9067d04ddabe19a777c76ff3a");
        assert!(!is_other_build("fontes-livres-c30002f", sha));
        assert!(!is_other_build("fontes-livres-C30002F0", sha));
        assert!(is_other_build("fontes-livres-26c762b", sha));
        assert!(!is_other_build("v0.9.0", sha), "a tag without a commit is not a build of this fork");
        assert!(!is_other_build("fontes-livres-26c762b", None), "a build that doesn't know its commit never updates itself");
        assert!(!is_other_build("fontes-livres-26c762b", Some("dev")));
    }
}
