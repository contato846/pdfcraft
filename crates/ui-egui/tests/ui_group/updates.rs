//! Help ▸ Check for updates (issue #28), with stand-in release sources (no network).

use std::sync::Arc;

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use pdfcraft_ui_egui::PdfCraftApp;
use pdfcraft_ui_egui::updates::{Release, UpdateSource, is_newer};

fn source(answer: Result<&str, &str>) -> UpdateSource {
    let answer = answer.map(str::to_string).map_err(str::to_string);
    Arc::new(move || {
        answer.clone().map(|v| Release { asset: None, url: format!("https://github.com/storytold/pdfcraft/releases/tag/{v}"), version: v })
    })
}

fn harness(answer: Result<&str, &str>) -> Harness<'static, PdfCraftApp> {
    Harness::builder().with_size(egui::vec2(1200.0, 800.0)).build_eframe(move |_cc| {
        let mut app = PdfCraftApp::new();
        app.set_option("language", "en").unwrap();
        app.update_source = Some(source(answer));
        app
    })
}

/// Run frames until the background check has reported. By what the dialog shows, with the
/// same 30-second deadline as `support::wait_until`, not for a fixed second: under load the
/// check's thread can take longer (#1277).
fn settle(h: &mut Harness<'static, PdfCraftApp>) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        h.run_steps(2);
        if h.query_by_label_contains("Checking for a newer version").is_none() {
            break;
        }
        assert!(std::time::Instant::now() < deadline, "timed out waiting for the update check");
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    h.run_steps(2);
}

#[test]
fn versions_compare_by_number() {
    assert!(is_newer("v0.2.0", "0.1.1"));
    assert!(is_newer("v0.1.10", "0.1.9"));
    assert!(is_newer("1", "0.9.9"));
    assert!(!is_newer("v0.1.1", "0.1.1"));
    assert!(!is_newer("v0.1.0", "0.1.1"));
    assert!(!is_newer("v0.1.1-beta.2", "0.1.1"), "suffixes are ignored");
    assert!(!is_newer("nightly", "0.1.1"), "a tag that isn't a version is never newer");
    assert!(!is_newer("v1.2.3.4", "0.1.1"));
    assert!(!is_newer("v99999999999999999999.0.0", "0.1.1"), "out of range");
}

#[test]
fn a_newer_release_is_offered_for_download() {
    let mut h = harness(Ok("v99.0.0"));
    h.state_mut().execute("help.check_updates");
    settle(&mut h);
    h.get_by_label_contains("PdfCraft 99.0.0 is available");
    h.get_by_label("Download");
    h.get_by_label("Later").click();
    h.run_steps(3);
    assert!(h.query_by_label_contains("is available").is_none(), "Later closes the dialog");
}

#[test]
fn an_up_to_date_or_failed_check_says_so() {
    let mut h = harness(Ok(concat!("v", env!("CARGO_PKG_VERSION"))));
    h.state_mut().execute("help.check_updates");
    settle(&mut h);
    h.get_by_label_contains("is up to date");
    assert!(h.query_by_label("Download").is_none());

    let mut h = harness(Err("couldn't reach GitHub"));
    h.state_mut().execute("help.check_updates");
    settle(&mut h);
    h.get_by_label_contains("Couldn't check for updates: couldn't reach GitHub");
}

#[test]
fn nothing_is_asked_until_the_user_checks() {
    // No check at start (the owner's decision): the source is only called from the command.
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counted = calls.clone();
    let mut h = Harness::builder().with_size(egui::vec2(1200.0, 800.0)).build_eframe(move |_cc| {
        let mut app = PdfCraftApp::new();
        app.set_option("language", "en").unwrap();
        // Settings from a build that had the startup option are ignored.
        app.restore(r#"{"check_updates_at_start": true}"#);
        let counted = counted.clone();
        app.update_source = Some(Arc::new(move || {
            counted.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(Release { version: "v99.0.0".into(), url: "https://github.com/storytold/pdfcraft/releases/tag/v99.0.0".into(), asset: None })
        }));
        app
    });
    settle(&mut h);
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 0);
    assert!(h.query_by_label_contains("is available").is_none());
    h.state_mut().execute("help.check_updates");
    settle(&mut h);
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    h.get_by_label_contains("PdfCraft 99.0.0 is available");
}

/// The portable build (one that knows its commit and can install): a release of another build
/// opens the dialog at start; Update now stages it and Restart now swaps it in. The same build
/// again says nothing.
#[test]
fn the_portable_build_updates_itself_from_a_release_of_another_commit() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let installs = Arc::new(AtomicUsize::new(0));
    let applies = Arc::new(AtomicUsize::new(0));
    let harness = |tag: &'static str, installs: Arc<AtomicUsize>, applies: Arc<AtomicUsize>| {
        Harness::builder().with_size(egui::vec2(1200.0, 800.0)).build_eframe(move |_cc| {
            let mut app = PdfCraftApp::new();
            app.set_option("language", "en").unwrap();
            app.build_sha = Some("c30002f0aca1ded9067d04ddabe19a777c76ff3a");
            app.update_source = Some(Arc::new(move || {
                Ok(Release {
                    version: tag.into(),
                    url: format!("https://github.com/contato846/pdfcraft/releases/tag/{tag}"),
                    asset: Some(format!(
                        "https://github.com/contato846/pdfcraft/releases/download/{tag}/pdfcraft-fontes-livres-windows-x64-portable.zip"
                    )),
                })
            }));
            let installs = installs.clone();
            app.update_installer = Some(Arc::new(move |asset: &str, progress: &mut dyn FnMut(u64)| {
                assert!(asset.ends_with("portable.zip"));
                progress(2_000_000);
                installs.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }));
            let applies = applies.clone();
            app.update_apply = Some(Arc::new(move || {
                applies.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }));
            app
        })
    };
    let wait_for = |h: &mut Harness<'static, PdfCraftApp>, text: &str| {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while h.query_by_label_contains(text).is_none() {
            assert!(std::time::Instant::now() < deadline, "timed out waiting for {text:?}");
            std::thread::sleep(std::time::Duration::from_millis(5));
            h.run_steps(2);
        }
    };
    let mut h = harness("fontes-livres-26c762b", installs.clone(), applies.clone());
    wait_for(&mut h, "A new version is available (fontes-livres-26c762b)");
    h.get_by_label("Update now").click();
    wait_for(&mut h, "The new version is ready");
    assert_eq!(installs.load(Ordering::SeqCst), 1);
    h.get_by_label("Restart now").click();
    h.run_steps(3);
    assert_eq!(applies.load(Ordering::SeqCst), 1);

    // The release of this very build: nothing to say.
    let mut h = harness("fontes-livres-c30002f", installs.clone(), applies.clone());
    settle(&mut h);
    std::thread::sleep(std::time::Duration::from_millis(100));
    h.run_steps(4);
    assert!(h.query_by_label_contains("is available").is_none());
    assert!(h.query_by_label("Update now").is_none());
}
