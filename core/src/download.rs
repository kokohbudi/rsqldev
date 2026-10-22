//! On-demand native driver download, DBeaver style: fetch, verify sha256, unpack into
//! `<data_dir>/rsqldev/drivers/<id>/<version>/`.

use crate::registry::{Archive, Driver, DriverFile};
use anyhow::{Context, Result, anyhow, bail};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};

const MARKER: &str = ".rsqldev-installed";

pub fn drivers_root() -> PathBuf {
    dirs::data_dir().unwrap_or_else(std::env::temp_dir).join("rsqldev").join("drivers")
}

fn target_dir(d: &Driver, f: &DriverFile) -> PathBuf {
    drivers_root().join(&d.id).join(&f.version)
}

/// Folder holding the installed client libraries for this platform, if present.
pub fn installed_dir(d: &Driver) -> Option<PathBuf> {
    let f = d.file_for_host()?;
    let dir = target_dir(d, f);
    dir.join(MARKER).exists().then_some(dir)
}

/// Installed version of a driver (any version folder with a marker), for update checks.
pub fn installed_version(d: &Driver) -> Option<String> {
    let rd = fs::read_dir(drivers_root().join(&d.id)).ok()?;
    rd.flatten().filter(|e| e.path().join(MARKER).exists()).map(|e| e.file_name().to_string_lossy().into_owned()).max()
}

pub fn remove(d: &Driver) -> Result<()> {
    let p = drivers_root().join(&d.id);
    if p.exists() {
        fs::remove_dir_all(p)?;
    }
    Ok(())
}

/// Download and install. `progress(done, total)` returns false to abort.
pub fn install(d: &Driver, progress: &mut dyn FnMut(u64, Option<u64>) -> bool) -> Result<PathBuf> {
    let f = d.file_for_host().ok_or_else(|| anyhow!("{} has no download for this platform; choose the client folder manually", d.label))?;
    let dest = target_dir(d, f);
    let parent = dest.parent().unwrap();
    fs::create_dir_all(parent)?;
    let tmp = parent.join(format!(".download-{}", f.version));
    fetch(&f.url, &f.sha256, &tmp, progress)?;
    let staging = parent.join(format!(".staging-{}", f.version));
    let _ = fs::remove_dir_all(&staging);
    fs::create_dir_all(&staging)?;
    let res = match f.archive {
        Archive::Zip => unzip(&tmp, &staging, f.strip),
        Archive::Tgz => untgz(&tmp, &staging, f.strip),
        Archive::Dmg => undmg(&tmp, &staging),
    };
    let _ = fs::remove_file(&tmp);
    if let Err(e) = res {
        let _ = fs::remove_dir_all(&staging);
        return Err(e);
    }
    let _ = fs::remove_dir_all(&dest);
    fs::rename(&staging, &dest)?;
    fs::write(dest.join(MARKER), &f.url)?;
    Ok(dest)
}

fn fetch(url: &str, sha256: &str, out: &Path, progress: &mut dyn FnMut(u64, Option<u64>) -> bool) -> Result<()> {
    let client = reqwest::blocking::Client::builder().timeout(None).user_agent("rsqldev").build()?;
    let mut resp = client.get(url).send()?.error_for_status().with_context(|| format!("GET {url}"))?;
    let total = resp.content_length();
    let mut file = fs::File::create(out)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 256 * 1024];
    let mut done = 0u64;
    loop {
        let n = resp.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        file.write_all(&buf[..n])?;
        done += n as u64;
        if !progress(done, total) {
            drop(file);
            let _ = fs::remove_file(out);
            bail!("download cancelled");
        }
    }
    let got = hex(&hasher.finalize());
    if !got.eq_ignore_ascii_case(sha256) {
        let _ = fs::remove_file(out);
        bail!("checksum mismatch for {url}: expected {sha256}, got {got}");
    }
    Ok(())
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// Drop the first `strip` components; refuse anything that is not a plain relative path.
fn safe_rel(p: &Path, strip: usize) -> Option<PathBuf> {
    let mut out = PathBuf::new();
    for (i, c) in p.components().enumerate() {
        match c {
            Component::Normal(s) if i >= strip => out.push(s),
            Component::Normal(_) => {}
            _ => return None,
        }
    }
    (!out.as_os_str().is_empty()).then_some(out)
}

