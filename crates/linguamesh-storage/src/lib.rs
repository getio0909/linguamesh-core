#![doc = "`LinguaMesh` 的 `SQLite` 迁移和最小配置存储。"]

use linguamesh_domain::{
    ErrorKind, ModelDescriptor, ModelSource, ProfileValidationError, ProviderProfile,
    ProviderProfileId, SecretRef, TranslationError, validate_model_identifier,
};
use rusqlite::{Connection, OptionalExtension, params};
use std::path::Path;

const INITIAL_MIGRATION: &str = include_str!("../../../migrations/0001_initial.sql");
const PROVIDER_PROFILE_STATE_MIGRATION: &str =
    include_str!("../../../migrations/0002_provider_profile_state.sql");
const LATEST_SCHEMA_VERSION: u32 = 2;
const MIGRATIONS: &[(u32, &str)] = &[
    (1, INITIAL_MIGRATION),
    (2, PROVIDER_PROFILE_STATE_MIGRATION),
];

/// 管理明确迁移的本地数据库。
pub struct Storage {
    connection: Connection,
}

impl Storage {
    /// 打开数据库并应用所有缺失迁移。
    pub fn open(path: impl AsRef<Path>) -> Result<Self, TranslationError> {
        let connection = Connection::open(path).map_err(|error| map_error(&error))?;
        if current_schema_version(&connection)? > LATEST_SCHEMA_VERSION {
            return Err(TranslationError::new(
                ErrorKind::Persistence,
                "Local database schema is newer than this Core version.",
            ));
        }
        configure_connection(&connection)?;
        let mut storage = Self { connection };
        storage.migrate()?;
        checkpoint_wal(&storage.connection)?;
        Ok(storage)
    }

    /// 创建隔离的内存数据库。
    pub fn in_memory() -> Result<Self, TranslationError> {
        let connection = Connection::open_in_memory().map_err(|error| map_error(&error))?;
        configure_connection(&connection)?;
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
        validate_model_identifier(model_id).map_err(|error| {
            TranslationError::new(
                ErrorKind::InvalidConfiguration,
                format!("Manual model ID is invalid: {error}"),
            )
        })?;
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

    /// 原子保存提供商配置及其最近模型。
    pub fn upsert_provider_profile(
        &mut self,
        profile: &ProviderProfile,
    ) -> Result<(), TranslationError> {
        let transaction = self
            .connection
            .transaction()
            .map_err(|error| map_error(&error))?;
        upsert_profile(&transaction, profile)?;
        if !profile.enabled() {
            transaction
                .execute(
                    "DELETE FROM active_provider_selection WHERE provider_id = ?1",
                    params![profile.id().as_str()],
                )
                .map_err(|error| map_error(&error))?;
        }
        transaction.commit().map_err(|error| map_error(&error))
    }

    /// 原子保存并激活一个已启用的提供商配置。
    pub fn save_and_activate_provider(
        &mut self,
        profile: &ProviderProfile,
    ) -> Result<(), TranslationError> {
        if !profile.enabled() {
            return Err(TranslationError::new(
                ErrorKind::InvalidConfiguration,
                "A disabled provider profile cannot be activated.",
            ));
        }
        let transaction = self
            .connection
            .transaction()
            .map_err(|error| map_error(&error))?;
        upsert_profile(&transaction, profile)?;
        transaction
            .execute(
                "INSERT INTO active_provider_selection (singleton, provider_id) VALUES (1, ?1) ON CONFLICT(singleton) DO UPDATE SET provider_id = excluded.provider_id",
                params![profile.id().as_str()],
            )
            .map_err(|error| map_error(&error))?;
        transaction.commit().map_err(|error| map_error(&error))
    }

    /// 激活一个已存在且启用的提供商配置。
    pub fn set_active_provider(
        &mut self,
        profile_id: &ProviderProfileId,
    ) -> Result<(), TranslationError> {
        let transaction = self
            .connection
            .transaction()
            .map_err(|error| map_error(&error))?;
        let enabled = transaction
            .query_row(
                "SELECT enabled FROM provider_profiles WHERE id = ?1",
                params![profile_id.as_str()],
                |row| row.get::<_, bool>(0),
            )
            .optional()
            .map_err(|error| map_error(&error))?;
        match enabled {
            Some(true) => {}
            Some(false) => {
                return Err(TranslationError::new(
                    ErrorKind::InvalidConfiguration,
                    "A disabled provider profile cannot be activated.",
                ));
            }
            None => {
                return Err(TranslationError::new(
                    ErrorKind::InvalidConfiguration,
                    "The provider profile does not exist.",
                ));
            }
        }
        transaction
            .execute(
                "INSERT INTO active_provider_selection (singleton, provider_id) VALUES (1, ?1) ON CONFLICT(singleton) DO UPDATE SET provider_id = excluded.provider_id",
                params![profile_id.as_str()],
            )
            .map_err(|error| map_error(&error))?;
        transaction.commit().map_err(|error| map_error(&error))
    }

    /// 返回指定提供商配置及其最近模型。
    pub fn provider_profile(
        &self,
        profile_id: &ProviderProfileId,
    ) -> Result<Option<ProviderProfile>, TranslationError> {
        self.connection
            .query_row(
                PROFILE_QUERY_BY_ID,
                params![profile_id.as_str()],
                stored_profile_from_row,
            )
            .optional()
            .map_err(|error| map_error(&error))?
            .map(StoredProfile::into_domain)
            .transpose()
    }

    /// 返回全部提供商配置及各自最近模型。
    pub fn provider_profiles(&self) -> Result<Vec<ProviderProfile>, TranslationError> {
        let mut statement = self
            .connection
            .prepare(PROFILE_QUERY_ALL)
            .map_err(|error| map_error(&error))?;
        statement
            .query_map([], stored_profile_from_row)
            .map_err(|error| map_error(&error))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| map_error(&error))?
            .into_iter()
            .map(StoredProfile::into_domain)
            .collect()
    }

