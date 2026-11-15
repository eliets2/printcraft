//! The security-claims register and the wording test over the repository's documents.
//!
//! `docs/security-claims.md` is the register: one row per statement about what the security code
//! does, with the test that shows it or `UNV` when no test shows it yet. This module reads the
//! repository as text (it never executes anything, and it reads no path outside the repository
//! root) and checks three things:
//!
//! - every row is well formed: its id is unique and looks like `CLAIM-nnn`, a `path.rs::test_fn`
//!   entry names a function the source tree has, and a `not-claimed` row gives its reason;
//! - a scanned line that uses never-claim wording must cite a register row that names evidence;
//! - a scanned line that uses qualification wording must cite a register row that records the
//!   limit, or be a reviewed use: a qualification word on an existing line of a document that
//!   names no security property. Every reviewed use must still match a line of its document, so a
//!   stale entry fails instead of excusing a sentence that changed or newly appeared.
//!
//! Scanned: `docs/**/*.md`, `crates/*/README.md`, the `notes` lines of
//! `parity/acrobat-features.toml`, and the `ROADMAP.md` lines that name a security term. The
//! register itself and Rust code comments are not scanned. A `notes` value written across several
//! lines would not be scanned; every value in the manifest is one line today.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

/// The register, relative to the repository root.
const REGISTER: &str = "docs/security-claims.md";

/// The largest prose document the scan reads; a bigger one is refused, never truncated.
const MAX_FILE_BYTES: u64 = 1024 * 1024;

/// The parity manifest is far larger than a prose page, and only its `notes` lines are kept.
const MAX_MANIFEST_BYTES: u64 = 8 * 1024 * 1024;

/// How deep the `docs/` walk goes.
const MAX_DEPTH: usize = 8;

/// A row the code backs with evidence.
const CLAIMED: &str = "claimed";

/// A stated limitation: the project makes no such promise.
const NOT_CLAIMED: &str = "not-claimed";

/// The never-claim wording of the security plan, without the words in `QUALIFICATION`: each of
/// these promises more than any test here shows, so a line that uses one must cite a register row
/// that names evidence.
const OVERCLAIM: &[&str] = &[
    "unbreakable",
    "uncrackable",
    "military-grade",
    "unrecoverable",
    "permanently removed",
    "complete pii removal",
    "acrobat-verified",
    "acrobat-compatible",
    "tamper-proof",
    "no known attack recovers",
];

/// Wording that names a qualification, a certification or a recognised trust list. A line that
/// uses one must cite a register row that records the limit, or be a reviewed use.
const QUALIFICATION: &[&str] = &["qualified", "eidas", "eidas-compliant", "legally valid", "legally binding", "certified", "aatl", "eutl"];

/// The claim cell of a reviewed use that names no security property at all.
const NO_CLAIM: &str = "none";

/// Stems that make a `ROADMAP.md` line worth scanning; the changelog is long and mostly about
/// features that have nothing to do with security.
const SECURITY_TERMS: &[&str] = &[
    "security",
    "encrypt",
    "decrypt",
    "password",
    "permission",
    "protect",
    "redact",
    "sanitiz",
    "signature",
    "signing",
    "certificate",
    "authenticat",
    "confidential",
    "privacy",
    "tamper",
];

/// One row of the register table.
struct Claim<'a> {
    id: &'a str,
    text: &'a str,
    evidence: &'a str,
    status: &'a str,
    residual: &'a str,
    area: &'a str,
}

impl Claim<'_> {
    /// Whether this row names evidence: a `path.rs::test_fn`, or the `UNV` label for a claim no
    /// test shows yet.
    fn has_evidence(&self) -> bool {
        !self.evidence.is_empty()
    }

    /// Whether this row records a limit: what the project does not promise, and why.
    fn records_a_limit(&self) -> bool {
        self.status == NOT_CLAIMED && !self.residual.is_empty()
    }
}

/// The table rows of a register: a row is a `|`-separated line whose first cell is a claim id.
fn parse_register(markdown: &str) -> Vec<Claim<'_>> {
    markdown
        .lines()
        .filter_map(|line| {
            let cells: Vec<&str> = line.trim().trim_matches('|').split('|').map(str::trim).collect();
            let [id, text, evidence, status, residual, area] = cells.as_slice() else { return None };
            if !id.starts_with("CLAIM-") {
                return None;
            }
            Some(Claim { id, text, evidence, status, residual, area })
        })
        .collect()
}

