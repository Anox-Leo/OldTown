//! Cœur OldTown sans réseau, compilé en WebAssembly pour l'extension Chrome
//! (étape 4, note 0023).
//!
//! Le service worker tient la connexion et le stockage ; ce module tient les
//! clés et tout ce qui les touche : identité, signatures d'authentification
//! et d'appairage, chiffrement, roster, époques, quarantaine. Les valeurs
//! secrètes vivent dans un [`Vault`] ; le JavaScript n'en relève que les
//! changements, pour les chiffrer et les ranger.
//!
//! Les structures passent en JSON (chaînes) : enveloppes chiffrées, trames,
//! événements. Les erreurs sont des `Error` JavaScript au message lisible,
//! sans secret.

use std::sync::Arc;

use ot_core::crypto::{PairingCode, token_hash};
use ot_core::{DeviceId, PAIR_URI_PREFIX, PairUri, StoreKeyring, init_group as core_init_group};
use ot_proto::{Envelope, Frame, NONCE_LEN, b64};
use serde::Serialize;
use wasm_bindgen::prelude::*;

mod core;
mod vault;

use crate::core::CoreState;
use crate::vault::VaultStore;

fn js(e: impl std::fmt::Display) -> JsError {
    JsError::new(&e.to_string())
}

fn to_json<T: Serialize + ?Sized>(value: &T) -> Result<String, JsError> {
    serde_json::to_string(value).map_err(js)
}

/// Version de `ot-wasm` (celle du dépôt).
#[wasm_bindgen]
pub fn version() -> String {
    env!("CARGO_PKG_VERSION").to_owned()
}

/// Identifiant d'appareil neuf, choisi par l'appareil pour le QR inversé.
#[wasm_bindgen(js_name = newDeviceId)]
pub fn new_device_id() -> String {
    DeviceId::generate().to_string()
}

/// Valide une trame reçue du relais (version, type, champs) et la renvoie en
/// JSON normalisé.
#[wasm_bindgen(js_name = decodeFrame)]
pub fn decode_frame(text: &str) -> Result<String, JsError> {
    Ok(Frame::from_json(text).map_err(js)?.to_json())
}

/// Valide une enveloppe JSON (avant de la remettre au cœur ou de la ranger).
fn parse_envelope(json: &str) -> Result<Envelope, JsError> {
    serde_json::from_str(json).map_err(|e| js(format!("enveloppe invalide : {e}")))
}

/// Texte collé pour rejoindre un groupe : QR « rejoindre »
/// (`oldtown://pair?s=…&c=…`) ou code `ot1-…` seul. Renvoie
/// `{"server_url": …|null, "code": …}`.
#[wasm_bindgen(js_name = parseJoin)]
pub fn parse_join(text: &str) -> Result<String, JsError> {
    #[derive(Serialize)]
    struct Join {
        server_url: Option<String>,
        code: String,
    }
    let text = text.trim();
    let join = if text.starts_with(PAIR_URI_PREFIX) {
        let uri = PairUri::parse(text).map_err(js)?;
        Join { server_url: Some(uri.server_url), code: uri.code }
    } else {
        PairingCode::parse(text).map_err(js)?;
        Join { server_url: None, code: text.to_owned() }
    };
    to_json(&join)
}

/// Modules d'un QR code (`size` × `size`, ligne par ligne, 1 = noir).
#[wasm_bindgen]
pub struct QrCode {
    size: u32,
    modules: Vec<u8>,
}

#[wasm_bindgen]
impl QrCode {
    #[wasm_bindgen(getter)]
    pub fn size(&self) -> u32 {
        self.size
    }

    #[wasm_bindgen(getter)]
    pub fn modules(&self) -> Vec<u8> {
        self.modules.clone()
    }
}

#[wasm_bindgen(js_name = qrCode)]
pub fn qr_code(text: &str) -> Result<QrCode, JsError> {
    let code = qrcode::QrCode::new(text.as_bytes()).map_err(js)?;
    let size = u32::try_from(code.width()).map_err(js)?;
    let modules =
        code.to_colors().into_iter().map(|c| u8::from(c == qrcode::Color::Dark)).collect();
    Ok(QrCode { size, modules })
}

/// Valeur modifiée dans le coffre ; `value` absente : effacée.
#[wasm_bindgen]
pub struct Change {
    name: String,
    value: Option<Vec<u8>>,
}

#[wasm_bindgen]
impl Change {
    #[wasm_bindgen(getter)]
    pub fn name(&self) -> String {
        self.name.clone()
    }

    /// Valeur en clair, à chiffrer aussitôt ; `undefined` si effacée.
    #[wasm_bindgen(getter)]
    pub fn value(&self) -> Option<Vec<u8>> {
        self.value.clone()
    }
}

/// Coffre des secrets de l'appareil, en mémoire du WebAssembly.
#[wasm_bindgen]
#[derive(Default)]
pub struct Vault {
    inner: Arc<VaultStore>,
}

