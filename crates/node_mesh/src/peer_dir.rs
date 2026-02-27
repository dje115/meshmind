//! Peer directory: maintains partial peer view capped at max_peers (default 30).

use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::membership::{MembershipState, PeerEntry};

const DEFAULT_MAX_PEERS: usize = 30;
const SUSPECT_TIMEOUT: Duration = Duration::from_secs(30);
const DEAD_TIMEOUT: Duration = Duration::from_secs(120);

pub struct PeerDirectory {
    peers: HashMap<String, PeerEntry>,
    max_peers: usize,
}

impl PeerDirectory {
    pub fn new() -> Self {
        Self {
            peers: HashMap::new(),
            max_peers: DEFAULT_MAX_PEERS,
        }
    }

    pub fn with_max_peers(max_peers: usize) -> Self {
        Self {
            peers: HashMap::new(),
            max_peers,
        }
    }

    /// Add or update a peer. Returns true if newly added.
    pub fn upsert(&mut self, node_id: &str, address: &str, port: u16) -> bool {
        if let Some(peer) = self.peers.get_mut(node_id) {
            peer.address = address.to_string();
            peer.port = port;
            peer.mark_alive();
            return false;
        }

        if self.peers.len() >= self.max_peers {
            self.evict_one();
        }

        if self.peers.len() < self.max_peers {
            self.peers.insert(
                node_id.to_string(),
                PeerEntry::new(node_id, address, port),
            );
            true
        } else {
            false
        }
    }

    /// Remove the worst peer (dead > quarantined > suspect > oldest alive).
    fn evict_one(&mut self) {
        let to_remove = self
            .peers
            .iter()
            .min_by_key(|(_, p)| {
                let priority = match p.state {
                    MembershipState::Dead => 0,
                    MembershipState::Quarantined => 1,
                    MembershipState::Suspect => 2,
                    MembershipState::Alive => 3,
                };
                (priority, p.last_seen)
            })
            .map(|(k, _)| k.clone());

        if let Some(key) = to_remove {
            self.peers.remove(&key);
        }
    }

    pub fn get(&self, node_id: &str) -> Option<&PeerEntry> {
        self.peers.get(node_id)
    }

    pub fn get_mut(&mut self, node_id: &str) -> Option<&mut PeerEntry> {
        self.peers.get_mut(node_id)
    }

    pub fn remove(&mut self, node_id: &str) -> Option<PeerEntry> {
        self.peers.remove(node_id)
    }

    pub fn alive_peers(&self) -> Vec<&PeerEntry> {
        self.peers
            .values()
            .filter(|p| p.state == MembershipState::Alive)
            .collect()
    }

    pub fn reachable_peers(&self) -> Vec<&PeerEntry> {
        self.peers.values().filter(|p| p.is_reachable()).collect()
    }

    pub fn all_peers(&self) -> Vec<&PeerEntry> {
        self.peers.values().collect()
    }

    pub fn len(&self) -> usize {
        self.peers.len()
    }

    pub fn is_empty(&self) -> bool {
        self.peers.is_empty()
    }

    /// Tick: promote suspect -> dead if timed out, alive -> suspect if not seen.
    pub fn tick(&mut self, alive_timeout: Duration) {
        let now = Instant::now();
        for peer in self.peers.values_mut() {
            match peer.state {
                MembershipState::Alive => {
                    if now.duration_since(peer.last_seen) > alive_timeout {
                        peer.mark_suspect();
                    }
                }
                MembershipState::Suspect => {
                    if let Some(since) = peer.suspect_since {
                        if now.duration_since(since) > SUSPECT_TIMEOUT {
                            peer.mark_dead();
                        }
                    }
                }
                MembershipState::Dead => {
                    if let Some(since) = peer.suspect_since {
                        if now.duration_since(since) > DEAD_TIMEOUT {
                            // keep dead, will be evicted when space needed
                        }
                    }
                }
                MembershipState::Quarantined => {}
            }
        }
    }
}

impl Default for PeerDirectory {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_and_get_peer() {
        let mut dir = PeerDirectory::new();
        assert!(dir.upsert("node-1", "192.168.1.10", 9000));
        assert!(!dir.upsert("node-1", "192.168.1.10", 9000)); // update

        let peer = dir.get("node-1").unwrap();
        assert_eq!(peer.node_id, "node-1");
        assert_eq!(peer.state, MembershipState::Alive);
    }

    #[test]
    fn max_peers_cap() {
        let mut dir = PeerDirectory::with_max_peers(3);
        dir.upsert("n1", "1.1.1.1", 9000);
        dir.upsert("n2", "1.1.1.2", 9000);
        dir.upsert("n3", "1.1.1.3", 9000);
        assert_eq!(dir.len(), 3);

        dir.upsert("n4", "1.1.1.4", 9000);
        assert_eq!(dir.len(), 3);
        assert!(dir.get("n4").is_some());
    }

    #[test]
    fn evicts_dead_first() {
        let mut dir = PeerDirectory::with_max_peers(2);
        dir.upsert("n1", "1.1.1.1", 9000);
        dir.upsert("n2", "1.1.1.2", 9000);
        dir.get_mut("n1").unwrap().mark_dead();

        dir.upsert("n3", "1.1.1.3", 9000);
        assert!(dir.get("n1").is_none());
        assert!(dir.get("n2").is_some());
        assert!(dir.get("n3").is_some());
    }

    #[test]
    fn alive_and_reachable() {
        let mut dir = PeerDirectory::new();
        dir.upsert("n1", "1.1.1.1", 9000);
        dir.upsert("n2", "1.1.1.2", 9000);
        dir.upsert("n3", "1.1.1.3", 9000);
        dir.get_mut("n2").unwrap().mark_suspect();
        dir.get_mut("n3").unwrap().mark_dead();

        assert_eq!(dir.alive_peers().len(), 1);
        assert_eq!(dir.reachable_peers().len(), 2);
        assert_eq!(dir.all_peers().len(), 3);
    }

    #[test]
    fn remove_peer() {
        let mut dir = PeerDirectory::new();
        dir.upsert("n1", "1.1.1.1", 9000);
        assert_eq!(dir.len(), 1);
        dir.remove("n1");
        assert!(dir.is_empty());
    }
}
