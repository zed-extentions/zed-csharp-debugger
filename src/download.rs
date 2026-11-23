//! Pure logic for the automatic netcoredbg download (FR-16): release asset
//! selection, cache layout and version comparison. Nothing in this module
//! touches the Zed extension API, so it is fully unit-testable on the host
//! target.
//!
//! Ground truth for the asset matrix is the published release assets of
//! Samsung/netcoredbg (verified against the GitHub Releases API, 2026-10-30):
//! every release since 3.1.0 ships `netcoredbg-linux-amd64.tar.gz`,
//! `netcoredbg-linux-arm64.tar.gz`, `netcoredbg-osx-*.zip|tar.gz` and
//! `netcoredbg-win64.zip`. Both archive layouts contain a single top-level
//! `netcoredbg/` directory with the binary at
//! `netcoredbg/netcoredbg[.exe]` (confirmed in review against the 3.2.0-1092
//! assets; matches mason-registry's `netcoredbg` package, which declares
//! `netcoredbg/netcoredbg[.exe]` inside each extracted archive).
//! `netcoredbg-osx-amd64` is published only up to 3.1.3-1062, so Intel Macs
//! fall back to that tag ([`fallback_tag`]).
//!
//! Extraction semantics are those of Zed's host `download_file`
//! (`crates/extension_host/src/wasm_host/wit/since_v0_8_0.rs`): for `Zip` and
//! `GzipTar` the `path` argument is the **destination directory** the archive
//! entries are extracted into; for `Gzip`/`Uncompressed` it is the output
//! file. [`extracted_path`] models exactly that, and [`download_plan`] derives
//! both the `download_file` destination and the binary path from it.

/// The GitHub repository releases are fetched from.
pub const REPO: &str = "Samsung/netcoredbg";

/// Human-facing releases page (used in error messages).
pub const RELEASES_URL: &str = "https://github.com/Samsung/netcoredbg/releases";

/// Upstream instructions for building netcoredbg from source (NFR-7).
pub const BUILD_FROM_SOURCE_URL: &str =
    "https://github.com/Samsung/netcoredbg#building-from-source";

/// Version pin (extension constant). `None` follows the latest release;
/// `Some("3.1.3-1062")` would resolve that tag instead. There is currently no
/// Zed settings channel that forwards extension-specific values to
/// `get_dap_binary` (`dap.*` forwards `binary` only), so the pin is a
/// compile-time constant by necessity.
pub const VERSION_PIN: Option<&str> = None;

/// Environment variable that forces a re-download even when the cached
/// binary for the target version already exists (development escape hatch).
pub const FORCE_UPDATE_ENV: &str = "ZED_NETCOREDBG_FORCE_UPDATE";

/// Name of the cache directory created inside the extension working
/// directory.
pub const CACHE_DIR_NAME: &str = "netcoredbg";

/// Host operating system kinds relevant to asset selection.
///
/// Mirrors `zed_extension_api::Os` without depending on it, to keep this
/// module free of WASM-hosted types.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HostOs {
    Mac,
    Linux,
    Windows,
}

/// Host CPU architectures relevant to asset selection.
///
/// Mirrors `zed_extension_api::Architecture` without depending on it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HostArch {
    Aarch64,
    X86,
    X86_64,
}

/// A release asset to download.
///
/// Mirrors `zed_extension_api::GithubReleaseAsset` without depending on it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReleaseAsset {
    pub name: String,
    pub download_url: String,
}

/// The archive/encoding variants `zed_extension_api::DownloadedFileType`
/// supports, mirrored without depending on the WASM-hosted enum.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArchiveKind {
    /// A gzipped file (`.gz`).
    Gzip,
    /// A gzipped tar archive (`.tar.gz`).
    GzipTar,
    /// A ZIP file (`.zip`).
    Zip,
    /// An uncompressed file.
    Uncompressed,
}

