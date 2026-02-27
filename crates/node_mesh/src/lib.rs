//! LAN discovery, membership states, peer directory, transport abstraction.
//!
//! Phase 1: mDNS discovery + TCP transport abstraction.
//! Phase 2 (future): QUIC transport.

pub mod consult;
pub mod membership;
pub mod peer_dir;
pub mod transport;

pub use consult::{ConsultConfig, ConsultResult};
pub use membership::{MembershipState, PeerEntry};
pub use peer_dir::PeerDirectory;
