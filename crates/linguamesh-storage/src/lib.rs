#![doc = "`LinguaMesh` 的 `SQLite` 迁移和最小配置存储。"]

use linguamesh_domain::{ErrorKind, ModelDescriptor, ModelSource, TranslationError};
use rusqlite::{Connection, OptionalExtension, params};
use std::path::Path;

const INITIAL_MIGRATION: &str = include_str!("../../../migrations/0001_initial.sql");

/// 管理明确迁移的本地数据库。
pub struct Storage {
    connection: Connection,
}

impl Storage {
    /// 打开数据库并应用所有缺失迁移。
    pub fn open(path: impl AsRef<Path>) -> Result<Self, TranslationError> {
        let connection = Connection::open(path).map_err(|error| map_error(&error))?;
        let mut storage = Self { connection };
        storage.migrate()?;
        Ok(storage)
    }

    /// 创建隔离的内存数据库。
    pub fn in_memory() -> Result<Self, TranslationError> {
        let connection = Connection::open_in_memory().map_err(|error| map_error(&error))?;
        let mut storage = Self { connection };
        storage.migrate()?;
        Ok(storage)
    }

    /// 返回当前数据库架构版本。
    pub fn schema_version(&self) -> Result<u32, TranslationError> {
        self.connection
            .query_row(
                "SELECT version FROM schema_metadata WHERE singleton = 1",
                [],
                |row| row.get(0),
            )
            .map_err(|error| map_error(&error))
    }

    /// 保存用户手动输入的模型。
    pub fn upsert_manual_model(&self, model_id: &str) -> Result<(), TranslationError> {
        self.connection
            .execute(
                "INSERT INTO model_descriptors (id, display_name, source) VALUES (?1, ?1, 'manual') ON CONFLICT(id) DO UPDATE SET display_name = excluded.display_name, source = 'manual'",
                params![model_id],
            )
            .map_err(|error| map_error(&error))?;
        Ok(())
    }

    /// 原子地选择下一次请求使用的模型。
    pub fn set_active_model(&self, model_id: &str) -> Result<(), TranslationError> {
        let exists = self
            .connection
            .query_row(
                "SELECT 1 FROM model_descriptors WHERE id = ?1",
                params![model_id],
                |_| Ok(()),
            )
            .optional()
            .map_err(|error| map_error(&error))?
            .is_some();
        if !exists {
            return Err(TranslationError::new(
                ErrorKind::ModelUnavailable,
                "The selected model is not registered.",
            ));
        }
        self.connection
            .execute(
                "INSERT INTO active_model_selection (singleton, model_id) VALUES (1, ?1) ON CONFLICT(singleton) DO UPDATE SET model_id = excluded.model_id",
                params![model_id],
            )
            .map_err(|error| map_error(&error))?;
        Ok(())
    }

    /// 返回当前明确选择的模型。
    pub fn active_model(&self) -> Result<Option<String>, TranslationError> {
        self.connection
            .query_row(
                "SELECT model_id FROM active_model_selection WHERE singleton = 1",
                [],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| map_error(&error))
    }

    /// 返回所有手动模型，供发现失败时继续使用。
    pub fn manual_models(&self) -> Result<Vec<ModelDescriptor>, TranslationError> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT id, display_name FROM model_descriptors WHERE source = 'manual' ORDER BY id",
            )
            .map_err(|error| map_error(&error))?;
        statement
            .query_map([], |row| {
                Ok(ModelDescriptor {
                    id: row.get(0)?,
                    display_name: row.get(1)?,
                    source: ModelSource::Manual,
                })
            })
            .map_err(|error| map_error(&error))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| map_error(&error))
    }

    fn migrate(&mut self) -> Result<(), TranslationError> {
        let transaction = self
            .connection
            .transaction()
            .map_err(|error| map_error(&error))?;
        transaction
            .execute_batch(INITIAL_MIGRATION)
            .map_err(|error| map_error(&error))?;
        transaction.commit().map_err(|error| map_error(&error))
    }
}

fn map_error(error: &rusqlite::Error) -> TranslationError {
    TranslationError::new(
        ErrorKind::Persistence,
        format!("Local database operation failed: {error}"),
    )
}

#[cfg(test)]
mod tests {
    use super::Storage;

    #[test]
    fn migration_and_manual_selection_are_persistent() {
        let storage = Storage::in_memory().expect("storage");
        assert_eq!(storage.schema_version().expect("version"), 1);
        storage.upsert_manual_model("manual-model").expect("insert");
        storage.set_active_model("manual-model").expect("select");
        assert_eq!(
            storage.active_model().expect("active").as_deref(),
            Some("manual-model")
        );
        assert_eq!(storage.manual_models().expect("models").len(), 1);
    }

    #[test]
    fn schema_has_no_credential_value_column() {
        let schema = super::INITIAL_MIGRATION.to_ascii_lowercase();
        assert!(!schema.contains("api_key"));
        assert!(!schema.contains("credential_value"));
        assert!(schema.contains("secret_ref"));
    }
}