/// Whether `id` is a claim id: `CLAIM-` and three digits.
fn valid_claim_id(id: &str) -> bool {
    id.strip_prefix("CLAIM-").is_some_and(|n| n.len() == 3 && n.chars().all(|c| c.is_ascii_digit()))
}

/// What is wrong with the register itself, row by row.
fn register_errors(claims: &[Claim<'_>]) -> Vec<String> {
    let mut errors = Vec::new();
    let mut seen = HashSet::new();
    for c in claims {
        if !valid_claim_id(c.id) {
            errors.push(format!("{}: id must look like `CLAIM-nnn`", c.id));
        }
        if !seen.insert(c.id) {
            errors.push(format!("{}: duplicate id", c.id));
        }
        if c.text.is_empty() {
            errors.push(format!("{}: empty claim", c.id));
        }
        if !matches!(c.status, CLAIMED | NOT_CLAIMED) {
            errors.push(format!("{}: status {:?} must be `claimed` or `not-claimed`", c.id, c.status));
        }
        if c.evidence.is_empty() {
            errors.push(format!("{}: evidence must be a `path.rs::test_fn` or `UNV`", c.id));
        }
        if c.area.is_empty() {
            errors.push(format!("{}: empty owner", c.id));
        }
        if c.status == NOT_CLAIMED && c.residual.is_empty() {
            errors.push(format!("{}: a `not-claimed` row must give its reason in `residual`", c.id));
        }
        if c.status != NOT_CLAIMED && QUALIFICATION.iter().any(|w| contains_word(c.text, w)) {
            errors.push(format!("{}: a row about a qualification or a trust list must be `not-claimed`", c.id));
        }
    }
    errors
}

/// One reviewed use: a qualification word on a line of an existing document that names no
/// security property, registered so that new wording can be told apart from old text.
struct ReviewedUse<'a> {
    file: &'a str,
    word: &'a str,
    contains: &'a str,
    claim: &'a str,
    why: &'a str,
}

/// The reviewed-use rows of a register: a `|`-separated line of five cells whose first cell is a
/// file the scan reads.
fn parse_reviewed(markdown: &str) -> Vec<ReviewedUse<'_>> {
    markdown
        .lines()
        .filter_map(|line| {
            let cells: Vec<&str> = line.trim().trim_matches('|').split('|').map(str::trim).collect();
            let [file, word, contains, claim, why] = cells.as_slice() else { return None };
            if !(file.ends_with(".md") || file.ends_with(".toml")) {
                return None;
            }
            Some(ReviewedUse { file, word, contains, claim, why })
        })
        .collect()
}

/// What is wrong with the reviewed uses: an empty reason, a word the scan does not check, a claim
/// row that does not record a limit, and an entry that no longer matches a line of its document.
fn reviewed_errors(reviewed: &[ReviewedUse<'_>], documents: &[(String, String)], claims: &[Claim<'_>]) -> Vec<String> {
    let mut errors = Vec::new();
    for r in reviewed {
        let label = format!("{} {:?} {:?}:", r.file, r.word, r.contains);
        if r.why.is_empty() {
            errors.push(format!("{label} empty reason"));
        }
        if !QUALIFICATION.iter().any(|w| w.eq_ignore_ascii_case(r.word)) {
            errors.push(format!("{label} the word is not in the qualification list"));
        }
        if r.claim != NO_CLAIM && !claims.iter().any(|c| c.id == r.claim && c.records_a_limit()) {
            errors.push(format!("{label} {:?} must be a register row that records a limit, or `{NO_CLAIM}`", r.claim));
        }
        let still_used =
            documents.iter().any(|(name, text)| name == r.file && text.lines().any(|l| l.contains(r.contains) && contains_word(l, r.word)));
        if !still_used {
            errors.push(format!("{label} no line of the file uses the word with this text"));
        }
    }
    errors
}

/// Why an `evidence` entry does not resolve to a test in the source tree, if it doesn't. `UNV`
/// (with an optional note) is the label for a claim no test shows yet.
fn evidence_error(evidence: &str, root: &Path) -> Option<String> {
    if evidence.starts_with("UNV") {
        return None;
    }
    // The register writes the entry in backticks so it reads as code in the table.
    let evidence = evidence.trim_matches('`');
    let Some((path, func)) = evidence.split_once("::") else {
        return Some(format!("evidence {evidence:?} must be `path.rs::test_fn` or `UNV`"));
    };
    match read_inside(root, &root.join(path), MAX_FILE_BYTES) {
        Ok(src) if src.contains(&format!("fn {func}(")) => None,
        Ok(_) => Some(format!("evidence: no `fn {func}` in {path}")),
        Err(e) => Some(format!("evidence: {e}")),
    }
}

