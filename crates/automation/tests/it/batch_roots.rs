//! The folder tools hold every path they read or write to `--root`, through the tool entry
//! points: an input or output outside the root, a link that leads out of it, and a hostile file
//! name that could otherwise write beside the output folder.

use std::path::{Path, PathBuf};

use pdfcraft_automation::{Automation, Content};
use pdfcraft_engine::Session;
use serde_json::{Value, json};

fn s(path: &Path) -> String {
    path.to_str().unwrap().to_owned()
}

fn synthetic_pdf() -> Vec<u8> {
    Session::new().create_from_text("Roots", "Roots text").unwrap().as_ref().clone()
}

/// A temporary folder holding `root/` (the `--root`) beside `outside/`, both empty.
fn confined() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let base = tempfile::tempdir().unwrap();
    let (root, outside) = (base.path().join("root"), base.path().join("outside"));
    std::fs::create_dir(&root).unwrap();
    std::fs::create_dir(&outside).unwrap();
    (base, root, outside)
}

/// The names in `dir`, sorted, so a call can be shown to have added none.
fn entries(dir: &Path) -> Vec<std::ffi::OsString> {
    let mut names: Vec<_> = std::fs::read_dir(dir).unwrap().map(|entry| entry.unwrap().file_name()).collect();
    names.sort();
    names
}

/// The single JSON report a tool returned, or its error text.
fn call(auto: &mut Automation, name: &str, args: &Value) -> Result<Value, String> {
    match auto.call(name, args) {
        Ok(mut out) => match out.pop() {
            Some(Content::Json(value)) => Ok(value),
            _ => Err(format!("{name} returned no JSON report")),
        },
        Err(error) => Err(error.to_string()),
    }
}

/// `action_run` arguments with one `set_title` step, which needs no OCR models.
fn title_action(paths: Value, folder: &str) -> Value {
    json!({"steps": [{"step": "set_title", "arg": "Roots"}], "paths": paths, "folder": folder})
}

/// A refusal of the whole call that names the root, with nothing added outside it.
fn assert_refused(auto: &mut Automation, name: &str, args: &Value, outside: &Path) {
    let before = entries(outside);
    let error = match call(auto, name, args) {
        Err(text) => text,
        Ok(report) => panic!("{name} accepted {args}: {report}"),
    };
    assert!(error.contains("outside"), "{name} {args}: {error}");
    assert_eq!(entries(outside), before, "{name} {args} added an entry outside the root");
}

/// A refusal of one file: `action_run` reports every path it cannot read that way.
fn assert_per_file_refused(auto: &mut Automation, name: &str, args: &Value, outside: &Path) {
    let before = entries(outside);
    let report = match call(auto, name, args) {
        Ok(report) => report,
        Err(text) => panic!("{name} refused the whole call: {text}"),
    };
    let entry = &report["files"][0];
    assert!(entry["error"].as_str().is_some_and(|e| e.contains("outside")), "{name} {args}: {report}");
    assert!(entry.get("output").is_none(), "{name} {args} named an output: {report}");
    assert_eq!(entries(outside), before, "{name} {args} added an entry outside the root");
}

#[test]
fn folder_tools_refuse_inputs_and_outputs_outside_the_root() {
    let (_base, root, outside) = confined();
    std::fs::write(outside.join("secret.pdf"), synthetic_pdf()).unwrap();
    std::fs::write(root.join("page.pdf"), synthetic_pdf()).unwrap();
    let mut auto = Automation::new().with_root(&root).unwrap();
    let secret = s(&outside.join("secret.pdf"));
    let outside_dir = s(&outside);
    let outside_pdf = s(&outside.join("new.pdf"));

    // Each case names one path outside the root; nothing may appear beside it.
    assert_refused(&mut auto, "doc_batch", &json!({"paths": [secret.clone()], "steps": ["convert"], "out_dir": "out"}), &outside);
    assert_refused(&mut auto, "doc_batch", &json!({"paths": ["page.pdf"], "steps": ["convert"], "out_dir": outside_dir.clone()}), &outside);
    assert_per_file_refused(&mut auto, "action_run", &title_action(json!([secret.clone()]), "out"), &outside);
    assert_refused(&mut auto, "action_run", &title_action(json!(["page.pdf"]), &outside_dir), &outside);
    assert_refused(&mut auto, "doc_bates_multi", &json!({"files": [{"input": secret.clone(), "output": "stamped.pdf"}]}), &outside);
    assert_refused(&mut auto, "doc_bates_multi", &json!({"files": [{"input": "page.pdf", "output": outside_pdf.clone()}]}), &outside);
    assert_refused(&mut auto, "ocr_recognize_files", &json!({"paths": [secret.clone()], "folder": "out"}), &outside);
    assert_refused(&mut auto, "ocr_recognize_files", &json!({"paths": ["page.pdf"], "folder": outside_dir.clone()}), &outside);
}

