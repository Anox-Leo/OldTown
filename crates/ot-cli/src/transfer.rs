//! Fichiers envoyés et reçus par la CLI (étape 5) : source et destination
//! sur disque pour `ot-core`, type MIME d'après l'extension, nom libre dans
//! le dossier de destination.

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::{Context, Result};
use ot_core::{ChunkSink, ChunkSource};

/// Fichier à envoyer, lu par morceaux.
pub struct FileSource {
    file: Mutex<File>,
    size: u64,
}

impl FileSource {
    pub fn open(path: &Path) -> Result<Self> {
        let file = File::open(path).with_context(|| format!("ouverture de {}", path.display()))?;
        let meta = file.metadata()?;
        if !meta.is_file() {
            anyhow::bail!("{} n'est pas un fichier", path.display());
        }
        Ok(Self { file: Mutex::new(file), size: meta.len() })
    }
}

impl ChunkSource for FileSource {
    fn size(&self) -> u64 {
        self.size
    }

    fn read_at(&self, offset: u64, len: u32) -> Result<Vec<u8>, String> {
        let mut file = self.file.lock().unwrap_or_else(|e| e.into_inner());
        file.seek(SeekFrom::Start(offset)).map_err(|e| e.to_string())?;
        let mut buf = Vec::with_capacity(len as usize);
        Read::by_ref(&mut *file)
            .take(u64::from(len))
            .read_to_end(&mut buf)
            .map_err(|e| e.to_string())?;
        Ok(buf)
    }
}

/// Identifiant de la source pour reprendre un envoi : chemin absolu et date
/// de modification (la taille est vérifiée à part).
pub fn source_id(path: &Path) -> Result<String> {
    let abs = fs::canonicalize(path).with_context(|| format!("chemin {}", path.display()))?;
    let modified = fs::metadata(&abs)?
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs());
    Ok(format!("{}#{modified}", abs.display()))
}

/// Fichier reçu : écrit sous un nom caché `.<nom>.oldtown-part`, puis
/// renommé à la fin sous un nom libre du dossier.
pub struct FileSink {
    dir: PathBuf,
    name: String,
    part: PathBuf,
    file: Mutex<Option<File>>,
    /// Chemin final, une fois le fichier vérifié et renommé.
    pub saved: Mutex<Option<PathBuf>>,
}

impl FileSink {
    /// `name` est déjà nettoyé par `ot-core`.
    pub fn create(dir: &Path, name: &str) -> Result<Self> {
        fs::create_dir_all(dir).with_context(|| format!("dossier {}", dir.display()))?;
        let part = dir.join(format!(".{name}.oldtown-part"));
        let file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&part)
            .with_context(|| format!("écriture de {}", part.display()))?;
        Ok(Self {
            dir: dir.to_owned(),
            name: name.to_owned(),
            part,
            file: Mutex::new(Some(file)),
            saved: Mutex::new(None),
        })
    }
}

impl ChunkSink for FileSink {
    fn write_at(&self, offset: u64, data: &[u8]) -> Result<(), String> {
        let mut guard = self.file.lock().unwrap_or_else(|e| e.into_inner());
        let file = guard.as_mut().ok_or("fichier déjà fermé")?;
        file.seek(SeekFrom::Start(offset)).map_err(|e| e.to_string())?;
        file.write_all(data).map_err(|e| e.to_string())
    }

    fn finish(&self) -> Result<(), String> {
        let file = self.file.lock().unwrap_or_else(|e| e.into_inner()).take();
        let file = file.ok_or("fichier déjà fermé")?;
        file.sync_all().map_err(|e| e.to_string())?;
        drop(file);
        let target = free_path(&self.dir, &self.name);
        fs::rename(&self.part, &target).map_err(|e| e.to_string())?;
        *self.saved.lock().unwrap_or_else(|e| e.into_inner()) = Some(target);
        Ok(())
    }

    fn abort(&self) {
        self.file.lock().unwrap_or_else(|e| e.into_inner()).take();
        let _ = fs::remove_file(&self.part);
    }
}

/// `dossier/nom`, ou `nom (2).ext`, `nom (3).ext`… s'il est déjà pris.
pub fn free_path(dir: &Path, name: &str) -> PathBuf {
    let first = dir.join(name);
    if !first.exists() {
        return first;
    }
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    };
    (2..).map(|n| dir.join(format!("{stem} ({n}){ext}"))).find(|p| !p.exists()).unwrap_or(first)
}

/// Type MIME d'après l'extension ; `application/octet-stream` sinon.
pub fn guess_mime(path: &Path) -> &'static str {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or_default().to_ascii_lowercase();
    match ext.as_str() {
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "heic" => "image/heic",
        "svg" => "image/svg+xml",
        "pdf" => "application/pdf",
        "txt" | "md" => "text/plain",
        "csv" => "text/csv",
        "html" | "htm" => "text/html",
        "json" => "application/json",
        "zip" => "application/zip",
        "mp4" => "video/mp4",
        "mov" => "video/quicktime",
        "webm" => "video/webm",
        "mp3" => "audio/mpeg",
        "m4a" => "audio/mp4",
        "ogg" => "audio/ogg",
        "doc" => "application/msword",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "odt" => "application/vnd.oasis.opendocument.text",
        _ => "application/octet-stream",
    }
}

/// Taille lisible : `1,5 Mo`.
pub fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["o", "Ko", "Mo", "Go"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} o")
    } else {
        format!("{value:.1} {}", UNITS[unit]).replace('.', ",")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn free_names() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(free_path(dir.path(), "photo.jpg"), dir.path().join("photo.jpg"));
        fs::write(dir.path().join("photo.jpg"), b"x").unwrap();
        assert_eq!(free_path(dir.path(), "photo.jpg"), dir.path().join("photo (2).jpg"));
        fs::write(dir.path().join("photo (2).jpg"), b"x").unwrap();
        assert_eq!(free_path(dir.path(), "photo.jpg"), dir.path().join("photo (3).jpg"));
        fs::write(dir.path().join("notes"), b"x").unwrap();
        assert_eq!(free_path(dir.path(), "notes"), dir.path().join("notes (2)"));
    }

    #[test]
    fn sink_writes_then_renames_or_cleans_up() {
        let dir = tempfile::tempdir().unwrap();
        let sink = FileSink::create(dir.path(), "a.txt").unwrap();
        sink.write_at(3, b"def").unwrap();
        sink.write_at(0, b"abc").unwrap();
        sink.finish().unwrap();
        assert_eq!(fs::read(dir.path().join("a.txt")).unwrap(), b"abcdef");
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1, "plus de fichier partiel");

        let sink = FileSink::create(dir.path(), "b.txt").unwrap();
        sink.write_at(0, b"abc").unwrap();
        sink.abort();
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn source_reads_ranges() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s.bin");
        fs::write(&path, b"0123456789").unwrap();
        let src = FileSource::open(&path).unwrap();
        assert_eq!(src.size(), 10);
        assert_eq!(src.read_at(8, 4).unwrap(), b"89");
        assert_eq!(src.read_at(2, 3).unwrap(), b"234");
    }

    #[test]
    fn mimes_and_sizes() {
        assert_eq!(guess_mime(Path::new("x.JPG")), "image/jpeg");
        assert_eq!(guess_mime(Path::new("x")), "application/octet-stream");
        assert_eq!(human_size(512), "512 o");
        assert_eq!(human_size(1536 * 1024), "1,5 Mo");
    }
}
