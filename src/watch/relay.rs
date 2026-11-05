//! The relay WebSocket client (`RelayClient.swift`): one connection to
//! `wss://<base>/m/<room>` on its own thread, pings, half-open detection,
//! and reconnection. It only moves frames; the `watch` thread owns state.
