//! Asks GitHub for the latest PdfCraft release (Help ▸ Check for updates, issue #28), and in
//! this personal build downloads it and swaps the program in place ([`install`], [`apply`]).

use std::io::Read;
use std::time::Duration;

use pdfcraft_ui_egui::updates::{RELEASES_PAGE, Release};

/// The fork's releases, newest first. Not `releases/latest`: GitHub leaves pre-releases out of
/// that one, and every build of this fork was published as one, so it answered 404 and the
/// check never found an update.
const RELEASES: &str = "https://api.github.com/repos/contato846/pdfcraft/releases?per_page=20";

// What the check says when it fails. Fixed English sentences, so the dialog shows them in the
// interface language (its catalogs have them); the details go to the log.
const UNREACHABLE: &str = "couldn't reach the update server; check the internet connection";
const UNEXPECTED: &str = "the update server didn't answer as expected";
const LIMITED_MINUTES: &str = "the update server is limiting checks for now; try again in a few minutes";
const LIMITED_HALF_HOUR: &str = "the update server is limiting checks for now; try again in about half an hour";
const LIMITED_HOUR: &str = "the update server is limiting checks for now; try again in about an hour";
const LIMITED: &str = "the update server is limiting checks for now; try again later";

/// The tags this fork's builds carry (`fontes-livres-<commit>`).
const TAG_PREFIX: &str = "fontes-livres-";

/// Where a release's downloads live; only an asset under it is ever fetched.
const DOWNLOADS: &str = "https://github.com/contato846/pdfcraft/releases/download/";

/// The portable build inside the release zip.
const PORTABLE_EXE: &str = "PdfCraft/pdfcraft.exe";

/// The most a release download may be.
const MAX_DOWNLOAD: u64 = 300 << 20;

/// The latest release. The answer is untrusted: its size is capped, and only a page under
/// [`RELEASES_PAGE`] is ever offered for download (anything else falls back to that list).
pub fn latest_release() -> Result<Release, String> {
    let agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(10)))
        .tls_config(ureq::tls::TlsConfig::builder().root_certs(os_roots()?).build())
        .build()
        .new_agent();
    let mut response = agent
        .get(RELEASES)
        .header("Accept", "application/vnd.github+json")
        .header("User-Agent", concat!("PdfCraft/", env!("CARGO_PKG_VERSION")))
        // Read error statuses ourselves: a 403 from GitHub usually means its anonymous
        // rate limit ran out, and the retry time rides along in a response header.
        .config()
        .http_status_as_error(false)
        .build()
        .call()
        .map_err(|e| {
            log::warn!("update check: {e}");
            UNREACHABLE.to_string()
        })?;
    let status = response.status().as_u16();
    if status == 403 {
        let reset = response.headers().get("x-ratelimit-reset").and_then(|v| v.to_str().ok());
        return Err(rate_limited(reset, now_unix()).into());
    }
    if !(200..300).contains(&status) {
        log::warn!("update check: http status {status}");
        return Err(UNEXPECTED.into());
    }
    let body = response.body_mut().with_config().limit(1 << 20).read_to_string().map_err(|e| {
        log::warn!("update check: {e}");
        UNEXPECTED.to_string()
    })?;
    parse_list(&body)
}

/// The newest published build of this fork in a list of releases (newest first): drafts and
/// other tags are skipped; a pre-release counts.
fn parse_list(body: &str) -> Result<Release, String> {
    let v: serde_json::Value = serde_json::from_str(body).map_err(|e| format!("unreadable answer ({e})"))?;
    let newest = v
        .as_array()
        .ok_or("no release found")?
        .iter()
        .find(|r| !r["draft"].as_bool().unwrap_or(false) && r["tag_name"].as_str().is_some_and(|t| t.starts_with(TAG_PREFIX)))
        .ok_or("no release found")?;
    parse_value(newest)
}

/// The certificate authorities the operating system trusts.
pub(crate) fn os_roots() -> Result<ureq::tls::RootCerts, String> {
    let found = rustls_native_certs::load_native_certs();
    let certs: Vec<ureq::tls::Certificate<'static>> = found.certs.iter().map(|c| ureq::tls::Certificate::from_der(c.as_ref()).to_owned()).collect();
    if certs.is_empty() {
        return Err("no trusted certificates found on this system".into());
    }
    Ok(ureq::tls::RootCerts::new_with_certs(&certs))
}