/// Whether `needle` appears in `haystack` as a whole word, ignoring case.
fn contains_word(haystack: &str, needle: &str) -> bool {
    let hay = haystack.to_lowercase();
    let pat = needle.to_lowercase();
    if pat.is_empty() {
        return false;
    }
    let mut from = 0;
    while let Some(i) = hay[from..].find(&pat) {
        let start = from + i;
        let end = start + pat.len();
        let before = hay[..start].chars().next_back().is_none_or(|c| !c.is_alphanumeric());
        let after = hay[end..].chars().next().is_none_or(|c| !c.is_alphanumeric());
        if before && after {
            return true;
        }
        from = end;
    }
    false
}

/// The claim ids a line cites, written `[CLAIM-nnn]`.
fn cited_ids(line: &str) -> Vec<&str> {
    let mut ids = Vec::new();
    let mut rest = line;
    while let Some(i) = rest.find("[CLAIM-") {
        let tail = &rest[i + 1..];
        let Some(end) = tail.find(']') else { break };
        ids.push(&tail[..end]);
        rest = &tail[end..];
    }
    ids
}

/// Whether the line cites a register row that names evidence: a test, or the `UNV` label.
fn cites_evidence(line: &str, claims: &[Claim<'_>]) -> bool {
    cited_ids(line).into_iter().any(|id| claims.iter().any(|c| c.id == id && c.has_evidence()))
}

/// Whether the line cites a register row that records a limit the project states.
fn cites_a_limit(line: &str, claims: &[Claim<'_>]) -> bool {
    cited_ids(line).into_iter().any(|id| claims.iter().any(|c| c.id == id && c.records_a_limit()))
}

/// Whether a reviewed use covers this word on this line of this file.
fn reviewed_covers(reviewed: &[ReviewedUse<'_>], file: &str, word: &str, line: &str) -> bool {
    reviewed.iter().any(|r| r.file == file && r.word.eq_ignore_ascii_case(word) && line.contains(r.contains))
}

/// The never-claim words in `line` that no cited row with evidence excuses.
fn over_claim_hits(line: &str, claims: &[Claim<'_>]) -> Vec<&'static str> {
    if cites_evidence(line, claims) {
        return Vec::new();
    }
    OVERCLAIM.iter().copied().filter(|w| contains_word(line, w)).collect()
}

/// The qualification words in `line` that no cited limit and no reviewed use excuses.
fn qualification_hits(file: &str, line: &str, claims: &[Claim<'_>], reviewed: &[ReviewedUse<'_>]) -> Vec<&'static str> {
    if cites_a_limit(line, claims) {
        return Vec::new();
    }
    QUALIFICATION.iter().copied().filter(|w| contains_word(line, w) && !reviewed_covers(reviewed, file, w, line)).collect()
}

/// Every finding in `documents` (name, text): wording with no excuse on its line.
fn scan_documents(documents: &[(String, String)], claims: &[Claim<'_>], reviewed: &[ReviewedUse<'_>]) -> Vec<String> {
    let mut findings = Vec::new();
    for (name, text) in documents {
        for (n, line) in text.lines().enumerate() {
            let at = format!("{name}:{}", n + 1);
            let over_demand = "needs a `[CLAIM-nnn]` citation of a row with evidence";
            findings.extend(over_claim_hits(line, claims).into_iter().map(|w| format!("{at}: {w:?} {over_demand}")));
            let limit_demand = "needs a `[CLAIM-nnn]` citation of a row that records the limit";
            findings.extend(qualification_hits(name, line, claims, reviewed).into_iter().map(|w| format!("{at}: {w:?} {limit_demand}")));
        }
    }
    findings
}

/// Keep the lines `keep` accepts; the others become empty, so line numbers stay true.
fn only_lines(text: &str, keep: impl Fn(&str) -> bool) -> String {
    text.lines().map(|line| if keep(line) { line } else { "" }).collect::<Vec<_>>().join("\n")
}

/// Read a file that lives inside `root`; anything else is an error, never a read. Both paths are
/// resolved before they are compared, so `..` and a symbolic link cannot lead out of the root, and
/// a link is refused even when it points back inside.
fn read_inside(root: &Path, path: &Path, cap: u64) -> Result<String, String> {
    let meta = fs::symlink_metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if meta.file_type().is_symlink() {
        return Err(format!("{}: a symbolic link is not followed", path.display()));
    }
    let root = fs::canonicalize(root).map_err(|e| format!("{}: {e}", root.display()))?;
    let real = fs::canonicalize(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if !real.starts_with(&root) {
        return Err(format!("{}: outside the repository root", path.display()));
    }
    if !meta.is_file() {
        return Err(format!("{}: not a file", path.display()));
    }
    if meta.len() > cap {
        return Err(format!("{}: larger than the {cap}-byte cap", path.display()));
    }
    fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))
}

/// Collect the `.md` files under `dir`, bounded in depth and confined to `root`. A symbolic link is
/// skipped, so a directory entry cannot lead the walk outside the repository.
fn markdown_files(dir: &Path, root: &Path, depth: usize, out: &mut Vec<PathBuf>) -> Result<(), String> {
    if depth > MAX_DEPTH {
        return Err(format!("{}: deeper than {MAX_DEPTH}", dir.display()));
    }
    let entries = fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("{}: {e}", dir.display()))?;
        // `DirEntry::file_type` reports a link as a link: it never follows one.
        let kind = entry.file_type().map_err(|e| format!("{}: {e}", dir.display()))?;
        let path = entry.path();
        if kind.is_symlink() || !path.starts_with(root) {
            continue;
        }
        if kind.is_dir() {
            markdown_files(&path, root, depth + 1, out)?;
        } else if path.extension().is_some_and(|e| e == "md") {
            out.push(path);
        }
    }
    Ok(())
}

/// Whether a `ROADMAP.md` line names a security term (a banned word always counts).
fn names_a_security_term(line: &str) -> bool {
    let line = line.to_lowercase();
    SECURITY_TERMS.iter().any(|t| line.contains(t)) || OVERCLAIM.iter().chain(QUALIFICATION).any(|w| contains_word(&line, w))
}

/// The documents the wording test walks, as (path from the root, text) pairs.
fn repo_documents(root: &Path) -> Result<Vec<(String, String)>, String> {
    let mut paths = Vec::new();
    markdown_files(&root.join("docs"), root, 0, &mut paths)?;
    let readmes = fs::read_dir(root.join("crates")).map_err(|e| format!("crates: {e}"))?;
    for entry in readmes {
        let readme = entry.map_err(|e| format!("crates: {e}"))?.path().join("README.md");
        // `symlink_metadata` reports a link as a link, so a linked README is never read through.
        if fs::symlink_metadata(&readme).is_ok_and(|m| m.is_file()) {
            paths.push(readme);
        }
    }
    let mut documents = Vec::new();
    for path in paths {
        // `docs/security-claims.md` in either separator, so the register is recognised on every host.
        let name = path.strip_prefix(root).unwrap_or(&path).to_string_lossy().replace('\\', "/");
        if name == REGISTER {
            continue;
        }
        documents.push((name, read_inside(root, &path, MAX_FILE_BYTES)?));
    }
    let roadmap = read_inside(root, &root.join("ROADMAP.md"), MAX_FILE_BYTES)?;
    documents.push(("ROADMAP.md".into(), only_lines(&roadmap, names_a_security_term)));
    let parity = read_inside(root, &root.join("parity/acrobat-features.toml"), MAX_MANIFEST_BYTES)?;
    documents.push(("parity/acrobat-features.toml".into(), only_lines(&parity, |line| line.trim_start().starts_with("notes ="))));
    Ok(documents)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The committed register, read as text.
    fn committed(root: &Path) -> String {
        read_inside(root, &root.join(REGISTER), MAX_FILE_BYTES).expect("read the committed register")
    }

    #[test]
    fn the_register_parses_and_every_row_is_well_formed() {
        let text = committed(&crate::gates::root());
        let claims = parse_register(&text);
        assert!(claims.len() >= 15, "{} rows; the register should cover the security claims", claims.len());
        assert_eq!(register_errors(&claims), Vec::<String>::new());
    }

    #[test]
    fn every_cited_test_exists_in_the_source_tree() {
        let root = crate::gates::root();
        let text = committed(&root);
        let claims = parse_register(&text);
        let errors: Vec<String> = claims.iter().filter_map(|c| evidence_error(c.evidence, &root)).collect();
        assert!(errors.is_empty(), "{}", errors.join("\n"));
    }

    #[test]
    fn the_register_answers_every_never_claim_word_with_a_limit() {
        let text = committed(&crate::gates::root());
        let claims = parse_register(&text);
        for word in OVERCLAIM.iter().chain(QUALIFICATION) {
            let recorded = claims
                .iter()
                .any(|c| c.status == NOT_CLAIMED && !c.residual.is_empty() && (contains_word(c.text, word) || contains_word(c.residual, word)));
            assert!(recorded, "no `not-claimed` row with a reason for {word:?}");
        }
    }

    #[test]
    fn the_wording_lists_are_the_never_claim_list_and_the_qualification_rule() {
        let mut words: Vec<&str> = OVERCLAIM.iter().chain(QUALIFICATION).copied().collect();
        words.sort_unstable();
        let expected: Vec<&str> = vec![
            "aatl",
            "acrobat-compatible",
            "acrobat-verified",
            "certified",
            "complete pii removal",
            "eidas",
            "eidas-compliant",
            "eutl",
            "legally binding",
            "legally valid",
            "military-grade",
            "no known attack recovers",
            "permanently removed",
            "qualified",
            "tamper-proof",
            "unbreakable",
            "uncrackable",
            "unrecoverable",
        ];
        assert_eq!(words, expected);
    }

    #[test]
    fn a_readme_line_calling_redaction_unbreakable_is_a_finding() {
        let text = committed(&crate::gates::root());
        let claims = parse_register(&text);
        let documents = [("crates/redact/README.md".to_string(), "this redaction is unbreakable\n".to_string())];
        let findings = scan_documents(&documents, &claims, &[]);
        assert_eq!(findings.len(), 1, "{findings:?}");
        assert!(findings[0].contains("unbreakable"), "{findings:?}");
    }

    #[test]
    fn every_banned_word_is_a_finding_and_a_cited_claim_excuses_it() {
        let text = committed(&crate::gates::root());
        let claims = parse_register(&text);
        for word in OVERCLAIM {
            let bare = format!("The report says: this {word} result.");
            assert_eq!(over_claim_hits(&bare, &claims), vec![*word], "{word:?} must be a finding on its own");
            let cited = format!("The report says: this {word} result. [CLAIM-001]");
            assert!(over_claim_hits(&cited, &claims).is_empty(), "{word:?} is excused by a backed claim");
        }
    }

    #[test]
    fn a_qualification_word_needs_a_row_that_records_the_limit() {
        let text = committed(&crate::gates::root());
        let claims = parse_register(&text);
        for word in QUALIFICATION {
            let bare = format!("The report says: this {word} result.");
            let hits = qualification_hits("docs/example.md", &bare, &claims, &[]);
            assert!(hits.contains(&word), "{word:?} must be a finding on its own: {hits:?}");
            let limit = format!("The report says: this {word} result. [CLAIM-014]");
            assert!(qualification_hits("docs/example.md", &limit, &claims, &[]).is_empty(), "{word:?} is excused by a recorded limit");
            let claimed = format!("The report says: this {word} result. [CLAIM-001]");
            let hits = qualification_hits("docs/example.md", &claimed, &claims, &[]);
            assert!(hits.contains(&word), "{word:?} needs a row that records the limit, not any row: {hits:?}");
        }
    }

    #[test]
    fn a_reviewed_use_excuses_its_word_on_its_own_line_only() {
        let text = committed(&crate::gates::root());
        let claims = parse_register(&text);
        let reviewed = parse_reviewed(&text);
        let row = reviewed.iter().find(|r| r.file == "docs/pdf-spec-parity.md").expect("a reviewed row for the parity table");
        let (file, word, contains) = (row.file, row.word, row.contains);
        let line = format!("| Trust | yes | {contains} |");
        assert!(qualification_hits(file, &line, &claims, &reviewed).is_empty(), "{word:?} on the reviewed line");
        let other = "PdfCraft signatures are qualified.";
        assert_eq!(qualification_hits(file, other, &claims, &reviewed).len(), 1, "a new line in the same file is still a finding");
        assert_eq!(qualification_hits("docs/gaps.md", &line, &claims, &reviewed).len(), 1, "the review is per file");
    }

    #[test]
    fn a_stale_reviewed_use_and_one_without_a_limit_are_errors() {
        let text = committed(&crate::gates::root());
        let claims = parse_register(&text);
        let documents = [("docs/example.md".to_string(), "the workflow is qualified.\n".to_string())];
        let stale = "| docs/example.md | qualified | not yet qualified | none | names an untested workflow |";
        let errors = reviewed_errors(&parse_reviewed(stale), &documents, &claims).join("\n");
        assert!(errors.contains("no line of the file"), "{errors}");
        let loose = "| docs/example.md | qualified | the workflow is qualified | CLAIM-001 | names an untested workflow |";
        let errors = reviewed_errors(&parse_reviewed(loose), &documents, &claims).join("\n");
        assert!(errors.contains("records a limit"), "{errors}");
        let kept = "| docs/example.md | qualified | the workflow is qualified | none | names an untested workflow |";
        assert_eq!(reviewed_errors(&parse_reviewed(kept), &documents, &claims), Vec::<String>::new());
    }

    #[test]
    fn every_reviewed_use_still_matches_a_line_of_its_document() {
        let root = crate::gates::root();
        let text = committed(&root);
        let claims = parse_register(&text);
        let reviewed = parse_reviewed(&text);
        let documents = repo_documents(&root).expect("read the repository's documents");
        assert!(reviewed.len() >= 5, "{} reviewed uses", reviewed.len());
        assert_eq!(reviewed_errors(&reviewed, &documents, &claims), Vec::<String>::new());
    }

    #[test]
    fn a_citation_of_a_missing_claim_does_not_excuse_a_banned_word() {
        let text = committed(&crate::gates::root());
        let claims = parse_register(&text);
        let line = "this redaction is unbreakable [CLAIM-999]";
        assert_eq!(over_claim_hits(line, &claims).len(), 1);
    }

    #[test]
    fn a_banned_word_matches_as_a_whole_word_in_any_case() {
        assert!(contains_word("This is UNBREAKABLE.", "unbreakable"));
        assert!(contains_word("a tamper-proof seal", "tamper-proof"));
        assert!(!contains_word("the unbreakables of it", "unbreakable"));
        assert!(!contains_word("tamper-proofing", "tamper-proof"));
        assert!(!contains_word("", "unbreakable"));
    }

    #[test]
    fn a_row_about_a_qualification_must_be_not_claimed() {
        let claimed = "| CLAIM-001 | PdfCraft signatures are qualified under eIDAS | UNV | claimed | none | signatures |";
        let errors = register_errors(&parse_register(claimed)).join("\n");
        assert!(errors.contains("not-claimed"), "{errors}");
        let limited =
            "| CLAIM-001 | No claim that PdfCraft signatures are qualified under eIDAS | UNV | not-claimed | no scheme is checked | signatures |";
        assert_eq!(register_errors(&parse_register(limited)), Vec::<String>::new());
    }

    #[test]
    fn a_row_without_evidence_or_a_reason_is_an_error() {
        let no_evidence = "| CLAIM-001 | Redaction removes glyphs |  | claimed | none | redaction |";
        let errors = register_errors(&parse_register(no_evidence)).join("\n");
        assert!(errors.contains("evidence"), "{errors}");
        let no_reason = "| CLAIM-001 | No claim of conformance | UNV | not-claimed |  | structure |";
        let errors = register_errors(&parse_register(no_reason)).join("\n");
        assert!(errors.contains("reason"), "{errors}");
    }

    #[test]
    fn no_document_states_a_guarantee_without_a_cited_claim() {
        let root = crate::gates::root();
        let text = committed(&root);
        let claims = parse_register(&text);
        let reviewed = parse_reviewed(&text);
        let documents = repo_documents(&root).expect("read the repository's documents");
        assert!(documents.len() > 20, "{} documents", documents.len());
        let findings = scan_documents(&documents, &claims, &reviewed);
        assert!(findings.is_empty(), "{}", findings.join("\n"));
    }

    #[test]
    fn the_scan_refuses_a_path_that_exists_outside_the_repository() {
        let root = crate::gates::root();
        // The parent directory exists and lies outside the root: only the guard can refuse it.
        let outside = root.join("..");
        assert!(outside.exists(), "{} should exist", outside.display());
        let err = read_inside(&root, &outside, MAX_FILE_BYTES).expect_err("a directory above the root is refused");
        assert!(err.contains("outside the repository root"), "{err}");
        assert!(read_inside(&root, &root.join("..").join("outside.md"), MAX_FILE_BYTES).is_err());
        assert!(read_inside(&root, &root.join("Cargo.toml"), MAX_FILE_BYTES).is_ok());
    }
}
