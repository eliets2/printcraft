# Security claims

Status: living register, revised whenever a security claim or its evidence changes.

A page in this repository may say what the security code does. Each such statement belongs in
the table below, with the test that shows it, or with the label `UNV` when no test shows it yet.
`cargo test -p xtask` reads the table: a scanned line that uses never-claim wording fails unless
it cites a `[CLAIM-nnn]` id whose row names evidence, and a line that uses qualification wording
fails unless it cites a row that records the limit, or is listed under "Reviewed uses" below. A
row that claims something the code does while naming a qualification must be `not-claimed` with
its reason. The scanned files are `docs/**/*.md`, `crates/*/README.md`, the `notes` lines of
`parity/acrobat-features.toml` and the `ROADMAP.md` lines that name a security term. The wording
lists and their test live in `xtask/src/claims.rs`.

Columns:

- **Claim ID** — `CLAIM-nnn`.
- **Claim** — one sentence about what the code does; for a `not-claimed` row, what the project
  does not promise.
- **Test ID or UNV** — `path.rs::test_fn`, or `UNV` with a note saying why no test exists.
- **Status** — `claimed` (the code does it and the test shows it) or `not-claimed` (the register
  records that the project makes no such promise).
- **Residual** — what the claim does not cover; for a `not-claimed` row, the reason.
- **Owner** — the area that keeps the row true: `redaction`, `audit`, `encryption`, `parser`,
  `signatures`, `structure`.

| Claim ID | Claim | Test ID or UNV | Status | Residual | Owner |
|---|---|---|---|---|---|
| CLAIM-001 | Applying a redaction removes the glyphs under the mark; the text beside it stays. | `crates/redact/src/tests.rs::glyphs_under_a_mark_go_and_the_rest_stays_put` | claimed | Text drawn as an image of the page is covered by the image row, not this one. | redaction |
| CLAIM-002 | Pixels under a mark in an image are cleared, or the image is removed. | `crates/redact/src/tests.rs::images_are_removed_or_have_their_pixels_cleared` | claimed | Only images the pass reaches while walking the redacted pages. | redaction |
| CLAIM-003 | Vector paths under a mark are removed or clipped, and inline images under it are removed. | `crates/redact/src/tests.rs::vectors_are_removed_or_clipped_and_inline_images_go` | claimed | Only the content streams the pass rewrites. | redaction |
| CLAIM-004 | A redacted image or form does not survive a full-rewrite save. | `crates/redact/src/tests.rs::redacted_images_and_forms_do_not_survive_a_full_rewrite_save` | claimed | Redaction is saved with a full rewrite, so the earlier revision is dropped. | redaction |
| CLAIM-005 | Hidden optional content under a mark is removed, in a form and in nested marked blocks. | `crates/redact/src/tests.rs::hidden_ocmd_content_in_a_form_and_nested_marked_blocks_is_removed` | claimed | Only hidden optional content on the redacted pages. | redaction |
| CLAIM-006 | The coverage audit reports text under a cover that was never applied. | `crates/audit/src/tests.rs::faux_redaction_is_found` | claimed | The audit reads content streams, not a rendered image. | audit |
| CLAIM-007 | The coverage audit fails closed on a stream it cannot decode. | `crates/audit/src/tests.rs::audit_fails_closed_on_undecodable_stream` | claimed | A stream that decodes but is not understood is reported as a bound, not as a finding. | audit |
| CLAIM-008 | A created file authenticates its owner and its user password. | `crates/crypt/src/tests.rs::created_encryption_authenticates_owner_and_user` | claimed | The handler's own authentication; what another tool can do with the file is not covered. | encryption |
| CLAIM-009 | Locked permissions limit what an edited document grants. | `crates/crypt/src/tests.rs::perms_limits_what_an_edited_p_grants` | claimed | PdfCraft and cooperating viewers honour the permission bits; a tool that ignores them is not covered. | encryption |
| CLAIM-010 | An unsupported security handler is reported, never opened as plain text. | `crates/crypt/src/tests.rs::unsupported_handlers_are_reported` | claimed | Which handlers are supported is a property of the crypt crate's list, not of this row. | encryption |
| CLAIM-011 | Garbage input does not crash the parser. | `crates/cos/src/parser.rs::garbage_never_panics` | claimed | One synthetic corpus, not every byte sequence. | parser |
| CLAIM-012 | A trusted signer whose certificate may not sign documents yields a signature that is not valid. | `crates/sign/tests/it/chain.rs::a_trusted_signer_whose_certificate_may_not_sign_is_not_valid` | claimed | Validation is against the trust store the caller supplies. | signatures |
| CLAIM-013 | Validation data and XMP added after signing are not a tampering change. | `crates/sign/tests/it/pdf.rs::validation_data_and_xmp_added_after_signing_are_not_tampering` | claimed | Only the change classes the classifier knows; an unknown object is reported as a change. | signatures |
| CLAIM-014 | No claim that PdfCraft signatures are qualified signatures under eIDAS or any national scheme. | UNV: no accredited scheme is checked | not-claimed | No accredited scheme, qualified certificate or signature-creation device is inspected; verdicts are against a trust store. | signatures |
| CLAIM-015 | No claim of approval by the AATL or EUTL trust lists. | UNV: neither list is shipped | not-claimed | Only the trust store the caller supplies is used; neither list is read. | signatures |
| CLAIM-016 | No claim that a signature has legal effect or is legally binding. | UNV: not a property of the code | not-claimed | PdfCraft reports what the bytes show; legal effect is a question for the parties and their advisers. | signatures |
| CLAIM-017 | No claim that a redacted file makes the removed content unrecoverable anywhere. | UNV: other copies are outside the file | not-claimed | Removal is from the saved file; earlier versions, other copies and backups are outside its reach. | redaction |
| CLAIM-018 | No claim of PDF/A, PDF/UA or PDF/X conformance. | UNV: no external validator result is recorded yet | not-claimed | The structural checks report the rules a document breaks; they are not a validation of the standard. | structure |
| CLAIM-019 | No claim that PdfCraft encryption or passwords are unbreakable, uncrackable or military-grade. | UNV: no strength measurement is recorded | not-claimed | The ciphers come from the standard, chosen per handler; this project measures no strength. | encryption |
| CLAIM-020 | No claim that a redacted area is permanently removed, or that a redaction leaves no personal data in the file (a complete PII removal). | UNV: the audit reports what it can read, not every copy | not-claimed | The pass removes what it reaches in the saved file; a copy elsewhere, or data it cannot read, is outside the claim. | redaction |
| CLAIM-021 | No claim that a signature is legally valid or certified, that the product is eIDAS-compliant, that a visible seal is tamper-proof, or that a behaviour is Acrobat-verified. | UNV: no scheme, seal or external run is checked | not-claimed | Verdicts come from the trust store the caller supplies and from the revision diff; no scheme, certification body or external viewer result is consulted. | signatures |
| CLAIM-022 | No claim that a feature is Acrobat-compatible without a recorded Acrobat run. | UNV: no Acrobat run is recorded | not-claimed | Acrobat is not part of the automated checks; behaviour is compared against the specification and against what the report shows. | signatures |
| CLAIM-023 | No claim that a signature verdict is valid without the revocation and time checks the report names. | UNV: no characterisation of that reporting is recorded yet | not-claimed | The checks a run performed are named in its report; this register claims no verdict is complete. | signatures |
| CLAIM-024 | No claim about glyph-position channels stronger than that no known attack recovers them, which only a recorded study supports. | UNV: no such study is recorded | not-claimed | Without a study, position channels are reported as a limit, never as a property of the file. | redaction |

