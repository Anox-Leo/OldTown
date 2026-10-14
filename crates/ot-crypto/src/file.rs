//! Chiffrement des fichiers (étape 5, note 0026) : construction STREAM de
//! RustCrypto (`aead-stream`, `StreamBE32<XChaCha20Poly1305>`) sous une clé
//! propre au fichier.
//!
//! Nonce de chaque morceau : `préfixe (19 o) ‖ index (u32 BE) ‖ dernier (1 o)`.
//! Les morceaux se chiffrent et se déchiffrent un par un, dans n'importe quel
//! ordre (reprise, parallélisme) ; le destinataire ouvre le morceau `n` avec
//! `dernier = (n == chunks - 1)`, si bien qu'un fichier tronqué, réordonné ou
//! altéré est refusé.

use std::fmt;

use aead_stream::{NewStream, StreamBE32, StreamPrimitive};
use chacha20poly1305::XChaCha20Poly1305;
use chacha20poly1305::aead::KeyInit;
use ot_proto::b64;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

use crate::{CryptoError, Result, random};

/// Taille des morceaux produite par les clients (1 Mio).
pub const FILE_CHUNK_SIZE: u32 = 1 << 20;

/// Taille de morceau maximale acceptée à la réception (4 Mio).
pub const MAX_CHUNK_SIZE: u32 = 4 << 20;

/// Taille du préfixe de nonce tiré pour chaque fichier.
pub const FILE_NONCE_LEN: usize = 19;

/// Surcoût de chaque morceau chiffré (étiquette Poly1305).
pub const CHUNK_TAG_LEN: u32 = 16;

/// Données associées de chaque morceau (séparation de domaine).
const FILE_DOMAIN: &[u8] = b"ot-file-v1";

/// Longueur maximale d'un nom de fichier nettoyé, en octets UTF-8.
const MAX_FILE_NAME_BYTES: usize = 255;

/// Clé d'un fichier (32 octets), tirée par l'expéditeur et transmise dans le
/// message chiffré `file`.
#[derive(Clone, PartialEq, Eq, Zeroize, ZeroizeOnDrop)]
pub struct FileKey([u8; 32]);

impl FileKey {
    pub fn generate() -> Self {
        let mut key = [0; 32];
        random(&mut key);
        Self(key)
    }

    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Debug for FileKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("FileKey(…)")
    }
}

impl Serialize for FileKey {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        b64::array::serialize(&self.0, s)
    }
}

impl<'de> Deserialize<'de> for FileKey {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        b64::array::deserialize(d).map(Self)
    }
}

/// Préfixe de nonce aléatoire d'un nouveau fichier.
pub fn new_file_nonce() -> [u8; FILE_NONCE_LEN] {
    let mut nonce = [0; FILE_NONCE_LEN];
    random(&mut nonce);
    nonce
}

/// Chiffre et déchiffre les morceaux d'un fichier. Sans état : un même
/// `FileCipher` sert à plusieurs morceaux en parallèle.
pub struct FileCipher(StreamBE32<XChaCha20Poly1305>);

impl FileCipher {
    pub fn new(key: &FileKey, nonce: &[u8; FILE_NONCE_LEN]) -> Self {
        // Invariant : la clé fait toujours 32 octets, la taille attendue.
        let aead = XChaCha20Poly1305::new_from_slice(key.as_bytes())
            .expect("clé XChaCha20-Poly1305 de 32 octets");
        Self(StreamBE32::from_aead(aead, &(*nonce).into()))
    }

    /// Chiffre le morceau `index` ; `last` marque le dernier du fichier.
    pub fn seal_chunk(&self, index: u32, last: bool, plain: &[u8]) -> Result<Vec<u8>> {
        let mut buf = Vec::with_capacity(plain.len() + CHUNK_TAG_LEN as usize);
        buf.extend_from_slice(plain);
        self.0
            .encrypt_in_place(index, last, FILE_DOMAIN, &mut buf)
            .map_err(|_| CryptoError::Malformed("morceau impossible à chiffrer"))?;
        Ok(buf)
    }

