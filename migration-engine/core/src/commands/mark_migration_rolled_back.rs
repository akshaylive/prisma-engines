use super::MigrationCommand;
use crate::{api::MigrationApi, CoreError, CoreResult};
use migration_connector::MigrationConnector;
use serde::Deserialize;
use std::collections::HashMap;
use user_facing_errors::migration_engine::{CannotRollBackSucceededMigration, CannotRollBackUnappliedMigration};

/// The input to the `markMigrationRolledBack` command.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MarkMigrationRolledBackInput {
    /// The name of the migration to mark rolled back.
    pub migration_name: String,
}

/// The output of the `markMigrationRolledBack` command.
pub type MarkMigrationRolledBackOutput = HashMap<(), ()>;

/// Mark a migration as rolled back.
pub struct MarkMigrationRolledBackCommand;

#[async_trait::async_trait]
impl MigrationCommand for MarkMigrationRolledBackCommand {
    type Input = MarkMigrationRolledBackInput;
    type Output = MarkMigrationRolledBackOutput;

    async fn execute<C: MigrationConnector>(input: &Self::Input, engine: &MigrationApi<C>) -> CoreResult<Self::Output> {
        // We should take a lock on the migrations table.

        let persistence = engine.connector().new_migration_persistence();

        let all_migrations = persistence.list_migrations().await?.map_err(|_err| {
            CoreError::Generic(anyhow::anyhow!(
                "Invariant violation: called markMigrationRolledBack on a database without migrations table."
            ))
        })?;

        let relevant_migrations: Vec<_> = all_migrations
            .into_iter()
            .filter(|migration| migration.migration_name == input.migration_name)
            .collect();

        if relevant_migrations.is_empty() {
            return Err(CoreError::user_facing(CannotRollBackUnappliedMigration {
                migration_name: input.migration_name.clone(),
            }));
        }

        if relevant_migrations
            .iter()
            .all(|migration| migration.finished_at.is_some())
        {
            return Err(CoreError::user_facing(CannotRollBackSucceededMigration {
                migration_name: input.migration_name.clone(),
            }));
        }

        let migrations_to_roll_back = relevant_migrations
            .iter()
            .filter(|migration| migration.finished_at.is_none() && migration.rolled_back_at.is_none());

        for migration in migrations_to_roll_back {
            tracing::info!(
                migration_id = migration.id.as_str(),
                migration_name = migration.migration_name.as_str(),
                "Marking migration as rolled back."
            );
            persistence.mark_migration_rolled_back_by_id(&migration.id).await?;
        }

        Ok(Default::default())
    }
}
