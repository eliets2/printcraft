//! Redaction proofs in the session: the proof taken when the marks are applied, the outcome the
//! document carries, and the save-time check of the bytes about to be written.

use std::sync::Arc;

use super::*;

fn session_with(bytes: Vec<u8>) -> (Session, DocId) {
    let mut s = Session::new().with_clock(|| 1_700_000_000);
    let id = s.open("fixture.pdf", None, Arc::new(bytes), None).expect("opens");
    (s, id)
}

/// A redact mark over `rect` (user space) on `page`.
fn mark(page: usize, rect: [f64; 4]) -> Edit {
    let shape = Shape::Redact { quads: vec![rect_quad(rect)], overlay: String::new(), look: Default::default() };
    Edit::AddAnnotation(NewAnnotation { page, style: Style::default_for(&shape), shape, contents: String::new(), author: "Ada".into() })
}

/// "Page 1" at 24 pt from x 20: the "1" starts near x 82.7 (Helvetica widths).
const ONE: [f64; 4] = [80.0, 140.0, 100.0, 180.0];

/// The font of the fixtures: Helvetica with the widths of "Page 1" (and a flat 556 elsewhere),
/// so neither the redaction nor the proof's own placement has to guess a width.
const FONT: &str = "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /FirstChar 32 /LastChar 122 /Widths [278 556 556 556 556 556 556 556 556 556 556 556 556 556 556 556 556 556 556 556 556 556 556 556 556 556 556 556 556 556 556 556 556 556 556 556 556 556 556 556 556 556 556 556 556 556 556 556 667 556 556 556 556 556 556 556 556 556 556 556 556 556 556 556 556 556 556 556 556 556 556 556 556 556 556 556 556 556 556 556 556 556 556 556 556 556 556 556 556 556 556] >>";

