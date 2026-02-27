//! LAN discovery, membership states, peer directory, transport abstraction.
//!
//! Phase 1: mDNS discovery + TCP+mTLS transport.
//! Phase 2 (future): QUIC transport.

pub mod consult;
pub mod discovery;
pub mod membership;
pub mod peer_dir;
pub mod tcp_transport;
pub mod transport;

pub use consult::{ConsultConfig, ConsultResult};
pub use discovery::{register_service, start_discovery, unregister_service};
pub use membership::{MembershipState, PeerEntry};
pub use peer_dir::PeerDirectory;
pub use tcp_transport::{TcpServer, TcpTransport};
