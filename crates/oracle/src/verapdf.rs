//! The external PDF/A validator as an oracle: run it on one file and read its verdict.
//!
//! [`verapdf`] probes for the program on `PATH`; [`validate`] runs it on one file with one flavour
//! and turns the machine-readable report it prints on stdout into a [`Verdict`]. Nothing of the
//! validator is linked, bundled or copied: it is a separate process, the file path travels as one
//! argument and never through a shell, the report is read under a size cap, and a run that outlives
//! its timeout is killed and reaped.
//!
//! The report's `isCompliant`, `passedRules`, `failedRules` and `profileName` are read from an
//! attribute when the report carries one and from a child element of that name when it does not;
//! [`field`] accepts either form, so a summary written either way reads the same.
//!
//! A non-zero exit with a readable report is a verdict, not an error: the validator exits non-zero
//! when it finds a violation, which is what a caller asked to hear about.
//!
//! # Not checked against an installed build
//!
//! The version argument [`verapdf`] uses, the allowlist below and the attribute-or-element form of
//! the report fields were written down without an installed validator to check them against, which
//! no test in this crate can do for the first two. `--version` is what the probe asks for, and a
//! build that does not accept it is reported as absent rather than used; both report forms are
//! read, so a build that differs there cannot turn a failed validation into a passing verdict.

use std::fmt;
use std::io::Read;
use std::path::Path;
use std::process::{Child, ChildStdout, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use crate::{Absent, Oracle};

/// How much of the validator's stdout is kept.
///
/// A report for an ordinary file is a few hundred kilobytes, so 16 MiB leaves room for an
/// unusually detailed one while bounding what a hostile file, or a tool that never stops talking,
/// can make this process hold. A report past the cap is refused rather than buffered.
const MAX_REPORT_BYTES: usize = 16 * 1024 * 1024;

/// How many XML nodes the report may hold, the same limit the XMP reader in `preflight` uses.
///
/// The report is written by a tool we do not control, so its shape is input: a report nested or
/// repeated without bound must be refused, not walked.
const MAX_REPORT_NODES: u32 = 100_000;

/// How often an unfinished child is asked whether it has exited.
///
/// Polling costs one system call every 50 ms, against a validation run that takes seconds; a
/// shorter interval buys nothing and a longer one makes a killed run wait visibly past its timeout.
const POLL_INTERVAL: Duration = Duration::from_millis(50);

/// The profiles a caller may ask for; a closed list, so a flavour can never become an arbitrary
/// argument to the tool.
const FLAVOURS: &[&str] = &["1a", "1b", "2a", "2b", "2u", "3a", "3b", "3u", "4", "4e", "4f", "ua1", "ua2", "wt1a"];

/// What the validator said about one file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verdict {
    /// Whether the file conforms to the flavour that was asked for.
    pub compliant: bool,
    /// Rules the report counts as passed.
    pub passed_rules: u32,
    /// Rules the report counts as failed.
    pub failed_rules: u32,
    /// The profile the report names, for example `PDF/A-2b`. Empty when the report leaves it out.
    pub profile: String,
}

/// Why the validator could not be run or its report could not be read.
///
/// A finding is not among these: a report that says `isCompliant="false"` is a successful call
/// returning `Ok`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VeraError {
    /// The process could not be started, asked or read; the text says which.
    Io(String),
    /// The run outlived its timeout and was killed and reaped.
    Timeout,
    /// The report grew past [`MAX_REPORT_BYTES`] and was refused rather than buffered.
    OutputTooLarge,
    /// The process printed something that is not a report this reader understands.
    BadReport(String),
    /// The flavour is not one of [`FLAVOURS`], and was refused before anything ran.
    UnknownFlavour,
}

impl fmt::Display for VeraError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(reason) => write!(f, "the external PDF/A validator could not be run: {reason}"),
            Self::Timeout => write!(f, "the external PDF/A validator did not finish within its timeout and was killed"),
            Self::OutputTooLarge => write!(f, "the external PDF/A validator wrote more than {MAX_REPORT_BYTES} bytes of report"),
            Self::BadReport(reason) => write!(f, "the external PDF/A validator's report could not be read: {reason}"),
            Self::UnknownFlavour => write!(f, "the PDF/A flavour is not one of {}", FLAVOURS.join(", ")),
        }
    }
}