/// The current Unix time in seconds (0 when the clock gives nothing usable).
fn now_unix() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// What to tell the user when the update server answers 403: its anonymous quota (60 checks an
/// hour, shared by everyone behind the same address) ran out. `reset` is the `x-ratelimit-reset`
/// header, the Unix time the quota refills; `now` is the current Unix time. The wait is said
/// roughly (a few minutes, half an hour, an hour), in a fixed sentence the dialog translates;
/// anything surprising (no header, not a number, already past) asks for patience.
fn rate_limited(reset: Option<&str>, now: u64) -> &'static str {
    let wait = reset.and_then(|r| r.trim().parse::<u64>().ok()).and_then(|reset| reset.checked_sub(now)).filter(|&wait| wait > 0);
    match wait {
        Some(w) if w <= 10 * 60 => LIMITED_MINUTES,
        Some(w) if w <= 40 * 60 => LIMITED_HALF_HOUR,
        Some(_) => LIMITED_HOUR,
        None => LIMITED,
    }
}

#[cfg(test)]
fn parse(body: &str) -> Result<Release, String> {
    let v: serde_json::Value = serde_json::from_str(body).map_err(|e| format!("unreadable answer ({e})"))?;
    parse_value(&v)
}

/// One release from GitHub's answer.
fn parse_value(v: &serde_json::Value) -> Result<Release, String> {
    let version = v["tag_name"].as_str().filter(|t| !t.is_empty() && t.len() <= 64).ok_or("no release found")?.to_string();
    let url = v["html_url"]
        .as_str()
        .filter(|u| u.strip_prefix(RELEASES_PAGE).is_some_and(|rest| rest.starts_with('/') && !rest.contains(['?', '#', '\\'])))
        .unwrap_or(RELEASES_PAGE)
        .to_string();
    // The portable Windows build: the one zip the program can install over itself.
    let asset = v["assets"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|a| a["name"].as_str().is_some_and(|n| n.ends_with("windows-x64-portable.zip")))
        .filter_map(|a| a["browser_download_url"].as_str())
        .find(|u| u.strip_prefix(DOWNLOADS).is_some_and(|rest| !rest.is_empty() && !rest.contains(['?', '#', '\\'])))
        .filter(|_| cfg!(target_os = "windows"))
        .map(str::to_owned);
    Ok(Release { version, url, asset })
}

/// The program's own file and the names the update uses beside it: the staged new one and the
/// old one left after a swap.
fn exe_paths() -> Result<(std::path::PathBuf, std::path::PathBuf, std::path::PathBuf), String> {
    let exe = std::env::current_exe().map_err(|e| format!("the program's own location is unknown ({e})"))?;
    let name = exe.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "pdfcraft.exe".into());
    let new = exe.with_file_name(format!("{name}.new"));
    let old = exe.with_file_name(format!("{name}.old"));
    Ok((exe, new, old))
}

/// Download the release `asset` and stage the new program beside the running one
/// (`pdfcraft.exe.new`). `progress` gets the bytes received so far.
pub fn install(asset: &str, progress: &mut dyn FnMut(u64)) -> Result<(), String> {
    if !asset.starts_with(DOWNLOADS) {
        return Err("the download is not on PdfCraft's release page".into());
    }
    let (_, new, _) = exe_paths()?;
    let agent = ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(20)))
        .timeout_recv_response(Some(Duration::from_secs(60)))
        // Redirects are followed below, by hand: the release download answers with a redirect
        // whose (chunked) body the automatic following failed to read, so every update stopped
        // there with "chunk expected crlf".
        .max_redirects(0)
        .http_status_as_error(false)
        .tls_config(ureq::tls::TlsConfig::builder().root_certs(os_roots()?).build())
        .build()
        .new_agent();
    let mut url = asset.to_owned();
    let mut hops = 0;
    let response = loop {
        let response = agent
            .get(&url)
            .header("User-Agent", concat!("PdfCraft/", env!("CARGO_PKG_VERSION")))
            .call()
            .map_err(|e| format!("couldn't download the new version ({e})"))?;
        let status = response.status().as_u16();
        if (200..300).contains(&status) {
            break response;
        }
        let location = response.headers().get("location").and_then(|v| v.to_str().ok()).map(str::to_owned);
        match location {
            Some(next) if (300..400).contains(&status) && hops < 5 && trusted_download(&next) => {
                hops += 1;
                url = next;
            }
            _ => return Err(format!("couldn't download the new version (http status: {status})")),
        }
    };
    let mut reader = response.into_body().into_reader().take(MAX_DOWNLOAD + 1);
    let mut zip = Vec::new();
    let mut chunk = [0u8; 64 << 10];
    loop {
        let n = reader.read(&mut chunk).map_err(|e| format!("the download was cut off ({e})"))?;
        if n == 0 {
            break;
        }
        zip.extend_from_slice(&chunk[..n]);
        if zip.len() as u64 > MAX_DOWNLOAD {
            return Err("the download is larger than a PdfCraft release".into());
        }
        progress(zip.len() as u64);
    }
    let exe = crate::zipread::entry(&zip, PORTABLE_EXE, MAX_DOWNLOAD as usize)?;
    if !exe.starts_with(b"MZ") {
        return Err("the download does not hold a Windows program".into());
    }
    std::fs::write(&new, &exe).map_err(|e| format!("couldn't write {} ({e})", new.display()))?;
    Ok(())
}