    /// 返回当前激活的提供商配置。
    pub fn active_provider_profile(&self) -> Result<Option<ProviderProfile>, TranslationError> {
        self.connection
            .query_row(PROFILE_QUERY_ACTIVE, [], stored_profile_from_row)
            .optional()
            .map_err(|error| map_error(&error))?
            .map(StoredProfile::into_domain)
            .transpose()
    }

    /// 删除配置及其活动选择和最近模型，不接触宿主秘密值。
    pub fn delete_provider_profile(
        &mut self,
        profile_id: &ProviderProfileId,
    ) -> Result<bool, TranslationError> {
        let transaction = self
            .connection
            .transaction()
            .map_err(|error| map_error(&error))?;
        let changed = transaction
            .execute(
                "DELETE FROM provider_profiles WHERE id = ?1",
                params![profile_id.as_str()],
            )
            .map_err(|error| map_error(&error))?
            > 0;
        transaction.commit().map_err(|error| map_error(&error))?;
        Ok(changed)
    }

    fn migrate(&mut self) -> Result<(), TranslationError> {
        let transaction = self
            .connection
            .transaction()
            .map_err(|error| map_error(&error))?;
        let current = current_schema_version(&transaction)?;
        if current > LATEST_SCHEMA_VERSION {
            return Err(TranslationError::new(
                ErrorKind::Persistence,
                "Local database schema is newer than this Core version.",
            ));
        }
        for &(version, migration) in MIGRATIONS {
            if version <= current {
                continue;
            }
            transaction
                .execute_batch(migration)
                .map_err(|error| map_error(&error))?;
            transaction
                .execute(
                    "UPDATE schema_metadata SET version = ?1 WHERE singleton = 1",
                    params![version],
                )
                .map_err(|error| map_error(&error))?;
        }
        transaction.commit().map_err(|error| map_error(&error))?;
        Ok(())
    }
}

const PROFILE_QUERY_BY_ID: &str = "SELECT p.id, p.display_name, p.preset_id, p.adapter_type, p.base_endpoint, p.secret_ref, p.enabled, s.model_id FROM provider_profiles p LEFT JOIN provider_model_selection s ON s.provider_id = p.id WHERE p.id = ?1";
const PROFILE_QUERY_ALL: &str = "SELECT p.id, p.display_name, p.preset_id, p.adapter_type, p.base_endpoint, p.secret_ref, p.enabled, s.model_id FROM provider_profiles p LEFT JOIN provider_model_selection s ON s.provider_id = p.id ORDER BY p.display_name, p.id";
const PROFILE_QUERY_ACTIVE: &str = "SELECT p.id, p.display_name, p.preset_id, p.adapter_type, p.base_endpoint, p.secret_ref, p.enabled, s.model_id FROM active_provider_selection a JOIN provider_profiles p ON p.id = a.provider_id LEFT JOIN provider_model_selection s ON s.provider_id = p.id WHERE a.singleton = 1";