/// The external PDF/A validator, probed with its version flag.
///
/// Whether it is installed decides whether a test runs or skips; a build that cannot be started is
/// [`Absent`], and [`crate::REQUIRED_VAR`] decides whether that absence is fatal.
pub fn verapdf() -> Result<Oracle, Absent> {
    crate::find("verapdf", "--version")
}

/// Run the validator on `pdf` with `flavour` and read its verdict.
///
/// `flavour` is checked against [`FLAVOURS`] before anything is spawned, so it can never become
/// another argument to the tool. `pdf` is passed as one argument, with the child's working
/// directory set to the file's own, so a path with spaces or non-ASCII characters is never quoted
/// or split by a shell.
///
/// `timeout` bounds the whole run: the child is killed and reaped at the deadline. Use a value at
/// least an order of magnitude above an ordinary run, whose start alone takes seconds.
pub fn validate(oracle: &Oracle, flavour: &str, pdf: &Path, timeout: Duration) -> Result<Verdict, VeraError> {
    if !FLAVOURS.contains(&flavour) {
        return Err(VeraError::UnknownFlavour);
    }
    let mut child = spawn(oracle, flavour, pdf)?;
    let Some(stdout) = child.stdout.take() else {
        stop(&mut child);
        return Err(VeraError::Io("its stdout was not captured, so there is no report to read".to_owned()));
    };
    let report = run_to_deadline(&mut child, stdout, timeout)?;
    parse_report(&report)
}

/// Start the validator on `pdf`: one argument per argv entry, no shell, stdin closed and stdout
/// piped.
///
/// The child runs in the file's own directory, so a validator that writes scratch files writes
/// them beside the input rather than into whatever directory the test or app happens to be in.
/// Because the working directory is changed, only the file name is passed: a relative path as an
/// argument would be resolved a second time against that directory.
fn spawn(oracle: &Oracle, flavour: &str, pdf: &Path) -> Result<Child, VeraError> {
    let name = pdf.file_name().ok_or_else(|| VeraError::Io(format!("`{}` does not name a file", pdf.display())))?;
    let directory = pdf.parent().filter(|parent| !parent.as_os_str().is_empty()).unwrap_or_else(|| Path::new("."));
    oracle
        .command()
        .arg("--flavour")
        .arg(flavour)
        .arg(name)
        .current_dir(directory)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| VeraError::Io(format!("it could not be started: {e}")))
}

