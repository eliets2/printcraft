//! Characterization of the signature verdicts and details recorded for the cases the
//! revocation change (S-19) will move.
//!
//! These tests record what `signatures()` reports today, not what it should report: a case
//! listed as wrong today passes while it is wrong, and its target in `TARGETS` names what the
//! owner chose it should say (decisions OD-1 and OD-2).
//!
//! | Case | Current | Target |
//! |---|---|---|
//! | 1. `openssl-signed.pdf`, signer trusted, no `/DSS` | `Valid` (wrong today) | `Unknown`: revocation was not checked |
//! | 2. the same file, empty trust store (control) | `Unknown` | `Unknown` |
//! | 3a. `/M` removed from the signed bytes | `Invalid` | `Invalid` |
//! | 3b. `/M` removed by a later revision | `Invalid` | `Invalid` |
//! | 5. one later comment annotation | `Valid` | `Unknown` |
//!
//! Case 4 (a file with neither `/M` nor a CMS `signingTime`) is not tested here: both times the
//! fixture carries come from the tool that made it, and a file with neither would need a new
//! fixture this card does not add. The residual is for `docs/security-claims.md` (S-01).
//!
//! The fixtures are the repository's own (`tests/data`). Nothing here opens a network
//! connection or starts a process.

use std::sync::Arc;

use pdfcraft_cos::{Dict, Document, ObjRef, Object, PdfString, SaveOptions, write_incremental};
use pdfcraft_sign::x509::load_certificates;
use pdfcraft_sign::{Certificate, Modification, SignatureInfo, Status, Time, TrustStore, kind, signatures};

fn data(name: &str) -> Vec<u8> {
    std::fs::read(format!("{}/tests/data/{name}", env!("CARGO_MANIFEST_DIR"))).unwrap()
}

fn open(bytes: &[u8]) -> Document {
    Document::open(Arc::new(bytes.to_vec())).unwrap()
}

/// The certificate the `openssl-signed.pdf` signature was made with and embeds (`rsa.crt.pem`).
fn signer() -> Certificate {
    load_certificates(&data("rsa.crt.pem")).unwrap().remove(0)
}

/// The one signature the fixture lists, validated against `trust`.
fn only_signature(bytes: &[u8], trust: &TrustStore) -> SignatureInfo {
    let doc = open(bytes);
    let mut listed = signatures(&doc, bytes, trust);
    assert_eq!(listed.len(), 1, "one signature: {listed:?}");
    listed.remove(0)
}

/// Whether one of the details says `needle`; `Detail::pattern` is the stable wording.
fn said(info: &SignatureInfo, needle: &str) -> bool {
    info.details.iter().any(|d| d.pattern.contains(needle))
}

/// The target verdict the owner chose for each case (OD-1, OD-2) and why.
///
/// A case whose target differs from what its test asserts is characterized wrong today: it
/// passes while the code is wrong. Flip the characterization when the verdict change lands; see
/// PLAN_SECURITY S-19.
const TARGETS: &[(&str, Status, &str)] = &[
    ("case 1", Status::Unknown, "revocation not checked: with no /DSS no evidence was examined"),
    ("case 2", Status::Unknown, "correct today: the signer is not in the trust store"),
    ("case 3a", Status::Invalid, "the signed bytes changed, so the signature cannot be checked"),
    ("case 3b", Status::Invalid, "a later revision rewrote the signature dictionary"),
    ("case 5", Status::Unknown, "the revocation gap of case 1 persists after a permitted change"),
];

/// The target verdict for `case`.
fn target(case: &str) -> Status {
    TARGETS.iter().find(|(id, _, _)| *id == case).map(|(_, status, _)| *status).expect("every case has a target")
}