#[wasm_bindgen]
impl Vault {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        Self::default()
    }

    /// Valeur relue (et déchiffrée) du stockage au démarrage.
    pub fn load(&self, name: &str, value: &[u8]) {
        self.inner.load(name, value);
    }

    pub fn has(&self, name: &str) -> bool {
        self.inner.has(name)
    }

    /// Changements depuis le dernier relevé, à chiffrer et ranger.
    #[wasm_bindgen(js_name = takeChanges)]
    pub fn take_changes(&self) -> Vec<Change> {
        self.inner
            .take_changes()
            .into_iter()
            .map(|c| Change { name: c.name, value: c.value.map(|v| v.to_vec()) })
            .collect()
    }

    /// Efface tout (oubli de l'appareil) ; chaque nom devient un changement.
    pub fn clear(&self) {
        self.inner.clear();
    }

    /// Crée l'identité Ed25519 si elle manque ; renvoie la clé publique en
    /// base64.
    #[wasm_bindgen(js_name = ensureIdentity)]
    pub fn ensure_identity(&self) -> Result<String, JsError> {
        Ok(self.inner.ensure_identity().map_err(js)?.public_key_b64())
    }

    /// Trame `auth` qui répond au défi `nonce` (base64).
    #[wasm_bindgen(js_name = authFrame)]
    pub fn auth_frame(&self, nonce: &str, device_id: &str) -> Result<String, JsError> {
        let identity = self.inner.identity().map_err(js)?.ok_or_else(|| js("identité absente"))?;
        let nonce = b64::decode_array::<NONCE_LEN>(nonce).ok_or_else(|| js("défi invalide"))?;
        let device_id: DeviceId = device_id.parse().map_err(js)?;
        let sig = identity.sign_challenge(&nonce, &device_id);
        Ok(Frame::Auth { device_id, sig }.to_json())
    }

    /// Trame `pair_redeem` : enregistrement auprès du relais avec le jeton du
    /// code. Le secret du code ne quitte pas l'appareil.
    #[wasm_bindgen(js_name = pairRedeemFrame)]
    pub fn pair_redeem_frame(
        &self,
        nonce: &str,
        code: &str,
        name: &str,
    ) -> Result<String, JsError> {
        let identity = self.inner.identity().map_err(js)?.ok_or_else(|| js("identité absente"))?;
        let nonce = b64::decode_array::<NONCE_LEN>(nonce).ok_or_else(|| js("défi invalide"))?;
        let code = PairingCode::parse(code).map_err(js)?;
        let sig = identity.sign_pair_redeem(&nonce, &token_hash(&code.token));
        Ok(Frame::PairRedeem {
            token: code.token,
            name: name.to_owned(),
            public_key: identity.public_key(),
            sig,
        }
        .to_json())
    }

    /// Crée un groupe dont cet appareil est le seul membre (premier appareil,
    /// et tests). Refuse d'écraser un groupe existant.
    #[wasm_bindgen(js_name = initGroup)]
    pub fn init_group(&self, device_id: &str, name: &str) -> Result<(), JsError> {
        let identity = self.inner.ensure_identity().map_err(js)?;
        let device_id: DeviceId = device_id.parse().map_err(js)?;
        let keyring = StoreKeyring::new(self.inner.clone());
        core_init_group(&keyring, &device_id, name, &identity, false).map_err(js)?;
        Ok(())
    }
}

/// Cœur de groupe de l'appareil : chiffre, ouvre, tient le roster.
#[wasm_bindgen]
pub struct Core {
    state: CoreState,
    startup: String,
}

#[wasm_bindgen]
impl Core {
    /// Ouvre le cœur de l'appareil `device_id` avec les secrets du coffre.
    pub fn open(vault: &Vault, device_id: &str) -> Result<Core, JsError> {
        let (state, out) = CoreState::open(vault.inner.clone(), device_id).map_err(js)?;
        Ok(Self { state, startup: to_json(&out)? })
    }

    /// Événements produits à l'ouverture (`{"events": […], "sends": []}`).
    #[wasm_bindgen(js_name = startupOutput)]
    pub fn startup_output(&self) -> String {
        self.startup.clone()
    }

    /// État du groupe en JSON, `undefined` sans groupe.
    pub fn group(&self) -> Result<Option<String>, JsError> {
        self.state.info().map(|g| to_json(&g)).transpose()
    }

    /// Appairage en attente (`{"kind": "offer", "qr": …, "expires_at": …}`
    /// ou `{"kind": "join"}`), `undefined` sinon.
    pub fn pending(&self) -> Result<Option<String>, JsError> {
        self.state.pending().map(to_json).transpose()
    }

    /// Enveloppe chiffrée (JSON) d'un texte pour `to` ; vide : tous.
    #[wasm_bindgen(js_name = sealText)]
    pub fn seal_text(&self, text: &str, to: Vec<String>) -> Result<String, JsError> {
        to_json(&self.state.seal_text(text, &to).map_err(js)?)
    }

    /// Traite une enveloppe livrée sous `seq` ; renvoie
    /// `{"events": […], "sends": [enveloppes]}`.
    pub fn process(&mut self, seq: f64, envelope: &str) -> Result<String, JsError> {
        let env = parse_envelope(envelope)?;
        if seq.fract() != 0.0 || !(0.0..=9_007_199_254_740_991.0).contains(&seq) {
            return Err(js("seq invalide"));
        }
        // Entier exact, vérifié juste au-dessus.
        to_json(&self.state.process(seq as i64, env))
    }

    /// QR inversé à afficher : `{"qr": …, "expires_at": …}`.
    pub fn offer(&mut self, name: &str) -> Result<String, JsError> {
        to_json(&self.state.offer(name).map_err(js)?)
    }

    /// Demande au parrain après l'enregistrement par code (enveloppe JSON).
    #[wasm_bindgen(js_name = startJoin)]
    pub fn start_join(&mut self, code: &str, sponsor: &str, name: &str) -> Result<String, JsError> {
        to_json(&self.state.start_join(code, sponsor, name).map_err(js)?)
    }

    /// Rotation : `{"epoch": n, "events": […], "sends": [group_key…]}`.
    pub fn rotate(&mut self, exclude: Vec<String>) -> Result<String, JsError> {
        #[derive(Serialize)]
        struct Rotation {
            epoch: u32,
            #[serde(flatten)]
            out: crate::core::Output,
        }
        let (epoch, out) = self.state.rotate(&exclude).map_err(js)?;
        to_json(&Rotation { epoch, out })
    }
}
