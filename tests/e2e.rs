use std::fs;

use assert_cmd::Command;
use tempfile::tempdir;

#[test]
fn organize_show_moves_video_and_sidecar_into_season_folder() {
    let workspace = tempdir().expect("create tempdir");
    let source = workspace.path().join("source_show");
    let destination = workspace.path().join("dest_show");

    fs::create_dir_all(&source).expect("create source dir");
    fs::write(source.join("Show.Name.S01E01.mkv"), b"video").expect("write show video");
    fs::write(source.join("Show.Name.S01E01.srt"), b"sub").expect("write subtitle");

    let mut cmd = Command::cargo_bin("organize").expect("binary available");
    cmd.arg("show")
        .arg("--yes")
        .arg(&source)
        .arg(&destination)
        .assert()
        .success();

    let expected_dir = destination.join("Show Name").join("Season 01");
    assert!(expected_dir.join("Show.Name.S01E01.mkv").exists());
    assert!(expected_dir.join("Show.Name.S01E01.srt").exists());
    assert!(!source.join("Show.Name.S01E01.mkv").exists());
    assert!(!source.join("Show.Name.S01E01.srt").exists());
}

#[test]
fn organize_movie_moves_video_and_sidecar_into_movie_folder() {
    let workspace = tempdir().expect("create tempdir");
    let source = workspace.path().join("source_movie");
    let destination = workspace.path().join("dest_movie");

    fs::create_dir_all(&source).expect("create source dir");
    fs::write(source.join("Movie.Name.2023.1080p.mkv"), b"video").expect("write movie video");
    fs::write(source.join("Movie.Name.2023.1080p.srt"), b"sub").expect("write subtitle");

    let mut cmd = Command::cargo_bin("organize").expect("binary available");
    cmd.arg("movie")
        .arg("--yes")
        .arg(&source)
        .arg(&destination)
        .assert()
        .success();

    let expected_dir = destination.join("Movie Name (2023)");
    assert!(expected_dir.join("Movie.Name.2023.1080p.mkv").exists());
    assert!(expected_dir.join("Movie.Name.2023.1080p.srt").exists());
    assert!(!source.join("Movie.Name.2023.1080p.mkv").exists());
    assert!(!source.join("Movie.Name.2023.1080p.srt").exists());
}

#[test]
fn scan_json_output_writes_report_file() {
    let workspace = tempdir().expect("create tempdir");
    let source = workspace.path().join("scan_source");
    let report = workspace.path().join("reports/scan.json");

    fs::create_dir_all(&source).expect("create source dir");
    fs::write(source.join("Show.Name.S01E01.mkv"), b"video").expect("write test media");

    let mut cmd = Command::cargo_bin("organize").expect("binary available");
    cmd.arg("scan")
        .arg("--json")
        .arg("--output")
        .arg(&report)
        .arg(&source)
        .assert()
        .success();

    let payload = fs::read_to_string(&report).expect("read report");
    let parsed: serde_json::Value = serde_json::from_str(&payload).expect("valid json report");

    assert_eq!(
        parsed["source"].as_str(),
        Some(source.to_string_lossy().as_ref())
    );
    assert_eq!(parsed["items"].as_array().map(|v| v.len()), Some(1));
    assert!(parsed["items"][0]["source_path"].is_string());
    assert!(parsed["items"][0]["extension"].is_string());
}