/// Case 1, and the reason it is characterized wrong today (the S1 issue).
///
/// The fixture is signed with the self-signed RSA certificate in `rsa.crt.pem`, that certificate
/// is in the trust store, and the file has no `/DSS` at all. The validator reports `Valid`, and
/// nothing in the details says where revocation evidence came from: with no `/DSS` it examines
/// none and treats the certificate as not revoked.
///
/// `tests/data/ca.crt.pem` is a different certificate (a self-signed P-256 test root, see
/// `tests/data/README.md` and `tests/it/crypto.rs`): it does not issue this signer, so trusting
/// it would leave the signer untrusted and the verdict `Unknown` for the wrong reason.
#[test]
fn case_1_a_trusted_signer_without_a_dss_reads_valid() {
    let bytes = data("openssl-signed.pdf");
    let doc = open(&bytes);
    let catalog = doc.get(doc.root().unwrap());
    assert!(!catalog.as_dict().is_some_and(|c| c.contains(b"DSS")), "the fixture has no /DSS");
    let s = only_signature(&bytes, &TrustStore { certs: vec![signer()] });
    assert_eq!(s.status, Status::Valid, "{:?}", s.details);
    assert_eq!(s.modification, Modification::None);
    assert!(said(&s, "has not been modified since this signature was applied"));
    assert!(said(&s, "The signer's identity is valid."));
    assert!(said(&s, "Signing time is from the clock on the signer's computer."));
    // The signing time is the CMS signingTime attribute, not the file's `/M` (12:00:00).
    assert_eq!(s.date.as_deref(), Some("D:20261002120000Z"));
    assert_eq!(s.signing_time, Some(Time { year: 2026, month: 10, day: 2, hour: 6, minute: 39, second: 53 }));
    // No revocation evidence was examined, and no detail mentions any.
    assert!(!said(&s, "has been revoked"), "{:?}", s.details);
    assert!(!said(&s, "revocation information"), "{:?}", s.details);
    // Wrong today: `Valid` claims more than the check covers. Flip when S-19 lands.
    assert_eq!(target("case 1"), Status::Unknown);
}

/// Case 2, the control: the same signature with an empty trust store, which reads `Unknown`.
#[test]
fn case_2_the_same_signature_without_trust_reads_unknown() {
    let bytes = data("openssl-signed.pdf");
    let s = only_signature(&bytes, &TrustStore::default());
    assert_eq!(s.status, Status::Unknown, "{:?}", s.details);
    assert_eq!(s.modification, Modification::None);
    assert!(said(&s, "The signer's identity is unknown because it has not been included in your list of trusted certificates"));
    assert!(!said(&s, "identity is valid"));
    // Correct today: the target is what the code already says.
    assert_eq!(target("case 2"), s.status);
}

/// Case 3a: `/M` removed from the signed revision by a byte edit.
///
/// The fixture's `/ByteRange` is `[0 766 17152 236]` and `/M (D:20261002120000Z)` sits inside the
/// first range: the signature dictionary is signed bytes. Replacing the entry with spaces keeps
/// every offset, so the range still excludes exactly the contents, and changes only the digest;
/// the validator stops at "altered or corrupted" before it reads any signing time.
///
/// The CMS SignerInfo does carry `signingTime` (OID 1.2.840.113549.1.9.5, 261002063953Z), the
/// attribute `crates/sign/src/pdf/validate.rs` prefers over `/M`; case 3b exercises that with a
/// signature that still verifies.
#[test]
fn case_3a_removing_m_from_the_signed_bytes_reads_invalid() {
    let bytes = data("openssl-signed.pdf");
    let entry = b"/M (D:20261002120000Z)";
    let at = bytes.windows(entry.len()).position(|w| w == entry).expect("the fixture has /M");
    let mut edited = bytes.clone();
    edited[at..at + entry.len()].fill(b' ');
    assert_eq!(edited.len(), bytes.len(), "spaces keep every offset");
    let s = only_signature(&edited, &TrustStore { certs: vec![signer()] });
    assert_eq!(s.status, Status::Invalid, "{:?}", s.details);
    assert!(said(&s, "The document has been altered or corrupted since the signature was applied."));
    assert_eq!(s.date, None, "the signature dictionary has no /M");
    assert_eq!(s.signing_time, None, "the time is read only after the digest check");
    assert_eq!(target("case 3a"), Status::Invalid);
}

