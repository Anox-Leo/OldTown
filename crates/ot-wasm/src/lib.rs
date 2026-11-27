use std::sync::Arc;

use ot_core::crypto::{InviteCode, PairingCode, token_hash};
use ot_core::{
    DeviceId, InviteRedeemed, InviteUri, PAIR_URI_PREFIX, PairUri, StoreKeyring, UserId,
    init_group as core_init_group,
};
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

#[wasm_bindgen]
pub fn version() -> String {
    env!("CARGO_PKG_VERSION").to_owned()
}

#[wasm_bindgen(js_name = newDeviceId)]
pub fn new_device_id() -> String {
    DeviceId::generate().to_string()
}

#[wasm_bindgen(js_name = decodeFrame)]
pub fn decode_frame(text: &str) -> Result<String, JsError> {
    Ok(Frame::from_json(text).map_err(js)?.to_json())
}

fn parse_envelope(json: &str) -> Result<Envelope, JsError> {
    serde_json::from_str(json).map_err(|e| js(format!("enveloppe invalide : {e}")))
}

#[wasm_bindgen(js_name = parseInvite)]
pub fn parse_invite(text: &str) -> Result<String, JsError> {
    #[derive(Serialize)]
    struct Out {
        server_url: String,
        code: String,
    }
    let uri = InviteUri::parse(text).map_err(js)?;
    to_json(&Out { server_url: uri.server_url, code: uri.code })
}

#[wasm_bindgen(js_name = inviteUri)]
pub fn invite_uri(server_url: &str, code: &str) -> Result<String, JsError> {
    InviteCode::parse(code).map_err(js)?;
    Ok(InviteUri { server_url: server_url.to_owned(), code: code.to_owned() }.encode())
}

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

#[wasm_bindgen]
#[derive(Default)]
pub struct Hasher(sha2::Sha256);

#[wasm_bindgen]
impl Hasher {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn update(&mut self, data: &[u8]) {
        sha2::Digest::update(&mut self.0, data);
    }

    pub fn finish(&mut self) -> Vec<u8> {
        sha2::Digest::finalize_reset(&mut self.0).to_vec()
    }
}

#[wasm_bindgen(js_name = fileGeometry)]
pub fn file_geometry(size: f64, chunk_size: u32) -> Result<String, JsError> {
    to_json(&crate::core::Geometry::new(exact_u64(size)?, chunk_size).map_err(js)?)
}

