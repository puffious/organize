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

#[test]
fn organize_show_flat_multi_season_dump_routes_by_season() {
    let workspace = tempdir().expect("create tempdir");
    let source = workspace.path().join("source_multi");
    let destination = workspace.path().join("dest_multi");

    fs::create_dir_all(&source).expect("create source dir");
    fs::write(source.join("Show.Name.S01E01.mkv"), b"s1e1").expect("write s1 video");
    fs::write(source.join("Show.Name.S02E01.mkv"), b"s2e1").expect("write s2 video");

    let mut cmd = Command::cargo_bin("organize").expect("binary available");
    cmd.arg("show")
        .arg("--yes")
        .arg(&source)
        .arg(&destination)
        .assert()
        .success();

    assert!(destination
        .join("Show Name")
        .join("Season 01")
        .join("Show.Name.S01E01.mkv")
        .exists());
    assert!(destination
        .join("Show Name")
        .join("Season 02")
        .join("Show.Name.S02E01.mkv")
        .exists());
}

#[test]
fn organize_show_specials_folder_maps_to_season_zero() {
    let workspace = tempdir().expect("create tempdir");
    let source = workspace.path().join("Show.Specials");
    let destination = workspace.path().join("dest_specials");

    fs::create_dir_all(&source).expect("create source dir");
    fs::write(source.join("Show.Name.E01.mkv"), b"special").expect("write special video");

    let mut cmd = Command::cargo_bin("organize").expect("binary available");
    cmd.arg("show")
        .arg("--yes")
        .arg(&source)
        .arg(&destination)
        .assert()
        .success();

    let mut found = false;
    for entry in walkdir::WalkDir::new(&destination)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_file())
    {
        let path = entry.path();
        if path
            .to_string_lossy()
            .contains("/Season 00/Show.Name.E01.mkv")
        {
            found = true;
            break;
        }
    }
    assert!(found, "expected special episode in Season 00");
}

#[test]
fn organize_show_non_media_ignore_skips_subtitle_file() {
    let workspace = tempdir().expect("create tempdir");
    let source = workspace.path().join("source_ignore");
    let destination = workspace.path().join("dest_ignore");

    fs::create_dir_all(&source).expect("create source dir");
    fs::write(source.join("Show.Name.S01E01.mkv"), b"video").expect("write video");
    fs::write(source.join("Show.Name.S01E01.srt"), b"sub").expect("write subtitle");

    let mut cmd = Command::cargo_bin("organize").expect("binary available");
    cmd.arg("show")
        .arg("--yes")
        .arg("--non-media")
        .arg("ignore")
        .arg(&source)
        .arg(&destination)
        .assert()
        .success();

    let target_dir = destination.join("Show Name").join("Season 01");
    assert!(target_dir.join("Show.Name.S01E01.mkv").exists());
    assert!(!target_dir.join("Show.Name.S01E01.srt").exists());
    assert!(source.join("Show.Name.S01E01.srt").exists());
}