/// Where a release download may lead: GitHub and its file servers, over https only.
fn trusted_download(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("https://") else { return false };
    let host = rest.split(['/', '?', '#']).next().unwrap_or_default().to_ascii_lowercase();
    !host.contains(['@', ':']) && (host == "github.com" || host.ends_with(".githubusercontent.com"))
}

/// Put the staged program in the running one's place and start it. On Windows a running
/// program can be renamed but not overwritten: the running one becomes `pdfcraft.exe.old`
/// (removed at the next start), the staged one takes its name.
pub fn apply() -> Result<(), String> {
    let (exe, new, old) = exe_paths()?;
    if !new.is_file() {
        return Err("no new version is staged".into());
    }
    let _ = std::fs::remove_file(&old);
    std::fs::rename(&exe, &old).map_err(|e| format!("couldn't move the running program aside ({e})"))?;
    if let Err(e) = std::fs::rename(&new, &exe) {
        // Put the running one back, so the next start still works.
        let _ = std::fs::rename(&old, &exe);
        return Err(format!("couldn't put the new version in place ({e})"));
    }
    std::process::Command::new(&exe).spawn().map_err(|e| format!("couldn't start the new version ({e})"))?;
    Ok(())
}

/// At start: the program a swap left behind is no longer running and goes.
pub fn remove_old_program() {
    if let Ok((_, new, old)) = exe_paths() {
        let _ = std::fs::remove_file(old);
        // A staged version that was never applied is stale by now too.
        let _ = std::fs::remove_file(new);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn downloads_follow_only_github_redirects() {
        assert!(trusted_download("https://release-assets.githubusercontent.com/github-production-release-asset/1/x?sp=r"));
        assert!(trusted_download("https://objects.githubusercontent.com/x"));
        assert!(trusted_download("https://github.com/contato846/pdfcraft/releases/download/t/x.zip"));
        for bad in [
            "http://objects.githubusercontent.com/x",
            "https://githubusercontent.com.evil.example/x",
            "https://evil.example/?h=github.com",
            "https://user@objects.githubusercontent.com/x",
            "https://objects.githubusercontent.com:444/x",
            "ftp://github.com/x",
        ] {
            assert!(!trusted_download(bad), "{bad}");
        }
    }

    #[test]
    fn the_newest_build_of_the_fork_is_found_in_the_release_list() {
        let rel = |tag: &str, draft: bool, pre: bool| {
            format!(
                r#"{{"tag_name":"{tag}","draft":{draft},"prerelease":{pre},"html_url":"https://github.com/contato846/pdfcraft/releases/tag/{tag}"}}"#
            )
        };
        // Pre-releases count (every build so far is one); a draft and another tag don't.
        let list = format!(
            "[{},{},{},{}]",
            rel("fontes-livres-aaaaaaa", true, false),
            rel("v9.0.0", false, false),
            rel("fontes-livres-f933f24", false, true),
            rel("fontes-livres-72d3746", false, true)
        );
        assert_eq!(parse_list(&list).unwrap().version, "fontes-livres-f933f24");
        assert!(parse_list("[]").is_err());
        assert!(parse_list(r#"{"message":"Not Found"}"#).is_err(), "what releases/latest answered");
        assert!(parse_list("<html>").is_err());
    }

    #[test]
    fn answers_are_read_and_only_our_release_pages_are_offered() {
        let r = parse(r#"{"tag_name":"v0.2.0","html_url":"https://github.com/contato846/pdfcraft/releases/tag/v0.2.0"}"#).unwrap();
        assert_eq!(r, Release { version: "v0.2.0".into(), url: "https://github.com/contato846/pdfcraft/releases/tag/v0.2.0".into(), asset: None });
        for elsewhere in ["https://example.com/pdfcraft.exe", "https://github.com/contato846/pdfcraft/releases.evil/x", "javascript:alert(1)"] {
            let r = parse(&format!(r#"{{"tag_name":"v9.9.9","html_url":"{elsewhere}"}}"#)).unwrap();
            assert_eq!(r.url, RELEASES_PAGE, "{elsewhere}");
        }
        assert!(parse(r#"{"message":"Not Found"}"#).is_err());
        assert!(parse("<html>").is_err());
    }

    #[test]
    fn only_the_portable_zip_on_our_release_page_is_installable() {
        let body = |url: &str| {
            format!(
                r#"{{"tag_name":"fontes-livres-26c762b","html_url":"https://github.com/contato846/pdfcraft/releases/tag/x","assets":[{{"name":"notes.txt","browser_download_url":"{DOWNLOADS}x/notes.txt"}},{{"name":"pdfcraft-fontes-livres-windows-x64-portable.zip","browser_download_url":"{url}"}}]}}"#
            )
        };
        let ours = format!("{DOWNLOADS}fontes-livres-26c762b/pdfcraft-fontes-livres-windows-x64-portable.zip");
        let r = parse(&body(&ours)).unwrap();
        if cfg!(target_os = "windows") {
            assert_eq!(r.asset.as_deref(), Some(ours.as_str()));
        } else {
            assert_eq!(r.asset, None, "only Windows installs itself");
        }
        for elsewhere in ["https://example.com/pdfcraft.zip", "https://github.com/storytold/pdfcraft/releases/download/v1/x.zip"] {
            assert_eq!(parse(&body(elsewhere)).unwrap().asset, None, "{elsewhere}");
        }
        assert!(install("https://example.com/x.zip", &mut |_| {}).is_err());
    }

    #[test]
    fn rate_limit_messages_name_the_wait() {
        assert_eq!(rate_limited(Some("1800000061"), 1800000000), LIMITED_MINUTES);
        assert_eq!(rate_limited(Some("1800000600"), 1800000000), LIMITED_MINUTES);
        assert_eq!(rate_limited(Some("1800001200"), 1800000000), LIMITED_HALF_HOUR);
        assert_eq!(rate_limited(Some("1800003600"), 1800000000), LIMITED_HOUR);
        // No header, not a number, or already past: ask for patience instead of a time.
        for bad in [None, Some(""), Some("soon"), Some("1799999999"), Some("1800000000")] {
            assert_eq!(rate_limited(bad, 1800000000), LIMITED);
        }
        // The word GitHub is never shown.
        for m in [UNREACHABLE, UNEXPECTED, LIMITED_MINUTES, LIMITED_HALF_HOUR, LIMITED_HOUR, LIMITED] {
            assert!(!m.to_lowercase().contains("github"), "{m}");
        }
    }

    /// Live: downloads a published build and stages it (`cargo test -p pdfcraft -- --ignored`).
    #[test]
    #[ignore = "needs network access"]
    fn a_published_build_downloads_and_stages() {
        let url = "https://github.com/contato846/pdfcraft/releases/download/fontes-livres-419070d/pdfcraft-fontes-livres-windows-x64-portable.zip";
        let mut got = 0;
        install(url, &mut |n| got = n).unwrap();
        let (_, new, _) = exe_paths().unwrap();
        let staged = std::fs::read(&new).unwrap();
        let _ = std::fs::remove_file(&new);
        assert!(staged.starts_with(b"MZ") && got > 1 << 20, "{got}");
    }

    /// Live: asks GitHub over TLS with the OS's roots (`cargo test -p pdfcraft -- --ignored`).
    #[test]
    #[ignore = "needs network access"]
    fn github_answers_with_the_latest_release() {
        let r = latest_release().unwrap();
        assert!(r.version.starts_with(TAG_PREFIX), "{r:?}");
        assert!(r.url.starts_with(RELEASES_PAGE), "{r:?}");
    }
}
