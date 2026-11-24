use source_rules::comment_lines;
use std::fs;
use std::path::{Path, PathBuf};

fn rust_files_under(dir: &Path, found: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).expect("crates directory is readable") {
        let path = entry.expect("directory entry is readable").path();
        if path.is_dir() && path.file_name() != Some("target".as_ref()) {
            rust_files_under(&path, found);
        } else if path.extension() == Some("rs".as_ref()) {
            found.push(path);
        }
    }
}

#[test]
fn no_rust_file_under_crates_carries_a_comment() {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("source-rules sits inside crates");
    let mut files = Vec::new();
    rust_files_under(crates, &mut files);
    assert!(files.len() > 1, "the scan reached the workspace crates");

    let offenders: Vec<String> = files
        .iter()
        .flat_map(|path| {
            let source = fs::read_to_string(path).expect("source file is UTF-8");
            comment_lines(&source)
                .into_iter()
                .map(move |line| format!("{}:{line}", path.display()))
        })
        .collect();
    assert!(
        offenders.is_empty(),
        "comments are forbidden; rename, split or type the meaning instead:\n{}",
        offenders.join("\n")
    );
}

#[test]
fn line_block_and_doc_comments_are_found_on_their_lines() {
    let source = "fn a() {}\n// note\n/// doc\n//! inner\nlet x = 1; /* block */\n";
    assert_eq!(comment_lines(source), [2, 3, 4, 5]);
}

#[test]
fn a_block_comment_is_reported_once_at_its_opening_line() {
    assert_eq!(comment_lines("/* one\n two */ fn a() {}\n// three"), [1, 3]);
}

#[test]
fn comment_tokens_inside_string_literals_are_not_comments() {
    let source = concat!(
        "let url = \"http://example\";\n",
        "let escaped = \"quote \\\" then // still text\";\n",
        "let raw = r#\"a \"// b\" /* c\"#;\n",
        "let bytes = b\"//\";\n",
    );
    assert_eq!(comment_lines(source), Vec::<usize>::new());
}

#[test]
fn char_literals_and_lifetimes_do_not_hide_a_following_comment() {
    let source =
        "fn f<'a>(s: &'a str) -> char { '/' }\nlet q = '\"'; // after\nlet e = '\\''; // end";
    assert_eq!(comment_lines(source), [2, 3]);
}

#[test]
fn a_line_marked_why_is_the_one_permitted_comment() {
    assert_eq!(
        comment_lines("// WHY: codec drops the first word\nlet x = 1;"),
        Vec::<usize>::new()
    );
}