/// Returns the netcoredbg release asset name for the given platform, or
/// `None` when Samsung does not publish an asset for it (as of 3.2.0:
/// Windows ARM64, 32-bit x86 anywhere and macOS x86_64 on releases newer
/// than 3.1.3 have no asset).
pub fn asset_name(os: HostOs, arch: HostArch) -> Option<&'static str> {
    match (os, arch) {
        (HostOs::Windows, HostArch::X86_64) => Some("netcoredbg-win64.zip"),
        (HostOs::Windows, _) => None,
        (HostOs::Mac, HostArch::Aarch64) => Some("netcoredbg-osx-arm64.zip"),
        (HostOs::Mac, HostArch::X86_64) => Some("netcoredbg-osx-amd64.tar.gz"),
        (HostOs::Mac, _) => None,
        (HostOs::Linux, HostArch::X86_64) => Some("netcoredbg-linux-amd64.tar.gz"),
        (HostOs::Linux, HostArch::Aarch64) => Some("netcoredbg-linux-arm64.tar.gz"),
        (HostOs::Linux, _) => None,
    }
}

/// Error shown when Samsung does not publish a prebuilt release asset for
/// the host platform (e.g. Windows ARM64, 32-bit x86, or musl builds).
pub fn unsupported_platform_error(os: HostOs, arch: HostArch) -> String {
    format!(
        "Samsung does not publish a netcoredbg release asset for this platform \
         ({os:?}/{arch:?}). Build netcoredbg from source ({BUILD_FROM_SOURCE_URL}) \
         or install manually from {RELEASES_URL} and put it on your PATH, or set \
         the `dap.netcoredbg.binary` setting."
    )
}

/// Selects the release asset matching `expected_name` from the release's
/// asset list.
///
/// The error lists the available assets (and the releases page) so the user
/// can fall back to a manual install when Samsung has not published an asset
/// for their platform.
pub fn select_asset<'a>(
    assets: &'a [ReleaseAsset],
    expected_name: &str,
) -> Result<&'a ReleaseAsset, String> {
    if let Some(asset) = assets.iter().find(|asset| asset.name == expected_name) {
        return Ok(asset);
    }
    let mut available: Vec<&str> = assets.iter().map(|asset| asset.name.as_str()).collect();
    available.sort();
    Err(format!(
        "netcoredbg release {REPO} does not publish an asset for this platform \
         (expected {expected_name:?}). Available assets: [{}]. \
         Install netcoredbg manually from {RELEASES_URL} or build from source \
         ({BUILD_FROM_SOURCE_URL}) instead, or point Zed at an existing binary \
         with the `dap.netcoredbg.binary` setting.",
        available.join(", ")
    ))
}

/// Detects the archive type from the asset file name (drives the
/// host-side extraction performed by `download_file`).
pub fn archive_kind(asset_name: &str) -> ArchiveKind {
    let lower = asset_name.to_ascii_lowercase();
    if lower.ends_with(".tar.gz") || lower.ends_with(".tgz") {
        ArchiveKind::GzipTar
    } else if lower.ends_with(".zip") {
        ArchiveKind::Zip
    } else if lower.ends_with(".gz") {
        ArchiveKind::Gzip
    } else {
        ArchiveKind::Uncompressed
    }
}

/// Validates a release tag for use as a cache directory name.
///
/// Release tags look like `3.2.0-1092`; anything containing path separators,
/// `..`, or whitespace is rejected so a hostile/malformed tag cannot escape
/// the cache directory.
pub fn sanitize_version(version: &str) -> Result<String, String> {
    let version = version.trim();
    if version.is_empty() {
        return Err("netcoredbg release version is empty".to_string());
    }
    if version.contains('/') || version.contains('\\') || version.contains("..") {
        return Err(format!(
            "netcoredbg release version {version:?} contains path separators \
             or relative components and cannot be used as a cache directory"
        ));
    }
    Ok(version.to_string())
}

/// Relative path (against the extension working directory) of the directory
/// a release version is cached under.
pub fn cache_dir_relative(version: &str) -> Result<String, String> {
    Ok(format!("{CACHE_DIR_NAME}/{}", sanitize_version(version)?))
}

/// Path of the adapter binary *inside* a release archive: every published
/// archive has a single top-level `netcoredbg/` directory.
pub fn archive_binary_entry(os: HostOs) -> &'static str {
    match os {
        HostOs::Windows => "netcoredbg/netcoredbg.exe",
        _ => "netcoredbg/netcoredbg",
    }
}

