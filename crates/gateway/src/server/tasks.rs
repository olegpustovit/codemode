use codemode_core::components::auth::Principal;
use rmcp::{
    ErrorData,
    model::{DetailedTask, Task},
    task_manager::{TaskContext, TaskFuture, TaskManager, TaskOptions},
};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex, MutexGuard},
};
use uuid::Uuid;

// shortcut: task handles are process-local; persist them when polling must survive a gateway restart.
#[derive(Clone, Default)]
pub struct Tasks(Arc<Inner>);
type Owners = BTreeMap<String, (Uuid, Uuid, String)>;
#[derive(Default)]
struct Inner {
    manager: TaskManager,
    owners: Mutex<Owners>,
}
impl Tasks {
    fn owners(&self) -> Result<MutexGuard<'_, Owners>, ErrorData> {
        self.0
            .owners
            .lock()
            .map_err(|_| ErrorData::internal_error("task ownership unavailable", None))
    }
    pub fn spawn(
        &self,
        owner: &Principal,
        make_future: impl FnOnce(TaskContext) -> TaskFuture,
    ) -> Result<Task, ErrorData> {
        let mut owners = self.owners()?;
        // Sweep through the SDK so ownership and retained results share the same expiry rule.
        owners.retain(|id, _| self.0.manager.get_task(id).is_ok());
        if owners.len() >= 64 {
            return Err(ErrorData::internal_error(
                "task capacity reached; retrieve existing results and retry after their five-minute retention expires",
                None,
            ));
        }
        let task = self.0.manager.spawn(
            TaskOptions::new()
                .with_ttl_ms(300_000)
                .with_poll_interval_ms(2_000),
            make_future,
        );
        owners.insert(
            task.task_id.clone(),
            (owner.tenant_id, owner.token_id, owner.audience.clone()),
        );
        Ok(task)
    }
    fn authorize(&self, owner: &Principal, id: &str) -> Result<(), ErrorData> {
        if self
            .owners()?
            .get(id)
            .is_some_and(|(tenant, token, audience)| {
                *tenant == owner.tenant_id
                    && *token == owner.token_id
                    && *audience == owner.audience
            })
        {
            Ok(())
        } else {
            Err(ErrorData::invalid_params("unknown task", None))
        }
    }
    pub fn get(&self, owner: &Principal, id: &str) -> Result<DetailedTask, ErrorData> {
        self.authorize(owner, id)?;
        self.0.manager.get_task(id)
    }
    pub fn update(
        &self,
        owner: &Principal,
        id: &str,
        responses: rmcp::model::InputResponses,
    ) -> Result<(), ErrorData> {
        self.authorize(owner, id)?;
        self.0.manager.update_task(id, responses)
    }
    pub fn cancel(&self, owner: &Principal, id: &str) -> Result<(), ErrorData> {
        self.authorize(owner, id)?;
        self.0.manager.cancel_task(id)
    }
    pub fn shutdown(&self) {
        self.0.manager.shutdown();
        if let Ok(mut owners) = self.owners() {
            owners.clear();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmcp::{model::TaskStatus, task_manager::TaskExit};
    #[tokio::test]
    async fn task_handles_are_caller_bound_bounded_and_cancellable() {
        let tasks = Tasks::default();
        let owner = Principal {
            tenant_id: Uuid::nil(),
            workspace_id: Uuid::nil(),
            resource: "/mcp".into(),
            token_id: Uuid::new_v4(),
            audience: "one.example".into(),
        };
        let task = tasks
            .spawn(&owner, |ctx| {
                Box::pin(async move {
                    ctx.cancelled().await;
                    Err(TaskExit::Cancelled)
                })
            })
            .unwrap();
        for foreign in [
            Principal {
                tenant_id: Uuid::nil(),
                workspace_id: Uuid::nil(),
                resource: "/mcp".into(),
                token_id: Uuid::new_v4(),
                audience: owner.audience.clone(),
            },
            Principal {
                tenant_id: Uuid::nil(),
                workspace_id: Uuid::nil(),
                resource: "/mcp".into(),
                token_id: owner.token_id,
                audience: "two.example".into(),
            },
        ] {
            assert!(tasks.get(&foreign, &task.task_id).is_err());
            assert!(
                tasks
                    .update(&foreign, &task.task_id, Default::default())
                    .is_err()
            );
            assert!(tasks.cancel(&foreign, &task.task_id).is_err());
        }
        assert_eq!(
            tasks.get(&owner, &task.task_id).unwrap().task.status,
            TaskStatus::Working
        );
        tasks.cancel(&owner, &task.task_id).unwrap();
        tokio::time::timeout(std::time::Duration::from_millis(500), async {
            while tasks.get(&owner, &task.task_id).unwrap().task.status != TaskStatus::Cancelled {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        for _ in 1..64 {
            tasks
                .spawn(&owner, |_| Box::pin(std::future::pending()))
                .unwrap();
        }
        assert!(
            tasks
                .spawn(&owner, |_| Box::pin(std::future::pending()))
                .is_err()
        );
        tasks.shutdown();
        assert!(tasks.get(&owner, &task.task_id).is_err());
    }
}
