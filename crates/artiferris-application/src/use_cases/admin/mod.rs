//! Admin use cases, one submodule per concern. Fakes shared by several concerns live in `admin_test_support`.

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