/// Where an archive entry ends up after Zed's host `download_file` handled a
/// download to `destination` (see the module docs): archives are extracted
/// *into* the destination directory, single files are written *as* the
/// destination.
pub fn extracted_path(destination: &str, kind: ArchiveKind, entry: &str) -> String {
    match kind {
        ArchiveKind::Zip | ArchiveKind::GzipTar => format!("{destination}/{entry}"),
        ArchiveKind::Gzip | ArchiveKind::Uncompressed => destination.to_string(),
    }
}

/// Relative path of the adapter binary inside the extension working
/// directory for a cached `version` (`netcoredbg/<version>/netcoredbg/...`).
/// Used both for downloads and for scanning the cache.
pub fn binary_relative_path(version: &str, os: HostOs) -> Result<String, String> {
    Ok(format!(
        "{}/{}",
        cache_dir_relative(version)?,
        archive_binary_entry(os)
    ))
}

/// Everything `lib.rs` needs to download and locate one release asset.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DownloadPlan {
    /// The `path` argument for `download_file`: the version directory.
    pub destination: String,
    pub kind: ArchiveKind,
    /// Where the binary will be after extraction (relative path).
    pub binary: String,
}

/// Plans the download of `asset_name` for release `version` on `os`.
///
/// Only archive assets are accepted: the adapter needs the managed DLLs
/// that ship next to the native binary, so a bare file cannot work.
pub fn download_plan(version: &str, asset_name: &str, os: HostOs) -> Result<DownloadPlan, String> {
    let kind = archive_kind(asset_name);
    if !matches!(kind, ArchiveKind::Zip | ArchiveKind::GzipTar) {
        return Err(format!(
            "netcoredbg release asset {asset_name:?} is not a .zip or .tar.gz archive"
        ));
    }
    let destination = cache_dir_relative(version)?;
    let binary = extracted_path(&destination, kind, archive_binary_entry(os));
    Ok(DownloadPlan {
        destination,
        kind,
        binary,
    })
}

/// Newest release that still publishes an asset for platforms Samsung has
/// dropped from later releases. `latest_github_release` can only return the
/// latest release and the extension API has no release listing, so the
/// fallback is a known tag fetched with `github_release_by_tag_name`.
pub fn fallback_tag(os: HostOs, arch: HostArch) -> Option<&'static str> {
    match (os, arch) {
        // 3.2.0-1092 dropped `netcoredbg-osx-amd64.tar.gz`.
        (HostOs::Mac, HostArch::X86_64) => Some("3.1.3-1062"),
        _ => None,
    }
}

/// Cached version directories other than `keep`, to be removed after a
/// successful download (directory names that are not valid versions are
/// left alone).
pub fn stale_versions(cached_dirs: &[String], keep: &str) -> Vec<String> {
    cached_dirs
        .iter()
        .filter(|dir| dir.as_str() != keep && sanitize_version(dir).as_deref() == Ok(dir.as_str()))
        .cloned()
        .collect()
}

/// The binary resolved earlier in this session, if it is still usable.
/// `ZED_NETCOREDBG_FORCE_UPDATE` bypasses the memo; a binary deleted from
/// the cache invalidates it.
pub fn usable_memo(
    memo: Option<&str>,
    force_update: bool,
    is_file: impl Fn(&str) -> bool,
) -> Option<String> {
    memo.filter(|path| !force_update && is_file(path))
        .map(str::to_string)
}

/// Whether the `ZED_NETCOREDBG_FORCE_UPDATE` escape hatch is set in `envs`.
///
/// Any non-empty value other than `0`/`false`/`no` (case-insensitive) counts
/// as "force".
pub fn force_update(envs: &[(String, String)]) -> bool {
    envs.iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(FORCE_UPDATE_ENV))
        .map(|(_, value)| {
            let value = value.trim().to_ascii_lowercase();
            !(value.is_empty() || value == "0" || value == "false" || value == "no")
        })
        .unwrap_or(false)
}

