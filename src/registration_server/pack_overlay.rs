//! The Windows asset is `rust-reg-<version>.exe`: this binary with a ustar overlay.
//! `--staging <dir> --no-service` writes the version payload and does not register a service.
//! A run with no arguments, on Windows, when the overlay is present, is the first install.

use std::io::{self, Read};
use std::path::Path;
#[cfg(windows)]
use std::path::PathBuf;
#[cfg(windows)]
use std::process::Command;

const MAGIC: &[u8] = b"RUSTREG1";

#[cfg(test)]
pub(crate) fn append_overlay(exe: &mut Vec<u8>, tar: &[u8]) {
    exe.extend_from_slice(tar);
    exe.extend_from_slice(&(tar.len() as u64).to_le_bytes());
    exe.extend_from_slice(MAGIC);
}

/// `Some` when this process is the pack asset, not the venue server.
pub fn take_pack_command() -> Option<Result<(), Box<dyn std::error::Error>>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("--staging") {
        return Some(stage(&args).map_err(|err| Box::new(err) as Box<dyn std::error::Error>));
    }
    #[cfg(windows)]
    if args.is_empty() {
        let exe = match std::env::current_exe() {
            Ok(exe) => exe,
            Err(err) => return Some(Err(Box::new(err))),
        };
        match read_overlay(&exe) {
            Ok(Some(_)) => {
                return Some(
                    first_install(&exe).map_err(|err| Box::new(err) as Box<dyn std::error::Error>),
                );
            }
            Ok(None) => return None,
            Err(err) => return Some(Err(Box::new(err))),
        }
    }
    None
}

fn stage(args: &[String]) -> io::Result<()> {
    if args.len() != 3 || args[2] != "--no-service" {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "usage: rust-reg --staging <dir> --no-service",
        ));
    }
    let exe = std::env::current_exe()?;
    extract_folder(&exe, Path::new(&args[1]), "stage")
}

#[cfg(windows)]
fn first_install(exe: &Path) -> io::Result<()> {
    let prefix = PathBuf::from(r"C:\Program Files\rust-reg");
    extract_folder(exe, &prefix, "tree")?;
    register_service(&prefix)
}

#[cfg(windows)]
fn register_service(prefix: &Path) -> io::Result<()> {
    let run = prefix.join("bin").join("rust-reg-run.exe");
    let bin_path = format!("\"{}\" --prefix \"{}\"", run.display(), prefix.display());
    let created = Command::new("sc.exe")
        .arg("create")
        .arg("rust-reg")
        .arg(format!("binPath= {bin_path}"))
        .arg("start= auto")
        .status()?;
    if !created.success() {
        return Err(io::Error::new(io::ErrorKind::Other, "sc create rust-reg"));
    }
    let failure = Command::new("sc.exe")
        .args([
            "failure",
            "rust-reg",
            "reset= 86400",
            "actions= restart/5000",
        ])
        .status()?;
    if !failure.success() {
        return Err(io::Error::new(io::ErrorKind::Other, "sc failure rust-reg"));
    }
    Ok(())
}

pub(crate) fn extract_folder(exe: &Path, dest: &Path, folder: &str) -> io::Result<()> {
    let Some(tar) = read_overlay(exe)? else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "no pack overlay",
        ));
    };
    let tmp = std::env::temp_dir().join(format!(
        "rust-reg-overlay-{}-{}",
        std::process::id(),
        folder
    ));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp)?;
    let extracted = super::update::untar_bytes(&tar, &tmp);
    if let Err(err) = extracted {
        let _ = std::fs::remove_dir_all(&tmp);
        return Err(err);
    }
    let copied = copy_children(&tmp.join(folder), dest);
    let _ = std::fs::remove_dir_all(&tmp);
    copied
}

fn read_overlay(exe: &Path) -> io::Result<Option<Vec<u8>>> {
    let mut file = std::fs::File::open(exe)?;
    let len = file.metadata()?.len() as usize;
    if len < 16 {
        return Ok(None);
    }
    let mut tail = [0u8; 16];
    file.read_exact_at_end(&mut tail)?;
    if &tail[8..] != MAGIC {
        return Ok(None);
    }
    let tar_len = u64::from_le_bytes(tail[..8].try_into().expect("8 bytes")) as usize;
    let start = len
        .checked_sub(16 + tar_len)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "overlay length"))?;
    let mut tar = vec![0u8; tar_len];
    use std::io::{Seek, SeekFrom};
    file.seek(SeekFrom::Start(start as u64))?;
    file.read_exact(&mut tar)?;
    Ok(Some(tar))
}

trait ReadEnd {
    fn read_exact_at_end(&mut self, buf: &mut [u8]) -> io::Result<()>;
}

impl ReadEnd for std::fs::File {
    fn read_exact_at_end(&mut self, buf: &mut [u8]) -> io::Result<()> {
        use std::io::{Seek, SeekFrom};
        self.seek(SeekFrom::End(-(buf.len() as i64)))?;
        self.read_exact(buf)
    }
}

fn copy_children(from: &Path, to: &Path) -> io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let dest = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_children(&entry.path(), &dest)?;
        } else {
            std::fs::copy(entry.path(), &dest)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn staging_overlay_writes_the_payload() {
        let tar = super::super::update::payload_archive(&[
            ("stage/rust-reg", b"new-server"),
            ("stage/admin/index.html", b"admin"),
            ("stage/form/index.html", b"form"),
            ("stage/RELEASE", b"v0.2.0\n"),
            ("tree/bin/rust-reg-run", b"supervisor"),
        ]);
        let mut bytes = b"MZ-not-used".to_vec();
        append_overlay(&mut bytes, &tar);
        let dir = std::env::temp_dir().join(format!("rust-reg-stage-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let exe = dir.join("asset.exe");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&exe, &bytes).unwrap();
        let dest = dir.join("out");
        extract_folder(&exe, &dest, "stage").unwrap();
        assert_eq!(std::fs::read(dest.join("rust-reg")).unwrap(), b"new-server");
        assert_eq!(
            std::fs::read(dest.join("admin").join("index.html")).unwrap(),
            b"admin"
        );
        assert_eq!(
            std::fs::read(dest.join("form").join("index.html")).unwrap(),
            b"form"
        );
        assert_eq!(std::fs::read(dest.join("RELEASE")).unwrap(), b"v0.2.0\n");
        assert!(!dest.join("bin").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