struct StoredProfile {
    id: String,
    display_name: String,
    preset_id: String,
    adapter_type: String,
    base_endpoint: String,
    secret_ref: Option<String>,
    enabled: bool,
    selected_model: Option<String>,
}

impl StoredProfile {
    fn into_domain(self) -> Result<ProviderProfile, TranslationError> {
        let id =
            ProviderProfileId::parse(self.id).map_err(|error| map_profile_validation(&error))?;
        let secret_ref = self
            .secret_ref
            .map(SecretRef::parse)
            .transpose()
            .map_err(|error| map_profile_validation(&error))?;
        ProviderProfile::new(
            id,
            self.display_name,
            self.preset_id,
            self.adapter_type,
            self.base_endpoint,
            secret_ref,
        )
        .map(|profile| profile.with_enabled(self.enabled))
        .and_then(|profile| profile.with_selected_model(self.selected_model))
        .map_err(|error| map_profile_validation(&error))
    }
}

fn stored_profile_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<StoredProfile> {
    Ok(StoredProfile {
        id: row.get(0)?,
        display_name: row.get(1)?,
        preset_id: row.get(2)?,
        adapter_type: row.get(3)?,
        base_endpoint: row.get(4)?,
        secret_ref: row.get(5)?,
        enabled: row.get(6)?,
        selected_model: row.get(7)?,
    })
}

fn upsert_profile(
    transaction: &rusqlite::Transaction<'_>,
    profile: &ProviderProfile,
) -> Result<(), TranslationError> {
    if profile
        .secret_ref()
        .is_some_and(|secret_ref| !secret_ref.is_persistent())
    {
        return Err(TranslationError::new(
            ErrorKind::InvalidConfiguration,
            "Session-only secret references cannot be persisted.",
        ));
    }
    transaction
        .execute(
            "INSERT INTO provider_profiles (id, display_name, base_endpoint, secret_ref, preset_id, adapter_type, enabled) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7) ON CONFLICT(id) DO UPDATE SET display_name = excluded.display_name, base_endpoint = excluded.base_endpoint, secret_ref = excluded.secret_ref, preset_id = excluded.preset_id, adapter_type = excluded.adapter_type, enabled = excluded.enabled",
            params![
                profile.id().as_str(),
                profile.display_name(),
                profile.base_endpoint(),
                profile.secret_ref().map(SecretRef::as_str),
                profile.preset_id(),
                profile.adapter_type(),
                profile.enabled(),
            ],
        )
        .map_err(|error| map_error(&error))?;
    match profile.selected_model() {
        Some(model_id) => {
            transaction
                .execute(
                    "INSERT INTO provider_model_selection (provider_id, model_id) VALUES (?1, ?2) ON CONFLICT(provider_id) DO UPDATE SET model_id = excluded.model_id",
                    params![profile.id().as_str(), model_id],
                )
                .map_err(|error| map_error(&error))?;
        }
        None => {
            transaction
                .execute(
                    "DELETE FROM provider_model_selection WHERE provider_id = ?1",
                    params![profile.id().as_str()],
                )
                .map_err(|error| map_error(&error))?;
        }
    }
    Ok(())
}

fn configure_connection(connection: &Connection) -> Result<(), TranslationError> {
    let journal_mode = connection
        .query_row("PRAGMA journal_mode = WAL", [], |row| {
            row.get::<_, String>(0)
        })
        .map_err(|error| map_error(&error))?;
    if journal_mode != "wal" && journal_mode != "memory" {
        return Err(TranslationError::new(
            ErrorKind::Persistence,
            "Local database could not enable the required journal mode.",
        ));
    }
    connection
        .execute_batch(
            "PRAGMA foreign_keys = ON; PRAGMA synchronous = NORMAL; PRAGMA secure_delete = ON;",
        )
        .map_err(|error| map_error(&error))
}

fn checkpoint_wal(connection: &Connection) -> Result<(), TranslationError> {
    let busy = connection
        .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| {
            row.get::<_, u32>(0)
        })
        .map_err(|error| map_error(&error))?;
    if busy == 0 {
        Ok(())
    } else {
        Err(TranslationError::new(
            ErrorKind::Persistence,
            "Local database WAL checkpoint is busy.",
        ))
    }
}

