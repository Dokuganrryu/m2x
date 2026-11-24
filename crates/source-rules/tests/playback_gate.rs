use std::fs;
use std::path::{Path, PathBuf};

fn crates() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
}

fn sources_under(folder: &Path, found: &mut Vec<(PathBuf, String)>) {
    for entry in
        fs::read_dir(folder).unwrap_or_else(|error| panic!("{}: {error}", folder.display()))
    {
        let path = entry.expect("the folder lists").path();
        if path.is_dir() {
            sources_under(&path, found);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            let text = fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
            found.push((path, text));
        }
    }
}

fn firmware_sources() -> Vec<(PathBuf, String)> {
    let mut found = Vec::new();
    for folder in ["board/src", "device/src", "stream/src", "dsp/src"] {
        sources_under(&crates().join(folder), &mut found);
    }
    found
}

fn places(needle: &str) -> Vec<String> {
    firmware_sources()
        .iter()
        .filter(|(_, text)| text.contains(needle))
        .flat_map(|(path, text)| {
            let name = path
                .file_name()
                .expect("a source has a name")
                .to_string_lossy()
                .into_owned();
            vec![name; text.matches(needle).count()]
        })
        .collect()
}

#[test]
fn the_playback_gate_is_opened_once_by_the_thread_when_the_boot_enters_the_volume_ramp() {
    assert_eq!(places(".open()"), ["thread.rs"]);
    let thread =
        fs::read_to_string(crates().join("board/src/firmware/thread.rs")).expect("thread.rs reads");
    let squeezed: String = thread.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(
        squeezed.contains(
            "if matches!(state, boot::State::VolumeRamp(..)) { self.playback_gate.open(); }"
        ),
        "the thread opens the gate somewhere other than the VolumeRamp entry"
    );
}

#[test]
fn the_playback_gate_is_written_in_one_place_and_read_once_on_the_receive_path() {
    assert_eq!(
        places("gate_is_open.store(true, Ordering::Relaxed)"),
        ["playback.rs"]
    );
    assert_eq!(
        places("gate_is_open.load(Ordering::Relaxed)"),
        ["playback.rs"]
    );
    assert_eq!(places("gate_is_open"), ["playback.rs"; 4]);
    assert_eq!(places("PlaybackGate {"), ["playback.rs"]);
}
