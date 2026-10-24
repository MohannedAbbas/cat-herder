//! Postgres persistence adapters for runs, agents, and sessions (see `PLAN.md` §7).
//!
//! All three repositories share a single pool via [`PostgresStore`], which also
//! creates the schema and seeds the reference/demo data. Backed by the bundled,
//! in-cluster PostgreSQL StatefulSet from the Helm chart (`dependencies.postgres`).

mod agent;
mod codec;
mod run;
mod schema;
mod session;

pub use agent::PostgresAgentRepository;
pub use run::PostgresRunRepository;
pub use schema::PostgresStore;
pub use session::PostgresSessionRepository;