fn exact_u64(n: f64) -> Result<u64, JsError> {
    if n.fract() != 0.0 || !(0.0..=9_007_199_254_740_991.0).contains(&n) {
        return Err(js("taille invalide"));
    }
    Ok(n as u64)
}

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

    #[wasm_bindgen(getter)]
    pub fn value(&self) -> Option<Vec<u8>> {
        self.value.clone()
    }
}

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

    pub fn load(&self, name: &str, value: &[u8]) {
        self.inner.load(name, value);
    }

    pub fn has(&self, name: &str) -> bool {
        self.inner.has(name)
    }

    #[wasm_bindgen(js_name = takeChanges)]
    pub fn take_changes(&self) -> Vec<Change> {
        self.inner
            .take_changes()
            .into_iter()
            .map(|c| Change { name: c.name, value: c.value.map(|v| v.to_vec()) })
            .collect()
    }

    pub fn clear(&self) {
        self.inner.clear();
    }

    #[wasm_bindgen(js_name = ensureIdentity)]
    pub fn ensure_identity(&self) -> Result<String, JsError> {
        Ok(self.inner.ensure_identity().map_err(js)?.public_key_b64())
    }

    #[wasm_bindgen(js_name = authFrame)]
    pub fn auth_frame(&self, nonce: &str, device_id: &str) -> Result<String, JsError> {
        let identity = self.inner.identity().map_err(js)?.ok_or_else(|| js("identité absente"))?;
        let nonce = b64::decode_array::<NONCE_LEN>(nonce).ok_or_else(|| js("défi invalide"))?;
        let device_id: DeviceId = device_id.parse().map_err(js)?;
        let sig = identity.sign_challenge(&nonce, &device_id);
        Ok(Frame::Auth { device_id, sig }.to_json())
    }

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

    #[wasm_bindgen(js_name = inviteRedeemFrame)]
    pub fn invite_redeem_frame(
        &self,
        nonce: &str,
        code: &str,
        name: &str,
    ) -> Result<String, JsError> {
        let identity = self.inner.identity().map_err(js)?.ok_or_else(|| js("identité absente"))?;
        let nonce = b64::decode_array::<NONCE_LEN>(nonce).ok_or_else(|| js("défi invalide"))?;
        let code = InviteCode::parse(code).map_err(js)?;
        let sig = identity.sign_invite_redeem(&nonce, &token_hash(&code.token));
        Ok(Frame::InviteRedeem {
            token: code.token,
            name: name.to_owned(),
            public_key: identity.public_key(),
            sig,
        }
        .to_json())
    }

    #[wasm_bindgen(js_name = savePendingInvite)]
    pub fn save_pending_invite(
        &self,
        code: &str,
        user: &str,
        device_id: &str,
        inviter: &str,
        name: &str,
    ) -> Result<(), JsError> {
        let code = InviteCode::parse(code).map_err(js)?;
        let redeemed = InviteRedeemed {
            user: user.parse::<UserId>().map_err(js)?,
            device_id: device_id.parse().map_err(js)?,
            inviter: inviter.parse().map_err(js)?,
        };
        let keyring = StoreKeyring::new(self.inner.clone());
        ot_core::save_pending_invite(&keyring, &code, &redeemed, name).map_err(js)
    }

    #[wasm_bindgen(js_name = signHttp)]
    pub fn sign_http(
        &self,
        device_id: &str,
        method: &str,
        path: &str,
        body: &[u8],
        now_ms: f64,
    ) -> Result<String, JsError> {
        let identity = self.inner.identity().map_err(js)?.ok_or_else(|| js("identité absente"))?;
        let device_id: DeviceId = device_id.parse().map_err(js)?;
        let headers = ot_core::files::signed_headers(
            &identity,
            &device_id,
            method,
            path,
            body,
            exact_u64(now_ms)?,
        );
        let map: std::collections::BTreeMap<_, _> = headers.into_iter().collect();
        to_json(&map)
    }

    #[wasm_bindgen(js_name = initGroup)]
    pub fn init_group(&self, device_id: &str, name: &str) -> Result<(), JsError> {
        let identity = self.inner.ensure_identity().map_err(js)?;
        let device_id: DeviceId = device_id.parse().map_err(js)?;
        let keyring = StoreKeyring::new(self.inner.clone());
        core_init_group(&keyring, &device_id, name, &identity, false).map_err(js)?;
        Ok(())
    }
}

#[wasm_bindgen]
pub struct Core {
    state: CoreState,
    startup: String,
}

#[wasm_bindgen]
impl Core {
    pub fn open(vault: &Vault, device_id: &str) -> Result<Core, JsError> {
        let (state, out) = CoreState::open(vault.inner.clone(), device_id).map_err(js)?;
        Ok(Self { state, startup: to_json(&out)? })
    }

    #[wasm_bindgen(js_name = startupOutput)]
    pub fn startup_output(&self) -> String {
        self.startup.clone()
    }

    pub fn group(&self) -> Result<Option<String>, JsError> {
        self.state.info().map(|g| to_json(&g)).transpose()
    }

    pub fn pending(&self) -> Result<Option<String>, JsError> {
        self.state.pending().map(to_json).transpose()
    }

    #[wasm_bindgen(js_name = sealText)]
    pub fn seal_text(&self, text: &str, to: Vec<String>) -> Result<String, JsError> {
        to_json(&self.state.seal_text(text, &to).map_err(js)?)
    }

    pub fn process(&mut self, seq: f64, envelope: &str) -> Result<String, JsError> {
        let env = parse_envelope(envelope)?;
        if seq.fract() != 0.0 || !(0.0..=9_007_199_254_740_991.0).contains(&seq) {
            return Err(js("seq invalide"));
        }
        to_json(&self.state.process(seq as i64, env))
    }

    pub fn offer(&mut self, name: &str) -> Result<String, JsError> {
        to_json(&self.state.offer(name).map_err(js)?)
    }

    #[wasm_bindgen(js_name = startJoin)]
    pub fn start_join(&mut self, code: &str, sponsor: &str, name: &str) -> Result<String, JsError> {
        to_json(&self.state.start_join(code, sponsor, name).map_err(js)?)
    }

