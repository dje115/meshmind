//! Membership states for peers: Alive, Suspect, Dead, Quarantined.

use std::time::Instant;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MembershipState {
    Alive,
    Suspect,
    Dead,
    Quarantined,
}

impl std::fmt::Display for MembershipState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Alive => write!(f, "Alive"),
            Self::Suspect => write!(f, "Suspect"),
            Self::Dead => write!(f, "Dead"),
            Self::Quarantined => write!(f, "Quarantined"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct PeerEntry {
    pub node_id: String,
    pub address: String,
    pub port: u16,
    pub state: MembershipState,
    pub capabilities: Vec<String>,
    pub version: String,
    pub rtt_ms: Option<u32>,
    pub last_seen: Instant,
    pub suspect_since: Option<Instant>,
}

impl PeerEntry {
    pub fn new(node_id: &str, address: &str, port: u16) -> Self {
        Self {
            node_id: node_id.to_string(),
            address: address.to_string(),
            port,
            state: MembershipState::Alive,
            capabilities: vec![],
            version: String::new(),
            rtt_ms: None,
            last_seen: Instant::now(),
            suspect_since: None,
        }
    }

    pub fn mark_alive(&mut self) {
        self.state = MembershipState::Alive;
        self.last_seen = Instant::now();
        self.suspect_since = None;
    }

    pub fn mark_suspect(&mut self) {
        if self.state == MembershipState::Alive {
            self.state = MembershipState::Suspect;
            self.suspect_since = Some(Instant::now());
        }
    }

    pub fn mark_dead(&mut self) {
        self.state = MembershipState::Dead;
    }

    pub fn mark_quarantined(&mut self) {
        self.state = MembershipState::Quarantined;
    }

    pub fn is_reachable(&self) -> bool {
        matches!(
            self.state,
            MembershipState::Alive | MembershipState::Suspect
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_peer_is_alive() {
        let peer = PeerEntry::new("node-1", "192.168.1.10", 9000);
        assert_eq!(peer.state, MembershipState::Alive);
        assert!(peer.is_reachable());
    }

    #[test]
    fn state_transitions() {
        let mut peer = PeerEntry::new("node-1", "192.168.1.10", 9000);

        peer.mark_suspect();
        assert_eq!(peer.state, MembershipState::Suspect);
        assert!(peer.is_reachable());
        assert!(peer.suspect_since.is_some());

        peer.mark_alive();
        assert_eq!(peer.state, MembershipState::Alive);
        assert!(peer.suspect_since.is_none());

        peer.mark_dead();
        assert_eq!(peer.state, MembershipState::Dead);
        assert!(!peer.is_reachable());

        peer.mark_quarantined();
        assert_eq!(peer.state, MembershipState::Quarantined);
        assert!(!peer.is_reachable());
    }

    #[test]
    fn display_states() {
        assert_eq!(format!("{}", MembershipState::Alive), "Alive");
        assert_eq!(format!("{}", MembershipState::Suspect), "Suspect");
        assert_eq!(format!("{}", MembershipState::Dead), "Dead");
        assert_eq!(format!("{}", MembershipState::Quarantined), "Quarantined");
    }
}
