//! Règles du roster : qui peut le changer, et comment.
//!
//! La confiance vient de l'appairage, jamais du relais : un roster n'est
//! accepté que d'un membre déjà connu, et seulement s'il est plus récent.

use ot_crypto::{Member, Roster};
use ot_proto::DeviceId;

/// Raison du refus d'un roster.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RosterError {
    #[error("l'expéditeur n'est pas membre du groupe")]
    NotMember,
    /// `(epoch, version)` pas plus grand que le roster courant (rejeu ou
    /// message en retard) : ignoré sans bruit.
    #[error("roster déjà connu ou plus ancien")]
    Stale,
    #[error("le roster ne contient pas cet appareil")]
    MissingSelf,
    #[error("le roster change les clés d'un membre existant")]
    KeyChanged,
    #[error("le roster contient deux fois le même appareil")]
    Duplicate,
    #[error("appareil déjà membre du groupe")]
    AlreadyMember,
}

/// Premier roster : l'appareil seul, époque 1.
pub fn genesis(me: Member) -> Roster {
    Roster { epoch: 1, version: 1, members: vec![me] }
}

/// Vérifie qu'un roster signé par `sender` peut remplacer `current`.
pub fn check_replacement(
    current: &Roster,
    new: &Roster,
    sender: &DeviceId,
    me: &DeviceId,
) -> Result<(), RosterError> {
    if !current.contains(sender) {
        return Err(RosterError::NotMember);
    }
    if (new.epoch, new.version) <= (current.epoch, current.version) {
        return Err(RosterError::Stale);
    }
    check_shape(new, me)?;
    check_keys_unchanged(current, new)
}

/// Un roster bien formé contient l'appareil, sans doublon.
pub fn check_shape(roster: &Roster, me: &DeviceId) -> Result<(), RosterError> {
    if !roster.contains(me) {
        return Err(RosterError::MissingSelf);
    }
    for (i, m) in roster.members.iter().enumerate() {
        if roster.members[..i].iter().any(|o| o.device_id == m.device_id) {
            return Err(RosterError::Duplicate);
        }
    }
    Ok(())
}

/// Les clés d'un membre ne changent jamais : un appareil qui change de clé
/// est un nouvel appareil, à appairer.
pub fn check_keys_unchanged(current: &Roster, new: &Roster) -> Result<(), RosterError> {
    for m in &new.members {
        if let Some(old) = current.member(&m.device_id)
            && (old.sign_key != m.sign_key || old.exchange_key != m.exchange_key)
        {
            return Err(RosterError::KeyChanged);
        }
    }
    Ok(())
}

/// Ajoute un membre (appairage) ; la version augmente.
pub fn with_member(current: &Roster, member: Member) -> Result<Roster, RosterError> {
    if current.contains(&member.device_id) {
        return Err(RosterError::AlreadyMember);
    }
    let mut next = current.clone();
    next.members.push(member);
    next.version += 1;
    Ok(next)
}

/// Roster d'une nouvelle époque, sans les exclus ; la version augmente.
pub fn rotated(current: &Roster, epoch: u32, excluded: &[DeviceId]) -> Roster {
    Roster {
        epoch,
        version: current.version + 1,
        members: current
            .members
            .iter()
            .filter(|m| !excluded.contains(&m.device_id))
            .cloned()
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn member(seed: u8) -> Member {
        Member {
            device_id: DeviceId::generate(),
            name: format!("appareil-{seed}"),
            sign_key: [seed; 32],
            exchange_key: [seed + 100; 32],
            added_epoch: 1,
        }
    }

    #[test]
    fn roster_from_non_member_is_refused() {
        let (a, b, stranger) = (member(1), member(2), member(3));
        let current = genesis(a.clone());
        let mut new = with_member(&current, b.clone()).unwrap();
        assert_eq!(check_replacement(&current, &new, &a.device_id, &b.device_id), Ok(()));
        assert_eq!(
            check_replacement(&current, &new, &stranger.device_id, &a.device_id),
            Err(RosterError::NotMember)
        );
        // Même un roster qui le contient ne rend pas un inconnu légitime.
        new.members.push(stranger.clone());
        assert_eq!(
            check_replacement(&current, &new, &stranger.device_id, &a.device_id),
            Err(RosterError::NotMember)
        );
    }

    #[test]
    fn older_or_equal_rosters_are_stale() {
        let (a, b) = (member(1), member(2));
        let v1 = genesis(a.clone());
        let v2 = with_member(&v1, b).unwrap();
        assert_eq!(
            check_replacement(&v2, &v1, &a.device_id, &a.device_id),
            Err(RosterError::Stale)
        );
        assert_eq!(
            check_replacement(&v2, &v2, &a.device_id, &a.device_id),
            Err(RosterError::Stale)
        );
    }

    #[test]
    fn keys_of_a_member_never_change() {
        let (a, b) = (member(1), member(2));
        let v1 = with_member(&genesis(a.clone()), b.clone()).unwrap();
        let mut v2 = v1.clone();
        v2.version += 1;
        v2.members[1].sign_key = [9; 32];
        assert_eq!(
            check_replacement(&v1, &v2, &a.device_id, &a.device_id),
            Err(RosterError::KeyChanged)
        );
    }

    #[test]
    fn rotation_removes_excluded() {
        let (a, b, c) = (member(1), member(2), member(3));
        let r =
            with_member(&with_member(&genesis(a.clone()), b.clone()).unwrap(), c.clone()).unwrap();
        let next = rotated(&r, 2, std::slice::from_ref(&c.device_id));
        assert_eq!(next.epoch, 2);
        assert!(next.contains(&a.device_id) && next.contains(&b.device_id));
        assert!(!next.contains(&c.device_id));
        assert_eq!(check_replacement(&r, &next, &a.device_id, &b.device_id), Ok(()));
        assert_eq!(
            check_replacement(&r, &next, &a.device_id, &c.device_id),
            Err(RosterError::MissingSelf)
        );
    }

    #[test]
    fn member_cannot_be_added_twice() {
        let a = member(1);
        assert_eq!(with_member(&genesis(a.clone()), a), Err(RosterError::AlreadyMember));
    }
}
