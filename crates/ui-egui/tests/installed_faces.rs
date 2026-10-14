//! A paragraph whose PDF font can't be drawn here (subset-tagged, not embedded as sfnt, as from
//! Distiller) opens in the matching installed face, and new characters keep that face.
//! Its own test binary: the installed-font scan is process-wide and reads `HOME` once.

use egui::{Key, Modifiers};
use egui_kittest::Harness;
use pdfcraft_ui_egui::PdfCraftApp;

fn fixture() -> Vec<u8> {
    let objs = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [4 0 R] /Count 1 /MediaBox [0 0 200 300] >>".into(),
        "<< /Type /Font /Subtype /Type1 /BaseFont /ABCDEF+Inter-Regular /Encoding /WinAnsiEncoding >>".into(),
        "<< /Type /Page /Parent 2 0 R /Contents 5 0 R /Resources << /Font << /F1 3 0 R >> >> >>".into(),
        {
            let body = "BT /F1 24 Tf 20 150 Td (Page 1) Tj ET";
            format!("<< /Length {} >>\nstream\n{body}\nendstream", body.len())
        },
    ];
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, o) in objs.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{o}\nendobj\n", i + 1).as_bytes());
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).as_bytes());
    for o in offsets {
        out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objs.len() + 1).as_bytes());
    out
}

#[cfg(target_os = "linux")]
#[test]
fn a_paragraph_opens_in_its_installed_face_and_new_characters_keep_it() {
    // An installed font folder holding the paragraph's face (the repository's own Inter). The
    // scan reads HOME, so the body runs in a child process with HOME pointing there.
    if std::env::var_os("PDFCRAFT_INSTALLED_FACES_CHILD").is_none() {
        let home = std::env::temp_dir().join(format!("pdfcraft-installed-faces-{}", std::process::id()));
        let fonts = home.join(".local/share/fonts");
        std::fs::create_dir_all(&fonts).unwrap();
        std::fs::copy(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/fonts/Inter-Regular.ttf"), fonts.join("Inter-Regular.ttf")).unwrap();
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["a_paragraph_opens_in_its_installed_face_and_new_characters_keep_it", "--exact", "--nocapture"])
            .env("HOME", &home)
            .env("PDFCRAFT_INSTALLED_FACES_CHILD", "1")
            .status()
            .unwrap();
        let _ = std::fs::remove_dir_all(&home);
        assert!(status.success());
        return;
    }
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_eframe(|_cc| {
        let mut app = PdfCraftApp::new();
        app.set_option("language", "en").unwrap();
        app.open_bytes("doc.pdf", None, fixture()).unwrap();
        app
    });
    h.run_steps(4);
    assert!(h.state_mut().execute("edit.edit_text"));
    h.run_steps(2);
    let r = h.state().views[0].page_screen_rect(0).unwrap();
    let at = egui::pos2(r.left() + 40.0 / 200.0 * r.width(), r.top() + (300.0 - 158.0) / 300.0 * r.height());
    // Hovering with the text tool starts the scan; give it time before the click.
    for _ in 0..40 {
        h.hover_at(at);
        h.run_steps(1);
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    h.drag_at(at);
    h.run_steps(1);
    h.drop_at(at);
    h.run_steps(4);
    assert!(h.state().views[0].line_editor.is_some());

    // The open paragraph is drawn in the installed face, not the interface font.
    let installed = h.output().shapes.iter().any(|s| match &s.shape {
        egui::Shape::Text(t) => {
            t.galley.text().starts_with("Page 1")
                && t.galley
                    .job
                    .sections
                    .iter()
                    .any(|sec| matches!(&sec.format.font_id.family, egui::FontFamily::Name(n) if &**n == "pdf-source-installed-interregular"))
        }
        _ => false,
    });
    assert!(installed, "the inline editor uses the installed Inter");
    // Opening it changed nothing in the document.
    let id = h.state().views[0].id;
    assert_eq!(h.state().session.get(id).unwrap().can_undo(), None);

    // A character outside the PDF font's encoding still comes out in Inter.
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.event(egui::Event::Text("Page 1ő".into()));
    h.run_steps(2);
    h.key_press_modifiers(Modifiers::COMMAND, Key::Enter);
    h.run_steps(4);
    let doc = h.state().session.get(id).unwrap();
    let lines = doc.text_lines(0);
    assert_eq!(lines.iter().map(|l| l.text.as_str()).collect::<String>().replace(' ', ""), "Page1ő");
    assert!(lines.iter().all(|l| pdfcraft_fonts::embedding::face_key(&l.base_font) == "interregular"), "{lines:?}");
}