/// A PDF from object bodies (object `n + 1` is `objs[n]`), with `/Root 1 0 R`.
fn pdf(objs: &[&str]) -> Vec<u8> {
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

fn text_stream(body: &str) -> String {
    format!("<< /Length {} >>\nstream\n{body}\nendstream", body.len())
}

/// One page showing "Page 1".
fn one_page() -> Vec<u8> {
    pdf(&[
        "<< /Type /Catalog /Pages 2 0 R >>",
        "<< /Type /Pages /Kids [4 0 R] /Count 1 /MediaBox [0 0 200 300] >>",
        FONT,
        "<< /Type /Page /Parent 2 0 R /Contents 5 0 R /Resources << /Font << /F1 3 0 R >> >> >>",
        &text_stream("BT /F1 24 Tf 20 150 Td (Page 1) Tj ET"),
    ])
}

/// `bytes` opened with `password` (the credentials of a protected document).
fn session_open(bytes: Vec<u8>, password: Option<&str>) -> (Session, DocId) {
    let mut s = Session::new().with_clock(|| 1_700_000_000);
    let id = s.open("fixture.pdf", None, Arc::new(bytes), password).expect("opens");
    (s, id)
}

/// `n` lines, each showing a different word, one mark over each line: a bulk of marks like the
/// ones the search-and-redact patterns leave behind.
fn bulk(n: usize) -> (Session, DocId) {
    let mut body = String::new();
    for i in 0..n {
        body.push_str(&format!("BT /F1 18 Tf 20 {} Td (SECRET{i:02}) Tj ET\n", 750 - i * 8));
    }
    let mut objs: Vec<String> =
        vec!["<< /Type /Catalog /Pages 2 0 R >>".into(), "<< /Type /Pages /Kids [4 0 R] /Count 1 /MediaBox [0 0 612 792] >>".into(), FONT.into()];
    objs.push("<< /Type /Page /Parent 2 0 R /Contents 5 0 R /Resources << /Font << /F1 3 0 R >> >> >>".into());
    objs.push(text_stream(body.trim_end()));
    let mut marks: Vec<Edit> = Vec::new();
    let mut s_bytes = pdf(&objs.iter().map(String::as_str).collect::<Vec<_>>());
    let mut s = Session::new().with_clock(|| 1_700_000_000);
    let id = s.open("bulk.pdf", None, Arc::new(std::mem::take(&mut s_bytes)), None).expect("opens");
    for i in 0..n {
        let y = 750.0 - i as f64 * 8.0;
        marks.push(mark(0, [20.0, y - 4.0, 130.0, y + 14.0]));
    }
    let edit = match <[Edit; 1]>::try_from(marks) {
        Ok([one]) => one,
        Err(marks) => Edit::Batch { label: "Mark for redaction".into(), edits: marks },
    };
    s.apply(id, edit).expect("marks");
    (s, id)
}

/// A comment whose text sits inside the marked region: applying removes the annotation, and its
/// `/Contents` leaves the document by another route than the page's glyphs.
fn with_comment(contents: &str) -> Vec<u8> {
    pdf(&[
        "<< /Type /Catalog /Pages 2 0 R >>",
        "<< /Type /Pages /Kids [4 0 R] /Count 1 /MediaBox [0 0 200 300] >>",
        FONT,
        "<< /Type /Page /Parent 2 0 R /Contents 5 0 R /Annots [6 0 R] /Resources << /Font << /F1 3 0 R >> >> >>",
        &text_stream("BT /F1 24 Tf 20 150 Td (Page 1) Tj ET"),
        &format!("<< /Type /Annot /Subtype /Text /Rect [80 140 100 180] /Contents ({contents}) >>"),
    ])
}

#[test]
fn applying_redactions_proves_the_output_and_the_outcome_surfaces_it() {
    let (mut s, id) = session_with(one_page());
    s.apply(id, mark(0, ONE)).unwrap();
    s.apply(id, Edit::ApplyRedactions { pages: None }).unwrap();
    let outcome = s.get(id).unwrap().last_redaction().expect("the apply carried an outcome");
    assert_eq!(outcome.report.marks, 1);
    assert!(outcome.report.glyphs > 0);
    assert!(outcome.proof.passed(), "{}", outcome.proof.to_text());
    assert_eq!(outcome.proof.entries.len(), 1);
    assert_eq!(outcome.proof.entries[0].status, pdfcraft_redact::verify::Status::Verified);
    assert!(outcome.proof.entries[0].runs_removed > 0);
    // The proof is about what a save writes: the bytes Save will write pass the same check.
    let bytes = s.save_bytes(id).unwrap();
    assert_eq!(s.check_save(id, &bytes), Ok(()));
    assert_eq!(s.get(id).unwrap().last_redaction(), Some(outcome));
}

#[test]
fn a_tampered_save_fails_the_check_and_the_reason_names_counts_only() {
    let original = one_page();
    let (mut s, id) = session_with(original.clone());
    s.apply(id, mark(0, ONE)).unwrap();
    s.apply(id, Edit::ApplyRedactions { pages: None }).unwrap();
    let good = s.save_bytes(id).unwrap();
    assert_eq!(s.check_save(id, &good), Ok(()));
    // The unredacted file reopens with the right page count but still holds the removed text.
    let err = s.check_save(id, &Arc::new(original)).unwrap_err();
    let m = match &err {
        EditError::SaveCheck(m) => m.as_str(),
        other => panic!("{other:?}"),
    };
    assert!(m.contains("survivor"), "{m}");
    assert!(!m.contains("Page"), "{m}");
    // Bytes nobody can reopen are refused by the reopen check as before.
    let truncated = Arc::new(good[..good.len() / 2].to_vec());
    assert!(s.check_save(id, &truncated).is_err());
}

#[test]
fn a_bulk_of_marks_proves_within_the_sweep_budget() {
    let (mut s, id) = bulk(90);
    s.apply(id, Edit::ApplyRedactions { pages: None }).expect("a bulk redaction proves");
    let outcome = s.get(id).unwrap().last_redaction().expect("an outcome");
    assert!(outcome.proof.passed(), "{}", outcome.proof.to_text());
    assert_eq!(outcome.proof.unswept(), 0, "{}", outcome.proof.to_text());
    assert_eq!(outcome.proof.entries.len(), 90);
    let bytes = s.save_bytes(id).unwrap();
    assert_eq!(s.check_save(id, &bytes), Ok(()));
}

#[test]
fn a_protected_document_is_proven_with_its_open_password() {
    let mut cos = pdfcraft_cos::Document::open(Arc::new(one_page())).unwrap();
    cos.set_encryption(&pdfcraft_cos::NewEncryption {
        algorithm: pdfcraft_cos::Algorithm::Aes256,
        user_password: "pw-secret",
        owner_password: "owner-secret",
        permissions: -1,
        encrypt_metadata: true,
        seed: [7; 32],
    })
    .unwrap();
    let bytes = pdfcraft_cos::write_full(&cos, &pdfcraft_cos::SaveOptions::default()).unwrap();
    let (mut s, id) = session_open(bytes, Some("pw-secret"));
    s.apply(id, mark(0, ONE)).unwrap();
    // The proof at apply time reads the encrypted output back with the open password.
    s.apply(id, Edit::ApplyRedactions { pages: None }).expect("proves with the credentials");
    assert!(s.get(id).unwrap().last_redaction().expect("an outcome").proof.passed());
    let saved = s.save_bytes(id).unwrap();
    assert!(saved.windows(8).any(|w| w == b"/Encrypt"), "the save stays encrypted");
    // The save-time sweep reads the bytes with the same credentials.
    assert_eq!(s.check_save(id, &saved), Ok(()));
}

#[test]
fn later_edits_undo_redo_and_revert_drop_the_stale_claims() {
    let (mut s, id) = session_with(one_page());
    s.apply(id, mark(0, ONE)).unwrap();
    s.apply(id, Edit::ApplyRedactions { pages: None }).unwrap();
    assert!(s.get(id).unwrap().last_redaction().is_some());
    // Any later edit makes the outcome a statement about an earlier document.
    s.apply(id, Edit::RotatePages { pages: vec![0], degrees: 90 }).unwrap();
    assert!(s.get(id).unwrap().last_redaction().is_none());
    s.undo(id).unwrap();
    assert!(s.get(id).unwrap().last_redaction().is_none());
    s.redo(id).unwrap();
    assert!(s.get(id).unwrap().last_redaction().is_none());
    // Undoing the redaction itself drops the claims too: the content is back.
    s.undo(id).unwrap();
    assert!(s.get(id).unwrap().last_redaction().is_none());
    s.undo(id).unwrap();
    assert!(s.get(id).unwrap().last_redaction().is_none());
    assert!(s.get(id).unwrap().redaction_marks() == 1);
    s.apply(id, Edit::ApplyRedactions { pages: None }).unwrap();
    assert!(s.get(id).unwrap().last_redaction().is_some());
    s.revert(id).unwrap();
    let d = s.get(id).unwrap();
    assert!(d.last_redaction().is_none());
    assert_eq!(d.redaction_marks(), 0, "revert goes back to the file as opened, which never held the mark");
}

#[test]
fn a_successful_save_drops_the_snapshot_but_keeps_the_outcome() {
    let (mut s, id) = session_with(one_page());
    s.apply(id, mark(0, ONE)).unwrap();
    s.apply(id, Edit::ApplyRedactions { pages: None }).unwrap();
    assert!(s.get(id).unwrap().redaction_snapshot().is_some());
    let bytes = s.save_bytes(id).unwrap();
    assert_eq!(s.check_save(id, &bytes), Ok(()));
    assert!(s.get(id).unwrap().redaction_snapshot().is_some(), "the check only borrows it");
    s.mark_saved(id, bytes, None).unwrap();
    assert!(s.get(id).unwrap().redaction_snapshot().is_none());
    assert!(s.get(id).unwrap().last_redaction().is_some(), "the report of what was removed stays");
}

#[test]
fn mark_saved_refuses_bytes_that_still_hold_the_removed_text() {
    let original = one_page();
    let (mut s, id) = session_with(original.clone());
    s.apply(id, mark(0, ONE)).unwrap();
    s.apply(id, Edit::ApplyRedactions { pages: None }).unwrap();
    let good = s.save_bytes(id).unwrap();
    // The unredacted file is refused as the saved file too, and the claims stay: the snapshot
    // only goes once bytes that pass its check are recorded.
    let err = s.mark_saved(id, Arc::new(original), None).unwrap_err();
    assert!(matches!(&err, EditError::SaveCheck(m) if m.contains("survivor")), "{err:?}");
    assert!(s.get(id).unwrap().redaction_snapshot().is_some());
    s.mark_saved(id, good, None).unwrap();
    assert!(s.get(id).unwrap().redaction_snapshot().is_none());
}

#[test]
fn a_batch_cannot_apply_redactions_around_the_proof() {
    // The proof's snapshot is recorded before the edit runs, so a batch that marks and applies
    // in one step would redact what no snapshot ever saw. It is refused outright.
    let (mut s, id) = session_with(one_page());
    let edit = Edit::Batch { label: "Mark and apply".into(), edits: vec![mark(0, ONE), Edit::ApplyRedactions { pages: None }] };
    let err = s.apply(id, edit).unwrap_err();
    assert!(matches!(err, EditError::Invalid(ref m) if m.contains("batch")), "{err}");
    assert_eq!(s.get(id).unwrap().redaction_marks(), 0, "nothing was changed");
    assert!(s.get(id).unwrap().last_redaction().is_none());
}

#[test]
fn a_failed_proof_fails_the_edit_and_changes_nothing() {
    // A stream nobody can decode can't be swept: the proof is incomplete, so the edit is refused.
    let junk = "<< /Filter /NoSuchDecode /Length 8 >>\nstream\n\u{1}\u{2} junk\nendstream";
    let bytes = pdf(&[
        "<< /Type /Catalog /Pages 2 0 R >>",
        "<< /Type /Pages /Kids [4 0 R] /Count 1 /MediaBox [0 0 200 300] >>",
        FONT,
        "<< /Type /Page /Parent 2 0 R /Contents 5 0 R /Extra 6 0 R /Resources << /Font << /F1 3 0 R >> >> >>",
        &text_stream("BT /F1 24 Tf 20 150 Td (Page 1) Tj ET"),
        junk,
    ]);
    let (mut s, id) = session_with(bytes);
    s.apply(id, mark(0, ONE)).unwrap();
    let (working, saved) = (s.get(id).unwrap().bytes.clone(), s.save_bytes(id).unwrap());
    let err = s.apply(id, Edit::ApplyRedactions { pages: None }).unwrap_err();
    assert!(matches!(err, EditError::Redact(pdfcraft_redact::RedactError::ProofIncomplete { .. })), "{err:?}");
    assert!(!err.to_string().contains("Page 1"), "{err}");
    let d = s.get(id).unwrap();
    assert_eq!(d.bytes, working);
    assert_eq!(s.save_bytes(id).unwrap(), saved);
    assert_eq!(d.redaction_marks(), 1);
    assert_eq!(d.can_undo(), Some("Add redaction mark"));
    assert!(d.last_redaction().is_none());
}

#[test]
fn text_removed_with_a_comment_is_swept_through_the_snapshot_too() {
    let original = with_comment("call back SECRET-NUMBER-WORD");
    let (mut s, id) = session_with(original.clone());
    s.apply(id, mark(0, ONE)).unwrap();
    s.apply(id, Edit::ApplyRedactions { pages: None }).expect("the comment text goes with the mark");
    let good = s.save_bytes(id).unwrap();
    assert_eq!(s.check_save(id, &good), Ok(()));
    // The removed comment's text is looked for too: the file as opened still holds it, and is
    // refused with counts only.
    let err = s.check_save(id, &Arc::new(original)).unwrap_err();
    assert!(matches!(&err, EditError::SaveCheck(m) if m.contains("survivor")), "{err:?}");
}
