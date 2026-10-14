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

/// One line drawn in two runs: "Page " in Helvetica 24 pt and "bold" in Helvetica-BoldOblique 18 pt.
fn mixed_fixture() -> Vec<u8> {
    let body = "BT /F1 24 Tf 20 150 Td (Page ) Tj /F2 18 Tf (bold) Tj ET";
    let objs = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [4 0 R] /Count 1 /MediaBox [0 0 200 300] >>".into(),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".into(),
        "<< /Type /Page /Parent 2 0 R /Contents 5 0 R /Resources << /Font << /F1 3 0 R /F2 6 0 R >> >> >>".into(),
        format!("<< /Length {} >>\nstream\n{body}\nendstream", body.len()),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica-BoldOblique >>".into(),
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

#[test]
fn the_format_panel_follows_the_run_under_the_caret() {
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_eframe(|_cc| {
        let mut app = PdfCraftApp::new();
        app.set_option("language", "en").unwrap();
        app.open_bytes("mixed.pdf", None, mixed_fixture()).unwrap();
        app
    });
    h.run_steps(4);
    assert!(h.state_mut().execute("edit.edit_text"));
    h.run_steps(2);
    let r = h.state().views[0].page_screen_rect(0).unwrap();
    let at = egui::pos2(r.left() + 30.0 / 200.0 * r.width(), r.top() + (300.0 - 158.0) / 300.0 * r.height());
    h.hover_at(at);
    h.run_steps(1);
    h.drag_at(at);
    h.run_steps(1);
    h.drop_at(at);
    h.run_steps(4);
    let blocks = h.state().session.get(h.state().views[0].id).unwrap().text_blocks(0);
    assert_eq!(blocks.len(), 1, "one paragraph with two runs: {blocks:?}");
    // The caret at the end of the line sits after "bold": the panel shows bold italic 18 pt.
    h.key_press(Key::End);
    h.run_steps(2);
    let look = h.state().views[0].line_editor.as_ref().unwrap().look.clone();
    assert!(look.bold && look.italic && look.size == 18.0, "{look:?}");
    // At the start of the line, after "P": regular 24 pt.
    h.key_press(Key::Home);
    h.key_press(Key::ArrowRight);
    h.run_steps(2);
    let look = h.state().views[0].line_editor.as_ref().unwrap().look.clone();
    assert!(!look.bold && !look.italic && look.size == 24.0, "{look:?}");
    // Nothing was changed by moving the caret.
    assert_eq!(h.state().session.get(h.state().views[0].id).unwrap().can_undo(), None);
}

/// One font list, in Format text beside the size: it names the paragraph's font, and choosing a
/// font (here, loading one, as the list's installed fonts and "Load font…" do) applies it at once.
#[test]
fn the_font_list_sits_in_format_text_and_applies_the_chosen_font() {
    use egui_kittest::kittest::Queryable;
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_eframe(|_cc| {
        let mut app = PdfCraftApp::new();
        app.set_option("language", "en").unwrap();
        app.open_bytes("mixed.pdf", None, mixed_fixture()).unwrap();
        app
    });
    h.run_steps(4);
    assert!(h.state_mut().execute("edit.edit_text"));
    h.run_steps(2);
    let r = h.state().views[0].page_screen_rect(0).unwrap();
    let at = egui::pos2(r.left() + 30.0 / 200.0 * r.width(), r.top() + (300.0 - 158.0) / 300.0 * r.height());
    h.hover_at(at);
    h.run_steps(1);
    h.drag_at(at);
    h.run_steps(1);
    h.drop_at(at);
    h.run_steps(4);
    h.key_press(Key::Home);
    h.key_press(Key::ArrowRight);
    h.run_steps(2);
    assert!(h.state().views[0].line_editor.is_some(), "editing");
    h.run_steps(4);
    // The font box names the run's font; the separate boxes above Format text are gone.
    let title = h.get_by_label("FORMAT TEXT").rect();
    let combo = |h: &Harness<'static, PdfCraftApp>, value: &str| {
        h.query_by(|n| n.role() == egui::accesskit::Role::ComboBox && n.value().as_deref() == Some(value)).map(|n| n.rect())
    };
    let font = combo(&h, "Helvetica").expect("font list");
    let size = combo(&h, "24 pt").expect("size list");
    assert!(
        font.top() > title.bottom() && (font.center().y - size.center().y).abs() <= 6.0 && font.right() <= size.left(),
        "{title:?} {font:?} {size:?}"
    );
    assert!(combo(&h, "Keep source fonts").is_none() && combo(&h, "Installed fonts…").is_none());
    assert!(h.query_by_label("Load font…").is_none(), "Load font… is inside the list");
    // Choosing a font applies it to the paragraph in one step.
    let inter = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/fonts/Inter-Regular.ttf")).unwrap();
    h.state_mut().use_files(pdfcraft_ui_egui::FilePurpose::TextFont { page: 0 }, vec![("Inter-Regular.ttf".into(), inter)]);
    h.run_steps(4);
    let id = h.state().views[0].id;
    let doc = h.state().session.get(id).unwrap();
    assert_eq!(doc.can_undo(), Some("Edit text"));
    assert!(doc.text_lines(0).iter().all(|l| l.base_font == "Inter-Regular"), "{:?}", doc.text_lines(0));
    assert!(h.state().views[0].line_editor.is_some(), "still editing");
    h.run_steps(2);
    assert!(combo(&h, "Inter-Regular").is_some(), "the list names the applied font");
}
