/// Wire-protocol version negotiated between the extension and the review daemon.
///
/// Bumped on a breaking change to the HTTP/WS API surface or the MCP tool
/// signatures. Minor bumps remain backwards-compatible per `08-distribution.md`.
pub const PROTOCOL_VERSION: u32 = 1;