    pub fn upload(&self, source_id: &str, size: f64) -> Result<Option<String>, JsError> {
        self.state.upload(source_id, exact_u64(size)?).map_err(js)?.map(|u| to_json(&u)).transpose()
    }

    #[wasm_bindgen(js_name = startUpload)]
    #[allow(clippy::too_many_arguments)]
    pub fn start_upload(
        &self,
        source_id: &str,
        name: &str,
        mime: &str,
        size: f64,
        chunk_size: u32,
        sha256: &[u8],
        to: Vec<String>,
        blob_id: &str,
        expires_at: &str,
        to_user: Option<String>,
    ) -> Result<String, JsError> {
        let up = self
            .state
            .start_upload(
                source_id,
                name,
                mime,
                exact_u64(size)?,
                chunk_size,
                sha256,
                &to,
                to_user.as_deref(),
                blob_id,
                expires_at,
            )
            .map_err(js)?;
        to_json(&up)
    }

    #[wasm_bindgen(js_name = sealChunk)]
    pub fn seal_chunk(
        &self,
        source_id: &str,
        index: u32,
        plain: &[u8],
    ) -> Result<Vec<u8>, JsError> {
        self.state.seal_chunk(source_id, index, plain).map_err(js)
    }

    #[wasm_bindgen(js_name = sealFile)]
    pub fn seal_file(&self, source_id: &str) -> Result<String, JsError> {
        to_json(&self.state.seal_file(source_id).map_err(js)?)
    }

    #[wasm_bindgen(js_name = forgetUpload)]
    pub fn forget_upload(&self, source_id: &str) -> Result<(), JsError> {
        self.state.forget_upload(source_id).map_err(js)
    }

    pub fn files(&self) -> Result<String, JsError> {
        to_json(&self.state.files().map_err(js)?)
    }

    #[wasm_bindgen(js_name = fileGeometry)]
    pub fn file_geometry(&self, blob_id: &str) -> Result<String, JsError> {
        to_json(&self.state.file_geometry(blob_id).map_err(js)?)
    }

    #[wasm_bindgen(js_name = openChunk)]
    pub fn open_chunk(&self, blob_id: &str, index: u32, sealed: &[u8]) -> Result<Vec<u8>, JsError> {
        self.state.open_chunk(blob_id, index, sealed).map_err(js)
    }

    #[wasm_bindgen(js_name = checkFile)]
    pub fn check_file(&self, blob_id: &str, sha256: &[u8]) -> Result<bool, JsError> {
        self.state.check_file(blob_id, sha256).map_err(js)
    }

    #[wasm_bindgen(js_name = forgetFile)]
    pub fn forget_file(&self, blob_id: &str) -> Result<(), JsError> {
        self.state.forget_file(blob_id).map_err(js)
    }

    #[wasm_bindgen(js_name = directoryStart)]
    pub fn directory_start(&mut self) -> Option<String> {
        self.state.directory_start().map(|f| f.to_json())
    }

    #[wasm_bindgen(js_name = directoryPage)]
    pub fn directory_page(&mut self, frame: &str) -> Result<String, JsError> {
        let frame = Frame::from_json(frame).map_err(js)?;
        to_json(&self.state.directory_page(frame).map_err(js)?)
    }

    pub fn people(&self) -> Result<String, JsError> {
        to_json(&self.state.people())
    }

    #[wasm_bindgen(js_name = declarationFrame)]
    pub fn declaration_frame(&self, kind: Option<String>) -> Result<Option<String>, JsError> {
        Ok(self.state.declaration_frame(kind).map_err(js)?.map(|f| f.to_json()))
    }

    pub fn recipients(&self, user: &str) -> Result<String, JsError> {
        to_json(&self.state.recipients(user).map_err(js)?)
    }

    #[wasm_bindgen(js_name = sealTextTo)]
    pub fn seal_text_to(&self, text: &str, user: &str) -> Result<String, JsError> {
        to_json(&self.state.seal_text_to(text, user).map_err(js)?)
    }