    /// Déchiffre le morceau `index`. Échoue si le morceau a été altéré, s'il
    /// vient d'une autre position ou d'un autre fichier, ou si `last` ne
    /// correspond pas à ce qu'avait marqué l'expéditeur.
    pub fn open_chunk(&self, index: u32, last: bool, sealed: &[u8]) -> Result<Zeroizing<Vec<u8>>> {
        if sealed.len() < CHUNK_TAG_LEN as usize {
            return Err(CryptoError::Malformed("morceau tronqué"));
        }
        let mut buf = Zeroizing::new(sealed.to_vec());
        self.0
            .decrypt_in_place(index, last, FILE_DOMAIN, &mut *buf)
            .map_err(|_| CryptoError::Decrypt)?;
        Ok(buf)
    }
}

impl fmt::Debug for FileCipher {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("FileCipher(…)")
    }
}

/// Nombre de morceaux d'un fichier de `size` octets : au moins un (un
/// fichier vide a un seul morceau, vide). `None` si `chunk_size` est nul ou
/// si le compte dépasse `u32`.
pub fn chunk_count(size: u64, chunk_size: u32) -> Option<u32> {
    if chunk_size == 0 {
        return None;
    }
    u32::try_from(size.div_ceil(u64::from(chunk_size)).max(1)).ok()
}

/// Taille en clair du morceau `index`, ou `None` s'il n'existe pas.
pub fn chunk_len(size: u64, chunk_size: u32, index: u32) -> Option<u32> {
    let chunks = chunk_count(size, chunk_size)?;
    if index >= chunks {
        return None;
    }
    let start = u64::from(index) * u64::from(chunk_size);
    // Invariant : le reste est inférieur ou égal à `chunk_size`, donc tient en u32.
    Some(size.saturating_sub(start).min(u64::from(chunk_size)) as u32)
}

/// Taille chiffrée totale d'un fichier : ce que le relais stocke.
pub fn sealed_size(size: u64, chunk_size: u32) -> Option<u64> {
    let chunks = chunk_count(size, chunk_size)?;
    size.checked_add(u64::from(chunks) * u64::from(CHUNK_TAG_LEN))
}

/// Vérifie la cohérence annoncée par un message `file`.
pub fn check_layout(size: u64, chunk_size: u32, chunks: u32) -> Result<()> {
    if chunk_size == 0 || chunk_size > MAX_CHUNK_SIZE {
        return Err(CryptoError::Body("taille de morceau hors limites".into()));
    }
    if chunk_count(size, chunk_size) != Some(chunks) {
        return Err(CryptoError::Body("nombre de morceaux incohérent".into()));
    }
    Ok(())
}