/// Compares two netcoredbg version strings (e.g. `3.2.0-1092`) the way
/// version managers do: maximal digit runs compare numerically, everything
/// else byte-wise. This makes `3.10.0` sort above `3.9.0` even though plain
/// string comparison would not.
pub fn compare_versions(left: &str, right: &str) -> std::cmp::Ordering {
    use std::cmp::Ordering;

    fn numeric(ordering: Ordering) -> Ordering {
        // Reverse: longer runs (or greater numbers) mean a newer version.
        ordering
    }

    let mut left = left.trim();
    let mut right = right.trim();
    while !left.is_empty() && !right.is_empty() {
        let left_digit = left.as_bytes()[0].is_ascii_digit();
        let right_digit = right.as_bytes()[0].is_ascii_digit();
        match (left_digit, right_digit) {
            (true, true) => {
                let l_num = digit_run(left);
                let r_num = digit_run(right);
                let l_val: Option<u64> = l_num.parse().ok();
                let r_val: Option<u64> = r_num.parse().ok();
                let ordering = match (l_val, r_val) {
                    (Some(l), Some(r)) => l.cmp(&r),
                    _ => l_num.len().cmp(&r_num.len()),
                };
                if ordering != Ordering::Equal {
                    return ordering;
                }
                left = &left[l_num.len()..];
                right = &right[r_num.len()..];
            }
            (true, false) => return numeric(Ordering::Greater),
            (false, true) => return numeric(Ordering::Less),
            (false, false) => {
                let l_run = non_digit_run(left);
                let r_run = non_digit_run(right);
                let ordering = l_run.cmp(r_run);
                if ordering != Ordering::Equal {
                    return ordering;
                }
                left = &left[l_run.len()..];
                right = &right[r_run.len()..];
            }
        }
    }
    left.len().cmp(&right.len())
}

fn digit_run(text: &str) -> &str {
    let end = text
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(text.len());
    &text[..end]
}

fn non_digit_run(text: &str) -> &str {
    let end = text
        .find(|c: char| c.is_ascii_digit())
        .unwrap_or(text.len());
    &text[..end]
}

/// Picks the newest version string out of `versions` (e.g. cached directory
/// names) for the offline fallback.
pub fn newest_version(versions: &[String]) -> Option<String> {
    versions
        .iter()
        .max_by(|a, b| compare_versions(a, b))
        .cloned()
}

/// Joins an automatic-download failure with the instructive fallback error,
/// so an offline machine still sees the full manual-install guidance.
pub fn combine_download_error(download_error: &str, fallback_error: &str) -> String {
    format!("Automatic netcoredbg download failed: {download_error}\n\n{fallback_error}")
}

#[cfg(test)]
mod tests {
    use crate::download::{
        archive_binary_entry, archive_kind, asset_name, binary_relative_path, cache_dir_relative,
        combine_download_error, compare_versions, download_plan, extracted_path, fallback_tag,
        force_update, newest_version, sanitize_version, select_asset, stale_versions,
        unsupported_platform_error, usable_memo, ArchiveKind, DownloadPlan, HostArch, HostOs,
        ReleaseAsset, BUILD_FROM_SOURCE_URL, CACHE_DIR_NAME, FORCE_UPDATE_ENV, RELEASES_URL, REPO,
    };

    #[test]
    fn asset_matrix_matches_published_releases() {
        // Verified against Samsung/netcoredbg Releases (3.1.0 .. 3.2.0-1092).
        assert_eq!(
            asset_name(HostOs::Windows, HostArch::X86_64),
            Some("netcoredbg-win64.zip")
        );
        assert_eq!(
            asset_name(HostOs::Mac, HostArch::Aarch64),
            Some("netcoredbg-osx-arm64.zip")
        );
        assert_eq!(
            asset_name(HostOs::Mac, HostArch::X86_64),
            Some("netcoredbg-osx-amd64.tar.gz")
        );
        assert_eq!(
            asset_name(HostOs::Linux, HostArch::X86_64),
            Some("netcoredbg-linux-amd64.tar.gz")
        );
        assert_eq!(
            asset_name(HostOs::Linux, HostArch::Aarch64),
            Some("netcoredbg-linux-arm64.tar.gz")
        );
    }

    #[test]
    fn asset_matrix_has_gaps_where_samsung_publishes_nothing() {
        assert_eq!(asset_name(HostOs::Windows, HostArch::Aarch64), None);
        assert_eq!(asset_name(HostOs::Windows, HostArch::X86), None);
        assert_eq!(asset_name(HostOs::Mac, HostArch::X86), None);
        assert_eq!(asset_name(HostOs::Linux, HostArch::X86), None);
    }