/// Kill the child and reap it, so nothing outlives a call that gave up on it.
///
/// Both calls fail harmlessly when the child has already exited, which is why their results are
/// dropped: the caller is already returning the reason it gave up.
fn stop(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

/// Read the child's stdout on its own thread, so a child that never closes it cannot block the
/// caller past its timeout.
fn start_reader(stdout: ChildStdout, sender: mpsc::Sender<Result<Vec<u8>, VeraError>>) -> Result<std::thread::JoinHandle<()>, VeraError> {
    std::thread::Builder::new()
        .name("pdfa-report".to_owned())
        .spawn(move || {
            // The receiver is gone when the wait below gave up first; the report is then nobody's.
            let _ = sender.send(read_capped(stdout, MAX_REPORT_BYTES));
        })
        .map_err(|e| VeraError::Io(format!("the thread reading its report could not be started: {e}")))
}

/// Wait for the child to exit, or kill it at the deadline, and hand back what its stdout said.
fn run_to_deadline(child: &mut Child, stdout: ChildStdout, timeout: Duration) -> Result<Vec<u8>, VeraError> {
    let (sender, receiver) = mpsc::channel();
    let reader = match start_reader(stdout, sender) {
        Ok(reader) => reader,
        Err(e) => {
            stop(child);
            return Err(e);
        }
    };
    let started = Instant::now();
    let mut report = None;
    let mut exited = false;
    while report.is_none() || !exited {
        if started.elapsed() >= timeout {
            stop(child);
            return Err(VeraError::Timeout);
        }
        if report.is_none() {
            match poll_report(&receiver) {
                Some(Ok(bytes)) => report = Some(bytes),
                Some(Err(e)) => {
                    stop(child);
                    return Err(e);
                }
                None => {}
            }
        }
        if !exited {
            match child.try_wait() {
                Ok(status) => exited = status.is_some(),
                Err(e) => {
                    stop(child);
                    return Err(VeraError::Io(format!("it could not be asked whether it had finished: {e}")));
                }
            }
        }
        if report.is_none() || !exited {
            std::thread::sleep(POLL_INTERVAL);
        }
    }
    let _ = reader.join();
    match report {
        Some(bytes) => Ok(bytes),
        // The loop only ends with a report in hand; this arm exists so no path has to unwrap.
        None => Err(VeraError::Io("the thread reading its report stopped without one".to_owned())),
    }
}

/// One look at what the reader thread has said, if anything yet.
///
/// A disconnected channel means the thread ended without sending: the report was lost mid-read,
/// which is an error rather than an empty report.
fn poll_report(receiver: &mpsc::Receiver<Result<Vec<u8>, VeraError>>) -> Option<Result<Vec<u8>, VeraError>> {
    match receiver.try_recv() {
        Ok(result) => Some(result),
        Err(mpsc::TryRecvError::Empty) => None,
        Err(mpsc::TryRecvError::Disconnected) => Some(Err(VeraError::Io("the thread reading its report stopped without one".to_owned()))),
    }
}

/// Read `reader` to its end, keeping at most `cap` bytes.
///
/// The read is bounded by `cap`, not by the tool: a report past the cap is refused as
/// [`VeraError::OutputTooLarge`] rather than buffered, so what this holds is bounded whatever the
/// child prints.
fn read_capped(reader: impl Read, cap: usize) -> Result<Vec<u8>, VeraError> {
    let limit = u64::try_from(cap).unwrap_or(u64::MAX).saturating_add(1);
    let mut bytes = Vec::new();
    reader.take(limit).read_to_end(&mut bytes).map_err(|e| VeraError::Io(format!("its report could not be read: {e}")))?;
    if bytes.len() > cap {
        return Err(VeraError::OutputTooLarge);
    }
    Ok(bytes)
}

/// Turn the report into a [`Verdict`], or say which field it lacks.
fn parse_report(bytes: &[u8]) -> Result<Verdict, VeraError> {
    let text = std::str::from_utf8(bytes).map_err(|e| VeraError::BadReport(format!("it is not UTF-8: {e}")))?;
    let options = roxmltree::ParsingOptions { allow_dtd: false, nodes_limit: MAX_REPORT_NODES, ..Default::default() };
    let document = roxmltree::Document::parse_with_options(text, options).map_err(|e| VeraError::BadReport(format!("it is not XML: {e}")))?;
    let compliant = match field(&document, "isCompliant") {
        Some(value) if value.eq_ignore_ascii_case("true") => true,
        Some(value) if value.eq_ignore_ascii_case("false") => false,
        Some(value) => return Err(VeraError::BadReport(format!("its `isCompliant` is `{value}`, not `true` or `false`"))),
        None => return Err(VeraError::BadReport("it does not say whether the file is compliant".to_owned())),
    };
    let passed_rules = rule_count(&document, "passedRules")?;
    let failed_rules = rule_count(&document, "failedRules")?;
    // The verdict does not depend on the profile name, so a report that leaves it out is still a
    // report; the name is empty rather than invented.
    let profile = field(&document, "profileName").unwrap_or_default().to_owned();
    Ok(Verdict { compliant, passed_rules, failed_rules, profile })
}

/// A rule count from the report, or the reason the field cannot be a count.
fn rule_count(document: &roxmltree::Document<'_>, name: &str) -> Result<u32, VeraError> {
    let value = field(document, name).ok_or_else(|| VeraError::BadReport(format!("it does not count `{name}`")))?;
    value.parse::<u32>().map_err(|_| VeraError::BadReport(format!("its `{name}` is `{value}`, which is not a count")))
}

/// The value of `name` in the report, from an attribute or from a child element of that name.
///
/// Both forms are accepted because which one the tool writes is a detail of its report that a
/// caller cannot check here; reading either means a report is understood whichever way it
/// summarises the run.
fn field<'a>(document: &'a roxmltree::Document<'a>, name: &str) -> Option<&'a str> {
    document.descendants().filter(roxmltree::Node::is_element).find_map(|node| {
        let value = node
            .attributes()
            .find(|attribute| attribute.name() == name)
            .map(|attribute| attribute.value())
            .or_else(|| node.children().find(|child| child.is_element() && child.tag_name().name() == name).and_then(|child| child.text()))?;
        Some(value.trim())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io;
    use std::path::{Path, PathBuf};

    /// A minimal compliant summary in the attribute form. Written for the reader, not copied from
    /// any particular build: the reader takes the four fields wherever they appear.
    const COMPLIANT_REPORT: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<report>
  <jobs>
    <job>
      <validationReports>
        <details passedRules="120" failedRules="0" isCompliant="true" profileName="PDF/A-2b"/>
      </validationReports>
    </job>
  </jobs>
</report>"#;

    /// A directory under the system temporary directory, removed when the test ends.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(test: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("pdfcraft-oracle-verapdf-{test}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            Self(dir)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// A minimal PDF 1.7 file: one page of literal text in an unembedded standard font, no
    /// identification and no output intent. Valid enough to parse, nothing a PDF/A profile accepts.
    fn minimal_pdf() -> Vec<u8> {
        let content = "BT /F1 24 Tf 20 150 Td (hello) Tj ET\n";
        let objects = [
            "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_owned(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 300] /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>".to_owned(),
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_owned(),
            format!("<< /Length {} >>\nstream\n{content}endstream", content.len()),
        ];
        let mut bytes = b"%PDF-1.7\n".to_vec();
        let mut offsets = Vec::new();
        for (i, body) in objects.iter().enumerate() {
            offsets.push(bytes.len());
            bytes.extend_from_slice(format!("{} 0 obj\n{body}\nendobj\n", i + 1).as_bytes());
        }
        let xref = bytes.len();
        bytes.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes());
        for offset in offsets {
            bytes.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        bytes.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objects.len() + 1).as_bytes());
        bytes
    }

    /// A reader that never runs out, the way a tool that prints without end would not.
    struct Generous;

    impl io::Read for Generous {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            buf.fill(b'x');
            Ok(buf.len())
        }
    }

    /// The flavour becomes an argument to the tool, so anything not on the list is refused before
    /// a process starts. The oracle names a program that does not exist: had a run been attempted,
    /// the result would be `Io` rather than `UnknownFlavour`.
    #[test]
    fn unknown_flavour_is_refused_without_running_anything() {
        let absent = Oracle { program: "pdfcraft-no-such-validator", version: String::new() };
        for flavour in ["", "2B", "--flavour", "-2b", "2b --format json", "ua3", " 2b", "2b "] {
            assert_eq!(
                validate(&absent, flavour, Path::new("plain text.pdf"), Duration::from_secs(1)),
                Err(VeraError::UnknownFlavour),
                "{flavour:?} must be refused"
            );
        }
        // Every listed flavour passes the allowlist, so the call gets as far as the probe; the
        // program does not exist, so an attempted run shows up as an I/O error.
        for flavour in FLAVOURS {
            let error = validate(&absent, flavour, Path::new("plain text.pdf"), Duration::from_secs(1)).expect_err("no such program");
            assert!(matches!(error, VeraError::Io(_)), "{flavour}: {error:?}");
        }
    }

    #[test]
    fn a_compliant_report_parses() {
        let verdict = parse_report(COMPLIANT_REPORT.as_bytes()).expect("a compliant report");
        assert!(verdict.compliant);
        assert_eq!(verdict.passed_rules, 120);
        assert_eq!(verdict.failed_rules, 0);
        assert_eq!(verdict.profile, "PDF/A-2b");
    }

    /// The same summary in the element form: the verdict reads the same as the attribute form's.
    #[test]
    fn a_failed_report_parses() {
        let report = r#"<report>
  <validationReport>
    <details>
      <profileName>PDF/A-2u</profileName>
      <isCompliant>false</isCompliant>
      <passedRules>7</passedRules>
      <failedRules>3</failedRules>
    </details>
  </validationReport>
</report>"#;
        let verdict = parse_report(report.as_bytes()).expect("a failed report");
        assert!(!verdict.compliant);
        assert_eq!(verdict.passed_rules, 7);
        assert_eq!(verdict.failed_rules, 3);
        assert_eq!(verdict.profile, "PDF/A-2u");
    }

    /// Each failure names the validator and what went wrong, including the flavour list an
    /// unknown flavour is checked against.
    #[test]
    fn failures_say_what_went_wrong() {
        let io = VeraError::Io("it could not be started".to_owned()).to_string();
        assert!(io.contains("external PDF/A validator"), "{io}");
        assert!(io.contains("it could not be started"), "{io}");
        assert!(VeraError::Timeout.to_string().contains("timeout"), "a timeout says so");
        assert!(VeraError::OutputTooLarge.to_string().contains(&MAX_REPORT_BYTES.to_string()), "the cap is named");
        let bad = VeraError::BadReport("it is not XML".to_owned()).to_string();
        assert!(bad.contains("it is not XML"), "{bad}");
        let flavour = VeraError::UnknownFlavour.to_string();
        assert!(flavour.contains("PDF/A flavour"), "{flavour}");
        assert!(flavour.contains("2b"), "{flavour}");
    }

    /// A path that names no file is refused before anything is started.
    #[test]
    fn a_path_without_a_file_name_is_refused() {
        let absent = Oracle { program: "pdfcraft-no-such-validator", version: String::new() };
        let error = validate(&absent, "2b", Path::new("/"), Duration::from_secs(1)).expect_err("no file name");
        assert!(matches!(error, VeraError::Io(_)), "{error:?}");
    }

    #[test]
    fn a_report_that_is_not_xml_is_bad_report() {
        for text in ["", "not xml at all", "<report", "\u{0}<report/>"] {
            let error = parse_report(text.as_bytes()).expect_err("not a report");
            assert!(matches!(error, VeraError::BadReport(_)), "{text:?} gave {error:?}");
        }
        // XML that is not a report: no verdict, no counts.
        for text in ["<report/>", "<report><isCompliant>maybe</isCompliant></report>", "<report isCompliant=\"true\"/>"] {
            let error = parse_report(text.as_bytes()).expect_err("not a verdict");
            assert!(matches!(error, VeraError::BadReport(_)), "{text:?} gave {error:?}");
        }
        // A count that is not a count is refused rather than defaulted to zero.
        let text = r#"<report isCompliant="false" passedRules="many" failedRules="1"/>"#;
        assert!(matches!(parse_report(text.as_bytes()), Err(VeraError::BadReport(_))));
    }

    /// The cap is exercised with a reader that produces bytes without end, so the test needs no
    /// clock and no process.
    #[test]
    fn the_output_cap_is_enforced() {
        assert_eq!(MAX_REPORT_BYTES, 16 * 1024 * 1024, "the cap is the one the module documents");
        assert_eq!(read_capped(Generous, 8), Err(VeraError::OutputTooLarge));
        assert_eq!(read_capped(Generous, 0), Err(VeraError::OutputTooLarge), "even an empty allowance is exceeded by one byte");
        assert_eq!(read_capped(io::repeat(b'x').take(8), 8), Ok(vec![b'x'; 8]), "a report exactly at the cap is kept");
        assert_eq!(read_capped(io::empty(), 8), Ok(Vec::new()), "an empty report is empty, not an error");
    }

    /// The validator's verdict on a file that is not PDF/A at all. Negative only, by design: a
    /// positive case needs a compliant file, which this crate does not own.
    #[test]
    fn plain_text_pdf_is_not_pdfa() {
        let oracle = crate::or_skip!(verapdf());
        let dir = TempDir::new("plain-text");
        std::fs::create_dir_all(&dir.0).unwrap();
        let pdf = dir.0.join("plain text.pdf");
        std::fs::write(&pdf, minimal_pdf()).unwrap();
        let verdict =
            validate(&oracle, "2b", &pdf, Duration::from_secs(120)).unwrap_or_else(|e| panic!("the validator ran but gave no verdict: {e}"));
        assert!(!verdict.compliant, "a file with none of the PDF/A-2b requirements is not compliant: {verdict:?}");
        assert!(verdict.failed_rules > 0, "a failed validation must count at least one failed rule: {verdict:?}");
    }
}
