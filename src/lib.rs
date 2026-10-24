//! Bomber Pengu game rules (no rendering).

pub mod ai;
pub mod game;
pub mod grid;
#[cfg(not(target_arch = "wasm32"))]
mod http;
pub mod i18n;
pub mod lobby;
pub mod net;
pub mod netgame;
pub mod player;
pub mod proto;
pub mod rng;
#[cfg(not(target_arch = "wasm32"))]
pub mod server;
pub mod world;
pub mod xml;