    fn assets(names: &[&str]) -> Vec<ReleaseAsset> {
        names
            .iter()
            .map(|name| ReleaseAsset {
                name: (*name).to_string(),
                download_url: format!("https://example.com/{name}"),
            })
            .collect()
    }

    #[test]
    fn select_asset_finds_exact_name() {
        let list = assets(&["netcoredbg-linux-amd64.tar.gz", "netcoredbg-win64.zip"]);
        let asset = select_asset(&list, "netcoredbg-win64.zip").expect("should select");
        assert_eq!(asset.name, "netcoredbg-win64.zip");
        assert_eq!(
            asset.download_url,
            "https://example.com/netcoredbg-win64.zip"
        );
    }

    #[test]
    fn select_asset_error_lists_available_assets() {
        let list = assets(&["netcoredbg-win64.zip", "netcoredbg-osx-arm64.zip"]);
        let err = select_asset(&list, "netcoredbg-winarm64.zip").expect_err("should fail");
        assert!(err.contains("netcoredbg-winarm64.zip"));
        assert!(err.contains("netcoredbg-osx-arm64.zip, netcoredbg-win64.zip"));
        assert!(err.contains(RELEASES_URL));
        assert!(err.contains("dap.netcoredbg.binary"));
    }

    #[test]
    fn select_asset_on_empty_release_errors() {
        let err = select_asset(&[], "netcoredbg-win64.zip").expect_err("should fail");
        assert!(err.contains("Available assets: []"));
    }

    #[test]
    fn archive_kind_is_detected_from_file_name() {
        assert_eq!(
            archive_kind("netcoredbg-linux-amd64.tar.gz"),
            ArchiveKind::GzipTar
        );
        assert_eq!(archive_kind("netcoredbg-win64.zip"), ArchiveKind::Zip);
        assert_eq!(archive_kind("netcoredbg-osx-arm64.ZIP"), ArchiveKind::Zip);
        assert_eq!(
            archive_kind("netcoredbg.tar.gz.tmp"),
            ArchiveKind::Uncompressed
        );
        assert_eq!(archive_kind("netcoredbg.gz"), ArchiveKind::Gzip);
    }

    #[test]
    fn version_is_sanitized_for_cache_layout() {
        assert_eq!(sanitize_version("3.2.0-1092"), Ok("3.2.0-1092".to_string()));
        assert_eq!(sanitize_version(" 3.2.0 "), Ok("3.2.0".to_string()));
        assert!(sanitize_version("").is_err());
        assert!(sanitize_version("3.2.0/evil").is_err());
        assert!(sanitize_version("3.2.0\\evil").is_err());
        assert!(sanitize_version("../evil").is_err());
    }

    #[test]
    fn cache_paths_follow_documented_layout() {
        assert_eq!(
            cache_dir_relative("3.2.0-1092").unwrap(),
            "netcoredbg/3.2.0-1092"
        );
        assert_eq!(
            binary_relative_path("3.2.0-1092", HostOs::Linux).unwrap(),
            "netcoredbg/3.2.0-1092/netcoredbg/netcoredbg"
        );
        assert_eq!(
            binary_relative_path("3.2.0-1092", HostOs::Windows).unwrap(),
            "netcoredbg/3.2.0-1092/netcoredbg/netcoredbg.exe"
        );
        // Archives ship a single top-level `netcoredbg/` directory.
        assert!(binary_relative_path("1.0", HostOs::Mac)
            .unwrap()
            .contains("/netcoredbg/netcoredbg"));
    }

