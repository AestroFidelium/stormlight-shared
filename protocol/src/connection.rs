//! The wire format — single source of truth, shared verbatim by server and
//! client. The only per-tick state on the wire is Bevy's own [`Transform`],
//! replicated through the compact quantized codec in [`crate::quantize`] (see
//! [`crate::protocol`]). No game content — the engine replicates generic ECS
//! state, nothing hero- or map-specific.

use bevy::prelude::*;
use std::net::{Ipv4Addr, SocketAddr};
use std::time::Duration;

/// Netcode protocol id. Server and client must agree byte-for-byte or the
/// handshake is rejected; bump it on any breaking wire change.
pub const PROTOCOL_ID: u64 = 0x5701_0000_0000_0002;

/// Default listen / connect port for local development.
pub const SERVER_PORT: u16 = 5000;

/// Simulation tick rate. Server and client build their Lightyear plugin groups
/// with the same `tick_duration` so timelines line up.
pub const TICK_HZ: f64 = 64.0;

/// How often the server flushes replication updates to each client. One value,
/// used by every per-client `ReplicationSender`.
pub const REPLICATION_SEND_INTERVAL: Duration = Duration::from_millis(50);

/// The address a local client dials.
#[must_use]
pub fn server_addr() -> SocketAddr {
    SocketAddr::new(Ipv4Addr::LOCALHOST.into(), SERVER_PORT)
}

/// The bind address for a listening server: all interfaces, fixed port.
#[must_use]
pub fn server_bind_addr() -> SocketAddr {
    SocketAddr::new(Ipv4Addr::UNSPECIFIED.into(), SERVER_PORT)
}

/// An ephemeral local bind for a connecting client — the OS picks the port.
#[must_use]
pub fn client_bind_addr() -> SocketAddr {
    SocketAddr::new(Ipv4Addr::UNSPECIFIED.into(), 0)
}

/// One simulation tick as a `Duration`.
#[must_use]
pub fn tick_duration() -> Duration {
    Duration::from_secs_f64(1.0 / TICK_HZ)
}

/// Component-wise interpolation between two `Transform` snapshots, the generic
/// smoothing primitive every renderer uses to hide the gap between confirmed
/// server states. Translation/scale lerp; rotation uses `slerp` so the result
/// stays a unit quaternion for any `t` (no shearing). `Transform` doesn't
/// implement `Ease`, so this is supplied explicitly wherever smoothing is
/// wired.
#[must_use]
pub fn lerp_transform(start: Transform, end: Transform, t: f32) -> Transform {
    Transform {
        translation: start.translation.lerp(end.translation, t),
        rotation: start.rotation.slerp(end.rotation, t),
        scale: start.scale.lerp(end.scale, t),
    }
}
