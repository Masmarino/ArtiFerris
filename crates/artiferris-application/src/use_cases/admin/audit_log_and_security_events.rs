use std::sync::Arc;

use artiferris_domain::audit::{redact_secrets, AdminAuditEvent, AuditPage, AuditQueryFilter, EventPublisherPort, SecurityEvent};
use uuid::Uuid;

use crate::error::ApplicationError;

pub struct QueryAuditLogUseCase {
    publisher: Arc<dyn EventPublisherPort>,
}

impl QueryAuditLogUseCase {
    pub fn new(publisher: Arc<dyn EventPublisherPort>) -> Self {
        Self { publisher }
    }

    /// Secrets in payloads are blanked, whichever view asks.
    pub async fn execute(&self, filter: AuditQueryFilter) -> Result<AuditPage, ApplicationError> {
        let mut page = self.publisher.query_audit_log(filter).await?;
        for entry in &mut page.entries {
            redact_secrets(&mut entry.payload);
        }
        Ok(page)
    }
}

pub struct RecordSecurityEventUseCase {
    publisher: Arc<dyn EventPublisherPort>,
}

impl RecordSecurityEventUseCase {
    pub fn new(publisher: Arc<dyn EventPublisherPort>) -> Self {
        Self { publisher }
    }

    pub async fn execute(&self, event: SecurityEvent, actor_id: Option<Uuid>) -> Result<(), ApplicationError> {
        Ok(self.publisher.publish_security_event(event.normalized(), actor_id).await?)
    }
}

pub struct RecordAdminEventUseCase {
    publisher: Arc<dyn EventPublisherPort>,
}

impl RecordAdminEventUseCase {
    pub fn new(publisher: Arc<dyn EventPublisherPort>) -> Self {
        Self { publisher }
    }

    pub async fn execute(&self, event: AdminAuditEvent, actor_id: Option<Uuid>) -> Result<(), ApplicationError> {
        Ok(self.publisher.publish_admin_event(event, actor_id).await?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use artiferris_domain::error::EventStoreError;
    use std::sync::Mutex;

    struct FakeEventPublisher {
        security_events: Mutex<Vec<SecurityEvent>>,
        admin_events: Mutex<Vec<(AdminAuditEvent, Option<Uuid>)>>,
    }

    impl FakeEventPublisher {
        fn new() -> Self {
            Self { security_events: Mutex::new(Vec::new()), admin_events: Mutex::new(Vec::new()) }
        }
    }

    #[async_trait]
    impl EventPublisherPort for FakeEventPublisher {
        async fn publish_security_event(&self, event: SecurityEvent, _actor_id: Option<Uuid>) -> Result<(), EventStoreError> {
            self.security_events.lock().unwrap().push(event);
            Ok(())
        }

        async fn publish_admin_event(&self, event: AdminAuditEvent, actor_id: Option<Uuid>) -> Result<(), EventStoreError> {
            self.admin_events.lock().unwrap().push((event, actor_id));
            Ok(())
        }

        async fn query_audit_log(&self, _filter: AuditQueryFilter) -> Result<AuditPage, EventStoreError> {
            Ok(AuditPage { entries: Vec::new(), next_cursor: None })
        }

        async fn publish_npm_event(
            &self,
            _event: artiferris_domain::audit::NpmPackageEvent,
            _npm_package_id: Uuid,
            _package_repository_id: Uuid,
            _actor_id: Option<Uuid>,
        ) -> Result<(), EventStoreError> {
            Ok(())
        }

        async fn publish_docker_event(
            &self,
            _event: artiferris_domain::audit::DockerRegistryEvent,
            _package_repository_id: Uuid,
            _actor_id: Option<Uuid>,
        ) -> Result<(), EventStoreError> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn records_and_queries_security_events() {
        let publisher = Arc::new(FakeEventPublisher::new());
        let record = RecordSecurityEventUseCase::new(publisher.clone());
        record
            .execute(
                SecurityEvent::LoginFailed { username: "florian".to_string(), ip: "127.0.0.1".to_string() },
                None,
            )
            .await
            .unwrap();
        assert_eq!(publisher.security_events.lock().unwrap().len(), 1);

        let query = QueryAuditLogUseCase::new(publisher);
        let page = query.execute(AuditQueryFilter::default()).await.unwrap();
        assert!(page.entries.is_empty());
    }

    #[tokio::test]
    async fn records_admin_events_with_their_actor() {
        let publisher = Arc::new(FakeEventPublisher::new());
        let actor = Uuid::new_v4();
        RecordAdminEventUseCase::new(publisher.clone())
            .execute(AdminAuditEvent::ConfigurationExported { users: 1, repositories: 2, permissions: 3 }, Some(actor))
            .await
            .unwrap();

        let recorded = publisher.admin_events.lock().unwrap();
        assert_eq!(recorded.len(), 1);
        assert_eq!(recorded[0].1, Some(actor));
    }
}