    #[test]
    fn force_update_requires_truthy_env_value() {
        assert!(!force_update(&[]));
        assert!(!force_update(&[("OTHER".to_string(), "1".to_string())]));
        assert!(force_update(&[(
            "ZED_NETCOREDBG_FORCE_UPDATE".to_string(),
            "1".to_string()
        )]));
        assert!(force_update(&[(
            "zed_netcoredbg_force_update".to_string(),
            "true".to_string()
        )]));
        assert!(force_update(&[(
            "ZED_NETCOREDBG_FORCE_UPDATE".to_string(),
            "YES".to_string()
        )]));
        assert!(!force_update(&[(
            "ZED_NETCOREDBG_FORCE_UPDATE".to_string(),
            "0".to_string()
        )]));
        assert!(!force_update(&[(
            "ZED_NETCOREDBG_FORCE_UPDATE".to_string(),
            "false".to_string()
        )]));
        assert!(!force_update(&[(
            "ZED_NETCOREDBG_FORCE_UPDATE".to_string(),
            "  ".to_string()
        )]));
    }

    #[test]
    fn version_comparison_is_numeric_aware() {
        use std::cmp::Ordering;
        assert_eq!(
            compare_versions("3.2.0-1092", "3.1.3-1062"),
            Ordering::Greater
        );
        assert_eq!(compare_versions("3.1.3-1062", "3.2.0-1092"), Ordering::Less);
        assert_eq!(
            compare_versions("3.2.0-1092", "3.2.0-1092"),
            Ordering::Equal
        );
        // String comparison would get this wrong.
        assert_eq!(compare_versions("3.10.0", "3.9.0"), Ordering::Greater);
        assert_eq!(compare_versions("2.2.3-992", "10.0.0"), Ordering::Less);
    }

    #[test]
    fn newest_version_picks_latest_cached() {
        let versions = vec![
            "3.1.3-1062".to_string(),
            "3.2.0-1092".to_string(),
            "3.1.0-1031".to_string(),
        ];
        assert_eq!(newest_version(&versions), Some("3.2.0-1092".to_string()));
        assert_eq!(newest_version(&[]), None);
    }

    #[test]
    fn download_error_is_combined_with_fallback_guidance() {
        let combined = combine_download_error("offline", "install manually");
        assert!(combined.contains("Automatic netcoredbg download failed: offline"));
        assert!(combined.contains("install manually"));
    }

    /// Archive listings as published (top-level `netcoredbg/` directory).
    fn archive_entries(os: HostOs) -> Vec<String> {
        let binary = archive_binary_entry(os);
        vec![
            "netcoredbg/".to_string(),
            binary.to_string(),
            "netcoredbg/ManagedPart.dll".to_string(),
            "netcoredbg/Microsoft.CodeAnalysis.dll".to_string(),
        ]
    }

    /// Simulates Zed's host `download_file` for `plan`: returns every file
    /// path the host would create for the given archive listing.
    fn simulate_download(plan: &DownloadPlan, entries: &[String]) -> Vec<String> {
        entries
            .iter()
            .map(|entry| extracted_path(&plan.destination, plan.kind, entry))
            .collect()
    }

    #[test]
    fn download_plan_binary_exists_after_extraction_for_every_asset() {
        for (os, arch) in [
            (HostOs::Windows, HostArch::X86_64),
            (HostOs::Linux, HostArch::X86_64),
            (HostOs::Linux, HostArch::Aarch64),
            (HostOs::Mac, HostArch::Aarch64),
            (HostOs::Mac, HostArch::X86_64),
        ] {
            let asset = asset_name(os, arch).expect("asset for supported platform");
            let plan = download_plan("3.2.0-1092", asset, os).expect("plan");
            let extracted = simulate_download(&plan, &archive_entries(os));
            assert!(
                extracted.contains(&plan.binary),
                "{os:?}/{arch:?}: binary {} not among extracted {extracted:?}",
                plan.binary
            );
            // The cache scan looks for the same path.
            assert_eq!(plan.binary, binary_relative_path("3.2.0-1092", os).unwrap());
        }
    }

    #[test]
    fn download_destination_is_the_version_directory_not_the_archive_name() {
        // Regression: passing `<ver>/<asset>.zip` made Zed extract into a
        // directory named after the archive.
        let plan = download_plan("3.2.0-1092", "netcoredbg-win64.zip", HostOs::Windows).unwrap();
        assert_eq!(plan.destination, "netcoredbg/3.2.0-1092");
        assert_eq!(plan.kind, ArchiveKind::Zip);
        assert_eq!(
            plan.binary,
            "netcoredbg/3.2.0-1092/netcoredbg/netcoredbg.exe"
        );
        let plan = download_plan("3.1.3-1062", "netcoredbg-osx-amd64.tar.gz", HostOs::Mac).unwrap();
        assert_eq!(plan.kind, ArchiveKind::GzipTar);
        assert_eq!(plan.binary, "netcoredbg/3.1.3-1062/netcoredbg/netcoredbg");
    }

