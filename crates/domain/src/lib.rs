//! Cat Herder — domain core (the hexagon center).
//!
//! Pure Rust, no I/O, no framework dependencies. Ports are `trait`s; adapters
//! (in `crates/adapters/*`) implement them and are wired in the composition root
//! (`crates/bin/cat-herder`). See `PLAN.md` §5.
//!
//! We use native `async fn` in traits (not `async-trait`) and generic dispatch
//! (not `dyn`), per `PLAN.md` §5. `Send` bounds on the futures will be added when
//! adapters are implemented and the traits are used across `tokio::spawn`.

#![allow(async_fn_in_trait)]

pub mod api;
pub mod entities;
pub mod ports;
pub mod state;

pub use entities::*;
pub use ports::*;