/// Nom de fichier sûr pour l'écrire sur le disque de n'importe quel OS. Le nom
/// vient d'un membre du groupe, mais finit dans un système de fichiers :
/// dernier composant seulement, sans caractère de contrôle, de mise en forme
/// bidirectionnelle ni caractère interdit sous Windows, sans point ni espace
/// au bord, jamais un nom réservé de Windows, 255 octets au plus (extension
/// gardée si possible). Un nom vide devient `fichier`.
pub fn sanitize_file_name(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or_default();
    let cleaned: String = base
        .chars()
        .filter(|&c| !c.is_control() && !is_bidi_format(c) && !r#"<>:"|?*"#.contains(c))
        .collect();
    let trimmed = cleaned.trim_matches(|c: char| c == '.' || c.is_whitespace());
    let mut out = if trimmed.is_empty() { "fichier".to_owned() } else { trimmed.to_owned() };
    if is_windows_reserved(&out) {
        out.insert(0, '_');
    }
    truncate_name(out)
}

/// Caractères de mise en forme bidirectionnelle (qui permettent de déguiser
/// une extension : `photo` ‖ U+202E ‖ `gpj.exe` s'affiche `photoexe.jpg`)
/// et marques invisibles.
fn is_bidi_format(c: char) -> bool {
    matches!(c, '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' | '\u{FEFF}')
}

/// `CON`, `PRN`, `AUX`, `NUL`, `COM1`…`COM9`, `LPT1`…`LPT9`, avec ou sans
/// extension et quelle que soit la casse.
fn is_windows_reserved(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or_default().trim_end().to_ascii_uppercase();
    matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ((stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.len() == 4
            && stem.as_bytes()[3].is_ascii_digit()
            && stem.as_bytes()[3] != b'0')
}

/// Coupe à [`MAX_FILE_NAME_BYTES`] sur une frontière de caractère, en gardant
/// une extension courte.
fn truncate_name(name: String) -> String {
    if name.len() <= MAX_FILE_NAME_BYTES {
        return name;
    }
    let ext = match name.rfind('.') {
        Some(i) if i > 0 && name.len() - i <= 16 => &name[i..],
        _ => "",
    };
    let mut cut = MAX_FILE_NAME_BYTES - ext.len();
    while !name.is_char_boundary(cut) {
        cut -= 1;
    }
    format!("{}{ext}", name[..cut].trim_end_matches(|c: char| c == '.' || c.is_whitespace()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cipher() -> FileCipher {
        FileCipher::new(&FileKey::from_bytes([7; 32]), &[9; FILE_NONCE_LEN])
    }

    /// Chiffre `data` en morceaux de `chunk_size`, comme un expéditeur.
    fn seal_all(c: &FileCipher, data: &[u8], chunk_size: u32) -> Vec<Vec<u8>> {
        let chunks = chunk_count(data.len() as u64, chunk_size).unwrap();
        (0..chunks)
            .map(|i| {
                let start = (i * chunk_size) as usize;
                let end = (start + chunk_size as usize).min(data.len());
                c.seal_chunk(i, i == chunks - 1, &data[start..end]).unwrap()
            })
            .collect()
    }

    fn open_all(c: &FileCipher, sealed: &[Vec<u8>]) -> Result<Vec<u8>> {
        let last = sealed.len() as u32 - 1;
        let mut out = Vec::new();
        for (i, chunk) in sealed.iter().enumerate() {
            out.extend_from_slice(&c.open_chunk(i as u32, i as u32 == last, chunk)?);
        }
        Ok(out)
    }

    #[test]
    fn roundtrip_in_any_order() {
        let data: Vec<u8> = (0..100u8).collect();
        let c = cipher();
        let sealed = seal_all(&c, &data, 32);
        assert_eq!(sealed.len(), 4);
        assert_eq!(sealed[3].len(), 4 + 16);
        assert_eq!(open_all(&c, &sealed).unwrap(), data);
        // Chaque morceau s'ouvre seul, sans les précédents (reprise).
        assert_eq!(&*c.open_chunk(2, false, &sealed[2]).unwrap(), &data[64..96]);
    }

    #[test]
    fn empty_file_is_one_empty_chunk() {
        assert_eq!(chunk_count(0, FILE_CHUNK_SIZE), Some(1));
        assert_eq!(chunk_len(0, FILE_CHUNK_SIZE, 0), Some(0));
        let c = cipher();
        let sealed = seal_all(&c, &[], 32);
        assert_eq!(sealed.len(), 1);
        assert!(open_all(&c, &sealed).unwrap().is_empty());
    }

    #[test]
    fn truncated_file_is_rejected() {
        let c = cipher();
        let sealed = seal_all(&c, &[1; 100], 32);
        // Dernier morceau retiré : l'avant-dernier n'est pas marqué dernier.
        assert_eq!(open_all(&c, &sealed[..3]), Err(CryptoError::Decrypt));
        // Le vrai dernier morceau présenté comme un morceau ordinaire.
        assert_eq!(c.open_chunk(3, false, &sealed[3]).map(|_| ()), Err(CryptoError::Decrypt));
    }

    #[test]
    fn reordered_chunks_are_rejected() {
        let c = cipher();
        let mut sealed = seal_all(&c, &(0..100u8).collect::<Vec<_>>(), 32);
        sealed.swap(0, 1);
        assert_eq!(open_all(&c, &sealed), Err(CryptoError::Decrypt));
    }

    #[test]
    fn tampered_chunk_or_wrong_key_is_rejected() {
        let c = cipher();
        let mut sealed = seal_all(&c, &[1; 100], 32);
        sealed[1][5] ^= 1;
        assert_eq!(open_all(&c, &sealed), Err(CryptoError::Decrypt));

        let sealed = seal_all(&c, &[1; 100], 32);
        let other_key = FileCipher::new(&FileKey::from_bytes([8; 32]), &[9; FILE_NONCE_LEN]);
        assert_eq!(open_all(&other_key, &sealed), Err(CryptoError::Decrypt));
        let other_nonce = FileCipher::new(&FileKey::from_bytes([7; 32]), &[10; FILE_NONCE_LEN]);
        assert_eq!(open_all(&other_nonce, &sealed), Err(CryptoError::Decrypt));
        assert_eq!(
            c.open_chunk(0, true, &[0; 15]).map(|_| ()),
            Err(CryptoError::Malformed("morceau tronqué"))
        );
    }

    #[test]
    fn layout() {
        assert_eq!(chunk_count(2_500_000, FILE_CHUNK_SIZE), Some(3));
        assert_eq!(chunk_count(2 * u64::from(FILE_CHUNK_SIZE), FILE_CHUNK_SIZE), Some(2));
        assert_eq!(chunk_count(10, 0), None);
        assert_eq!(chunk_len(2_500_000, FILE_CHUNK_SIZE, 2), Some(2_500_000 - 2 * (1 << 20)));
        assert_eq!(chunk_len(2_500_000, FILE_CHUNK_SIZE, 3), None);
        assert_eq!(sealed_size(2_500_000, FILE_CHUNK_SIZE), Some(2_500_000 + 3 * 16));
        assert!(check_layout(2_500_000, FILE_CHUNK_SIZE, 3).is_ok());
        assert!(check_layout(2_500_000, FILE_CHUNK_SIZE, 4).is_err());
        assert!(check_layout(0, FILE_CHUNK_SIZE, 0).is_err());
        assert!(check_layout(10, MAX_CHUNK_SIZE + 1, 1).is_err());
        assert!(check_layout(10, 0, 1).is_err());
        // Plus de 2³² morceaux : refusé plutôt que tronqué.
        assert!(check_layout(u64::MAX, 1, u32::MAX).is_err());
    }

    #[test]
    fn sanitized_names() {
        assert_eq!(sanitize_file_name("photo.jpg"), "photo.jpg");
        assert_eq!(sanitize_file_name("../../etc/passwd"), "passwd");
        assert_eq!(sanitize_file_name(r"C:\Users\x\rapport.pdf"), "rapport.pdf");
        assert_eq!(sanitize_file_name(".bashrc"), "bashrc");
        assert_eq!(sanitize_file_name("a\u{0}b\nc.txt"), "abc.txt");
        assert_eq!(sanitize_file_name("photo\u{202E}gpj.exe"), "photogpj.exe");
        assert_eq!(sanitize_file_name("que? <oui>|non*.txt"), "que ouinon.txt");
        assert_eq!(sanitize_file_name("CON"), "_CON");
        assert_eq!(sanitize_file_name("com1.txt"), "_com1.txt");
        assert_eq!(sanitize_file_name("COM0.txt"), "COM0.txt");
        assert_eq!(sanitize_file_name("console.txt"), "console.txt");
        assert_eq!(sanitize_file_name("fin. "), "fin");
        assert_eq!(sanitize_file_name(""), "fichier");
        assert_eq!(sanitize_file_name("../.."), "fichier");
        assert_eq!(sanitize_file_name("été 2026.heic"), "été 2026.heic");

        let long = format!("{}.pdf", "é".repeat(200));
        let cut = sanitize_file_name(&long);
        assert!(cut.len() <= MAX_FILE_NAME_BYTES);
        assert!(cut.ends_with("é.pdf"));
        let no_ext = sanitize_file_name(&"x".repeat(300));
        assert_eq!(no_ext.len(), MAX_FILE_NAME_BYTES);
    }

    #[test]
    fn debug_never_shows_keys() {
        assert_eq!(format!("{:?}", FileKey::from_bytes([0xab; 32])), "FileKey(…)");
        assert_eq!(format!("{:?}", cipher()), "FileCipher(…)");
    }
}