## Reviewed uses

A qualification word on an existing line that names no security property is listed here with the
text that identifies the line, so the wording test can tell old text from a new sentence. A line
that uses one of these words fails unless it cites a register row that records the limit, or is
one of the lines below. Every row is checked against the document it names: an entry whose text no
longer appears on a line that uses the word fails the test, so the list cannot outlive the text it
describes. The claim cell names the row whose limit covers the use, or `none` when the line names
no security property at all. An entry keeps excusing its word when the line it names is edited
around that text; a different sentence about the same word needs its own row or a citation.

| File | Word | Line contains | Claim | Why this line names no security property |
|---|---|---|---|---|
| docs/localization-parity.md | qualified | not yet qualified | none | names right-to-left workflows tested nowhere yet |
| docs/roadmap.md | qualified | Native workflow qualified on macOS | none | names a workflow tested on one platform |
| docs/pdf-spec-parity.md | aatl | AATL out of scope | CLAIM-015 | records that the trust list is not shipped |
| docs/pdf-spec-parity.md | eutl | OS and EUTL planned | CLAIM-015 | records that the trust list is not shipped |
| docs/gaps.md | eutl | `sign.eutl` planned | CLAIM-015 | names the tool id of a feature not shipped yet |
| ROADMAP.md | qualified | Telekom qualified-signature PDF | CLAIM-014 | historical note about a file another product produced |
| parity/acrobat-features.toml | qualified | regrouping can be qualified | none | names a pending test of text alignment |
| parity/acrobat-features.toml | qualified | are not yet qualified | none | names right-to-left workflows tested nowhere yet |
| parity/acrobat-features.toml | certified | certified bars | none | names the interface bar that shows a signature's certification level |
| parity/acrobat-features.toml | certified | a certified form | none | names a certification level that a save keeps |
| ROADMAP.md | aatl | AATL/EUTL trust lists | CLAIM-015 | records that the trust lists are not read |
| ROADMAP.md | eutl | AATL/EUTL trust lists | CLAIM-015 | records that the trust lists are not read |
| parity/acrobat-features.toml | qualified | found by fully-qualified name | none | names a field's fully qualified name |
| parity/acrobat-features.toml | qualified | Still not qualified | none | names right-to-left workflows tested nowhere yet |

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-11-04 | minor | Created: the first register rows, and the wording test that reads them. |
| 2026-11-04 | minor | Rows for the rest of the never-claim bullets; the qualification wording checked in the scanned documents, with its reviewed uses; and a path check that resolves both sides. |