    #[wasm_bindgen(js_name = nextEntry)]
    pub fn next_entry(&mut self, what: &str) -> Result<String, JsError> {
        let what = serde_json::from_str(what).map_err(js)?;
        to_json(&self.state.next_entry(what).map_err(js)?)
    }

    #[wasm_bindgen(js_name = entryPublished)]
    pub fn entry_published(&mut self) -> Result<String, JsError> {
        to_json(&self.state.entry_published().map_err(js)?)
    }

    #[wasm_bindgen(js_name = createInvite)]
    pub fn create_invite(
        &mut self,
        user: &str,
        name: &str,
        token: &str,
        expires_at: &str,
    ) -> Result<String, JsError> {
        self.state.create_invite(user, name, token, expires_at).map_err(js)
    }

    #[wasm_bindgen(js_name = invitesToCertify)]
    pub fn invites_to_certify(&self) -> Result<String, JsError> {
        to_json(&self.state.invites_to_certify())
    }

    #[wasm_bindgen(js_name = inviteCert)]
    pub fn invite_cert(&mut self, user: &str) -> Result<String, JsError> {
        #[derive(Serialize)]
        struct Out {
            cert: ot_proto::SignedBlob,
            certified: bool,
        }
        let (cert, certified) = self.state.invite_cert(user).map_err(js)?;
        to_json(&Out { cert, certified })
    }

    #[wasm_bindgen(js_name = inviteCertified)]
    pub fn invite_certified(&mut self, user: &str) -> Result<(), JsError> {
        self.state.invite_certified(user).map_err(js)
    }

    #[wasm_bindgen(js_name = inviteWelcome)]
    pub fn invite_welcome(&self, user: &str, head: &str) -> Result<String, JsError> {
        let head = serde_json::from_str(head).map_err(js)?;
        to_json(&self.state.invite_welcome(user, head).map_err(js)?)
    }

    #[wasm_bindgen(js_name = inviteDone)]
    pub fn invite_done(&mut self, user: &str) -> Result<String, JsError> {
        to_json(&self.state.invite_done(user).map_err(js)?)
    }

    #[wasm_bindgen(js_name = inviteRequest)]
    pub fn invite_request(&self) -> Result<Option<String>, JsError> {
        self.state.invite_request().map(|r| r.map_err(js).and_then(|e| to_json(&e))).transpose()
    }

    #[wasm_bindgen(js_name = awaitsDirectory)]
    pub fn awaits_directory(&self) -> bool {
        self.state.awaits_directory()
    }

    #[wasm_bindgen(js_name = adminInitStart)]
    pub fn admin_init_start(&self) -> Result<String, JsError> {
        Ok(self.state.admin_init_start().map_err(js)?.to_json())
    }

    #[wasm_bindgen(js_name = adminRoot)]
    pub fn admin_root(&mut self, frame: &str) -> Result<String, JsError> {
        let frame = Frame::from_json(frame).map_err(js)?;
        to_json(&self.state.admin_root(frame).map_err(js)?)
    }

    #[wasm_bindgen(js_name = adoptAdminRoot)]
    pub fn adopt_admin_root(&mut self, user: &str) -> Result<String, JsError> {
        to_json(&self.state.adopt_admin_root(user).map_err(js)?)
    }

    #[wasm_bindgen(js_name = certifiedBy)]
    pub fn certified_by(&self, device: &str) -> Result<String, JsError> {
        to_json(&self.state.certified_by(device).map_err(js)?)
    }

    pub fn recertify(&self, user: &str, name: Option<String>) -> Result<String, JsError> {
        to_json(&self.state.recertify(user, name.as_deref()).map_err(js)?)
    }

    #[wasm_bindgen(js_name = servicesToRepublish)]
    pub fn services_to_republish(&self) -> Result<String, JsError> {
        to_json(&self.state.services_to_republish())
    }

    #[wasm_bindgen(js_name = nextServiceEntry)]
    pub fn next_service_entry(&mut self, user: &str) -> Result<String, JsError> {
        to_json(&self.state.next_service_entry(user).map_err(js)?)
    }

    #[wasm_bindgen(js_name = serviceEntryPublished)]
    pub fn service_entry_published(&mut self, user: &str, seq: f64) -> Result<String, JsError> {
        to_json(&self.state.service_entry_published(user, exact_u64(seq)?).map_err(js)?)
    }

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
