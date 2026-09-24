//! Admin use cases, split by concern (Q-6). Each submodule owns one concern's production code
//! and its own tests; fakes shared by two or more concerns' tests live in the sibling
//! `crate::use_cases::admin_test_support` module (this crate's existing convention — see also
//! `npm_test_support.rs`, `docker_test_support.rs`), while fakes used by only one concern's tests
//! stay local to that concern's file.

mod admin_stats;
mod api_tokens;
mod audit_log_and_security_events;
mod config_export;
mod config_import;
mod metrics_history;
mod system_health;
mod system_settings;
mod usage_metrics;

pub use admin_stats::{AdminStats, GetAdminStatsUseCase};
pub use api_tokens::{AdminApiTokenEntry, AdminListApiTokensUseCase, AdminRevokeApiTokenUseCase, MAX_ADMIN_TOKEN_PAGE};
pub use audit_log_and_security_events::{QueryAuditLogUseCase, RecordAdminEventUseCase, RecordSecurityEventUseCase};
pub use config_export::{ConfigurationExport, ExportConfigurationUseCase, ExportedPermission, ExportedRepository, ExportedUser};
pub use config_import::{ConfigurationImport, ImportConfigurationUseCase, ImportReport};
pub use metrics_history::{GetMetricsHistoryUseCase, RecordMetricsSnapshotUseCase};
pub use system_health::{GetHealthStatusUseCase, HealthStatus, StorageHealth};
pub use system_settings::{GetSystemSettingsUseCase, UpdateSystemSettingsUseCase};
pub use usage_metrics::{GetUsageMetricsUseCase, RepositoryUsage};