/// A link inside the root that resolves to the file `outside/secret.pdf`, as the path to hand the
/// tool, or `None` when the system will not make one: Windows needs Developer Mode or elevation
/// for a symbolic link, and this test starts no other program to make a junction.
fn link_to_outside(root: &Path, outside: &Path) -> Option<String> {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(outside.join("secret.pdf"), root.join("leak.pdf")).ok()?;
        Some("leak.pdf".to_owned())
    }
    #[cfg(windows)]
    {
        std::os::windows::fs::symlink_dir(outside, root.join("leak")).ok()?;
        Some("leak/secret.pdf".to_owned())
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = (root, outside);
        None
    }
}

#[test]
fn action_run_refuses_a_link_input_that_leaves_the_root() {
    let (_base, root, outside) = confined();
    let original = synthetic_pdf();
    std::fs::write(outside.join("secret.pdf"), &original).unwrap();
    let Some(inside) = link_to_outside(&root, &outside) else {
        eprintln!("skipped: this system will not make a link without a privilege it does not hold");
        return;
    };
    let mut auto = Automation::new().with_root(&root).unwrap();
    assert_per_file_refused(&mut auto, "action_run", &title_action(json!([inside]), "out"), &outside);
    assert_eq!(std::fs::read(outside.join("secret.pdf")).unwrap(), original, "the linked file was changed");
    let out = root.join("out");
    assert!(out.is_dir(), "the folder argument is created before the paths are read");
    assert_eq!(std::fs::read_dir(&out).unwrap().count(), 0, "the link's target was copied inside");
}

#[cfg(unix)]
#[test]
fn doc_bates_multi_refuses_a_link_output_that_leaves_the_root() {
    let (_base, root, outside) = confined();
    std::fs::write(root.join("page.pdf"), synthetic_pdf()).unwrap();
    std::os::unix::fs::symlink(outside.join("new.pdf"), root.join("linked.pdf")).unwrap();
    let mut auto = Automation::new().with_root(&root).unwrap();
    let error = match call(&mut auto, "doc_bates_multi", &json!({"files": [{"input": "page.pdf", "output": "linked.pdf"}]})) {
        Ok(report) => {
            let per_file = report["files"][0]["error"].as_str().unwrap_or_default().to_owned();
            assert!(!per_file.is_empty(), "the call was neither refused nor given a file error: {report}");
            per_file
        }
        Err(text) => text,
    };
    assert!(error.contains("outside"), "{error}");
    assert!(!outside.join("new.pdf").exists(), "the link's target was created: {error}");
}

#[cfg(unix)]
#[test]
fn action_run_output_stays_inside_the_folder_for_hostile_names() {
    let (_base, root, outside) = confined();
    std::fs::write(root.join("a:b.pdf"), synthetic_pdf()).unwrap();
    let mut auto = Automation::new().with_root(&root).unwrap();
    let report = call(&mut auto, "action_run", &title_action(json!(["a:b.pdf"]), "out")).unwrap();
    assert!(report["files"][0]["error"].is_null(), "{report}");
    let written = root.join("out").join("a_b.pdf");
    assert!(written.is_file(), "{report}");
    let reported = PathBuf::from(report["files"][0]["output"].as_str().unwrap());
    assert!(reported.starts_with(root.canonicalize().unwrap()), "{report}");
    assert!(entries(&outside).is_empty(), "a name with a colon wrote outside the folder: {reported:?}");
}