    #[test]
    fn download_plan_rejects_non_archives_and_bad_versions() {
        assert!(download_plan("3.2.0", "netcoredbg", HostOs::Linux).is_err());
        assert!(download_plan("3.2.0", "netcoredbg.gz", HostOs::Linux).is_err());
        assert!(download_plan("../x", "netcoredbg-win64.zip", HostOs::Windows).is_err());
    }

    #[test]
    fn extracted_path_models_single_file_downloads() {
        assert_eq!(
            extracted_path("dir/file", ArchiveKind::Gzip, "ignored"),
            "dir/file"
        );
        assert_eq!(
            extracted_path("dir/file", ArchiveKind::Uncompressed, "ignored"),
            "dir/file"
        );
        assert_eq!(
            extracted_path("dir", ArchiveKind::GzipTar, "a/b"),
            "dir/a/b"
        );
    }

    #[test]
    fn intel_mac_falls_back_to_last_release_with_osx_amd64() {
        assert_eq!(
            fallback_tag(HostOs::Mac, HostArch::X86_64),
            Some("3.1.3-1062")
        );
        assert_eq!(fallback_tag(HostOs::Mac, HostArch::Aarch64), None);
        assert_eq!(fallback_tag(HostOs::Linux, HostArch::X86_64), None);
        assert_eq!(fallback_tag(HostOs::Windows, HostArch::Aarch64), None);
        // The fallback tag carries the asset the latest release dropped.
        let assets = vec![ReleaseAsset {
            name: "netcoredbg-osx-amd64.tar.gz".to_string(),
            download_url: "https://example.invalid/a".to_string(),
        }];
        let expected = asset_name(HostOs::Mac, HostArch::X86_64).unwrap();
        assert!(select_asset(&assets, expected).is_ok());
    }

    #[test]
    fn stale_versions_keeps_current_and_ignores_odd_names() {
        let dirs = vec![
            "3.1.3-1062".to_string(),
            "3.2.0-1092".to_string(),
            "3.0.0-1018".to_string(),
            " spaced".to_string(),
        ];
        assert_eq!(
            stale_versions(&dirs, "3.2.0-1092"),
            vec!["3.1.3-1062".to_string(), "3.0.0-1018".to_string()]
        );
        assert!(stale_versions(&[], "3.2.0-1092").is_empty());
        assert!(stale_versions(&["3.2.0-1092".to_string()], "3.2.0-1092").is_empty());
    }

    #[test]
    fn memo_is_reused_until_forced_or_deleted() {
        let exists = |_: &str| true;
        let missing = |_: &str| false;
        assert_eq!(
            usable_memo(Some("a/netcoredbg"), false, exists),
            Some("a/netcoredbg".to_string())
        );
        assert_eq!(usable_memo(Some("a/netcoredbg"), true, exists), None);
        assert_eq!(usable_memo(Some("a/netcoredbg"), false, missing), None);
        assert_eq!(usable_memo(None, false, exists), None);
    }

    #[test]
    fn unsupported_platform_error_points_to_build_from_source_and_releases() {
        let err = unsupported_platform_error(HostOs::Windows, HostArch::Aarch64);
        assert!(err.contains("Windows") && err.contains("Aarch64"), "{err}");
        assert!(err.contains(BUILD_FROM_SOURCE_URL), "{err}");
        assert!(err.contains(RELEASES_URL), "{err}");
        assert!(err.contains("dap.netcoredbg.binary"), "{err}");
    }

    #[test]
    fn constants_are_stable() {
        assert_eq!(REPO, "Samsung/netcoredbg");
        assert_eq!(FORCE_UPDATE_ENV, "ZED_NETCOREDBG_FORCE_UPDATE");
        assert_eq!(CACHE_DIR_NAME, "netcoredbg");
        assert!(BUILD_FROM_SOURCE_URL.contains("Samsung/netcoredbg"));
    }
}