fn current_schema_version(connection: &Connection) -> Result<u32, TranslationError> {
    let has_metadata = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'schema_metadata')",
            [],
            |row| row.get::<_, bool>(0),
        )
        .map_err(|error| map_error(&error))?;
    if !has_metadata {
        return Ok(0);
    }
    connection
        .query_row(
            "SELECT version FROM schema_metadata WHERE singleton = 1",
            [],
            |row| row.get(0),
        )
        .map_err(|error| map_error(&error))
}

fn map_profile_validation(error: &ProfileValidationError) -> TranslationError {
    TranslationError::new(
        ErrorKind::Persistence,
        format!("Stored provider profile is invalid: {error}"),
    )
}

fn map_error(error: &rusqlite::Error) -> TranslationError {
    TranslationError::new(
        ErrorKind::Persistence,
        format!("Local database operation failed: {error}"),
    )
}

#[cfg(test)]
mod tests {
    use super::{INITIAL_MIGRATION, Storage};
    use linguamesh_domain::{ErrorKind, ProviderProfile, ProviderProfileId, SecretRef};
    use rusqlite::Connection;
    use std::fs;
    use tempfile::tempdir;

    const PERSISTENT_SECRET_REF: &str = "secret-service:88888888-8888-4888-8888-888888888888";
    const SESSION_SECRET_REF: &str = "session:99999999-9999-4999-8999-999999999999";
    const LEGACY_SECRET_CANARY: &str = concat!("s", "k", "-LM_LEGACY_DATABASE_SECRET_1234567890");

    fn profile(id: &str, secret_ref: Option<&str>, model: Option<&str>) -> ProviderProfile {
        ProviderProfile::new(
            ProviderProfileId::parse(id).expect("profile id"),
            format!("Provider {id}"),
            "local-loopback",
            "openai_chat_completions",
            "http://127.0.0.1:11434/v1/",
            secret_ref.map(|value| SecretRef::parse(value).expect("secret ref")),
        )
        .expect("profile")
        .with_selected_model(model.map(str::to_owned))
        .expect("selected model")
    }

    #[test]
    fn migration_and_manual_selection_are_persistent() {
        let storage = Storage::in_memory().expect("storage");
        assert_eq!(storage.schema_version().expect("version"), 2);
        storage.upsert_manual_model("manual-model").expect("insert");
        storage.set_active_model("manual-model").expect("select");
        assert_eq!(
            storage.active_model().expect("active").as_deref(),
            Some("manual-model")
        );
        assert_eq!(storage.manual_models().expect("models").len(), 1);
    }

    #[test]
    fn manual_model_identifier_cannot_persist_a_credential() {
        const SECRET_CANARY: &str = concat!("s", "k", "-LM_MODEL_SECRET_1234567890");
        let storage = Storage::in_memory().expect("storage");
        let error = storage
            .upsert_manual_model(SECRET_CANARY)
            .expect_err("credential-shaped model ID");
        assert_eq!(error.kind, ErrorKind::InvalidConfiguration);
        assert!(storage.manual_models().expect("models").is_empty());
    }

    #[test]
    fn migrated_schema_has_no_credential_columns_and_valid_foreign_keys() {
        let storage = Storage::in_memory().expect("storage");
        let mut statement = storage
            .connection
            .prepare(
                "SELECT m.name, p.name FROM sqlite_master AS m, pragma_table_info(m.name) AS p WHERE m.type = 'table' ORDER BY m.name, p.cid",
            )
            .expect("schema query");
        let columns = statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .expect("schema rows")
            .collect::<Result<Vec<_>, _>>()
            .expect("schema columns");
        assert!(
            columns
                .iter()
                .any(|(table, column)| table == "provider_profiles" && column == "secret_ref")
        );
        assert!(columns.iter().all(|(_, column)| {
            !matches!(
                column.as_str(),
                "api_key" | "credential_value" | "secret_value"
            )
        }));

        let foreign_keys_enabled = storage
            .connection
            .query_row("PRAGMA foreign_keys", [], |row| row.get::<_, bool>(0))
            .expect("foreign key mode");
        assert!(foreign_keys_enabled);
        let secure_delete = storage
            .connection
            .query_row("PRAGMA secure_delete", [], |row| row.get::<_, bool>(0))
            .expect("secure delete mode");
        assert!(secure_delete);
        let synchronous = storage
            .connection
            .query_row("PRAGMA synchronous", [], |row| row.get::<_, u32>(0))
            .expect("synchronous mode");
        assert_eq!(synchronous, 1);
        let mut foreign_key_check = storage
            .connection
            .prepare("PRAGMA foreign_key_check")
            .expect("foreign key check");
        assert!(
            foreign_key_check
                .query([])
                .expect("foreign key rows")
                .next()
                .expect("foreign key result")
                .is_none()
        );
    }