fn unzip(file: &Path, dest: &Path, strip: usize) -> Result<()> {
    let mut z = zip::ZipArchive::new(fs::File::open(file)?)?;
    for i in 0..z.len() {
        let mut e = z.by_index(i)?;
        let Some(name) = e.enclosed_name() else { continue };
        let Some(rel) = safe_rel(&name, strip) else { continue };
        let path = dest.join(rel);
        if e.is_dir() {
            fs::create_dir_all(&path)?;
            continue;
        }
        fs::create_dir_all(path.parent().unwrap())?;
        // Linux Instant Client ships libclntsh.so etc. as symlinks to the versioned library.
        #[cfg(unix)]
        if e.is_symlink() {
            let mut target = String::new();
            std::io::Read::read_to_string(&mut e, &mut target)?;
            // Only links to a sibling file: never out of the install folder.
            if target.is_empty() || target.contains(['/', '\\']) || target == ".." {
                continue;
            }
            let _ = fs::remove_file(&path);
            std::os::unix::fs::symlink(&target, &path)?;
            continue;
        }
        std::io::copy(&mut e, &mut fs::File::create(&path)?)?;
        #[cfg(unix)]
        if let Some(mode) = e.unix_mode() {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(mode & 0o777))?;
        }
    }
    Ok(())
}

fn untgz(file: &Path, dest: &Path, strip: usize) -> Result<()> {
    let mut a = tar::Archive::new(flate2::read::GzDecoder::new(fs::File::open(file)?));
    for e in a.entries()? {
        let mut e = e?;
        let Some(rel) = safe_rel(&e.path()?, strip) else { continue };
        let path = dest.join(rel);
        if let Some(p) = path.parent() {
            fs::create_dir_all(p)?;
        }
        e.unpack(&path)?;
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn undmg(file: &Path, dest: &Path) -> Result<()> {
    use std::process::Command;
    let mnt = dest.with_extension("mnt");
    fs::create_dir_all(&mnt)?;
    let ok = Command::new("hdiutil").args(["attach", "-nobrowse", "-readonly", "-mountpoint"]).arg(&mnt).arg(file).status()?.success();
    if !ok {
        bail!("hdiutil attach failed");
    }
    // ditto keeps the libclntsh.dylib.* symlinks intact.
    let copied = Command::new("ditto").arg(&mnt).arg(dest).status();
    let _ = Command::new("hdiutil").args(["detach", "-quiet"]).arg(&mnt).status();
    let _ = fs::remove_dir(&mnt);
    if !copied?.success() {
        bail!("copy from disk image failed");
    }
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn undmg(_: &Path, _: &Path) -> Result<()> {
    bail!(".dmg archives are macOS only")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn zip_symlinks() {
        use std::io::Write;
        let dir = std::env::temp_dir().join(format!("rsq-zip-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let zf = dir.join("t.zip");
        let mut w = zip::ZipWriter::new(fs::File::create(&zf).unwrap());
        let o = zip::write::SimpleFileOptions::default();
        w.start_file("ic/libclntsh.so.23.1", o).unwrap();
        w.write_all(b"lib").unwrap();
        w.add_symlink("ic/libclntsh.so", "libclntsh.so.23.1", o).unwrap();
        w.add_symlink("ic/evil", "../../etc/passwd", o).unwrap();
        w.finish().unwrap();
        let out = dir.join("out");
        unzip(&zf, &out, 1).unwrap();
        assert_eq!(fs::read(out.join("libclntsh.so")).unwrap(), b"lib");
        assert!(fs::symlink_metadata(out.join("libclntsh.so")).unwrap().file_type().is_symlink());
        assert!(!out.join("evil").exists() && fs::symlink_metadata(out.join("evil")).is_err());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn rel_paths() {
        assert_eq!(safe_rel(Path::new("instantclient_23/oci.dll"), 1), Some(PathBuf::from("oci.dll")));
        assert_eq!(safe_rel(Path::new("a/../../etc/passwd"), 0), None);
        assert_eq!(safe_rel(Path::new("/abs"), 0), None);
        assert_eq!(safe_rel(Path::new("top"), 1), None);
    }
}