/// Case 3b: a later revision rewrites the signature dictionary with `/M` removed.
///
/// Written by this crate's incremental writer (`write_incremental` with the default
/// `SaveOptions`, so no `/ModDate` is stamped and nothing else is rewritten), which leaves
/// `/ByteRange` and `/Contents` untouched: the signature still verifies. With `/M` gone the
/// signing time can only be the CMS `signingTime` attribute, which is the one the validator uses
/// anyway. The verdict is `Invalid` for another reason: no permission allows rewriting the
/// dictionary of a signature that has already been signed.
#[test]
fn case_3b_a_later_revision_without_m_keeps_the_cms_signing_time() {
    let bytes = data("openssl-signed.pdf");
    let sig = ObjRef { num: 7, generation: 0 };
    let mut doc = open(&bytes);
    doc.update_dict(sig, |d| {
        d.remove(b"M");
    })
    .unwrap();
    let edited = write_incremental(&doc, &SaveOptions::default()).unwrap();
    assert!(edited.starts_with(&bytes), "an incremental update");
    let s = only_signature(&edited, &TrustStore { certs: vec![signer()] });
    assert_eq!(s.date, None, "the later revision has no /M");
    assert_eq!(s.signing_time, Some(Time { year: 2026, month: 10, day: 2, hour: 6, minute: 39, second: 53 }));
    assert_eq!(s.modification, Modification::Disallowed(vec![kind::OTHER]), "{:?}", s.details);
    assert!(said(&s, "in ways it does not permit"));
    assert_eq!(s.status, Status::Invalid, "{:?}", s.details);
    assert_eq!(target("case 3b"), s.status);
}

/// Case 5: a benign change after signing, one later revision that only adds a comment annotation
/// to the page.
///
/// Written by this crate's incremental writer (`write_incremental` with `object_streams: false`,
/// the way `tests/it/pdf.rs` appends one). Under an approval signature a comment is a permitted
/// change, so the verdict stays `Valid`; the revocation gap of case 1 is still there, which is
/// why the target is `Unknown`.
#[test]
fn case_5_a_later_comment_annotation_keeps_the_verdict_valid() {
    let bytes = data("openssl-signed.pdf");
    let mut doc = open(&bytes);
    let mut comment = Dict::new();
    comment.set(b"Type".to_vec(), Object::name("Annot"));
    comment.set(b"Subtype".to_vec(), Object::name("Text"));
    comment.set(b"Rect".to_vec(), Object::Array(vec![Object::Int(10), Object::Int(10), Object::Int(30), Object::Int(30)]));
    comment.set(b"Contents".to_vec(), PdfString::text("A note"));
    let r = doc.add(Object::Dict(comment));
    let page = ObjRef { num: 3, generation: 0 };
    doc.update_dict(page, |p| {
        if let Some(Object::Array(a)) = p.get_mut(b"Annots") {
            a.push(Object::Ref(r));
        }
    })
    .unwrap();
    let edited = write_incremental(&doc, &SaveOptions { object_streams: false, ..SaveOptions::default() }).unwrap();
    assert!(edited.starts_with(&bytes), "an incremental update");
    let s = only_signature(&edited, &TrustStore { certs: vec![signer()] });
    assert_eq!(s.status, Status::Valid, "{:?}", s.details);
    assert_eq!(s.modification, Modification::Allowed(vec![kind::COMMENTS]), "{:?}", s.details);
    assert!(said(&s, "but the changes are permitted"));
    assert_eq!(target("case 5"), Status::Unknown);
}

/// The target table lists no case twice and says why each one has its target.
#[test]
fn the_target_table_names_each_case_once() {
    let mut ids: Vec<&str> = TARGETS.iter().map(|(id, _, _)| *id).collect();
    let listed = ids.len();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), listed, "no case is listed twice: {ids:?}");
    for (id, _, why) in TARGETS {
        assert!(!why.is_empty(), "{id} says why it has that target");
    }
}
