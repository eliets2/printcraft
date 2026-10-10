//! Scan & OCR ▸ Recognize text in the real shell (egui_kittest): the dialog, the run and its
//! result (skipped without the OCR models: `cargo xtask models`).

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use pdfcraft_engine::ocr::{OcrPage, OcrSettings, PlacedWord};
use pdfcraft_engine::{Session, export};
use pdfcraft_ui_egui::PdfCraftApp;

/// A one-page PDF that is only a picture of a sentence.
fn scan() -> Vec<u8> {
    let mut s = Session::new();
    let text = s.create_from_text("t", "Recognize these scanned words").unwrap();
    let id = s.open("text.pdf", None, text, None).unwrap();
    let png = export::Exporter::new(s.get(id).unwrap()).png(0, 150.0).unwrap();
    s.create_from_images(&[("scan.png".into(), png)]).unwrap().to_vec()
}

#[test]
fn recognize_text_dialog_adds_searchable_text() {
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_eframe(|_cc| {
        let mut app = PdfCraftApp::new();
        app.open_bytes("scan.pdf", None, scan()).unwrap();
        app.run_inline = true;
        app
    });
    h.run_steps(4);
    assert!(h.state_mut().execute("ocr.recognize"));
    h.run_steps(2);
    h.get_by_label("Document language");
    h.get_by_label("Output");
    if !pdfcraft_engine::ocr::available() {
        eprintln!("skipped: OCR models not installed");
        return;
    }
    h.get_by_label("Recognize text").click();
    h.run_steps(3);
    assert!(h.state().ocr_run.is_none(), "finished");
    let toast = h.state().toast.clone().unwrap().0;
    assert!(toast.starts_with("Recognized ") && toast.contains("on 1 page"), "{toast}");
    let app = h.state();
    let id = app.active_ids().unwrap().1;
    assert_eq!(app.session.get(id).unwrap().can_undo(), Some("Recognize text"));
}