    #[test]
    fn schema_one_database_migrates_without_losing_provider_rows() {
        let directory = tempdir().expect("temp directory");
        let path = directory.path().join("migration.sqlite3");
        let connection = Connection::open(&path).expect("legacy database");
        connection
            .execute_batch(INITIAL_MIGRATION)
            .expect("initial migration");
        connection
            .execute(
                "INSERT INTO provider_profiles (id, display_name, base_endpoint, secret_ref) VALUES (?1, ?2, ?3, ?4)",
                (
                    "legacy-profile",
                    "Legacy provider",
                    "http://127.0.0.1:11434/v1/",
                    LEGACY_SECRET_CANARY,
                ),
            )
            .expect("legacy profile");
        drop(connection);

        let storage = Storage::open(&path).expect("migrated storage");
        assert_eq!(storage.schema_version().expect("version"), 2);
        let id = ProviderProfileId::parse("legacy-profile").expect("profile id");
        let loaded = storage
            .provider_profile(&id)
            .expect("profile query")
            .expect("legacy profile");
        assert_eq!(loaded.display_name(), "Legacy provider");
        assert_eq!(loaded.preset_id(), "generic-openai-compatible");
        assert_eq!(loaded.adapter_type(), "openai_chat_completions");
        assert_eq!(loaded.secret_ref().map(SecretRef::as_str), None);
        assert!(loaded.enabled());
        for entry in fs::read_dir(directory.path()).expect("database directory") {
            let path = entry.expect("database artifact").path();
            if path.is_file() {
                let bytes = fs::read(path).expect("database artifact bytes");
                assert!(
                    !bytes
                        .windows(LEGACY_SECRET_CANARY.len())
                        .any(|window| window == LEGACY_SECRET_CANARY.as_bytes())
                );
            }
        }
    }

