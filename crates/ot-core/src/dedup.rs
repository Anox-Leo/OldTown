use std::collections::{HashSet, VecDeque};

use ot_proto::Ulid;

/// Ensemble borné des derniers `msg_id` vus : le plus ancien sort quand la
/// capacité est atteinte.
#[derive(Debug, Clone)]
pub struct Dedup {
    capacity: usize,
    set: HashSet<Ulid>,
    order: VecDeque<Ulid>,
}

impl Dedup {
    pub const DEFAULT_CAPACITY: usize = 1000;

    pub fn new(capacity: usize) -> Self {
        let capacity = capacity.max(1);
        Self {
            capacity,
            set: HashSet::with_capacity(capacity),
            order: VecDeque::with_capacity(capacity),
        }
    }

    pub fn with_seen(capacity: usize, seen: impl IntoIterator<Item = Ulid>) -> Self {
        let mut d = Self::new(capacity);
        for id in seen {
            d.insert(id);
        }
        d
    }

    /// Ajoute `id` ; renvoie `false` s'il avait déjà été vu.
    pub fn insert(&mut self, id: Ulid) -> bool {
        if !self.set.insert(id) {
            return false;
        }
        self.order.push_back(id);
        if self.order.len() > self.capacity
            && let Some(old) = self.order.pop_front()
        {
            self.set.remove(&old);
        }
        true
    }

    /// Identifiants vus, du plus ancien au plus récent.
    pub fn iter(&self) -> impl Iterator<Item = &Ulid> {
        self.order.iter()
    }
}

impl Default for Dedup {
    fn default() -> Self {
        Self::new(Self::DEFAULT_CAPACITY)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evicts_oldest() {
        let ids: Vec<Ulid> = (0..4).map(|i| Ulid::from_parts(i, 0)).collect();
        let mut d = Dedup::new(3);
        assert!(ids.iter().all(|id| d.insert(*id)));
        assert!(!d.insert(ids[3]));
        assert!(d.insert(ids[0]), "le plus ancien a été oublié");
        assert_eq!(d.iter().count(), 3);
    }
}
