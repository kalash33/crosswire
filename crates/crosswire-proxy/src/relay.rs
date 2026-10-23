//! The HTTP/1.1 relay. Placeholder until Task 5 replaces it.

use crosswire_client::{Conn, Target};

use crate::listener::ConnCtx;
use crate::ProxyError;

/// Receives the client stream after a successful MITM handshake. Task 5
/// replaces this with `relay::run`; for now it logs and closes.
pub(crate) async fn run_stub(
    client: Box<dyn Conn>,
    target: Target,
    ctx: ConnCtx,
) -> Result<(), ProxyError> {
    tracing::debug!(
        peer = %ctx.peer,
        host = target.host(),
        port = target.port(),
        "TLS client accepted (relay not yet implemented)"
    );
    drop(client);
    Ok(())
}
