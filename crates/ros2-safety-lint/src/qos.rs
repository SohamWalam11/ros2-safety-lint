use std::cmp::Ordering;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reliability {
    BestEffort,
    Reliable,
}

impl PartialOrd for Reliability {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Reliability {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self, other) {
            (Reliability::BestEffort, Reliability::Reliable) => Ordering::Less,
            (Reliability::Reliable, Reliability::BestEffort) => Ordering::Greater,
            _ => Ordering::Equal,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Durability {
    Volatile,
    TransientLocal,
}

impl PartialOrd for Durability {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Durability {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self, other) {
            (Durability::Volatile, Durability::TransientLocal) => Ordering::Less,
            (Durability::TransientLocal, Durability::Volatile) => Ordering::Greater,
            _ => Ordering::Equal,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Liveliness {
    Automatic,
    ManualByTopic,
}

impl PartialOrd for Liveliness {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Liveliness {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self, other) {
            (Liveliness::Automatic, Liveliness::ManualByTopic) => Ordering::Less,
            (Liveliness::ManualByTopic, Liveliness::Automatic) => Ordering::Greater,
            _ => Ordering::Equal,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum History {
    KeepLast(u32),
    KeepAll,
}

// KeepAll is generally considered "more strict" than KeepLast for compatibility if evaluated.
impl PartialOrd for History {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for History {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self, other) {
            (History::KeepLast(a), History::KeepLast(b)) => a.cmp(b),
            (History::KeepLast(_), History::KeepAll) => Ordering::Less,
            (History::KeepAll, History::KeepLast(_)) => Ordering::Greater,
            (History::KeepAll, History::KeepAll) => Ordering::Equal,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Duration {
    pub sec: i32,
    pub nsec: u32,
}

pub fn check_compatibility(
    pub_reliability: Reliability,
    sub_reliability: Reliability,
    pub_durability: Durability,
    sub_durability: Durability,
    pub_liveliness: Liveliness,
    sub_liveliness: Liveliness,
    pub_deadline: Duration,
    sub_deadline: Duration,
    pub_liveliness_lease: Duration,
    sub_liveliness_lease: Duration,
) -> Vec<String> {
    let mut errors = Vec::new();

    // Rx QoS must be less strict than or equal to Tx QoS
    if pub_reliability < sub_reliability {
        errors.push("Offered RELIABILITY is less than Requested".to_string());
    }

    if pub_durability < sub_durability {
        errors.push("Offered DURABILITY is less than Requested".to_string());
    }

    if pub_liveliness < sub_liveliness {
        errors.push("Offered LIVELINESS is less than Requested".to_string());
    }

    // Deadlines: Offered deadline must be <= Requested deadline (publisher guarantees to publish faster than subscriber expects)
    if pub_deadline > sub_deadline {
        errors.push("Offered DEADLINE is greater than Requested (Publisher too slow)".to_string());
    }

    // Liveliness Lease: Offered lease must be <= Requested lease
    if pub_liveliness_lease > sub_liveliness_lease {
        errors.push("Offered LIVELINESS LEASE is greater than Requested".to_string());
    }

    errors
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_reliability_compatibility() {
        assert!(check_compatibility(
            Reliability::Reliable,
            Reliability::Reliable,
            Durability::Volatile,
            Durability::Volatile,
            Liveliness::Automatic,
            Liveliness::Automatic,
            Duration { sec: 1, nsec: 0 },
            Duration { sec: 1, nsec: 0 },
            Duration { sec: 1, nsec: 0 },
            Duration { sec: 1, nsec: 0 },
        )
        .is_empty());

        assert!(check_compatibility(
            Reliability::Reliable,
            Reliability::BestEffort,
            Durability::Volatile,
            Durability::Volatile,
            Liveliness::Automatic,
            Liveliness::Automatic,
            Duration { sec: 1, nsec: 0 },
            Duration { sec: 1, nsec: 0 },
            Duration { sec: 1, nsec: 0 },
            Duration { sec: 1, nsec: 0 },
        )
        .is_empty());

        let errs = check_compatibility(
            Reliability::BestEffort,
            Reliability::Reliable,
            Durability::Volatile,
            Durability::Volatile,
            Liveliness::Automatic,
            Liveliness::Automatic,
            Duration { sec: 1, nsec: 0 },
            Duration { sec: 1, nsec: 0 },
            Duration { sec: 1, nsec: 0 },
            Duration { sec: 1, nsec: 0 },
        );
        assert_eq!(errs.len(), 1);
        assert_eq!(errs[0], "Offered RELIABILITY is less than Requested");
    }

    #[test]
    fn test_durability_compatibility() {
        let errs = check_compatibility(
            Reliability::Reliable,
            Reliability::Reliable,
            Durability::Volatile,
            Durability::TransientLocal,
            Liveliness::Automatic,
            Liveliness::Automatic,
            Duration { sec: 1, nsec: 0 },
            Duration { sec: 1, nsec: 0 },
            Duration { sec: 1, nsec: 0 },
            Duration { sec: 1, nsec: 0 },
        );
        assert_eq!(errs.len(), 1);
        assert_eq!(errs[0], "Offered DURABILITY is less than Requested");
    }

    #[test]
    fn test_deadline_compatibility() {
        // Pub deadline 2s > Sub deadline 1s -> Error
        let errs = check_compatibility(
            Reliability::Reliable,
            Reliability::Reliable,
            Durability::Volatile,
            Durability::Volatile,
            Liveliness::Automatic,
            Liveliness::Automatic,
            Duration { sec: 2, nsec: 0 },
            Duration { sec: 1, nsec: 0 },
            Duration { sec: 1, nsec: 0 },
            Duration { sec: 1, nsec: 0 },
        );
        assert_eq!(errs.len(), 1);
        assert_eq!(
            errs[0],
            "Offered DEADLINE is greater than Requested (Publisher too slow)"
        );

        // Pub deadline 1s <= Sub deadline 2s -> OK
        assert!(check_compatibility(
            Reliability::Reliable,
            Reliability::Reliable,
            Durability::Volatile,
            Durability::Volatile,
            Liveliness::Automatic,
            Liveliness::Automatic,
            Duration { sec: 1, nsec: 0 },
            Duration { sec: 2, nsec: 0 },
            Duration { sec: 1, nsec: 0 },
            Duration { sec: 1, nsec: 0 },
        )
        .is_empty());
    }
}