#[test]
fn recognize_text_in_multiple_files_writes_searchable_copies() {
    if !pdfcraft_engine::ocr::available() {
        eprintln!("skipped: OCR models not installed");
        return;
    }
    let dir = std::env::temp_dir().join(format!("pdfcraft-ocr-ui-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut app = PdfCraftApp::new();
    app.run_inline = true;
    app.export_dir_override = Some(dir.to_string_lossy().into_owned());
    app.use_files(pdfcraft_ui_egui::FilePurpose::Ocr, vec![("one.pdf".into(), scan()), ("two.pdf".into(), scan())]);
    assert!(app.ocr_batch.is_none(), "finished");
    let toast = app.toast.clone().unwrap().0;
    assert!(toast.starts_with("Recognized ") && toast.ends_with("in 2 files"), "{toast}");
    for name in ["one.pdf", "two.pdf"] {
        let bytes = std::fs::read(dir.join(name)).unwrap();
        assert!(bytes.len() > scan().len(), "an incremental update was appended");
    }
}

/// A placed word the review can hold (its box sits where a real one would).
fn placed(text: &str, confidence: f32) -> PlacedWord {
    PlacedWord { text: text.into(), origin: [40.0, 40.0], across: [50.0, 0.0], up: [0.0, 12.0], confidence: Some(confidence), source: "ocrs".into() }
}

/// The OCR Verify screen (Scan & OCR ▸ Correct recognized text) in the real shell: every
/// control is live, the keyboard walks the uncertain words, the inspector removes and restores
/// through clicks, and Accept applies the review as one undoable step. The recognition is
/// loaded directly, so this runs without the OCR models.
#[test]
fn ocr_verify_screen_controls_keyboard_and_accept() {
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_eframe(|_cc| {
        let mut app = PdfCraftApp::new();
        app.open_bytes("scan.pdf", None, scan()).unwrap();
        app.run_inline = true;
        app
    });
    h.run_steps(4);
    assert!(h.state_mut().execute("ocr.correct"));
    h.run_steps(3);
    // The screen owns the window and its controls are all present (no dead controls).
    h.get_by_label("Run OCR");
    h.get_by_label("Reject");
    h.get_by_label("Next uncertain");
    h.get_by_label("Previous uncertain");
    h.get_by_label("Page verified");
    h.get_by_label("Exit");
    use pdfcraft_ui_egui::Phase;
    assert_eq!(h.state().ocr_verify.phase, Phase::Idle);
    // The guards hold in the real UI: Reject outside ReviewReady is ignored, not acted on.
    h.get_by_label("Reject").click();
    h.run_steps(2);
    assert_eq!(h.state().ocr_verify.phase, Phase::Idle);
    // A recognition result, as the worker would deliver it.
    {
        let app = h.state_mut();
        let id = app.active_ids().unwrap().1;
        let job = app.session.ocr_job(id, &[0], OcrSettings::default()).unwrap();
        let page = OcrPage { page: 0, words: vec![placed("Helo", 40.0), placed("wrold", 95.0)], skipped: None, notes: Vec::new() };
        app.ocr_verify.load(id, job.guard.clone(), vec![page]);
    }
    h.run_steps(3);
    assert_eq!(h.state().ocr_verify.phase, Phase::ReviewReady);
    // F8 walks to the Low-band word; the inspector shows it.
    h.key_press(egui::Key::F8);
    h.run_steps(2);
    assert_eq!(h.state().ocr_verify.selected, Some((0, 0)));
    h.get_by_label("Remove").click();
    h.run_steps(2);
    assert!(h.state().ocr_verify.is_removed((0, 0)), "Remove strikes the word out");
    h.get_by_label("Restore").click();
    h.run_steps(2);
    assert!(!h.state().ocr_verify.is_removed((0, 0)), "Restore brings it back");
    // Ctrl+T marks the page reviewed (the checkbox flips).
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::T);
    h.run_steps(2);
    assert!(h.state().ocr_verify.review.verified_pages.contains(&0), "Ctrl+T marks the page verified");
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::T);
    h.run_steps(2);
    assert!(!h.state().ocr_verify.review.verified_pages.contains(&0));
    // Accept applies the whole review as ONE undoable step on the open document.
    h.get_by_label("Accept").click();
    h.run_steps(3);
    assert_eq!(h.state().ocr_verify.phase, Phase::Idle, "a successful accept ends the review");
    let app = h.state();
    let id = app.active_ids().unwrap().1;
    assert_eq!(app.session.get(id).unwrap().can_undo(), Some("Recognize text"));
    let toast = app.toast.clone().map(|t| t.0).unwrap_or_default();
    assert!(toast.contains("one undoable step"), "{toast}");
}

/// Ctrl+Tab moves the pane focus ring, and F8 with nothing to review is a no-op: the walk
/// never invents a selection. The recognition is loaded directly (no models needed).
#[test]
fn ocr_verify_screen_focus_ring_and_empty_walk() {
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_eframe(|_cc| {
        let mut app = PdfCraftApp::new();
        app.open_bytes("scan.pdf", None, scan()).unwrap();
        app.run_inline = true;
        app
    });
    h.run_steps(4);
    assert!(h.state_mut().execute("ocr.correct"));
    h.run_steps(3);
    {
        let app = h.state_mut();
        let id = app.active_ids().unwrap().1;
        let job = app.session.ocr_job(id, &[0], OcrSettings::default()).unwrap();
        let page = OcrPage { page: 0, words: vec![placed("Fine", 95.0)], skipped: None, notes: Vec::new() };
        app.ocr_verify.load(id, job.guard.clone(), vec![page]);
    }
    h.run_steps(3);
    // Nothing asks for review: F8 selects nothing instead of wrapping into an error.
    h.key_press(egui::Key::F8);
    h.run_steps(2);
    assert_eq!(h.state().ocr_verify.selected, None);
    // Ctrl+Tab cycles the four panes; four steps come back to the start.
    let start = h.state().ocr_verify.focus;
    let mut moved = false;
    for _ in 0..4 {
        h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Tab);
        h.run_steps(2);
        moved |= h.state().ocr_verify.focus != start;
    }
    assert!(moved, "Ctrl+Tab moves the focus");
    assert_eq!(h.state().ocr_verify.focus, start, "the ring returns after the four panes");
}