    #[test]
    fn busy_migration_checkpoint_is_retried_on_next_open() {
        let directory = tempdir().expect("temp directory");
        let path = directory.path().join("busy-migration.sqlite3");
        let mut reader = Connection::open(&path).expect("legacy database");
        let journal_mode = reader
            .query_row("PRAGMA journal_mode = WAL", [], |row| {
                row.get::<_, String>(0)
            })
            .expect("journal mode");
        assert_eq!(journal_mode, "wal");
        reader
            .execute_batch(INITIAL_MIGRATION)
            .expect("initial migration");
        reader
            .execute(
                "INSERT INTO provider_profiles (id, display_name, base_endpoint, secret_ref) VALUES (?1, ?2, ?3, ?4)",
                (
                    "busy-legacy-profile",
                    "Busy legacy provider",
                    "http://127.0.0.1:11434/v1/",
                    LEGACY_SECRET_CANARY,
                ),
            )
            .expect("legacy profile");
        reader
            .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()))
            .expect("initial checkpoint");
        let transaction = reader.transaction().expect("reader transaction");
        let value = transaction
            .query_row(
                "SELECT secret_ref FROM provider_profiles WHERE id = 'busy-legacy-profile'",
                [],
                |row| row.get::<_, String>(0),
            )
            .expect("reader snapshot");
        assert_eq!(value, LEGACY_SECRET_CANARY);

        let Err(error) = Storage::open(&path) else {
            panic!("busy checkpoint was accepted");
        };
        assert_eq!(error.kind, ErrorKind::Persistence);
        assert_eq!(error.message, "Local database WAL checkpoint is busy.");
        drop(transaction);

        let mut saw_canary_before_retry = false;
        for entry in fs::read_dir(directory.path()).expect("database directory") {
            let path = entry.expect("database artifact").path();
            if path.is_file() {
                let bytes = fs::read(path).expect("database artifact bytes");
                saw_canary_before_retry |= bytes
                    .windows(LEGACY_SECRET_CANARY.len())
                    .any(|window| window == LEGACY_SECRET_CANARY.as_bytes());
            }
        }
        assert!(saw_canary_before_retry);

        let storage = Storage::open(&path).expect("checkpoint retry");
        assert_eq!(storage.schema_version().expect("version"), 2);
        for entry in fs::read_dir(directory.path()).expect("database directory") {
            let path = entry.expect("database artifact").path();
            if path.is_file() {
                let bytes = fs::read(path).expect("database artifact bytes");
                assert!(
                    !bytes
                        .windows(LEGACY_SECRET_CANARY.len())
                        .any(|window| window == LEGACY_SECRET_CANARY.as_bytes())
                );
            }
        }
        drop(reader);
    }

    #[test]
    fn future_schema_is_rejected_before_journal_mutation() {
        let directory = tempdir().expect("temp directory");
        let path = directory.path().join("future.sqlite3");
        let connection = Connection::open(&path).expect("future database");
        connection
            .execute_batch(INITIAL_MIGRATION)
            .expect("initial migration");
        connection
            .execute(
                "UPDATE schema_metadata SET version = 99 WHERE singleton = 1",
                [],
            )
            .expect("future version");
        drop(connection);

        let Err(error) = Storage::open(&path) else {
            panic!("future schema was accepted");
        };
        assert_eq!(error.kind, ErrorKind::Persistence);
        let connection = Connection::open(&path).expect("inspect future database");
        let journal_mode = connection
            .query_row("PRAGMA journal_mode", [], |row| row.get::<_, String>(0))
            .expect("journal mode");
        assert_eq!(journal_mode, "delete");
    }

    #[test]
    fn profile_selection_and_last_model_survive_reopen() {
        let directory = tempdir().expect("temp directory");
        let path = directory.path().join("profiles.sqlite3");
        let first = profile(
            "first-profile",
            Some(PERSISTENT_SECRET_REF),
            Some("first-model"),
        );
        let second = profile("second-profile", None, Some("second-model"));
        {
            let mut storage = Storage::open(&path).expect("storage");
            storage
                .save_and_activate_provider(&first)
                .expect("first profile");
            storage
                .save_and_activate_provider(&second)
                .expect("second profile");
            storage
                .set_active_provider(first.id())
                .expect("activate first");
        }

        let storage = Storage::open(&path).expect("reopened storage");
        assert_eq!(storage.provider_profiles().expect("profiles").len(), 2);
        let active = storage
            .active_provider_profile()
            .expect("active query")
            .expect("active profile");
        assert_eq!(active.id(), first.id());
        assert_eq!(active.selected_model(), Some("first-model"));
        assert_eq!(
            active.secret_ref().map(SecretRef::as_str),
            Some(PERSISTENT_SECRET_REF)
        );
    }

    #[test]
    fn session_secret_reference_cannot_be_persisted() {
        let mut storage = Storage::in_memory().expect("storage");
        let profile = profile("session-profile", Some(SESSION_SECRET_REF), Some("model"));
        let error = storage
            .upsert_provider_profile(&profile)
            .expect_err("session reference must be rejected");
        assert_eq!(error.kind, ErrorKind::InvalidConfiguration);
        assert!(
            storage
                .provider_profile(profile.id())
                .expect("profile query")
                .is_none()
        );
    }

    #[test]
    fn deleting_profile_cascades_active_selection() {
        let mut storage = Storage::in_memory().expect("storage");
        let profile = profile("delete-profile", None, Some("model"));
        storage
            .save_and_activate_provider(&profile)
            .expect("save profile");
        assert!(
            storage
                .delete_provider_profile(profile.id())
                .expect("delete")
        );
        assert!(
            storage
                .active_provider_profile()
                .expect("active query")
                .is_none()
        );
    }

    #[test]
    fn disabling_active_profile_clears_active_selection() {
        let mut storage = Storage::in_memory().expect("storage");
        let profile = profile("disabled-profile", None, Some("model"));
        storage
            .save_and_activate_provider(&profile)
            .expect("save profile");
        storage
            .upsert_provider_profile(&profile.clone().with_enabled(false))
            .expect("disable profile");
        assert!(
            storage
                .active_provider_profile()
                .expect("active query")
                .is_none()
        );
    }
}
