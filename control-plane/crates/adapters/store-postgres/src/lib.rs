//! Postgres persistence adapters for runs, agents, and sessions (see `PLAN.md` §7).
//!
//! All three repositories share a single pool via [`PostgresStore`], which also
//! creates the schema and seeds the reference/demo data. Backed by the bundled,
//! in-cluster PostgreSQL StatefulSet from the Helm chart (`dependencies.postgres`).

mod agent;
mod approval;
mod audit;
mod budget;
mod checkpoint;
mod codec;
mod credential_grant;
mod event;
mod git_repo;
mod model;
mod provider;
mod run;
mod schema;
mod session;
mod setting;
mod tool;
mod transition;

pub use agent::PostgresAgentRepository;
pub use approval::PostgresApprovalRepository;
pub use audit::PostgresAuditRepository;
pub use budget::PostgresBudgetRepository;
pub use checkpoint::PostgresCheckpointRepository;
pub use credential_grant::PostgresCredentialGrantRepository;
pub use event::PostgresEventRepository;
pub use git_repo::PostgresGitRepoRepository;
pub use model::PostgresModelRepository;
pub use provider::PostgresProviderRepository;
pub use run::PostgresRunRepository;
pub use schema::PostgresStore;
pub use session::PostgresSessionRepository;
pub use setting::PostgresSettingRepository;
pub use tool::PostgresToolRepository;
pub use transition::PostgresTransitionRepository;
