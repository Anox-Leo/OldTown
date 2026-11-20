//! Invitation d'une personne (étape 10, § 4.5), partie sans réseau : ce que
//! l'admin garde de chaque invitation (`invites.json`) et ce que l'invité
//! garde jusqu'à son entrée dans l'annuaire (`pending_invite.json`), tous
//! deux dans le [`crate::Keyring`] : ils portent le secret `S` du code.
//!
//! Le déroulé (le [`crate::Client`] le mène, `Secure` vérifie et range) :
//!
//! 1. admin : `user_invite`, puis [`InviteRecord`] et code `ot2-…` ;
//! 2. invité : `invite_redeem` (jeton), [`save_pending_invite`], puis
//!    `invite_request` scellé sous `S` à l'appareil qui a invité, renvoyé à
//!    chaque connexion jusqu'à la bienvenue ;
//! 3. admin : demande vérifiée et rangée **avant** l'acquittement, puis
//!    certificat signé (gardé pour reprendre à l'identique), `user_certify`,
//!    entrée de son journal qui engage le certificat, `invite_welcome
//!    {cert, admin_head}` sous `S`, invitation effacée ;
//! 4. invité : bienvenue rangée, annuaire relu jusqu'à `admin_head` ; une
//!    fois son certificat vérifié par l'annuaire, il crée son groupe et
//!    publie l'entrée 1 de son journal.

use ot_crypto::directory::LogHead;
use ot_crypto::{InviteCode, PairingSecret};
use ot_proto::{DeviceId, SignedBlob, UserId, b64};
use serde::{Deserialize, Serialize};
use time::{Duration, OffsetDateTime};

use crate::directory::DirectoryState;
use crate::keyring::{Keyring, KeyringError};

/// Délai laissé à l'échange après la fin du jeton : l'invité peut racheter
/// le jeton à la dernière minute, et l'admin être hors ligne ensuite (le
/// relais garde l'échange ouvert 48 h après usage, note 0056).
pub const INVITE_EXCHANGE: Duration = Duration::hours(48);

/// Appareil de l'invité, tel que sa demande (vérifiée) le décrit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Guest {
    pub device_id: DeviceId,
    pub name: String,
    #[serde(with = "b64::array")]
    pub sign_key: [u8; 32],
    #[serde(with = "b64::array")]
    pub exchange_key: [u8; 32],
}

/// Invitation créée par cet appareil de l'admin. Seul lui détient `S` : la
/// demande attend dans la file du relais s'il est hors ligne.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InviteRecord {
    pub user: UserId,
    /// Nom de la personne, signé dans son certificat.
    pub name: String,
    #[serde(with = "b64::array")]
    pub token_id: [u8; 8],
    #[serde(with = "b64::array")]
    pub secret: [u8; 16],
    /// Fin du jeton sur le relais.
    #[serde(with = "time::serde::rfc3339")]
    pub expires_at: OffsetDateTime,
    /// Demande reçue et vérifiée.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guest: Option<Guest>,
    /// Certificat signé pour l'invité : le même à chaque reprise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cert: Option<SignedBlob>,
    /// `user_certify` accepté par le relais.
    #[serde(default)]
    pub certified: bool,
}

impl InviteRecord {
    pub fn secret(&self) -> PairingSecret {
        PairingSecret::from_bytes(self.secret)
    }

    /// L'échange peut encore aboutir.
    pub fn alive(&self, now: OffsetDateTime) -> bool {
        now < self.expires_at + INVITE_EXCHANGE
    }
}

impl Drop for InviteRecord {
    fn drop(&mut self) {
        zeroize::Zeroize::zeroize(&mut self.secret);
    }
}

impl Drop for PendingInvite {
    fn drop(&mut self) {
        zeroize::Zeroize::zeroize(&mut self.secret);
    }
}

impl std::fmt::Debug for InviteRecord {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InviteRecord")
            .field("user", &self.user)
            .field("guest", &self.guest.as_ref().map(|g| &g.device_id))
            .field("certified", &self.certified)
            .finish_non_exhaustive()
    }
}

/// Invitation rachetée par cet appareil, jusqu'à ce que sa personne soit
/// dans l'annuaire vérifié.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PendingInvite {
    pub user: UserId,
    /// Appareil de l'admin qui a invité : lui seul détient `S`.
    pub inviter: DeviceId,
    /// Nom de cet appareil.
    pub name: String,
    #[serde(with = "b64::array")]
    pub token_id: [u8; 8],
    #[serde(with = "b64::array")]
    pub secret: [u8; 16],
    #[serde(with = "b64::array")]
    pub anchor: [u8; 16],
    /// Bienvenue reçue : certificat, et tête du journal de l'admin qui
    /// l'engage. Reste à la voir confirmée par l'annuaire.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub welcome: Option<Welcome>,
}

impl PendingInvite {
    pub fn secret(&self) -> PairingSecret {
        PairingSecret::from_bytes(self.secret)
    }
}

impl std::fmt::Debug for PendingInvite {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PendingInvite")
            .field("user", &self.user)
            .field("inviter", &self.inviter)
            .field("welcome", &self.welcome.is_some())
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Welcome {
    pub cert: SignedBlob,
    pub admin_head: LogHead,
}

/// Code d'invitation à transmettre à la personne (il contient `S`).
#[derive(Clone, PartialEq, Eq)]
pub struct PersonInvitation {
    pub user: UserId,
    /// Code `ot2-…`.
    pub code: String,
    /// QR ou lien : `oldtown://invite?s=<relais>&c=<code>`.
    pub uri: String,
    pub expires_at: OffsetDateTime,
}

impl std::fmt::Debug for PersonInvitation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PersonInvitation")
            .field("user", &self.user)
            .field("expires_at", &self.expires_at)
            .finish_non_exhaustive()
    }
}

/// Réponse du relais à `invite_redeem`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InviteRedeemed {
    pub user: UserId,
    pub device_id: DeviceId,
    /// Appareil de l'admin à qui envoyer la demande.
    pub inviter: DeviceId,
}

/// Invité, après `invite_redeem` et avant de démarrer le client : range
/// l'invitation et l'ancre du code. Le client envoie la demande à chaque
/// connexion jusqu'à la bienvenue.
pub fn save_pending_invite(
    keyring: &dyn Keyring,
    code: &InviteCode,
    redeemed: &InviteRedeemed,
    name: &str,
) -> Result<(), KeyringError> {
    let pending = PendingInvite {
        user: redeemed.user.clone(),
        inviter: redeemed.inviter.clone(),
        name: name.to_owned(),
        token_id: code.token_id(),
        secret: *code.secret.as_bytes(),
        anchor: code.anchor,
        welcome: None,
    };
    keyring.save_directory(&DirectoryState::with_anchor(code.anchor))?;
    keyring.save_pending_invite(Some(&pending))
}
