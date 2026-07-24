#![doc = "`LinguaMesh` 的 `SQLite` 迁移和最小配置存储。"]

use linguamesh_document::{
    DocumentFormat, DocumentJob, DocumentJobState, DocumentSegment, DocumentSegmentKind,
    MAX_DOCUMENT_BYTES,
};
use linguamesh_domain::{
    ErrorKind, Glossary, GlossaryEntry, MAX_ROUTING_PROFILE_JSON_BYTES, ModelDescriptor,
    ModelSource, ProfileValidationError, ProviderProfile, ProviderProfileId, RoutingProfile,
    SecretRef, TranslationError, TranslationPreset, TranslationQualityMode, TranslationRequest,
    UsageRecord, UsageSource, deserialize_routing_profile, serialize_routing_profile,
    validate_model_identifier,
};
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use std::path::{Component, Path};

const INITIAL_MIGRATION: &str = include_str!("../../../migrations/0001_initial.sql");
const PROVIDER_PROFILE_STATE_MIGRATION: &str =
    include_str!("../../../migrations/0002_provider_profile_state.sql");
const TRANSLATION_HISTORY_MIGRATION: &str =
    include_str!("../../../migrations/0003_translation_history.sql");
const TRANSLATION_HISTORY_POLICY_MIGRATION: &str =
    include_str!("../../../migrations/0004_translation_history_policy.sql");
const TRANSLATION_MEMORY_MIGRATION: &str =
    include_str!("../../../migrations/0005_translation_memory.sql");
const DOCUMENT_JOBS_MIGRATION: &str = include_str!("../../../migrations/0006_document_jobs.sql");
const DOCUMENT_JOB_PAUSE_MIGRATION: &str =
    include_str!("../../../migrations/0007_document_job_pause.sql");
const DOCUMENT_JOB_OPTIONS_MIGRATION: &str =
    include_str!("../../../migrations/0008_document_job_options.sql");
const DOCUMENT_FORMATS_MIGRATION: &str =
    include_str!("../../../migrations/0009_document_formats.sql");
const DOCUMENT_PACKAGES_MIGRATION: &str =
    include_str!("../../../migrations/0010_document_packages.sql");
const DOCUMENT_PPTX_MIGRATION: &str = include_str!("../../../migrations/0011_document_pptx.sql");
const DOCUMENT_XLSX_MIGRATION: &str = include_str!("../../../migrations/0012_document_xlsx.sql");
const DOCUMENT_EPUB_MIGRATION: &str = include_str!("../../../migrations/0013_document_epub.sql");
const DOCUMENT_PDF_MIGRATION: &str = include_str!("../../../migrations/0014_document_pdf.sql");
const ROUTING_PROFILES_MIGRATION: &str =
    include_str!("../../../migrations/0015_routing_profiles.sql");
const DOCUMENT_ROUTING_PROFILE_MIGRATION: &str =
    include_str!("../../../migrations/0016_document_routing_profile.sql");
const DOCUMENT_QUALITY_MODE_MIGRATION: &str =
    include_str!("../../../migrations/0017_document_quality_mode.sql");
const DOCUMENT_TRANSLATION_PRESET_MIGRATION: &str =
    include_str!("../../../migrations/0018_document_translation_preset.sql");
const PROVIDER_PROFILE_NOTES_MIGRATION: &str =
    include_str!("../../../migrations/0019_provider_profile_notes.sql");
const PROVIDER_PROFILE_ORGANIZATION_MIGRATION: &str =
    include_str!("../../../migrations/0020_provider_profile_organization.sql");
const PROVIDER_PROFILE_PROJECT_MIGRATION: &str =
    include_str!("../../../migrations/0021_provider_profile_project.sql");
const PROVIDER_PROFILE_REGION_ACCOUNT_MIGRATION: &str =
    include_str!("../../../migrations/0022_provider_profile_region_account.sql");
const PROVIDER_PROFILE_CUSTOM_HEADERS_MIGRATION: &str =
    include_str!("../../../migrations/0023_provider_profile_custom_headers.sql");
const PROVIDER_PROFILE_SECRET_CUSTOM_HEADERS_MIGRATION: &str =
    include_str!("../../../migrations/0024_provider_profile_secret_custom_headers.sql");
const PROVIDER_PROFILE_PROXY_MIGRATION: &str =
    include_str!("../../../migrations/0025_provider_profile_proxy.sql");
const PROVIDER_PROFILE_TIMEOUT_MIGRATION: &str =
    include_str!("../../../migrations/0026_provider_profile_timeout.sql");
const PROVIDER_PROFILE_CONNECTION_TIMEOUT_MIGRATION: &str =
    include_str!("../../../migrations/0027_provider_profile_connection_timeout.sql");
const PROVIDER_PROFILE_STREAMING_IDLE_TIMEOUT_MIGRATION: &str =
    include_str!("../../../migrations/0028_provider_profile_streaming_idle_timeout.sql");
const PROVIDER_PROFILE_TRUSTED_CERTIFICATES_MIGRATION: &str =
    include_str!("../../../migrations/0029_provider_profile_trusted_certificates.sql");
const PROVIDER_PROFILE_PROXY_AUTH_MIGRATION: &str =
    include_str!("../../../migrations/0030_provider_profile_proxy_auth.sql");
const PROVIDER_PROFILE_CLIENT_CERTIFICATE_IDENTITY_MIGRATION: &str =
    include_str!("../../../migrations/0031_provider_profile_client_certificate_identity.sql");
const USAGE_RECORDS_MIGRATION: &str = include_str!("../../../migrations/0032_usage_records.sql");
const GLOSSARY_LIBRARIES_MIGRATION: &str =
    include_str!("../../../migrations/0033_glossary_libraries.sql");
const PROVIDER_PROFILE_HEALTH_MIGRATION: &str =
    include_str!("../../../migrations/0034_provider_profile_health.sql");
const LATEST_SCHEMA_VERSION: u32 = 34;
/// 限制本地历史记录的数量，避免数据库无限增长。
pub const MAX_TRANSLATION_HISTORY_ENTRIES: usize = 100;
/// 限制单条历史记录中源文本和译文的大小。
pub const MAX_TRANSLATION_HISTORY_TEXT_BYTES: usize = 4 * 1024 * 1024;
/// 限制本地翻译记忆条目的数量，避免数据库无限增长。
pub const MAX_TRANSLATION_MEMORY_ENTRIES: usize = 100;
/// 限制单条翻译记忆源文本和译文的大小。
pub const MAX_TRANSLATION_MEMORY_TEXT_BYTES: usize = 4 * 1024 * 1024;
/// 翻译记忆身份中的保护策略版本。
pub const TRANSLATION_MEMORY_PROTECTED_SPAN_POLICY: &str = "protected-spans-v1";
/// 翻译记忆身份中的提示模板版本。
pub const TRANSLATION_MEMORY_PROMPT_TEMPLATE_VERSION: &str = "translation-prompt-v3";
/// 限制本地可恢复文档任务的数量，避免任务队列无限增长。
pub const MAX_DOCUMENT_JOBS: usize = 100;
/// 限制单个文档任务的段数量，避免恶意输入制造过大元数据。
pub const MAX_DOCUMENT_SEGMENTS: usize = 10_000;
/// 限制文档任务快照中源文本和译文的总大小。
pub const MAX_DOCUMENT_JOB_TEXT_BYTES: usize = MAX_DOCUMENT_BYTES;
/// 限制可恢复文档参数中的单个文本字段大小。
pub const MAX_DOCUMENT_JOB_OPTION_TEXT_BYTES: usize = 256;
/// 限制可恢复文档参数中的词汇表 JSON 大小。
pub const MAX_DOCUMENT_JOB_GLOSSARY_BYTES: usize = 256 * 1024;
/// 限制可恢复文档参数中的翻译预设 JSON 大小。
pub const MAX_DOCUMENT_JOB_PRESET_BYTES: usize = 8 * 1024;
/// 限制可保存的路由配置数量，避免配置数据库无限增长。
pub const MAX_ROUTING_PROFILES: usize = 32;
/// 限制本地可保存的词汇表库数量，避免配置数据库无限增长。
pub const MAX_GLOSSARIES: usize = 32;
/// 限制单个路由配置 JSON 大小，确保候选和约束元数据有界。
const MIGRATIONS: &[(u32, &str)] = &[
    (1, INITIAL_MIGRATION),
    (2, PROVIDER_PROFILE_STATE_MIGRATION),
    (3, TRANSLATION_HISTORY_MIGRATION),
    (4, TRANSLATION_HISTORY_POLICY_MIGRATION),
    (5, TRANSLATION_MEMORY_MIGRATION),
    (6, DOCUMENT_JOBS_MIGRATION),
    (7, DOCUMENT_JOB_PAUSE_MIGRATION),
    (8, DOCUMENT_JOB_OPTIONS_MIGRATION),
    (9, DOCUMENT_FORMATS_MIGRATION),
    (10, DOCUMENT_PACKAGES_MIGRATION),
    (11, DOCUMENT_PPTX_MIGRATION),
    (12, DOCUMENT_XLSX_MIGRATION),
    (13, DOCUMENT_EPUB_MIGRATION),
    (14, DOCUMENT_PDF_MIGRATION),
    (15, ROUTING_PROFILES_MIGRATION),
    (16, DOCUMENT_ROUTING_PROFILE_MIGRATION),
    (17, DOCUMENT_QUALITY_MODE_MIGRATION),
    (18, DOCUMENT_TRANSLATION_PRESET_MIGRATION),
    (19, PROVIDER_PROFILE_NOTES_MIGRATION),
    (20, PROVIDER_PROFILE_ORGANIZATION_MIGRATION),
    (21, PROVIDER_PROFILE_PROJECT_MIGRATION),
    (22, PROVIDER_PROFILE_REGION_ACCOUNT_MIGRATION),
    (23, PROVIDER_PROFILE_CUSTOM_HEADERS_MIGRATION),
    (24, PROVIDER_PROFILE_SECRET_CUSTOM_HEADERS_MIGRATION),
    (25, PROVIDER_PROFILE_PROXY_MIGRATION),
    (26, PROVIDER_PROFILE_TIMEOUT_MIGRATION),
    (27, PROVIDER_PROFILE_CONNECTION_TIMEOUT_MIGRATION),
    (28, PROVIDER_PROFILE_STREAMING_IDLE_TIMEOUT_MIGRATION),
    (29, PROVIDER_PROFILE_TRUSTED_CERTIFICATES_MIGRATION),
    (30, PROVIDER_PROFILE_PROXY_AUTH_MIGRATION),
    (31, PROVIDER_PROFILE_CLIENT_CERTIFICATE_IDENTITY_MIGRATION),
    (32, USAGE_RECORDS_MIGRATION),
    (33, GLOSSARY_LIBRARIES_MIGRATION),
    (34, PROVIDER_PROFILE_HEALTH_MIGRATION),
];

/// 描述一条已完成且允许持久化的文本翻译历史。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TranslationHistoryEntry {
    /// 一次翻译操作的稳定标识。
    pub operation_id: String,
    /// 记录写入时的 Unix 秒时间戳。
    pub created_at: i64,
    /// 原始源文本。
    pub source_text: String,
    /// 已完成的译文。
    pub translated_text: String,
    /// 可选的源语言标签。
    pub source_locale: Option<String>,
    /// 目标语言标签。
    pub target_locale: String,
    /// 使用的模型标识。
    pub model_id: String,
}

/// 描述一条不含正文和秘密的归一化 usage 记录。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UsageRecordEntry {
    /// 对应翻译操作的稳定标识。
    pub operation_id: String,
    /// 记录写入时的 Unix 秒时间戳。
    pub created_at: i64,
    /// 可选的稳定提供商标识，不包含端点或凭据。
    pub provider_id: Option<String>,
    /// 使用的模型标识。
    pub model_id: String,
    /// 归一化 usage 数据。
    pub usage: UsageRecord,
}

/// 描述一条可复用的本地翻译记忆。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TranslationMemoryEntry {
    /// 由全部相关请求输入构成的稳定缓存键。
    pub cache_key: String,
    /// 记录写入时的 Unix 秒时间戳。
    pub created_at: i64,
    /// 原始源文本。
    pub source_text: String,
    /// 已完成的译文。
    pub translated_text: String,
    /// 可选的源语言标签。
    pub source_locale: Option<String>,
    /// 目标语言标签。
    pub target_locale: String,
    /// 使用的模型标识。
    pub model_id: String,
    /// 可审计的身份字段 JSON，不包含秘密值。
    pub identity_json: String,
}

/// 描述一个可在进程重启后恢复的文档任务。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DocumentJobSnapshot {
    /// 由宿主生成且不包含本地路径的任务标识。
    pub job_id: String,
    /// 当前任务生命周期状态。
    pub state: DocumentJobState,
    /// 文档内容和分段快照。
    pub job: DocumentJob,
    /// 可在重启后复用的非秘密翻译参数。
    pub options: Option<DocumentJobOptions>,
    /// 任务首次写入时的 Unix 秒时间戳。
    pub created_at: i64,
    /// 任务最近一次状态或段更新时的 Unix 秒时间戳。
    pub updated_at: i64,
}

/// 描述文档任务需要复用的非秘密翻译参数。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DocumentJobOptions {
    /// 可选的源语言标签。
    pub source_locale: Option<String>,
    /// 目标语言标签。
    pub target_locale: String,
    /// 明确选择的模型标识。
    pub model_id: String,
    /// 明确选择的提供商配置标识，不包含端点或秘密。
    pub provider_id: String,
    /// 可选的已保存路由配置标识，用于重启后重新选择同一候选。
    pub routing_profile_id: Option<String>,
    /// 文档每个可翻译段使用的质量与调用策略。
    pub quality_mode: TranslationQualityMode,
    /// 文档每个可翻译段使用的有界语言风格预设。
    pub translation_preset: TranslationPreset,
    /// 请求级词汇表；只保存经过校验的结构化规则。
    pub glossary: Option<Glossary>,
}

/// 描述一条已持久化的非秘密路由配置。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RoutingProfileRecord {
    /// 配置稳定标识。
    pub id: String,
    /// 路由配置内容，不含端点、凭据或源文。
    pub profile: RoutingProfile,
    /// 配置首次写入时的 Unix 秒时间戳。
    pub created_at: i64,
    /// 配置最近一次更新时的 Unix 秒时间戳。
    pub updated_at: i64,
}

/// 描述一个可跨请求复用的本地词汇表库。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GlossaryRecord {
    /// 词汇表库稳定标识。
    pub id: String,
    /// 经过核心校验的术语规则。
    pub glossary: Glossary,
    /// 词汇表库首次写入时的 Unix 秒时间戳。
    pub created_at: i64,
    /// 词汇表库最近一次更新时的 Unix 秒时间戳。
    pub updated_at: i64,
}

/// 管理明确迁移的本地数据库。
pub struct Storage {
    connection: Connection,
}

impl Storage {
    /// 打开数据库并应用所有缺失迁移。
    pub fn open(path: impl AsRef<Path>) -> Result<Self, TranslationError> {
        Self::open_with_flags(path, OpenFlags::default() | OpenFlags::SQLITE_OPEN_NOFOLLOW)
    }

    /// 使用宿主已经固定的 Linux 文件描述符路径打开数据库。
    pub fn open_from_trusted_descriptor(path: impl AsRef<Path>) -> Result<Self, TranslationError> {
        let path = path.as_ref();
        if !is_proc_self_fd_path(path) {
            return Err(TranslationError::new(
                ErrorKind::InvalidConfiguration,
                "Trusted database paths must refer to /proc/self/fd/<fd>.",
            ));
        }
        Self::open_with_flags(path, OpenFlags::default())
    }

    fn open_with_flags(path: impl AsRef<Path>, flags: OpenFlags) -> Result<Self, TranslationError> {
        let connection =
            Connection::open_with_flags(path, flags).map_err(|error| map_error(&error))?;
        Self::finish_open(connection)
    }

    #[cfg(target_os = "linux")]
    #[cfg(test)]
    fn open_with_vfs(path: impl AsRef<Path>, vfs: &str) -> Result<Self, TranslationError> {
        let connection = Connection::open_with_flags_and_vfs(
            path,
            OpenFlags::default() | OpenFlags::SQLITE_OPEN_NOFOLLOW,
            vfs,
        )
        .map_err(|error| map_error(&error))?;
        Self::finish_open(connection)
    }

    fn finish_open(connection: Connection) -> Result<Self, TranslationError> {
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

    /// 保存或替换一个文档任务及其全部段快照。
    pub fn save_document_job(
        &mut self,
        job_id: &str,
        job: &DocumentJob,
        state: DocumentJobState,
    ) -> Result<DocumentJobSnapshot, TranslationError> {
        validate_document_job_identity(job_id, job)?;
        if state == DocumentJobState::Completed && job.pending_count() != 0 {
            return Err(document_configuration_error(
                "A completed document job still has untranslated segments.",
            ));
        }
        let transaction = self
            .connection
            .transaction()
            .map_err(|error| map_error(&error))?;
        let exists = transaction
            .query_row(
                "SELECT 1 FROM document_jobs WHERE job_id = ?1",
                params![job_id],
                |_| Ok(()),
            )
            .optional()
            .map_err(|error| map_error(&error))?
            .is_some();
        if !exists {
            let count = transaction
                .query_row("SELECT COUNT(*) FROM document_jobs", [], |row| {
                    row.get::<_, i64>(0)
                })
                .map_err(|error| map_error(&error))?;
            if usize::try_from(count).unwrap_or(usize::MAX) >= MAX_DOCUMENT_JOBS {
                return Err(document_configuration_error(
                    "The local document job limit has been reached.",
                ));
            }
        }
        transaction
                .execute(
                "INSERT INTO document_jobs (job_id, state, format, source_name, package_blob) VALUES (?1, ?2, ?3, ?4, ?5) ON CONFLICT(job_id) DO UPDATE SET state = excluded.state, format = excluded.format, source_name = excluded.source_name, package_blob = excluded.package_blob, updated_at = unixepoch()",
                params![job_id, document_job_state_name(state), document_format_name(job.format), job.source_name.as_str(), job.package.as_deref()],
            )
            .map_err(|error| map_error(&error))?;
        transaction
            .execute(
                "DELETE FROM document_segments WHERE job_id = ?1",
                params![job_id],
            )
            .map_err(|error| map_error(&error))?;
        for segment in &job.segments {
            transaction
                .execute(
                    "INSERT INTO document_segments (job_id, segment_index, kind, source_text, translated_text, line_ending) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![
                        job_id,
                        i64::try_from(segment.index).unwrap_or(i64::MAX),
                        document_segment_kind_name(segment.kind),
                        segment.source_text.as_str(),
                        segment.translated_text.as_deref(),
                        segment.line_ending.as_str(),
                    ],
                )
                .map_err(|error| map_error(&error))?;
        }
        transaction.commit().map_err(|error| map_error(&error))?;
        self.document_job(job_id)?.ok_or_else(|| {
            TranslationError::new(
                ErrorKind::Persistence,
                "The saved document job could not be reloaded.",
            )
        })
    }

    /// 按稳定标识读取一个文档任务快照。
    pub fn document_job(
        &self,
        job_id: &str,
    ) -> Result<Option<DocumentJobSnapshot>, TranslationError> {
        load_document_job(&self.connection, job_id)
    }

    /// 保存一个文档任务可复用的非秘密翻译参数。
    pub fn save_document_job_options(
        &mut self,
        job_id: &str,
        options: &DocumentJobOptions,
    ) -> Result<DocumentJobSnapshot, TranslationError> {
        validate_document_job_options(options)?;
        let glossary_json = options
            .glossary
            .as_ref()
            .map(serde_json::to_string)
            .transpose()
            .map_err(|_| {
                TranslationError::new(
                    ErrorKind::InvalidConfiguration,
                    "The document glossary could not be serialized.",
                )
            })?;
        let changed = self
            .connection
            .execute(
                "INSERT INTO document_job_options (job_id, source_locale, target_locale, model_id, provider_id, routing_profile_id, quality_mode, translation_preset_json, glossary_json) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9) ON CONFLICT(job_id) DO UPDATE SET source_locale = excluded.source_locale, target_locale = excluded.target_locale, model_id = excluded.model_id, provider_id = excluded.provider_id, routing_profile_id = excluded.routing_profile_id, quality_mode = excluded.quality_mode, translation_preset_json = excluded.translation_preset_json, glossary_json = excluded.glossary_json, updated_at = unixepoch()",
                params![
                    job_id,
                    options.source_locale.as_deref(),
                    options.target_locale.as_str(),
                    options.model_id.as_str(),
                    options.provider_id.as_str(),
                    options.routing_profile_id.as_deref(),
                    options.quality_mode.as_str(),
                    serde_json::to_string(&options.translation_preset).map_err(|_| {
                        TranslationError::new(
                            ErrorKind::InvalidConfiguration,
                            "The document translation preset could not be serialized.",
                        )
                    })?,
                    glossary_json.as_deref(),
                ],
            )
            .map_err(|error| map_error(&error))?;
        if changed == 0 {
            return Err(TranslationError::new(
                ErrorKind::InvalidConfiguration,
                "The document job options could not be saved.",
            ));
        }
        self.document_job(job_id)?.ok_or_else(|| {
            TranslationError::new(
                ErrorKind::Persistence,
                "The document job options could not be reloaded.",
            )
        })
    }

    /// 返回按最近更新时间排序的本地文档任务。
    pub fn document_jobs(
        &self,
        limit: usize,
    ) -> Result<Vec<DocumentJobSnapshot>, TranslationError> {
        let limit = limit.min(MAX_DOCUMENT_JOBS);
        let mut statement = self
            .connection
            .prepare(
                "SELECT job_id FROM document_jobs ORDER BY updated_at DESC, job_id DESC LIMIT ?1",
            )
            .map_err(|error| map_error(&error))?;
        let job_ids = statement
            .query_map(params![i64::try_from(limit).unwrap_or(i64::MAX)], |row| {
                row.get::<_, String>(0)
            })
            .map_err(|error| map_error(&error))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| map_error(&error))?;
        job_ids
            .into_iter()
            .map(|job_id| {
                self.document_job(&job_id)?.ok_or_else(|| {
                    TranslationError::new(
                        ErrorKind::Persistence,
                        "A document job disappeared while it was being listed.",
                    )
                })
            })
            .collect()
    }

    /// 返回进程重启后需要继续处理的文档任务。
    pub fn resumable_document_jobs(&self) -> Result<Vec<DocumentJobSnapshot>, TranslationError> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT job_id FROM document_jobs WHERE state IN ('pending', 'running', 'paused') ORDER BY updated_at ASC, job_id ASC",
            )
            .map_err(|error| map_error(&error))?;
        let job_ids = statement
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|error| map_error(&error))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| map_error(&error))?;
        job_ids
            .into_iter()
            .map(|job_id| {
                self.document_job(&job_id)?.ok_or_else(|| {
                    TranslationError::new(
                        ErrorKind::Persistence,
                        "A resumable document job disappeared while it was being listed.",
                    )
                })
            })
            .collect()
    }

    /// 更新一个可翻译段，并自动推进任务状态。
    pub fn update_document_segment(
        &mut self,
        job_id: &str,
        index: usize,
        translated_text: &str,
    ) -> Result<DocumentJobSnapshot, TranslationError> {
        if translated_text.len() > MAX_DOCUMENT_JOB_TEXT_BYTES {
            return Err(document_configuration_error(
                "The translated document segment exceeds the local size limit.",
            ));
        }
        let snapshot = self.document_job(job_id)?.ok_or_else(|| {
            TranslationError::new(
                ErrorKind::InvalidConfiguration,
                "The document job was not found.",
            )
        })?;
        let mut job = snapshot.job;
        job.apply_translation(index, translated_text.to_owned())
            .map_err(map_document_error)?;
        let state = if job.pending_count() == 0 {
            DocumentJobState::Completed
        } else {
            DocumentJobState::Running
        };
        self.save_document_job(job_id, &job, state)
    }

    /// 更新一个文档任务的生命周期状态，不修改段内容。
    pub fn set_document_job_state(
        &mut self,
        job_id: &str,
        state: DocumentJobState,
    ) -> Result<DocumentJobSnapshot, TranslationError> {
        let changed = self
            .connection
            .execute(
                "UPDATE document_jobs SET state = ?2, updated_at = unixepoch() WHERE job_id = ?1",
                params![job_id, document_job_state_name(state)],
            )
            .map_err(|error| map_error(&error))?;
        if changed == 0 {
            return Err(TranslationError::new(
                ErrorKind::InvalidConfiguration,
                "The document job was not found.",
            ));
        }
        self.document_job(job_id)?.ok_or_else(|| {
            TranslationError::new(
                ErrorKind::Persistence,
                "The document job state could not be reloaded.",
            )
        })
    }

    /// 删除一个文档任务及其段快照。
    pub fn delete_document_job(&mut self, job_id: &str) -> Result<bool, TranslationError> {
        self.connection
            .execute(
                "DELETE FROM document_jobs WHERE job_id = ?1",
                params![job_id],
            )
            .map(|count| count != 0)
            .map_err(|error| map_error(&error))
    }

    /// 清空全部本地文档任务。
    pub fn clear_document_jobs(&mut self) -> Result<(), TranslationError> {
        self.connection
            .execute("DELETE FROM document_jobs", [])
            .map(|_| ())
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

    /// 返回本地翻译历史记录数。
    pub fn translation_history_count(&self) -> Result<usize, TranslationError> {
        self.connection
            .query_row("SELECT COUNT(*) FROM translation_history", [], |row| {
                row.get::<_, i64>(0)
            })
            .map(|count| usize::try_from(count).unwrap_or(usize::MAX))
            .map_err(|error| map_error(&error))
    }

    /// 返回本地归一化 usage 记录数。
    pub fn usage_record_count(&self) -> Result<usize, TranslationError> {
        self.connection
            .query_row("SELECT COUNT(*) FROM usage_records", [], |row| {
                row.get::<_, i64>(0)
            })
            .map(|count| usize::try_from(count).unwrap_or(usize::MAX))
            .map_err(|error| map_error(&error))
    }

    /// 按最近写入顺序返回有限数量的 usage 记录。
    pub fn usage_records(&self, limit: usize) -> Result<Vec<UsageRecordEntry>, TranslationError> {
        let limit = limit.min(MAX_TRANSLATION_HISTORY_ENTRIES);
        let mut statement = self
            .connection
            .prepare(
                "SELECT operation_id, created_at, provider_id, model_id, source, input_tokens, output_tokens, total_tokens FROM usage_records ORDER BY created_at DESC, operation_id DESC LIMIT ?1",
            )
            .map_err(|error| map_error(&error))?;
        statement
            .query_map(
                params![i64::try_from(limit).unwrap_or(i64::MAX)],
                usage_record_from_row,
            )
            .map_err(|error| map_error(&error))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| map_error(&error))
    }

    /// 返回是否允许新的标准请求写入本地翻译历史。
    pub fn translation_history_enabled(&self) -> Result<bool, TranslationError> {
        self.connection
            .query_row(
                "SELECT enabled FROM translation_history_policy WHERE singleton = 1",
                [],
                |row| row.get::<_, bool>(0),
            )
            .map_err(|error| map_error(&error))
    }

    /// 持久化新的本地翻译历史写入策略，不删除既有记录。
    pub fn set_translation_history_enabled(
        &mut self,
        enabled: bool,
    ) -> Result<(), TranslationError> {
        self.connection
            .execute(
                "INSERT INTO translation_history_policy (singleton, enabled) VALUES (1, ?1) ON CONFLICT(singleton) DO UPDATE SET enabled = excluded.enabled",
                params![enabled],
            )
            .map(|_| ())
            .map_err(|error| map_error(&error))
    }

    /// 返回是否允许新的标准请求读写本地翻译记忆。
    pub fn translation_memory_enabled(&self) -> Result<bool, TranslationError> {
        self.connection
            .query_row(
                "SELECT enabled FROM translation_memory_policy WHERE singleton = 1",
                [],
                |row| row.get::<_, bool>(0),
            )
            .map_err(|error| map_error(&error))
    }

    /// 持久化本地翻译记忆策略，不删除既有条目。
    pub fn set_translation_memory_enabled(
        &mut self,
        enabled: bool,
    ) -> Result<(), TranslationError> {
        self.connection
            .execute(
                "INSERT INTO translation_memory_policy (singleton, enabled) VALUES (1, ?1) ON CONFLICT(singleton) DO UPDATE SET enabled = excluded.enabled",
                params![enabled],
            )
            .map(|_| ())
            .map_err(|error| map_error(&error))
    }

    /// 返回本地翻译记忆条目数。
    pub fn translation_memory_count(&self) -> Result<usize, TranslationError> {
        self.connection
            .query_row("SELECT COUNT(*) FROM translation_memory", [], |row| {
                row.get::<_, i64>(0)
            })
            .map(|count| usize::try_from(count).unwrap_or(usize::MAX))
            .map_err(|error| map_error(&error))
    }

    /// 按请求输入查找可复用的本地翻译记忆。
    pub fn lookup_translation_memory(
        &self,
        request: &TranslationRequest,
    ) -> Result<Option<TranslationMemoryEntry>, TranslationError> {
        if request.is_incognito() || !self.translation_memory_enabled()? {
            return Ok(None);
        }
        let key = translation_memory_key(request)?;
        self.connection
            .query_row(
                "SELECT cache_key, created_at, source_text, translated_text, source_locale, target_locale, model_id, identity_json FROM translation_memory WHERE cache_key = ?1",
                params![key],
                translation_memory_from_row,
            )
            .optional()
            .map_err(|error| map_error(&error))
    }

    /// 写入一条非隐身翻译记忆并裁剪最旧条目。
    pub fn record_translation_memory(
        &mut self,
        request: &TranslationRequest,
        translated_text: &str,
    ) -> Result<(), TranslationError> {
        if request.is_incognito() || !self.translation_memory_enabled()? {
            return Ok(());
        }
        if request.source_text.len() > MAX_TRANSLATION_MEMORY_TEXT_BYTES
            || translated_text.len() > MAX_TRANSLATION_MEMORY_TEXT_BYTES
        {
            return Err(TranslationError::new(
                ErrorKind::InvalidConfiguration,
                "Translation memory entry exceeds the local size limit.",
            ));
        }
        let key = translation_memory_key(request)?;
        let identity_json = translation_memory_identity_json(request)?;
        let transaction = self
            .connection
            .transaction()
            .map_err(|error| map_error(&error))?;
        transaction
            .execute(
                "INSERT INTO translation_memory (cache_key, source_text, translated_text, source_locale, target_locale, model_id, identity_json) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7) ON CONFLICT(cache_key) DO UPDATE SET source_text = excluded.source_text, translated_text = excluded.translated_text, source_locale = excluded.source_locale, target_locale = excluded.target_locale, model_id = excluded.model_id, identity_json = excluded.identity_json, created_at = unixepoch()",
                params![
                    key,
                    request.source_text.as_str(),
                    translated_text,
                    request.source_locale.as_deref(),
                    request.target_locale.as_str(),
                    request.model_id.as_str(),
                    identity_json,
                ],
            )
            .map_err(|error| map_error(&error))?;
        transaction
            .execute(
                "DELETE FROM translation_memory WHERE cache_key IN (SELECT cache_key FROM translation_memory ORDER BY created_at DESC, cache_key DESC LIMIT -1 OFFSET ?1)",
                params![i64::try_from(MAX_TRANSLATION_MEMORY_ENTRIES).unwrap_or(i64::MAX)],
            )
            .map_err(|error| map_error(&error))?;
        transaction.commit().map_err(|error| map_error(&error))
    }

    /// 按最新写入顺序返回有限数量的本地翻译记忆。
    pub fn translation_memory(
        &self,
        limit: usize,
    ) -> Result<Vec<TranslationMemoryEntry>, TranslationError> {
        let limit = limit.min(MAX_TRANSLATION_MEMORY_ENTRIES);
        let mut statement = self
            .connection
            .prepare(
                "SELECT cache_key, created_at, source_text, translated_text, source_locale, target_locale, model_id, identity_json FROM translation_memory ORDER BY created_at DESC, cache_key DESC LIMIT ?1",
            )
            .map_err(|error| map_error(&error))?;
        statement
            .query_map(
                params![i64::try_from(limit).unwrap_or(i64::MAX)],
                translation_memory_from_row,
            )
            .map_err(|error| map_error(&error))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| map_error(&error))
    }

    /// 清空全部本地翻译记忆。
    pub fn clear_translation_memory(&mut self) -> Result<(), TranslationError> {
        self.connection
            .execute("DELETE FROM translation_memory", [])
            .map(|_| ())
            .map_err(|error| map_error(&error))
    }

    /// 删除指定缓存键对应的本地翻译记忆。
    pub fn delete_translation_memory_entry(
        &mut self,
        cache_key: &str,
    ) -> Result<bool, TranslationError> {
        let deleted = self
            .connection
            .execute(
                "DELETE FROM translation_memory WHERE cache_key = ?1",
                params![cache_key],
            )
            .map_err(|error| map_error(&error))?;
        Ok(deleted != 0)
    }

    /// 按最新写入顺序返回有限数量的本地翻译历史。
    pub fn translation_history(
        &self,
        limit: usize,
    ) -> Result<Vec<TranslationHistoryEntry>, TranslationError> {
        let limit = limit.min(MAX_TRANSLATION_HISTORY_ENTRIES);
        let mut statement = self
            .connection
            .prepare(
                "SELECT operation_id, created_at, source_text, translated_text, source_locale, target_locale, model_id FROM translation_history ORDER BY created_at DESC, operation_id DESC LIMIT ?1",
            )
            .map_err(|error| map_error(&error))?;
        statement
            .query_map(params![i64::try_from(limit).unwrap_or(i64::MAX)], |row| {
                Ok(TranslationHistoryEntry {
                    operation_id: row.get(0)?,
                    created_at: row.get(1)?,
                    source_text: row.get(2)?,
                    translated_text: row.get(3)?,
                    source_locale: row.get(4)?,
                    target_locale: row.get(5)?,
                    model_id: row.get(6)?,
                })
            })
            .map_err(|error| map_error(&error))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| map_error(&error))
    }

    /// 写入一条非隐身翻译历史并裁剪最旧记录。
    pub fn record_translation_history(
        &mut self,
        request: &TranslationRequest,
        translated_text: &str,
    ) -> Result<(), TranslationError> {
        self.record_translation_history_with_usage(request, translated_text, None)
    }

    /// 原子写入翻译历史和可选的归一化 usage 记录。
    pub fn record_translation_history_with_usage(
        &mut self,
        request: &TranslationRequest,
        translated_text: &str,
        usage: Option<&UsageRecord>,
    ) -> Result<(), TranslationError> {
        if request.is_incognito() {
            return Ok(());
        }
        if !self.translation_history_enabled()? {
            return Ok(());
        }
        if request.source_text.len() > MAX_TRANSLATION_HISTORY_TEXT_BYTES
            || translated_text.len() > MAX_TRANSLATION_HISTORY_TEXT_BYTES
        {
            return Err(TranslationError::new(
                ErrorKind::InvalidConfiguration,
                "Translation history entry exceeds the local size limit.",
            ));
        }
        let transaction = self
            .connection
            .transaction()
            .map_err(|error| map_error(&error))?;
        transaction
            .execute(
                "INSERT INTO translation_history (operation_id, source_text, translated_text, source_locale, target_locale, model_id) VALUES (?1, ?2, ?3, ?4, ?5, ?6) ON CONFLICT(operation_id) DO UPDATE SET source_text = excluded.source_text, translated_text = excluded.translated_text, source_locale = excluded.source_locale, target_locale = excluded.target_locale, model_id = excluded.model_id, created_at = unixepoch()",
                params![
                    request.operation_id.as_str(),
                    request.source_text.as_str(),
                    translated_text,
                    request.source_locale.as_deref(),
                    request.target_locale.as_str(),
                    request.model_id.as_str(),
                ],
            )
            .map_err(|error| map_error(&error))?;
        if let Some(usage) = usage {
            transaction
                .execute(
                    "INSERT INTO usage_records (operation_id, provider_id, model_id, source, input_tokens, output_tokens, total_tokens) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7) ON CONFLICT(operation_id) DO UPDATE SET provider_id = excluded.provider_id, model_id = excluded.model_id, source = excluded.source, input_tokens = excluded.input_tokens, output_tokens = excluded.output_tokens, total_tokens = excluded.total_tokens, created_at = unixepoch()",
                    params![
                        request.operation_id.as_str(),
                        safe_usage_provider_id(request.provider_identity.as_deref()),
                        request.model_id.as_str(),
                        usage_source_name(usage.source),
                        usage.input_tokens.map(usage_token_sql_value),
                        usage.output_tokens.map(usage_token_sql_value),
                        usage.total_tokens.map(usage_token_sql_value),
                    ],
                )
                .map_err(|error| map_error(&error))?;
        }
        transaction
            .execute(
                "DELETE FROM translation_history WHERE operation_id IN (SELECT operation_id FROM translation_history ORDER BY created_at DESC, operation_id DESC LIMIT -1 OFFSET ?1)",
                params![i64::try_from(MAX_TRANSLATION_HISTORY_ENTRIES).unwrap_or(i64::MAX)],
            )
            .map_err(|error| map_error(&error))?;
        transaction
            .execute(
                "DELETE FROM usage_records WHERE operation_id NOT IN (SELECT operation_id FROM translation_history)",
                [],
            )
            .map_err(|error| map_error(&error))?;
        transaction.commit().map_err(|error| map_error(&error))
    }

    /// 清空全部本地翻译历史。
    pub fn clear_translation_history(&mut self) -> Result<(), TranslationError> {
        let transaction = self
            .connection
            .transaction()
            .map_err(|error| map_error(&error))?;
        transaction
            .execute("DELETE FROM translation_history", [])
            .map_err(|error| map_error(&error))?;
        transaction
            .execute("DELETE FROM usage_records", [])
            .map_err(|error| map_error(&error))?;
        transaction.commit().map_err(|error| map_error(&error))
    }

    /// 删除指定操作标识对应的本地翻译历史，并返回是否找到记录。
    pub fn delete_translation_history_entry(
        &mut self,
        operation_id: &str,
    ) -> Result<bool, TranslationError> {
        let transaction = self
            .connection
            .transaction()
            .map_err(|error| map_error(&error))?;
        let deleted = transaction
            .execute(
                "DELETE FROM translation_history WHERE operation_id = ?1",
                params![operation_id],
            )
            .map_err(|error| map_error(&error))?;
        transaction
            .execute(
                "DELETE FROM usage_records WHERE operation_id = ?1",
                params![operation_id],
            )
            .map_err(|error| map_error(&error))?;
        transaction.commit().map_err(|error| map_error(&error))?;
        Ok(deleted != 0)
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

    /// 记录一次已完成的提供商健康检查，并清除过期失败类别。
    pub fn record_provider_health_success(
        &mut self,
        profile_id: &ProviderProfileId,
        timestamp: i64,
    ) -> Result<bool, TranslationError> {
        if timestamp < 0 {
            return Err(TranslationError::new(
                ErrorKind::InvalidConfiguration,
                "Provider health timestamps must be non-negative.",
            ));
        }
        let changed = self
            .connection
            .execute(
                "UPDATE provider_profiles SET last_successful_health_check = ?1, last_failure_category = NULL WHERE id = ?2",
                params![timestamp, profile_id.as_str()],
            )
            .map_err(|error| map_error(&error))?;
        Ok(changed != 0)
    }

    /// 记录一次失败的提供商健康检查，仅保存规范化错误类别。
    pub fn record_provider_health_failure(
        &mut self,
        profile_id: &ProviderProfileId,
        category: ErrorKind,
    ) -> Result<bool, TranslationError> {
        let changed = self
            .connection
            .execute(
                "UPDATE provider_profiles SET last_failure_category = ?1 WHERE id = ?2",
                params![serialize_error_kind(category), profile_id.as_str()],
            )
            .map_err(|error| map_error(&error))?;
        Ok(changed != 0)
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

    /// 保存或替换一个不含秘密的路由配置。
    pub fn save_routing_profile(
        &mut self,
        profile: &RoutingProfile,
    ) -> Result<RoutingProfileRecord, TranslationError> {
        profile.validate().map_err(|error| {
            TranslationError::new(ErrorKind::InvalidConfiguration, error.to_string())
        })?;
        let profile_json = serialize_routing_profile(profile).map_err(|error| {
            TranslationError::new(ErrorKind::InvalidConfiguration, error.to_string())
        })?;
        let transaction = self
            .connection
            .transaction()
            .map_err(|error| map_error(&error))?;
        let exists = transaction
            .query_row(
                "SELECT 1 FROM routing_profiles WHERE profile_id = ?1",
                params![profile.id.as_str()],
                |_| Ok(()),
            )
            .optional()
            .map_err(|error| map_error(&error))?
            .is_some();
        if !exists {
            let count = transaction
                .query_row("SELECT COUNT(*) FROM routing_profiles", [], |row| {
                    row.get::<_, i64>(0)
                })
                .map_err(|error| map_error(&error))?;
            if usize::try_from(count).unwrap_or(usize::MAX) >= MAX_ROUTING_PROFILES {
                return Err(TranslationError::new(
                    ErrorKind::InvalidConfiguration,
                    "The local routing profile limit has been reached.",
                ));
            }
        }
        transaction
            .execute(
                "INSERT INTO routing_profiles (profile_id, profile_json) VALUES (?1, ?2) ON CONFLICT(profile_id) DO UPDATE SET profile_json = excluded.profile_json, updated_at = unixepoch()",
                params![profile.id.as_str(), profile_json.as_str()],
            )
            .map_err(|error| map_error(&error))?;
        transaction.commit().map_err(|error| map_error(&error))?;
        self.routing_profile(&profile.id)?.ok_or_else(|| {
            TranslationError::new(
                ErrorKind::Persistence,
                "The saved routing profile could not be reloaded.",
            )
        })
    }

    /// 按稳定标识读取一个路由配置。
    pub fn routing_profile(
        &self,
        profile_id: &str,
    ) -> Result<Option<RoutingProfileRecord>, TranslationError> {
        self.connection
            .query_row(
                "SELECT profile_id, profile_json, created_at, updated_at FROM routing_profiles WHERE profile_id = ?1",
                params![profile_id],
                routing_profile_from_row,
            )
            .optional()
            .map_err(|error| map_error(&error))?
            .map(|(id, profile_json, created_at, updated_at)| {
                parse_routing_profile_record(id, &profile_json, created_at, updated_at)
            })
            .transpose()
    }

    /// 返回按更新时间排序的全部路由配置。
    pub fn routing_profiles(&self) -> Result<Vec<RoutingProfileRecord>, TranslationError> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT profile_id, profile_json, created_at, updated_at FROM routing_profiles ORDER BY updated_at DESC, profile_id ASC LIMIT ?1",
            )
            .map_err(|error| map_error(&error))?;
        statement
            .query_map(
                params![i64::try_from(MAX_ROUTING_PROFILES).unwrap_or(i64::MAX)],
                routing_profile_from_row,
            )
            .map_err(|error| map_error(&error))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| map_error(&error))?
            .into_iter()
            .map(|(id, profile_json, created_at, updated_at)| {
                parse_routing_profile_record(id, &profile_json, created_at, updated_at)
            })
            .collect()
    }

    /// 删除一个路由配置。
    pub fn delete_routing_profile(&mut self, profile_id: &str) -> Result<bool, TranslationError> {
        let changed = self
            .connection
            .execute(
                "DELETE FROM routing_profiles WHERE profile_id = ?1",
                params![profile_id],
            )
            .map_err(|error| map_error(&error))?;
        Ok(changed > 0)
    }

    /// 保存或替换一个经过校验的本地词汇表库。
    pub fn save_glossary(
        &mut self,
        glossary_id: &str,
        glossary: &Glossary,
    ) -> Result<GlossaryRecord, TranslationError> {
        validate_glossary_id(glossary_id)?;
        glossary.validate().map_err(|error| {
            TranslationError::new(ErrorKind::InvalidConfiguration, error.to_string())
        })?;
        let transaction = self
            .connection
            .transaction()
            .map_err(|error| map_error(&error))?;
        let exists = transaction
            .query_row(
                "SELECT 1 FROM glossaries WHERE glossary_id = ?1",
                params![glossary_id],
                |_| Ok(()),
            )
            .optional()
            .map_err(|error| map_error(&error))?
            .is_some();
        if !exists {
            let count = transaction
                .query_row("SELECT COUNT(*) FROM glossaries", [], |row| {
                    row.get::<_, i64>(0)
                })
                .map_err(|error| map_error(&error))?;
            if usize::try_from(count).unwrap_or(usize::MAX) >= MAX_GLOSSARIES {
                return Err(TranslationError::new(
                    ErrorKind::InvalidConfiguration,
                    "The local glossary library limit has been reached.",
                ));
            }
        }
        transaction
            .execute(
                "INSERT INTO glossaries (glossary_id) VALUES (?1) ON CONFLICT(glossary_id) DO UPDATE SET updated_at = unixepoch()",
                params![glossary_id],
            )
            .map_err(|error| map_error(&error))?;
        transaction
            .execute(
                "DELETE FROM glossary_terms WHERE glossary_id = ?1",
                params![glossary_id],
            )
            .map_err(|error| map_error(&error))?;
        for (term_index, entry) in glossary.entries().iter().enumerate() {
            transaction
                .execute(
                    "INSERT INTO glossary_terms (glossary_id, term_index, source_term, target_term, source_locale, target_locale, case_sensitive, whole_word, immutable, domain, priority, notes, enabled) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
                    params![
                        glossary_id,
                        i64::try_from(term_index).unwrap_or(i64::MAX),
                        entry.source_term.as_str(),
                        entry.target_term.as_str(),
                        entry.source_locale.as_deref(),
                        entry.target_locale.as_deref(),
                        entry.case_sensitive,
                        entry.whole_word,
                        entry.immutable,
                        entry.domain.as_deref(),
                        entry.priority,
                        entry.notes.as_deref(),
                        entry.enabled,
                    ],
                )
                .map_err(|error| map_error(&error))?;
        }
        transaction.commit().map_err(|error| map_error(&error))?;
        self.glossary(glossary_id)?.ok_or_else(|| {
            TranslationError::new(
                ErrorKind::Persistence,
                "The saved glossary library could not be reloaded.",
            )
        })
    }

    /// 按稳定标识读取一个本地词汇表库。
    pub fn glossary(&self, glossary_id: &str) -> Result<Option<GlossaryRecord>, TranslationError> {
        validate_glossary_id(glossary_id)?;
        self.connection
            .query_row(
                "SELECT glossary_id, created_at, updated_at FROM glossaries WHERE glossary_id = ?1",
                params![glossary_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, i64>(2)?,
                    ))
                },
            )
            .optional()
            .map_err(|error| map_error(&error))?
            .map(|(id, created_at, updated_at)| {
                load_glossary_record(&self.connection, id, created_at, updated_at)
            })
            .transpose()
    }

    /// 返回按更新时间排序的全部本地词汇表库。
    pub fn glossaries(&self) -> Result<Vec<GlossaryRecord>, TranslationError> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT glossary_id, created_at, updated_at FROM glossaries ORDER BY updated_at DESC, glossary_id ASC LIMIT ?1",
            )
            .map_err(|error| map_error(&error))?;
        let rows = statement
            .query_map(
                params![i64::try_from(MAX_GLOSSARIES).unwrap_or(i64::MAX)],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, i64>(2)?,
                    ))
                },
            )
            .map_err(|error| map_error(&error))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| map_error(&error))?;
        rows.into_iter()
            .map(|(id, created_at, updated_at)| {
                load_glossary_record(&self.connection, id, created_at, updated_at)
            })
            .collect()
    }

    /// 删除一个本地词汇表库及其全部术语。
    pub fn delete_glossary(&mut self, glossary_id: &str) -> Result<bool, TranslationError> {
        validate_glossary_id(glossary_id)?;
        let changed = self
            .connection
            .execute(
                "DELETE FROM glossaries WHERE glossary_id = ?1",
                params![glossary_id],
            )
            .map_err(|error| map_error(&error))?;
        Ok(changed > 0)
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

fn is_proc_self_fd_path(path: &Path) -> bool {
    let mut components = path.components();
    let is_named = |component: Option<Component<'_>>, expected: &str| matches!(component, Some(Component::Normal(value)) if value == expected);
    matches!(components.next(), Some(Component::RootDir))
        && is_named(components.next(), "proc")
        && is_named(components.next(), "self")
        && is_named(components.next(), "fd")
        && matches!(
            components.next(),
            Some(Component::Normal(value))
                if value.to_str().is_some_and(|value| {
                    !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit())
                })
        )
        && components.next().is_none()
}

fn validate_document_job_identity(job_id: &str, job: &DocumentJob) -> Result<(), TranslationError> {
    if job_id.is_empty() || job_id.len() > 128 || job_id.chars().any(char::is_control) {
        return Err(document_configuration_error(
            "The document job ID is invalid.",
        ));
    }
    if job.source_name.is_empty()
        || job.source_name.len() > 255
        || job
            .source_name
            .chars()
            .any(|character| character.is_control() || matches!(character, '/' | '\\'))
    {
        return Err(document_configuration_error(
            "The document source name is invalid.",
        ));
    }
    if job.segments.len() > MAX_DOCUMENT_SEGMENTS {
        return Err(document_configuration_error(
            "The document contains too many segments.",
        ));
    }
    if matches!(
        job.format,
        DocumentFormat::Docx
            | DocumentFormat::Pptx
            | DocumentFormat::Xlsx
            | DocumentFormat::Epub
            | DocumentFormat::Pdf
    ) != job.package.is_some()
    {
        return Err(document_configuration_error(
            "The package payload is missing or attached to another format.",
        ));
    }
    if job
        .package
        .as_ref()
        .is_some_and(|package| package.len() > MAX_DOCUMENT_BYTES)
    {
        return Err(document_configuration_error(
            "The package exceeds the local size limit.",
        ));
    }
    let mut source_bytes = 0usize;
    let mut translated_bytes = 0usize;
    for (expected_index, segment) in job.segments.iter().enumerate() {
        if segment.index != expected_index
            || !matches!(segment.line_ending.as_str(), "" | "\n" | "\r\n" | "\r")
        {
            return Err(document_configuration_error(
                "The document segment ordering or line ending is invalid.",
            ));
        }
        if segment.kind == DocumentSegmentKind::Verbatim && segment.translated_text.is_some() {
            return Err(document_configuration_error(
                "A document structure segment cannot contain a translation.",
            ));
        }
        source_bytes = source_bytes
            .checked_add(segment.source_text.len())
            .ok_or_else(|| document_configuration_error("The document source is too large."))?;
        if let Some(translated_text) = &segment.translated_text {
            translated_bytes = translated_bytes
                .checked_add(translated_text.len())
                .ok_or_else(|| {
                    document_configuration_error("The translated document is too large.")
                })?;
        }
    }
    if source_bytes > MAX_DOCUMENT_JOB_TEXT_BYTES || translated_bytes > MAX_DOCUMENT_JOB_TEXT_BYTES
    {
        return Err(document_configuration_error(
            "The document text exceeds the local size limit.",
        ));
    }
    Ok(())
}

fn validate_document_job_options(options: &DocumentJobOptions) -> Result<(), TranslationError> {
    if let Some(source_locale) = options.source_locale.as_deref() {
        validate_document_option_text(source_locale, "The document source locale is invalid.")?;
    }
    validate_document_option_text(
        &options.target_locale,
        "The document target locale is invalid.",
    )?;
    validate_model_identifier(&options.model_id)
        .map_err(|_| document_configuration_error("The document model identifier is invalid."))?;
    ProviderProfileId::parse(&options.provider_id).map_err(|_| {
        document_configuration_error("The document provider identifier is invalid.")
    })?;
    if let Some(routing_profile_id) = options.routing_profile_id.as_deref() {
        validate_document_option_text(
            routing_profile_id,
            "The document routing profile identifier is invalid.",
        )?;
    }
    if let Some(glossary) = options.glossary.as_ref() {
        glossary
            .validate()
            .map_err(|_| document_configuration_error("The document glossary is invalid."))?;
        let encoded = serde_json::to_vec(glossary).map_err(|_| {
            document_configuration_error("The document glossary could not be serialized.")
        })?;
        if encoded.len() > MAX_DOCUMENT_JOB_GLOSSARY_BYTES {
            return Err(document_configuration_error(
                "The document glossary exceeds the local size limit.",
            ));
        }
    }
    options
        .translation_preset
        .validate()
        .map_err(|_| document_configuration_error("The document translation preset is invalid."))?;
    let preset_bytes = serde_json::to_vec(&options.translation_preset).map_err(|_| {
        document_configuration_error("The document translation preset could not be serialized.")
    })?;
    if preset_bytes.len() > MAX_DOCUMENT_JOB_PRESET_BYTES {
        return Err(document_configuration_error(
            "The document translation preset exceeds the local size limit.",
        ));
    }
    Ok(())
}

fn validate_document_option_text(value: &str, message: &str) -> Result<(), TranslationError> {
    if value.is_empty()
        || value.len() > MAX_DOCUMENT_JOB_OPTION_TEXT_BYTES
        || value.chars().any(char::is_control)
    {
        return Err(document_configuration_error(message));
    }
    Ok(())
}

fn document_configuration_error(message: &str) -> TranslationError {
    TranslationError::new(ErrorKind::InvalidConfiguration, message)
}

fn map_document_error(error: linguamesh_document::DocumentError) -> TranslationError {
    TranslationError::new(ErrorKind::InvalidConfiguration, error.to_string())
}

fn document_format_name(format: DocumentFormat) -> &'static str {
    match format {
        DocumentFormat::Txt => "txt",
        DocumentFormat::Markdown => "markdown",
        DocumentFormat::Srt => "srt",
        DocumentFormat::WebVtt => "webvtt",
        DocumentFormat::Csv => "csv",
        DocumentFormat::Html => "html",
        DocumentFormat::Json => "json",
        DocumentFormat::Docx => "docx",
        DocumentFormat::Pptx => "pptx",
        DocumentFormat::Xlsx => "xlsx",
        DocumentFormat::Epub => "epub",
        DocumentFormat::Pdf => "pdf",
    }
}

fn parse_document_format(value: &str) -> Result<DocumentFormat, TranslationError> {
    match value {
        "txt" => Ok(DocumentFormat::Txt),
        "markdown" => Ok(DocumentFormat::Markdown),
        "srt" => Ok(DocumentFormat::Srt),
        "webvtt" => Ok(DocumentFormat::WebVtt),
        "csv" => Ok(DocumentFormat::Csv),
        "html" => Ok(DocumentFormat::Html),
        "json" => Ok(DocumentFormat::Json),
        "docx" => Ok(DocumentFormat::Docx),
        "pptx" => Ok(DocumentFormat::Pptx),
        "xlsx" => Ok(DocumentFormat::Xlsx),
        "epub" => Ok(DocumentFormat::Epub),
        "pdf" => Ok(DocumentFormat::Pdf),
        _ => Err(TranslationError::new(
            ErrorKind::Persistence,
            "The stored document format is invalid.",
        )),
    }
}

fn document_job_state_name(state: DocumentJobState) -> &'static str {
    match state {
        DocumentJobState::Pending => "pending",
        DocumentJobState::Running => "running",
        DocumentJobState::Paused => "paused",
        DocumentJobState::Completed => "completed",
        DocumentJobState::Cancelled => "cancelled",
        DocumentJobState::Failed => "failed",
    }
}

fn parse_document_job_state(value: &str) -> Result<DocumentJobState, TranslationError> {
    match value {
        "pending" => Ok(DocumentJobState::Pending),
        "running" => Ok(DocumentJobState::Running),
        "paused" => Ok(DocumentJobState::Paused),
        "completed" => Ok(DocumentJobState::Completed),
        "cancelled" => Ok(DocumentJobState::Cancelled),
        "failed" => Ok(DocumentJobState::Failed),
        _ => Err(TranslationError::new(
            ErrorKind::Persistence,
            "The stored document job state is invalid.",
        )),
    }
}

fn document_segment_kind_name(kind: DocumentSegmentKind) -> &'static str {
    match kind {
        DocumentSegmentKind::Prose => "prose",
        DocumentSegmentKind::Verbatim => "verbatim",
    }
}

fn parse_document_segment_kind(value: &str) -> Result<DocumentSegmentKind, TranslationError> {
    match value {
        "prose" => Ok(DocumentSegmentKind::Prose),
        "verbatim" => Ok(DocumentSegmentKind::Verbatim),
        _ => Err(TranslationError::new(
            ErrorKind::Persistence,
            "The stored document segment kind is invalid.",
        )),
    }
}

fn load_document_job(
    connection: &Connection,
    job_id: &str,
) -> Result<Option<DocumentJobSnapshot>, TranslationError> {
    let metadata = connection
        .query_row(
            "SELECT state, format, source_name, package_blob, created_at, updated_at FROM document_jobs WHERE job_id = ?1",
            params![job_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<Vec<u8>>>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, i64>(5)?,
                ))
            },
        )
        .optional()
        .map_err(|error| map_error(&error))?;
    let Some((state, format, source_name, package, created_at, updated_at)) = metadata else {
        return Ok(None);
    };
    let mut statement = connection
        .prepare(
            "SELECT segment_index, kind, source_text, translated_text, line_ending FROM document_segments WHERE job_id = ?1 ORDER BY segment_index ASC",
        )
        .map_err(|error| map_error(&error))?;
    let segments = statement
        .query_map(params![job_id], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, String>(4)?,
            ))
        })
        .map_err(|error| map_error(&error))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| map_error(&error))?;
    let segments = segments
        .into_iter()
        .map(
            |(index, kind, source_text, translated_text, line_ending)| -> Result<_, TranslationError> {
                let index = usize::try_from(index).map_err(|_| {
                    TranslationError::new(
                        ErrorKind::Persistence,
                        "The stored document segment index is invalid.",
                    )
                })?;
                Ok(DocumentSegment {
                    index,
                    kind: parse_document_segment_kind(&kind)?,
                    source_text,
                    translated_text,
                    line_ending,
                })
            },
        )
        .collect::<Result<Vec<_>, _>>()?;
    let job = DocumentJob {
        format: parse_document_format(&format)?,
        source_name,
        segments,
        package,
    };
    validate_document_job_identity(job_id, &job)?;
    let options = load_document_job_options(connection, job_id)?;
    Ok(Some(DocumentJobSnapshot {
        job_id: job_id.to_owned(),
        state: parse_document_job_state(&state)?,
        job,
        options,
        created_at,
        updated_at,
    }))
}

fn load_document_job_options(
    connection: &Connection,
    job_id: &str,
) -> Result<Option<DocumentJobOptions>, TranslationError> {
    let stored = connection
        .query_row(
            "SELECT source_locale, target_locale, model_id, provider_id, routing_profile_id, quality_mode, translation_preset_json, glossary_json FROM document_job_options WHERE job_id = ?1",
            params![job_id],
            |row| {
                Ok((
                    row.get::<_, Option<String>>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, Option<String>>(7)?,
                ))
            },
        )
        .optional()
        .map_err(|error| map_error(&error))?;
    let Some((
        source_locale,
        target_locale,
        model_id,
        provider_id,
        routing_profile_id,
        quality_mode,
        translation_preset_json,
        glossary_json,
    )) = stored
    else {
        return Ok(None);
    };
    let quality_mode = quality_mode
        .as_deref()
        .map(parse_document_quality_mode)
        .transpose()?
        .unwrap_or_default();
    let translation_preset = translation_preset_json
        .as_deref()
        .map(serde_json::from_str)
        .transpose()
        .map_err(|_| {
            TranslationError::new(
                ErrorKind::Persistence,
                "The stored document translation preset is invalid.",
            )
        })?
        .unwrap_or_else(TranslationPreset::general);
    let glossary = glossary_json
        .as_deref()
        .map(serde_json::from_str)
        .transpose()
        .map_err(|_| {
            TranslationError::new(
                ErrorKind::Persistence,
                "The stored document glossary is invalid.",
            )
        })?;
    let options = DocumentJobOptions {
        source_locale,
        target_locale,
        model_id,
        provider_id,
        routing_profile_id,
        quality_mode,
        translation_preset,
        glossary,
    };
    validate_document_job_options(&options).map_err(|_| {
        TranslationError::new(
            ErrorKind::Persistence,
            "The stored document translation options are invalid.",
        )
    })?;
    Ok(Some(options))
}

fn parse_document_quality_mode(value: &str) -> Result<TranslationQualityMode, TranslationError> {
    match value {
        "fast" => Ok(TranslationQualityMode::Fast),
        "balanced" => Ok(TranslationQualityMode::Balanced),
        "best" => Ok(TranslationQualityMode::Best),
        _ => Err(TranslationError::new(
            ErrorKind::Persistence,
            "The stored document quality mode is invalid.",
        )),
    }
}

fn routing_profile_from_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<(String, String, i64, i64)> {
    Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
}

fn parse_routing_profile_record(
    id: String,
    profile_json: &str,
    created_at: i64,
    updated_at: i64,
) -> Result<RoutingProfileRecord, TranslationError> {
    let profile = deserialize_routing_profile(profile_json).map_err(|_| {
        TranslationError::new(
            ErrorKind::Persistence,
            "The stored routing profile is invalid.",
        )
    })?;
    if profile.id != id || profile_json.len() > MAX_ROUTING_PROFILE_JSON_BYTES {
        return Err(TranslationError::new(
            ErrorKind::Persistence,
            "The stored routing profile is invalid.",
        ));
    }
    Ok(RoutingProfileRecord {
        id,
        profile,
        created_at,
        updated_at,
    })
}

const PROFILE_QUERY_BY_ID: &str = "SELECT p.id, p.display_name, p.preset_id, p.adapter_type, p.base_endpoint, p.secret_ref, p.user_notes, p.organization, p.project, p.region, p.account_identifier, p.custom_headers, p.secret_custom_headers_ref, p.proxy_url, p.proxy_auth_ref, p.request_timeout_secs, p.connection_timeout_secs, p.streaming_idle_timeout_secs, p.trusted_certificates_pem, p.client_certificate_identity_ref, p.enabled, s.model_id, p.last_successful_health_check, p.last_failure_category FROM provider_profiles p LEFT JOIN provider_model_selection s ON s.provider_id = p.id WHERE p.id = ?1";
const PROFILE_QUERY_ALL: &str = "SELECT p.id, p.display_name, p.preset_id, p.adapter_type, p.base_endpoint, p.secret_ref, p.user_notes, p.organization, p.project, p.region, p.account_identifier, p.custom_headers, p.secret_custom_headers_ref, p.proxy_url, p.proxy_auth_ref, p.request_timeout_secs, p.connection_timeout_secs, p.streaming_idle_timeout_secs, p.trusted_certificates_pem, p.client_certificate_identity_ref, p.enabled, s.model_id, p.last_successful_health_check, p.last_failure_category FROM provider_profiles p LEFT JOIN provider_model_selection s ON s.provider_id = p.id ORDER BY p.display_name, p.id";
const PROFILE_QUERY_ACTIVE: &str = "SELECT p.id, p.display_name, p.preset_id, p.adapter_type, p.base_endpoint, p.secret_ref, p.user_notes, p.organization, p.project, p.region, p.account_identifier, p.custom_headers, p.secret_custom_headers_ref, p.proxy_url, p.proxy_auth_ref, p.request_timeout_secs, p.connection_timeout_secs, p.streaming_idle_timeout_secs, p.trusted_certificates_pem, p.client_certificate_identity_ref, p.enabled, s.model_id, p.last_successful_health_check, p.last_failure_category FROM active_provider_selection a JOIN provider_profiles p ON p.id = a.provider_id LEFT JOIN provider_model_selection s ON s.provider_id = p.id WHERE a.singleton = 1";

struct StoredProfile {
    id: String,
    display_name: String,
    preset_id: String,
    adapter_type: String,
    base_endpoint: String,
    secret_ref: Option<String>,
    user_notes: Option<String>,
    organization: Option<String>,
    project: Option<String>,
    region: Option<String>,
    account_identifier: Option<String>,
    custom_headers: Option<String>,
    secret_custom_headers_ref: Option<String>,
    proxy_url: Option<String>,
    proxy_auth_ref: Option<String>,
    request_timeout_secs: u32,
    connection_timeout_secs: u32,
    streaming_idle_timeout_secs: u32,
    trusted_certificates_pem: Option<String>,
    client_certificate_identity_ref: Option<String>,
    enabled: bool,
    selected_model: Option<String>,
    last_successful_health_check: Option<i64>,
    last_failure_category: Option<String>,
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
        let secret_custom_headers_ref = self
            .secret_custom_headers_ref
            .map(SecretRef::parse)
            .transpose()
            .map_err(|error| map_profile_validation(&error))?;
        let proxy_auth_ref = self
            .proxy_auth_ref
            .map(SecretRef::parse)
            .transpose()
            .map_err(|error| map_profile_validation(&error))?;
        let client_certificate_identity_ref = self
            .client_certificate_identity_ref
            .map(SecretRef::parse)
            .transpose()
            .map_err(|error| map_profile_validation(&error))?;
        let last_failure_category = self
            .last_failure_category
            .as_deref()
            .map(parse_error_kind)
            .transpose()
            .map_err(|()| {
                TranslationError::new(
                    ErrorKind::Persistence,
                    "Stored provider health category is invalid.",
                )
            })?;
        ProviderProfile::new(
            id,
            self.display_name,
            self.preset_id,
            self.adapter_type,
            self.base_endpoint,
            secret_ref,
        )
        .and_then(|profile| profile.with_user_notes(self.user_notes))
        .and_then(|profile| profile.with_organization(self.organization))
        .and_then(|profile| profile.with_project(self.project))
        .and_then(|profile| profile.with_region(self.region))
        .and_then(|profile| profile.with_account_identifier(self.account_identifier))
        .and_then(|profile| profile.with_custom_headers(self.custom_headers))
        .and_then(|profile| profile.with_proxy_url(self.proxy_url))
        .map(|profile| profile.with_proxy_auth_ref(proxy_auth_ref))
        .and_then(|profile| profile.with_request_timeout_secs(self.request_timeout_secs))
        .and_then(|profile| profile.with_connection_timeout_secs(self.connection_timeout_secs))
        .and_then(|profile| {
            profile.with_streaming_idle_timeout_secs(self.streaming_idle_timeout_secs)
        })
        .and_then(|profile| profile.with_trusted_certificates_pem(self.trusted_certificates_pem))
        .map(|profile| {
            profile.with_client_certificate_identity_ref(client_certificate_identity_ref)
        })
        .map(|profile| profile.with_secret_custom_headers_ref(secret_custom_headers_ref))
        .map(|profile| profile.with_enabled(self.enabled))
        .and_then(|profile| profile.with_selected_model(self.selected_model))
        .and_then(|profile| {
            profile.with_last_successful_health_check(self.last_successful_health_check)
        })
        .map(|profile| profile.with_last_failure_category(last_failure_category))
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
        user_notes: row.get(6)?,
        organization: row.get(7)?,
        project: row.get(8)?,
        region: row.get(9)?,
        account_identifier: row.get(10)?,
        custom_headers: row.get(11)?,
        secret_custom_headers_ref: row.get(12)?,
        proxy_url: row.get(13)?,
        proxy_auth_ref: row.get(14)?,
        request_timeout_secs: row.get(15)?,
        connection_timeout_secs: row.get(16)?,
        streaming_idle_timeout_secs: row.get(17)?,
        trusted_certificates_pem: row.get(18)?,
        client_certificate_identity_ref: row.get(19)?,
        enabled: row.get(20)?,
        selected_model: row.get(21)?,
        last_successful_health_check: row.get(22)?,
        last_failure_category: row.get(23)?,
    })
}

fn serialize_error_kind(kind: ErrorKind) -> &'static str {
    match kind {
        ErrorKind::Cancelled => "cancelled",
        ErrorKind::InvalidEndpoint => "invalid_endpoint",
        ErrorKind::Network => "network",
        ErrorKind::Timeout => "timeout",
        ErrorKind::Authentication => "authentication",
        ErrorKind::RateLimited => "rate_limited",
        ErrorKind::ModelUnavailable => "model_unavailable",
        ErrorKind::MalformedResponse => "malformed_response",
        ErrorKind::Persistence => "persistence",
        ErrorKind::ProtocolIncompatible => "protocol_incompatible",
        ErrorKind::InvalidConfiguration => "invalid_configuration",
        ErrorKind::UnsupportedCapability => "unsupported_capability",
        ErrorKind::SecretUnavailable => "secret_unavailable",
        ErrorKind::SecureStorageUnavailable => "secure_storage_unavailable",
        ErrorKind::Internal => "internal",
    }
}

fn parse_error_kind(value: &str) -> Result<ErrorKind, ()> {
    Ok(match value {
        "cancelled" => ErrorKind::Cancelled,
        "invalid_endpoint" => ErrorKind::InvalidEndpoint,
        "network" => ErrorKind::Network,
        "timeout" => ErrorKind::Timeout,
        "authentication" => ErrorKind::Authentication,
        "rate_limited" => ErrorKind::RateLimited,
        "model_unavailable" => ErrorKind::ModelUnavailable,
        "malformed_response" => ErrorKind::MalformedResponse,
        "persistence" => ErrorKind::Persistence,
        "protocol_incompatible" => ErrorKind::ProtocolIncompatible,
        "invalid_configuration" => ErrorKind::InvalidConfiguration,
        "unsupported_capability" => ErrorKind::UnsupportedCapability,
        "secret_unavailable" => ErrorKind::SecretUnavailable,
        "secure_storage_unavailable" => ErrorKind::SecureStorageUnavailable,
        "internal" => ErrorKind::Internal,
        _ => return Err(()),
    })
}

fn upsert_profile(
    transaction: &rusqlite::Transaction<'_>,
    profile: &ProviderProfile,
) -> Result<(), TranslationError> {
    if profile
        .secret_ref()
        .is_some_and(|secret_ref| !secret_ref.is_persistent())
        || profile
            .secret_custom_headers_ref()
            .is_some_and(|secret_ref| !secret_ref.is_persistent())
        || profile
            .proxy_auth_ref()
            .is_some_and(|secret_ref| !secret_ref.is_persistent())
        || profile
            .client_certificate_identity_ref()
            .is_some_and(|secret_ref| !secret_ref.is_persistent())
    {
        return Err(TranslationError::new(
            ErrorKind::InvalidConfiguration,
            "Session-only secret references cannot be persisted.",
        ));
    }
    transaction
        .execute(
            "INSERT INTO provider_profiles (id, display_name, base_endpoint, secret_ref, user_notes, organization, project, region, account_identifier, custom_headers, secret_custom_headers_ref, proxy_url, proxy_auth_ref, request_timeout_secs, connection_timeout_secs, streaming_idle_timeout_secs, trusted_certificates_pem, client_certificate_identity_ref, preset_id, adapter_type, enabled, last_successful_health_check, last_failure_category) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23) ON CONFLICT(id) DO UPDATE SET display_name = excluded.display_name, base_endpoint = excluded.base_endpoint, secret_ref = excluded.secret_ref, user_notes = excluded.user_notes, organization = excluded.organization, project = excluded.project, region = excluded.region, account_identifier = excluded.account_identifier, custom_headers = excluded.custom_headers, secret_custom_headers_ref = excluded.secret_custom_headers_ref, proxy_url = excluded.proxy_url, proxy_auth_ref = excluded.proxy_auth_ref, request_timeout_secs = excluded.request_timeout_secs, connection_timeout_secs = excluded.connection_timeout_secs, streaming_idle_timeout_secs = excluded.streaming_idle_timeout_secs, trusted_certificates_pem = excluded.trusted_certificates_pem, client_certificate_identity_ref = excluded.client_certificate_identity_ref, preset_id = excluded.preset_id, adapter_type = excluded.adapter_type, enabled = excluded.enabled",
            params![
                profile.id().as_str(),
                profile.display_name(),
                profile.base_endpoint(),
                profile.secret_ref().map(SecretRef::as_str),
                profile.user_notes(),
                profile.organization(),
                profile.project(),
                profile.region(),
                profile.account_identifier(),
                profile.custom_headers(),
                profile
                    .secret_custom_headers_ref()
                    .map(SecretRef::as_str),
                profile.proxy_url(),
                profile.proxy_auth_ref().map(SecretRef::as_str),
                profile.request_timeout_secs(),
                profile.connection_timeout_secs(),
                profile.streaming_idle_timeout_secs(),
                profile.trusted_certificates_pem(),
                profile
                    .client_certificate_identity_ref()
                    .map(SecretRef::as_str),
                profile.preset_id(),
                profile.adapter_type(),
                profile.enabled(),
                profile.last_successful_health_check(),
                profile.last_failure_category().map(serialize_error_kind),
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
            "PRAGMA foreign_keys = ON; PRAGMA synchronous = FULL; PRAGMA secure_delete = ON;",
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

fn translation_memory_from_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<TranslationMemoryEntry> {
    Ok(TranslationMemoryEntry {
        cache_key: row.get(0)?,
        created_at: row.get(1)?,
        source_text: row.get(2)?,
        translated_text: row.get(3)?,
        source_locale: row.get(4)?,
        target_locale: row.get(5)?,
        model_id: row.get(6)?,
        identity_json: row.get(7)?,
    })
}

// 将数据库中的 usage 行恢复为不含正文和秘密的领域记录。
fn usage_record_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<UsageRecordEntry> {
    let source_name: String = row.get(4)?;
    let source = usage_source_from_name(&source_name)?;
    Ok(UsageRecordEntry {
        operation_id: row.get(0)?,
        created_at: row.get(1)?,
        provider_id: row.get(2)?,
        model_id: row.get(3)?,
        usage: UsageRecord {
            source,
            input_tokens: usage_token_from_sql(row.get(5)?)?,
            output_tokens: usage_token_from_sql(row.get(6)?)?,
            total_tokens: usage_token_from_sql(row.get(7)?)?,
        },
    })
}

// 将无符号 token 数量安全地映射到 SQLite 的整数列。
fn usage_token_sql_value(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

// 拒绝负数 usage，避免损坏数据库产生伪造统计。
fn usage_token_from_sql(value: Option<i64>) -> rusqlite::Result<Option<u64>> {
    value
        .map(|value| {
            u64::try_from(value).map_err(|_| {
                rusqlite::Error::FromSqlConversionFailure(
                    0,
                    rusqlite::types::Type::Integer,
                    Box::new(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "usage token count cannot be negative",
                    )),
                )
            })
        })
        .transpose()
}

// 将受限 source 标签映射回领域枚举。
fn usage_source_from_name(value: &str) -> rusqlite::Result<UsageSource> {
    match value {
        "provider_reported" => Ok(UsageSource::ProviderReported),
        "locally_estimated" => Ok(UsageSource::LocallyEstimated),
        "unknown" => Ok(UsageSource::Unknown),
        _ => Err(rusqlite::Error::FromSqlConversionFailure(
            4,
            rusqlite::types::Type::Text,
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "usage source is invalid",
            )),
        )),
    }
}

// 只保留稳定提供商标识，绝不把端点或其他身份后缀写入 usage 表。
fn safe_usage_provider_id(identity: Option<&str>) -> Option<String> {
    let value = identity?.split('@').next()?.trim();
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        return None;
    }
    Some(value.to_owned())
}

// 将 usage 来源枚举编码为稳定的数据库标签。
const fn usage_source_name(source: UsageSource) -> &'static str {
    match source {
        UsageSource::ProviderReported => "provider_reported",
        UsageSource::LocallyEstimated => "locally_estimated",
        UsageSource::Unknown => "unknown",
    }
}

fn translation_memory_identity_json(
    request: &TranslationRequest,
) -> Result<String, TranslationError> {
    let identity = serde_json::json!({
        "normalized_source": normalize_translation_memory_source(&request.source_text),
        "source_locale": &request.source_locale,
        "target_locale": &request.target_locale,
        "max_chunk_bytes": request.max_chunk_bytes,
        "glossary": &request.glossary,
        "protected_span_policy": TRANSLATION_MEMORY_PROTECTED_SPAN_POLICY,
        "prompt_template_version": TRANSLATION_MEMORY_PROMPT_TEMPLATE_VERSION,
        "quality_mode": request.quality_mode.as_str(),
        "translation_preset": &request.preset,
        "provider_model": {
            "provider": &request.provider_identity,
            "model": &request.model_id,
        },
    });
    serde_json::to_string(&identity).map_err(|error| {
        TranslationError::new(
            ErrorKind::InvalidConfiguration,
            format!("Translation memory identity could not be serialized: {error}"),
        )
    })
}

fn translation_memory_key(request: &TranslationRequest) -> Result<String, TranslationError> {
    translation_memory_identity_json(request)
}

fn normalize_translation_memory_source(source: &str) -> String {
    source.split_whitespace().collect::<Vec<_>>().join(" ")
}

// 校验词汇表库标识，避免控制字符和路径样式数据进入持久化层。
fn validate_glossary_id(glossary_id: &str) -> Result<(), TranslationError> {
    if glossary_id.is_empty()
        || glossary_id.len() > 128
        || glossary_id
            .bytes()
            .any(|byte| !(byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.')))
    {
        return Err(TranslationError::new(
            ErrorKind::InvalidConfiguration,
            "The glossary library ID is invalid.",
        ));
    }
    Ok(())
}

// 从规范化数据库行恢复词汇表，并在读取边界重新执行领域校验。
fn load_glossary_record(
    connection: &Connection,
    id: String,
    created_at: i64,
    updated_at: i64,
) -> Result<GlossaryRecord, TranslationError> {
    let mut statement = connection
        .prepare(
            "SELECT source_term, target_term, source_locale, target_locale, case_sensitive, whole_word, immutable, domain, priority, notes, enabled FROM glossary_terms WHERE glossary_id = ?1 ORDER BY term_index ASC",
        )
        .map_err(|error| map_error(&error))?;
    let entries = statement
        .query_map(params![id.as_str()], |row| {
            Ok(GlossaryEntry {
                source_term: row.get(0)?,
                target_term: row.get(1)?,
                source_locale: row.get(2)?,
                target_locale: row.get(3)?,
                case_sensitive: row.get(4)?,
                whole_word: row.get(5)?,
                immutable: row.get(6)?,
                domain: row.get(7)?,
                priority: row.get(8)?,
                notes: row.get(9)?,
                enabled: row.get(10)?,
            })
        })
        .map_err(|error| map_error(&error))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| map_error(&error))?;
    let glossary = Glossary::new(entries).map_err(|error| {
        TranslationError::new(
            ErrorKind::Persistence,
            format!("Stored glossary library is invalid: {error}"),
        )
    })?;
    Ok(GlossaryRecord {
        id,
        glossary,
        created_at,
        updated_at,
    })
}

#[cfg(test)]
mod tests {
    use super::{DocumentJobOptions, INITIAL_MIGRATION, MAX_DOCUMENT_JOBS, MIGRATIONS, Storage};
    use linguamesh_document::{DocumentFormat, DocumentJob, DocumentJobState};
    use linguamesh_domain::{
        ErrorKind, Glossary, GlossaryEntry, ProviderProfile, ProviderProfileId, RoutingCandidate,
        RoutingConstraints, RoutingMode, RoutingProfile, SecretRef, SecretRefNamespace,
        TranslationPreset, TranslationPrivacyMode, TranslationQualityMode, TranslationRequest,
        UsageRecord, UsageSource,
    };
    use rusqlite::{Connection, OpenFlags};
    #[cfg(unix)]
    use std::env;
    use std::fs;
    use std::io::{Cursor, Read, Write};
    #[cfg(target_os = "linux")]
    use std::os::fd::AsRawFd;
    #[cfg(unix)]
    use std::os::unix::fs::symlink;
    #[cfg(unix)]
    use std::path::PathBuf;
    #[cfg(unix)]
    use std::process::Command;
    use tempfile::tempdir;
    use zip::ZipArchive;
    use zip::write::{SimpleFileOptions, ZipWriter};

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
        assert_eq!(storage.schema_version().expect("version"), 34);
        storage.upsert_manual_model("manual-model").expect("insert");
        storage.set_active_model("manual-model").expect("select");
        assert_eq!(
            storage.active_model().expect("active").as_deref(),
            Some("manual-model")
        );
        assert_eq!(storage.manual_models().expect("models").len(), 1);
    }

    #[test]
    fn schema_fifteen_document_options_migrate_with_routing_profile_id() {
        let directory = tempdir().expect("temp directory");
        let path = directory.path().join("schema-fifteen.sqlite3");
        let connection = Connection::open(&path).expect("legacy database");
        for &(version, migration) in MIGRATIONS.iter().take(15) {
            connection
                .execute_batch(migration)
                .expect("legacy migration");
            connection
                .execute(
                    "UPDATE schema_metadata SET version = ?1 WHERE singleton = 1",
                    [version],
                )
                .expect("legacy schema version");
        }
        drop(connection);

        let mut storage = Storage::open(&path).expect("schema 20 migration");
        assert_eq!(storage.schema_version().expect("version"), 34);
        let job = DocumentJob::from_text("route.txt", DocumentFormat::Txt, "one");
        storage
            .save_document_job("route-job", &job, DocumentJobState::Pending)
            .expect("save route job");
        storage
            .connection
            .execute(
                "INSERT INTO document_job_options (job_id, source_locale, target_locale, model_id, provider_id, routing_profile_id, quality_mode, translation_preset_json, glossary_json) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL, NULL)",
                (
                    "route-job",
                    Some("en"),
                    "zh-CN",
                    "fake-translator",
                    "route-provider",
                    Some("route-profile"),
                    "balanced",
                ),
            )
            .expect("insert legacy options");
        let legacy = storage
            .document_job("route-job")
            .expect("load legacy options")
            .expect("legacy job");
        assert_eq!(
            legacy.options.expect("legacy options").translation_preset,
            TranslationPreset::general()
        );
        let options = DocumentJobOptions {
            source_locale: Some("en".to_owned()),
            target_locale: "zh-CN".to_owned(),
            model_id: "fake-translator".to_owned(),
            provider_id: "route-provider".to_owned(),
            routing_profile_id: Some("route-profile".to_owned()),
            quality_mode: TranslationQualityMode::Balanced,
            translation_preset: TranslationPreset::general(),
            glossary: None,
        };
        let snapshot = storage
            .save_document_job_options("route-job", &options)
            .expect("save migrated options");
        assert_eq!(snapshot.options, Some(options));
    }

    #[test]
    fn routing_profiles_round_trip_without_secrets() {
        let mut storage = Storage::in_memory().expect("storage");
        let candidate = RoutingCandidate::new("local", "model", true, 4096).expect("candidate");
        let profile = RoutingProfile::new(
            "safe-routing",
            RoutingMode::Automatic,
            vec![candidate],
            RoutingConstraints {
                local_only: true,
                explicit_fallback_allowed: true,
                ..RoutingConstraints::default()
            },
        )
        .expect("routing profile");
        let saved = storage.save_routing_profile(&profile).expect("save");
        assert_eq!(saved.profile, profile);
        assert_eq!(storage.schema_version().expect("version"), 34);
        assert_eq!(
            storage.routing_profile("safe-routing").expect("read"),
            Some(saved)
        );
        assert_eq!(storage.routing_profiles().expect("list").len(), 1);
        assert!(
            storage
                .delete_routing_profile("safe-routing")
                .expect("delete")
        );
        assert!(
            storage
                .routing_profile("safe-routing")
                .expect("missing")
                .is_none()
        );
    }

    #[test]
    fn glossary_libraries_round_trip_across_reopen_and_delete_terms_atomically() {
        let directory = tempdir().expect("directory");
        let path = directory.path().join("glossaries.sqlite3");
        let mut storage = Storage::open(&path).expect("storage");
        let entry = GlossaryEntry::new("LinguaMesh", "凌瓦网")
            .expect("entry")
            .with_source_locale("en")
            .with_target_locale("zh-CN")
            .with_case_sensitive(false)
            .with_immutable(true)
            .with_domain("product")
            .with_priority(10)
            .with_notes("brand term")
            .with_enabled(true);
        let glossary = Glossary::new(vec![entry]).expect("glossary");
        let saved = storage
            .save_glossary("product-terms", &glossary)
            .expect("save");
        assert_eq!(saved.id, "product-terms");
        assert_eq!(saved.glossary, glossary);
        assert_eq!(storage.schema_version().expect("version"), 34);
        assert_eq!(storage.glossaries().expect("list"), vec![saved.clone()]);
        drop(storage);

        let mut reopened = Storage::open(&path).expect("reopen");
        assert_eq!(
            reopened.glossary("product-terms").expect("read"),
            Some(saved)
        );
        assert!(reopened.delete_glossary("product-terms").expect("delete"));
        assert!(
            reopened
                .glossary("product-terms")
                .expect("missing")
                .is_none()
        );
        let term_count = reopened
            .connection
            .query_row("SELECT COUNT(*) FROM glossary_terms", [], |row| {
                row.get::<_, u32>(0)
            })
            .expect("term count");
        assert_eq!(term_count, 0);
    }

    #[test]
    fn glossary_library_id_and_count_limits_are_enforced() {
        let mut storage = Storage::in_memory().expect("storage");
        let glossary = Glossary::new(vec![]).expect("empty glossary");
        assert!(storage.save_glossary("invalid id", &glossary).is_err());
        for index in 0..super::MAX_GLOSSARIES {
            storage
                .save_glossary(&format!("library-{index}"), &glossary)
                .expect("save glossary");
        }
        assert!(
            storage
                .save_glossary("library-overflow", &glossary)
                .is_err()
        );
    }

    #[test]
    fn document_job_round_trip_resumes_and_completes_across_reopen() {
        let directory = tempdir().expect("directory");
        let path = directory.path().join("document-jobs.sqlite3");
        let mut storage = Storage::open(&path).expect("storage");
        let mut job = DocumentJob::from_text("notes.txt", DocumentFormat::Txt, "one\ntwo");
        let pending = storage
            .save_document_job("job-1", &job, DocumentJobState::Pending)
            .expect("save pending");
        assert_eq!(pending.state, DocumentJobState::Pending);
        storage
            .update_document_segment("job-1", 0, "一")
            .expect("save first segment");
        let resumed = storage.resumable_document_jobs().expect("resumable jobs");
        assert_eq!(resumed.len(), 1);
        assert_eq!(resumed[0].job.pending_count(), 1);
        job.apply_translation(0, "一").expect("local first segment");
        drop(storage);

        let mut reopened = Storage::open(&path).expect("reopened storage");
        let completed = reopened
            .update_document_segment("job-1", 1, "二")
            .expect("complete job");
        assert_eq!(completed.state, DocumentJobState::Completed);
        assert_eq!(completed.job.reconstruct().expect("reconstruct"), "一\n二");
        assert!(
            reopened
                .resumable_document_jobs()
                .expect("no resumable jobs")
                .is_empty()
        );
        assert!(reopened.delete_document_job("job-1").expect("delete job"));
        assert!(reopened.document_job("job-1").expect("lookup").is_none());
    }

    #[test]
    fn csv_document_format_and_encoded_translation_survive_reopen() {
        let directory = tempdir().expect("directory");
        let path = directory.path().join("csv-document-job.sqlite3");
        let mut storage = Storage::open(&path).expect("storage");
        let job = DocumentJob::from_utf8_with_csv_columns(
            "comments.csv",
            b"id,comment\n1,\"Hello, world\"\n",
            Some(&[1]),
        )
        .expect("csv job");
        storage
            .save_document_job("csv-job", &job, DocumentJobState::Pending)
            .expect("save csv job");
        let comment = job
            .segments
            .iter()
            .position(|segment| segment.source_text.starts_with("\"Hello"))
            .expect("comment segment");
        let header = job
            .segments
            .iter()
            .position(|segment| segment.source_text == "comment")
            .expect("header segment");
        storage
            .update_document_segment("csv-job", comment, "译文, 世界")
            .expect("translate csv field");
        storage
            .update_document_segment("csv-job", header, "comment")
            .expect("translate csv header");
        drop(storage);

        let reopened = Storage::open(&path).expect("reopen storage");
        let snapshot = reopened
            .document_job("csv-job")
            .expect("load csv job")
            .expect("csv snapshot");
        assert_eq!(snapshot.job.format, DocumentFormat::Csv);
        assert_eq!(
            snapshot.job.reconstruct().expect("reconstruct csv"),
            "id,comment\n1,\"译文, 世界\"\n"
        );
    }

    #[test]
    fn json_document_format_and_encoded_translation_survive_reopen() {
        let directory = tempdir().expect("directory");
        let path = directory.path().join("json-document-job.sqlite3");
        let mut storage = Storage::open(&path).expect("storage");
        let job = DocumentJob::from_utf8("payload.json", br#"{"name":"Alice","count":2}"#)
            .expect("json job");
        storage
            .save_document_job("json-job", &job, DocumentJobState::Pending)
            .expect("save json job");
        let name = job
            .segments
            .iter()
            .position(|segment| segment.source_text == "\"Alice\"")
            .expect("name segment");
        storage
            .update_document_segment("json-job", name, "爱丽丝")
            .expect("translate json value");
        drop(storage);

        let reopened = Storage::open(&path).expect("reopen storage");
        let snapshot = reopened
            .document_job("json-job")
            .expect("load json job")
            .expect("json snapshot");
        assert_eq!(snapshot.job.format, DocumentFormat::Json);
        assert_eq!(
            snapshot.job.reconstruct().expect("reconstruct json"),
            r#"{"name":"爱丽丝","count":2}"#
        );
    }

    #[test]
    fn html_document_format_and_encoded_translation_survive_reopen() {
        let directory = tempdir().expect("directory");
        let path = directory.path().join("html-document-job.sqlite3");
        let mut storage = Storage::open(&path).expect("storage");
        let job = DocumentJob::from_utf8("page.html", b"<p>Hello</p>").expect("html job");
        storage
            .save_document_job("html-job", &job, DocumentJobState::Pending)
            .expect("save html job");
        let text = job
            .segments
            .iter()
            .position(|segment| segment.source_text == "Hello")
            .expect("html text segment");
        storage
            .update_document_segment("html-job", text, "Hello <safe>")
            .expect("translate html text");
        drop(storage);

        let reopened = Storage::open(&path).expect("reopen storage");
        let snapshot = reopened
            .document_job("html-job")
            .expect("load html job")
            .expect("html snapshot");
        assert_eq!(snapshot.job.format, DocumentFormat::Html);
        assert_eq!(
            snapshot.job.reconstruct().expect("reconstruct html"),
            "<p>Hello &lt;safe&gt;</p>"
        );
    }

    #[test]
    fn docx_package_and_segments_survive_reopen() {
        let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
        let options = SimpleFileOptions::default();
        writer
            .start_file("[Content_Types].xml", options)
            .expect("content types");
        writer.write_all(b"<Types/>").expect("content types bytes");
        writer
            .start_file("word/document.xml", options)
            .expect("document");
        writer
            .write_all(br#"<w:document xmlns:w="urn:w"><w:body><w:p><w:r><w:t>Hello</w:t></w:r></w:p></w:body></w:document>"#)
            .expect("document bytes");
        writer
            .start_file("word/media/image.bin", options)
            .expect("image");
        writer.write_all(&[7, 8, 9]).expect("image bytes");
        let package = writer.finish().expect("package").into_inner();

        let directory = tempdir().expect("directory");
        let path = directory.path().join("docx-document-job.sqlite3");
        let mut storage = Storage::open(&path).expect("storage");
        let job = DocumentJob::from_utf8("sample.docx", &package).expect("docx job");
        storage
            .save_document_job("docx-job", &job, DocumentJobState::Pending)
            .expect("save docx job");
        let text = job
            .segments
            .iter()
            .position(|segment| segment.source_text == "Hello")
            .expect("docx text segment");
        storage
            .update_document_segment("docx-job", text, "你好")
            .expect("translate docx text");
        drop(storage);

        let reopened = Storage::open(&path).expect("reopen storage");
        let snapshot = reopened
            .document_job("docx-job")
            .expect("load docx job")
            .expect("docx snapshot");
        assert_eq!(snapshot.job.format, DocumentFormat::Docx);
        let rebuilt = snapshot.job.reconstruct_bytes().expect("reconstruct docx");
        let mut archive = ZipArchive::new(Cursor::new(rebuilt)).expect("rebuilt archive");
        let mut document = String::new();
        archive
            .by_name("word/document.xml")
            .expect("document entry")
            .read_to_string(&mut document)
            .expect("document xml");
        assert!(document.contains("你好"));
        let mut image = Vec::new();
        archive
            .by_name("word/media/image.bin")
            .expect("image entry")
            .read_to_end(&mut image)
            .expect("image bytes");
        assert_eq!(image, [7, 8, 9]);
    }

    #[test]
    fn pptx_package_and_segments_survive_reopen() {
        let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
        let options = SimpleFileOptions::default();
        writer
            .start_file("[Content_Types].xml", options)
            .expect("content types");
        writer.write_all(b"<Types/>").expect("content types bytes");
        writer
            .start_file("ppt/presentation.xml", options)
            .expect("presentation");
        writer
            .write_all(b"<p:presentation xmlns:p=\"urn:ppt\"/>")
            .expect("presentation bytes");
        writer
            .start_file("ppt/slides/slide1.xml", options)
            .expect("slide");
        writer
            .write_all(br#"<p:sld xmlns:p="urn:ppt" xmlns:a="urn:dml"><a:p><a:r><a:t>Hello</a:t></a:r></a:p></p:sld>"#)
            .expect("slide bytes");
        writer
            .start_file("ppt/media/image.bin", options)
            .expect("image");
        writer.write_all(&[4, 5, 6]).expect("image bytes");
        let package = writer.finish().expect("package").into_inner();

        let directory = tempdir().expect("directory");
        let path = directory.path().join("pptx-document-job.sqlite3");
        let mut storage = Storage::open(&path).expect("storage");
        let job = DocumentJob::from_utf8("sample.pptx", &package).expect("pptx job");
        storage
            .save_document_job("pptx-job", &job, DocumentJobState::Pending)
            .expect("save pptx job");
        let text = job
            .segments
            .iter()
            .position(|segment| segment.source_text == "Hello")
            .expect("pptx text segment");
        storage
            .update_document_segment("pptx-job", text, "你好")
            .expect("translate pptx text");
        drop(storage);

        let reopened = Storage::open(&path).expect("reopen storage");
        let snapshot = reopened
            .document_job("pptx-job")
            .expect("load pptx job")
            .expect("pptx snapshot");
        assert_eq!(snapshot.job.format, DocumentFormat::Pptx);
        let rebuilt = snapshot.job.reconstruct_bytes().expect("reconstruct pptx");
        let mut archive = ZipArchive::new(Cursor::new(rebuilt)).expect("rebuilt archive");
        let mut slide = String::new();
        archive
            .by_name("ppt/slides/slide1.xml")
            .expect("slide entry")
            .read_to_string(&mut slide)
            .expect("slide xml");
        assert!(slide.contains("你好"));
        let mut image = Vec::new();
        archive
            .by_name("ppt/media/image.bin")
            .expect("image entry")
            .read_to_end(&mut image)
            .expect("image bytes");
        assert_eq!(image, [4, 5, 6]);
    }

    #[test]
    fn xlsx_package_and_segments_survive_reopen() {
        let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
        let options = SimpleFileOptions::default();
        writer
            .start_file("[Content_Types].xml", options)
            .expect("content types");
        writer.write_all(b"<Types/>").expect("content types bytes");
        writer
            .start_file("xl/workbook.xml", options)
            .expect("workbook");
        writer.write_all(b"<workbook/>").expect("workbook bytes");
        writer
            .start_file("xl/sharedStrings.xml", options)
            .expect("shared strings");
        writer
            .write_all(br"<sst><si><t>Hello</t></si></sst>")
            .expect("shared strings bytes");
        writer
            .start_file("xl/worksheets/sheet1.xml", options)
            .expect("worksheet");
        writer
            .write_all(br#"<worksheet><sheetData><row><c t="s"><v>0</v></c></row></sheetData></worksheet>"#)
            .expect("worksheet bytes");
        writer
            .start_file("xl/media/image.bin", options)
            .expect("image");
        writer.write_all(&[11, 12, 13]).expect("image bytes");
        let package = writer.finish().expect("package").into_inner();

        let directory = tempdir().expect("directory");
        let path = directory.path().join("xlsx-document-job.sqlite3");
        let mut storage = Storage::open(&path).expect("storage");
        let job = DocumentJob::from_utf8("sample.xlsx", &package).expect("xlsx job");
        storage
            .save_document_job("xlsx-job", &job, DocumentJobState::Pending)
            .expect("save xlsx job");
        let text = job
            .segments
            .iter()
            .position(|segment| segment.source_text == "Hello")
            .expect("xlsx text segment");
        storage
            .update_document_segment("xlsx-job", text, "你好")
            .expect("translate xlsx text");
        drop(storage);

        let reopened = Storage::open(&path).expect("reopen storage");
        let snapshot = reopened
            .document_job("xlsx-job")
            .expect("load xlsx job")
            .expect("xlsx snapshot");
        assert_eq!(snapshot.job.format, DocumentFormat::Xlsx);
        let rebuilt = snapshot.job.reconstruct_bytes().expect("reconstruct xlsx");
        let mut archive = ZipArchive::new(Cursor::new(rebuilt)).expect("rebuilt archive");
        let mut shared_strings = String::new();
        archive
            .by_name("xl/sharedStrings.xml")
            .expect("shared strings entry")
            .read_to_string(&mut shared_strings)
            .expect("shared strings xml");
        assert!(shared_strings.contains("你好"));
        let mut image = Vec::new();
        archive
            .by_name("xl/media/image.bin")
            .expect("image entry")
            .read_to_end(&mut image)
            .expect("image bytes");
        assert_eq!(image, [11, 12, 13]);
    }

    #[test]
    fn epub_package_and_segments_survive_reopen() {
        let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
        let options = SimpleFileOptions::default();
        writer.start_file("mimetype", options).expect("mimetype");
        writer
            .write_all(b"application/epub+zip")
            .expect("mimetype bytes");
        writer
            .start_file("META-INF/container.xml", options)
            .expect("container");
        writer
            .write_all(br#"<container xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="OEBPS/book.opf"/></rootfiles></container>"#)
            .expect("container bytes");
        writer.start_file("OEBPS/book.opf", options).expect("opf");
        writer
            .write_all(br#"<package xmlns="http://www.idpf.org/2007/opf" xmlns:dc="http://purl.org/dc/elements/1.1/"><metadata><dc:language>en</dc:language></metadata><manifest/><spine/></package>"#)
            .expect("opf bytes");
        writer
            .start_file("OEBPS/chapter.xhtml", options)
            .expect("chapter");
        writer
            .write_all(
                br#"<html xmlns="http://www.w3.org/1999/xhtml"><body><p>Hello</p></body></html>"#,
            )
            .expect("chapter bytes");
        let package = writer.finish().expect("package").into_inner();

        let directory = tempdir().expect("directory");
        let path = directory.path().join("epub-document-job.sqlite3");
        let mut storage = Storage::open(&path).expect("storage");
        let job = DocumentJob::from_utf8("book.epub", &package).expect("epub job");
        storage
            .save_document_job("epub-job", &job, DocumentJobState::Pending)
            .expect("save epub job");
        storage
            .update_document_segment("epub-job", 0, "你好")
            .expect("translate epub text");
        drop(storage);

        let reopened = Storage::open(&path).expect("reopen storage");
        let snapshot = reopened
            .document_job("epub-job")
            .expect("load epub job")
            .expect("epub snapshot");
        assert_eq!(snapshot.job.format, DocumentFormat::Epub);
        let rebuilt = snapshot
            .job
            .reconstruct_bytes_with_target_locale(Some("zh-CN"))
            .expect("reconstruct epub");
        let mut archive = ZipArchive::new(Cursor::new(rebuilt)).expect("rebuilt archive");
        let mut chapter = String::new();
        archive
            .by_name("OEBPS/chapter.xhtml")
            .expect("chapter entry")
            .read_to_string(&mut chapter)
            .expect("chapter xml");
        assert!(chapter.contains("你好"));
    }

    #[test]
    fn pdf_package_and_page_segments_survive_reopen() {
        let package = br"%PDF-1.4
1 0 obj
<< /Type /Catalog /Pages 2 0 R >>
endobj
2 0 obj
<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>
endobj
3 0 obj
<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 5 0 R >>
endobj
4 0 obj
<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 6 0 R >>
endobj
5 0 obj
<< /Length 30 >>
stream
BT
72 720 Td
(Hello) Tj
ET
endstream
endobj
6 0 obj
<< /Length 32 >>
stream
BT
72 720 Td
(Second) Tj
ET
endstream
endobj
trailer
<< /Root 1 0 R >>
%%EOF
";
        let directory = tempdir().expect("directory");
        let path = directory.path().join("pdf-document-job.sqlite3");
        let mut storage = Storage::open(&path).expect("storage");
        let job = DocumentJob::from_utf8("book.pdf", package).expect("pdf job");
        storage
            .save_document_job("pdf-job", &job, DocumentJobState::Pending)
            .expect("save pdf job");
        storage
            .update_document_segment("pdf-job", 0, "Bonjour")
            .expect("translate first page");
        storage
            .update_document_segment("pdf-job", 1, "Deuxieme")
            .expect("translate second page");
        drop(storage);

        let reopened = Storage::open(&path).expect("reopen storage");
        let snapshot = reopened
            .document_job("pdf-job")
            .expect("load pdf job")
            .expect("pdf snapshot");
        assert_eq!(snapshot.job.format, DocumentFormat::Pdf);
        let rebuilt = snapshot.job.reconstruct_bytes().expect("reconstruct pdf");
        assert!(rebuilt.starts_with(b"%PDF-1.4"));
        assert!(
            rebuilt
                .windows(b"(Bonjour)".len())
                .any(|window| window == b"(Bonjour)")
        );
        assert!(
            rebuilt
                .windows(b"(Deuxieme)".len())
                .any(|window| window == b"(Deuxieme)")
        );
    }

    #[test]
    fn paused_document_job_survives_reopen_and_is_resumable() {
        let directory = tempdir().expect("directory");
        let path = directory.path().join("paused-document-job.sqlite3");
        let mut storage = Storage::open(&path).expect("storage");
        let job = DocumentJob::from_text("notes.txt", DocumentFormat::Txt, "one\ntwo");
        storage
            .save_document_job("job-paused", &job, DocumentJobState::Pending)
            .expect("save pending");
        let paused = storage
            .set_document_job_state("job-paused", DocumentJobState::Paused)
            .expect("pause job");
        assert_eq!(paused.state, DocumentJobState::Paused);
        drop(storage);

        let reopened = Storage::open(&path).expect("reopened storage");
        let resumable = reopened.resumable_document_jobs().expect("resumable jobs");
        assert_eq!(resumable.len(), 1);
        assert_eq!(resumable[0].state, DocumentJobState::Paused);
    }

    #[test]
    fn document_job_options_round_trip_without_provider_secrets() {
        let directory = tempdir().expect("directory");
        let path = directory.path().join("document-job-options.sqlite3");
        let mut storage = Storage::open(&path).expect("storage");
        let job = DocumentJob::from_text("notes.txt", DocumentFormat::Txt, "one");
        storage
            .save_document_job("job-options", &job, DocumentJobState::Pending)
            .expect("save pending");
        let glossary =
            Glossary::new(vec![GlossaryEntry::new("one", "一").expect("entry")]).expect("glossary");
        let options = DocumentJobOptions {
            source_locale: Some("en".to_owned()),
            target_locale: "zh-CN".to_owned(),
            model_id: "model-options".to_owned(),
            provider_id: "legacy-profile".to_owned(),
            routing_profile_id: Some("document-route".to_owned()),
            quality_mode: TranslationQualityMode::Balanced,
            translation_preset: TranslationPreset::technical(),
            glossary: Some(glossary),
        };
        let saved = storage
            .save_document_job_options("job-options", &options)
            .expect("save options");
        assert_eq!(saved.options, Some(options.clone()));
        drop(storage);

        let reopened = Storage::open(&path).expect("reopened storage");
        let loaded = reopened
            .document_job("job-options")
            .expect("load job")
            .expect("job");
        assert_eq!(loaded.options, Some(options));
        let mut statement = reopened
            .connection
            .prepare("SELECT routing_profile_id, translation_preset_json, glossary_json FROM document_job_options")
            .expect("options query");
        let (routing_profile_id, translation_preset_json, glossary_json): (
            Option<String>,
            String,
            String,
        ) = statement
            .query_row([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .expect("glossary row");
        assert_eq!(routing_profile_id.as_deref(), Some("document-route"));
        assert!(translation_preset_json.contains("technical"));
        assert!(!translation_preset_json.contains("credential"));
        assert!(!glossary_json.contains("api_key"));
        assert!(!glossary_json.contains("credential"));
    }

    #[test]
    fn document_job_options_reject_invalid_translation_preset() {
        let mut storage = Storage::in_memory().expect("storage");
        let job = DocumentJob::from_text("notes.txt", DocumentFormat::Txt, "one");
        storage
            .save_document_job("job-invalid-preset", &job, DocumentJobState::Pending)
            .expect("save pending");
        let options = DocumentJobOptions {
            source_locale: Some("en".to_owned()),
            target_locale: "zh-CN".to_owned(),
            model_id: "model-options".to_owned(),
            provider_id: "legacy-profile".to_owned(),
            routing_profile_id: None,
            quality_mode: TranslationQualityMode::Balanced,
            translation_preset: TranslationPreset {
                id: "general".to_owned(),
                custom_instructions: Some(concat!("s", "k-LM_DOCUMENT_PRESET_SECRET").to_owned()),
                ..TranslationPreset::general()
            },
            glossary: None,
        };
        let error = storage
            .save_document_job_options("job-invalid-preset", &options)
            .expect_err("credential-shaped preset");
        assert_eq!(error.kind, ErrorKind::InvalidConfiguration);
    }

    #[test]
    fn document_jobs_are_bounded_and_do_not_store_paths_or_credentials() {
        let mut storage = Storage::in_memory().expect("storage");
        let job = DocumentJob::from_text("notes.txt", DocumentFormat::Txt, "one");
        for index in 0..MAX_DOCUMENT_JOBS {
            storage
                .save_document_job(&format!("job-{index}"), &job, DocumentJobState::Pending)
                .expect("bounded job");
        }
        let error = storage
            .save_document_job("overflow", &job, DocumentJobState::Pending)
            .expect_err("job limit");
        assert_eq!(error.kind, ErrorKind::InvalidConfiguration);
        let mut statement = storage
            .connection
            .prepare("SELECT name, sql FROM sqlite_master WHERE type = 'table'")
            .expect("schema query");
        let schema = statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .expect("schema rows")
            .collect::<Result<Vec<_>, _>>()
            .expect("schema");
        assert!(schema.iter().all(|(_, sql)| {
            !sql.to_ascii_lowercase().contains("api_key")
                && !sql.to_ascii_lowercase().contains("credential_value")
        }));
    }

    #[test]
    fn translation_history_respects_incognito_and_clear_controls() {
        let mut storage = Storage::in_memory().expect("storage");
        let request = TranslationRequest::new("Hello", "zh-CN", "fake-translator");
        storage
            .record_translation_history(&request, "你好")
            .expect("history");
        assert_eq!(storage.translation_history_count().expect("count"), 1);

        let incognito = TranslationRequest::new("Private", "zh-CN", "fake-translator")
            .with_privacy_mode(TranslationPrivacyMode::Incognito);
        storage
            .record_translation_history(&incognito, "私密")
            .expect("incognito history is skipped");
        assert_eq!(storage.translation_history_count().expect("count"), 1);

        storage.clear_translation_history().expect("clear history");
        assert_eq!(storage.translation_history_count().expect("count"), 0);
        assert_eq!(storage.usage_record_count().expect("usage count"), 0);
    }

    #[test]
    fn usage_records_round_trip_without_endpoints_and_follow_history_controls() {
        let mut storage = Storage::in_memory().expect("storage");
        let request = TranslationRequest::new("Hello", "zh-CN", "fake-translator")
            .with_provider_identity("profile-a@https://provider.invalid/v1/");
        let usage = UsageRecord::provider_reported(Some(8), Some(3), Some(11));
        storage
            .record_translation_history_with_usage(&request, "你好", Some(&usage))
            .expect("history and usage");

        let entries = storage.usage_records(10).expect("usage entries");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].provider_id.as_deref(), Some("profile-a"));
        assert_eq!(entries[0].model_id, "fake-translator");
        assert_eq!(entries[0].usage, usage);
        assert!(
            storage.usage_records(10).expect("usage entries")[0]
                .usage
                .source
                == UsageSource::ProviderReported
        );

        storage
            .delete_translation_history_entry(request.operation_id.as_str())
            .expect("delete history");
        assert_eq!(storage.usage_record_count().expect("usage count"), 0);
    }

    #[test]
    fn usage_records_skip_incognito_and_clear_with_history() {
        let mut storage = Storage::in_memory().expect("storage");
        let request = TranslationRequest::new("Hello", "zh-CN", "model")
            .with_provider_identity("profile-a@model");
        let usage = UsageRecord::locally_estimated("Hello", "你好");
        storage
            .record_translation_history_with_usage(&request, "你好", Some(&usage))
            .expect("usage");
        let incognito = TranslationRequest::new("Private", "zh-CN", "model")
            .with_privacy_mode(TranslationPrivacyMode::Incognito);
        storage
            .record_translation_history_with_usage(&incognito, "私密", Some(&usage))
            .expect("incognito usage is skipped");
        assert_eq!(storage.usage_record_count().expect("usage count"), 1);
        storage.clear_translation_history().expect("clear history");
        assert_eq!(storage.usage_record_count().expect("usage count"), 0);
    }

    #[test]
    fn translation_history_policy_persists_without_deleting_existing_entries() {
        let directory = tempfile::tempdir().expect("directory");
        let database_path = directory.path().join("history-policy.sqlite3");
        let mut storage = Storage::open(&database_path).expect("storage");
        let mut first = TranslationRequest::new("hello", "zh-CN", "model");
        first.operation_id = linguamesh_domain::OperationId::from_value("policy-operation");
        storage
            .record_translation_history(&first, "你好")
            .expect("record history");
        assert!(storage.translation_history_enabled().expect("policy"));

        storage
            .set_translation_history_enabled(false)
            .expect("disable history");
        let mut second = TranslationRequest::new("second", "zh-CN", "model");
        second.operation_id = linguamesh_domain::OperationId::from_value("disabled-operation");
        storage
            .record_translation_history(&second, "第二条")
            .expect("disabled record");
        assert_eq!(storage.translation_history_count().expect("count"), 1);
        assert_eq!(
            storage.translation_history(10).expect("entries")[0].operation_id,
            "policy-operation"
        );
        drop(storage);

        let mut reopened = Storage::open(&database_path).expect("reopened storage");
        assert!(
            !reopened
                .translation_history_enabled()
                .expect("persisted policy")
        );
        reopened
            .set_translation_history_enabled(true)
            .expect("enable history");
        let mut third = TranslationRequest::new("third", "zh-CN", "model");
        third.operation_id = linguamesh_domain::OperationId::from_value("enabled-operation");
        reopened
            .record_translation_history(&third, "第三条")
            .expect("enabled record");
        assert_eq!(reopened.translation_history_count().expect("count"), 2);
    }

    #[test]
    fn translation_history_is_bounded_and_rejects_oversized_text() {
        let mut storage = Storage::in_memory().expect("storage");
        for index in 0..(super::MAX_TRANSLATION_HISTORY_ENTRIES + 3) {
            let request =
                TranslationRequest::new(format!("source-{index}"), "zh-CN", "fake-translator");
            storage
                .record_translation_history(&request, "translated")
                .expect("history");
        }
        assert_eq!(
            storage.translation_history_count().expect("count"),
            super::MAX_TRANSLATION_HISTORY_ENTRIES
        );

        let request = TranslationRequest::new("oversized", "zh-CN", "fake-translator");
        let oversized = "x".repeat(super::MAX_TRANSLATION_HISTORY_TEXT_BYTES + 1);
        let error = storage
            .record_translation_history(&request, &oversized)
            .expect_err("oversized history");
        assert_eq!(error.kind, ErrorKind::InvalidConfiguration);
    }

    #[test]
    fn translation_history_lists_newest_entries_and_deletes_one_entry() {
        let mut storage = Storage::in_memory().expect("storage");
        let mut first = TranslationRequest::new("first", "zh-CN", "model-a");
        first.operation_id = linguamesh_domain::OperationId::from_value("operation-a");
        let mut second = TranslationRequest::new("second", "zh-CN", "model-b");
        second.operation_id = linguamesh_domain::OperationId::from_value("operation-b");
        storage
            .record_translation_history(&first, "第一条")
            .expect("first history");
        storage
            .record_translation_history(&second, "第二条")
            .expect("second history");

        let entries = storage.translation_history(10).expect("list history");
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].operation_id, "operation-b");
        assert_eq!(entries[0].source_text, "second");
        assert!(entries[0].created_at > 0);
        assert!(
            storage
                .delete_translation_history_entry("operation-a")
                .expect("delete history")
        );
        assert!(
            !storage
                .delete_translation_history_entry("missing")
                .expect("missing delete")
        );
        assert_eq!(storage.translation_history_count().expect("count"), 1);
    }

    #[test]
    fn translation_memory_reuses_only_matching_request_identity() {
        let mut storage = Storage::in_memory().expect("storage");
        let request = TranslationRequest::new("  Hello   world\n", "zh-CN", "model-a")
            .with_max_chunk_bytes(2048);
        storage
            .record_translation_memory(&request, "你好世界")
            .expect("record memory");
        let hit = storage
            .lookup_translation_memory(&request)
            .expect("lookup memory")
            .expect("matching memory");
        assert_eq!(hit.translated_text, "你好世界");
        assert!(hit.identity_json.contains("Hello world"));

        let different_model =
            TranslationRequest::new("Hello world", "zh-CN", "model-b").with_max_chunk_bytes(2048);
        assert!(
            storage
                .lookup_translation_memory(&different_model)
                .expect("different model lookup")
                .is_none()
        );
        let different_chunking =
            TranslationRequest::new("Hello world", "zh-CN", "model-a").with_max_chunk_bytes(4096);
        assert!(
            storage
                .lookup_translation_memory(&different_chunking)
                .expect("different chunk lookup")
                .is_none()
        );
    }

    #[test]
    fn translation_memory_policy_and_controls_persist_without_leaking_incognito() {
        let directory = tempfile::tempdir().expect("directory");
        let database_path = directory.path().join("translation-memory.sqlite3");
        let mut storage = Storage::open(&database_path).expect("storage");
        let request = TranslationRequest::new("hello", "zh-CN", "model");
        storage
            .record_translation_memory(&request, "你好")
            .expect("record memory");
        assert_eq!(storage.translation_memory_count().expect("count"), 1);
        assert!(storage.translation_memory_enabled().expect("policy"));
        storage
            .set_translation_memory_enabled(false)
            .expect("disable memory");
        let disabled = TranslationRequest::new("disabled", "zh-CN", "model");
        storage
            .record_translation_memory(&disabled, "禁用")
            .expect("disabled record");
        assert_eq!(storage.translation_memory_count().expect("count"), 1);
        assert!(
            storage
                .lookup_translation_memory(&disabled)
                .expect("disabled lookup")
                .is_none()
        );
        let incognito = TranslationRequest::new("private", "zh-CN", "model")
            .with_privacy_mode(TranslationPrivacyMode::Incognito);
        storage
            .set_translation_memory_enabled(true)
            .expect("enable memory");
        storage
            .record_translation_memory(&incognito, "私密")
            .expect("incognito record");
        assert_eq!(storage.translation_memory_count().expect("count"), 1);
        let entries = storage.translation_memory(10).expect("list memory");
        assert_eq!(entries.len(), 1);
        assert!(
            storage
                .delete_translation_memory_entry(&entries[0].cache_key)
                .expect("delete memory")
        );
        assert_eq!(storage.translation_memory_count().expect("count"), 0);
        storage
            .record_translation_memory(&request, "你好")
            .expect("record again");
        storage.clear_translation_memory().expect("clear memory");
        assert_eq!(storage.translation_memory_count().expect("count"), 0);
        drop(storage);
        let reopened = Storage::open(&database_path).expect("reopened storage");
        assert!(reopened.translation_memory_enabled().expect("policy"));
    }

    #[cfg(unix)]
    #[test]
    fn symbolic_link_database_is_rejected_before_migration() {
        let directory = tempdir().expect("temp directory");
        let target = directory.path().join("target.sqlite3");
        let link = directory.path().join("link.sqlite3");
        Connection::open(&target).expect("target database");
        symlink(&target, &link).expect("database symbolic link");

        let Err(error) = Storage::open(&link) else {
            panic!("symbolic link was accepted");
        };
        assert_eq!(error.kind, ErrorKind::Persistence);
        let connection = Connection::open(&target).expect("target database inspection");
        let table_count = connection
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table'",
                [],
                |row| row.get::<_, u32>(0),
            )
            .expect("target schema count");
        assert_eq!(table_count, 0);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn trusted_descriptor_path_requires_proc_self_fd() {
        let directory = tempdir().expect("temp directory");
        let path = directory.path().join("state.sqlite3");
        let file = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&path)
            .expect("database file");
        let descriptor_path = PathBuf::from(format!("/proc/self/fd/{}", file.as_raw_fd()));
        let storage = Storage::open_from_trusted_descriptor(&descriptor_path).expect("storage");
        assert_eq!(storage.schema_version().expect("schema version"), 34);
        assert!(matches!(
            Storage::open_from_trusted_descriptor(&path),
            Err(error) if error.kind == ErrorKind::InvalidConfiguration
        ));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn unix_exclusive_vfs_preserves_migrations_and_committed_profiles() {
        let directory = tempdir().expect("temp directory");
        let path = directory.path().join("unix-exclusive.sqlite3");
        let mut storage = Storage::open_with_vfs(&path, "unix-excl").expect("storage");
        assert_eq!(storage.schema_version().expect("schema version"), 34);
        storage
            .upsert_provider_profile(&profile(
                "unix-exclusive-provider",
                Some(PERSISTENT_SECRET_REF),
                Some("unix-exclusive-model"),
            ))
            .expect("profile");
        drop(storage);

        let reopened = Storage::open_with_vfs(&path, "unix-excl").expect("reopened storage");
        let profile_id = ProviderProfileId::parse("unix-exclusive-provider").expect("profile id");
        assert_eq!(
            reopened
                .provider_profile(&profile_id)
                .expect("profile lookup")
                .expect("saved profile")
                .selected_model()
                .map(str::to_owned),
            Some("unix-exclusive-model".to_owned())
        );

        let target = directory.path().join("symlink-target.sqlite3");
        let link = directory.path().join("symlink-alias.sqlite3");
        Connection::open(&target).expect("symlink target database");
        symlink(&target, &link).expect("database symbolic link");
        assert!(matches!(
            Storage::open_with_vfs(&link, "unix-excl"),
            Err(error) if error.kind == ErrorKind::Persistence
        ));

        let real_parent = directory.path().join("real-parent");
        let parent_alias = directory.path().join("parent-alias");
        fs::create_dir(&real_parent).expect("real parent directory");
        symlink(&real_parent, &parent_alias).expect("parent symbolic link");
        let nested_path = parent_alias.join("nested.sqlite3");
        assert!(matches!(
            Storage::open_with_vfs(&nested_path, "unix-excl"),
            Err(error) if error.kind == ErrorKind::Persistence
        ));
        assert!(!real_parent.join("nested.sqlite3").exists());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn unix_dotfile_vfs_fails_closed_without_required_wal() {
        let directory = tempdir().expect("temp directory");
        let path = directory.path().join("unix-dotfile.sqlite3");
        let Err(error) = Storage::open_with_vfs(&path, "unix-dotfile") else {
            panic!("unix-dotfile VFS unexpectedly bypassed the required WAL mode");
        };
        assert_eq!(error.kind, ErrorKind::Persistence);
        let connection = Connection::open(&path).expect("inspect rejected database");
        let table_count = connection
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table'",
                [],
                |row| row.get::<_, u32>(0),
            )
            .expect("schema count");
        assert_eq!(table_count, 0);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn unix_none_vfs_fails_closed_without_required_wal() {
        let directory = tempdir().expect("temp directory");
        let path = directory.path().join("unix-none.sqlite3");
        let Err(error) = Storage::open_with_vfs(&path, "unix-none") else {
            panic!("unix-none VFS unexpectedly bypassed the required WAL mode");
        };
        assert_eq!(error.kind, ErrorKind::Persistence);
        let connection = Connection::open(&path).expect("inspect rejected database");
        let table_count = connection
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table'",
                [],
                |row| row.get::<_, u32>(0),
            )
            .expect("schema count");
        assert_eq!(table_count, 0);
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
        assert_eq!(synchronous, 2);
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
        assert_eq!(storage.schema_version().expect("version"), 34);
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
        assert_eq!(storage.schema_version().expect("version"), 34);
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
    fn wal_replay_preserves_committed_profile_after_writer_disconnect() {
        let directory = tempdir().expect("temp directory");
        let path = directory.path().join("wal-replay.sqlite3");
        let profile = profile(
            "wal-replay-profile",
            Some(PERSISTENT_SECRET_REF),
            Some("wal-model"),
        );
        let mut storage = Storage::open(&path).expect("storage");
        let reader = Connection::open(&path).expect("reader connection");
        reader.execute_batch("BEGIN").expect("reader transaction");
        reader
            .query_row("SELECT COUNT(*) FROM provider_profiles", [], |row| {
                row.get::<_, u32>(0)
            })
            .expect("reader snapshot");

        storage
            .save_and_activate_provider(&profile)
            .expect("committed profile");
        drop(storage);
        assert!(path.with_extension("sqlite3-wal").is_file());

        drop(reader);
        let reopened = Storage::open(&path).expect("replayed storage");
        let restored = reopened
            .provider_profile(profile.id())
            .expect("profile query")
            .expect("replayed profile");
        assert_eq!(restored.selected_model(), Some("wal-model"));
        assert_eq!(restored.secret_ref(), profile.secret_ref());
    }

    #[cfg(unix)]
    #[test]
    fn wal_replay_survives_process_termination_after_commit() {
        const CHILD_PATH_ENV: &str = "LINGUAMESH_WAL_CRASH_CHILD_PATH";

        if let Some(child_path) = env::var_os(CHILD_PATH_ENV) {
            let path = PathBuf::from(child_path);
            let profile = profile(
                "wal-crash-profile",
                Some(PERSISTENT_SECRET_REF),
                Some("wal-crash-model"),
            );
            let mut storage = Storage::open(&path).expect("child storage");
            let reader = Connection::open(&path).expect("child reader");
            reader
                .execute_batch("BEGIN")
                .expect("child reader transaction");
            reader
                .query_row("SELECT COUNT(*) FROM provider_profiles", [], |row| {
                    row.get::<_, u32>(0)
                })
                .expect("child reader snapshot");
            storage
                .save_and_activate_provider(&profile)
                .expect("child committed profile");
            assert!(path.with_extension("sqlite3-wal").is_file());
            std::process::abort();
        }

        let directory = tempdir().expect("temp directory");
        let path = directory.path().join("wal-crash.sqlite3");
        let child = Command::new(std::env::current_exe().expect("test executable"))
            .args([
                "--exact",
                "tests::wal_replay_survives_process_termination_after_commit",
                "--nocapture",
            ])
            .env(CHILD_PATH_ENV, &path)
            .status()
            .expect("spawn crash child");
        assert!(!child.success(), "crash child unexpectedly completed");

        let reopened = Storage::open(&path).expect("recovered storage");
        let profile_id = ProviderProfileId::parse("wal-crash-profile").expect("profile id");
        let restored = reopened
            .provider_profile(&profile_id)
            .expect("recovered profile query")
            .expect("recovered profile");
        assert_eq!(restored.selected_model(), Some("wal-crash-model"));
        assert_eq!(
            restored.secret_ref().map(SecretRef::as_str),
            Some(PERSISTENT_SECRET_REF)
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn unix_exclusive_vfs_wal_replay_survives_process_termination_after_commit() {
        const CHILD_PATH_ENV: &str = "LINGUAMESH_UNIX_EXCL_WAL_CRASH_CHILD_PATH";

        if let Some(child_path) = env::var_os(CHILD_PATH_ENV) {
            let path = PathBuf::from(child_path);
            let profile = profile(
                "unix-exclusive-wal-crash-profile",
                Some(PERSISTENT_SECRET_REF),
                Some("unix-exclusive-wal-crash-model"),
            );
            let mut storage = Storage::open_with_vfs(&path, "unix-excl").expect("child storage");
            let reader = Connection::open_with_flags_and_vfs(
                &path,
                OpenFlags::default() | OpenFlags::SQLITE_OPEN_NOFOLLOW,
                "unix-excl",
            )
            .expect("child reader");
            reader
                .execute_batch("BEGIN")
                .expect("child reader transaction");
            reader
                .query_row("SELECT COUNT(*) FROM provider_profiles", [], |row| {
                    row.get::<_, u32>(0)
                })
                .expect("child reader snapshot");
            storage
                .save_and_activate_provider(&profile)
                .expect("child committed profile");
            assert!(path.with_extension("sqlite3-wal").is_file());
            std::process::abort();
        }

        let directory = tempdir().expect("temp directory");
        let path = directory.path().join("unix-exclusive-wal-crash.sqlite3");
        let child = Command::new(std::env::current_exe().expect("test executable"))
            .args([
                "--exact",
                "tests::unix_exclusive_vfs_wal_replay_survives_process_termination_after_commit",
                "--nocapture",
            ])
            .env(CHILD_PATH_ENV, &path)
            .status()
            .expect("spawn crash child");
        assert!(!child.success(), "crash child unexpectedly completed");

        let reopened = Storage::open_with_vfs(&path, "unix-excl").expect("recovered storage");
        let profile_id =
            ProviderProfileId::parse("unix-exclusive-wal-crash-profile").expect("profile id");
        let restored = reopened
            .provider_profile(&profile_id)
            .expect("recovered profile query")
            .expect("recovered profile");
        assert_eq!(
            restored.selected_model(),
            Some("unix-exclusive-wal-crash-model")
        );
        assert_eq!(
            restored.secret_ref().map(SecretRef::as_str),
            Some(PERSISTENT_SECRET_REF)
        );
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
    fn provider_profile_health_round_trip_and_failure_normalization() {
        let directory = tempdir().expect("temp directory");
        let path = directory.path().join("profile-health.sqlite3");
        let profile = profile("health-profile", None, Some("health-model"));
        {
            let mut storage = Storage::open(&path).expect("storage");
            storage
                .save_and_activate_provider(&profile)
                .expect("profile");
            assert!(
                storage
                    .record_provider_health_failure(profile.id(), ErrorKind::Authentication)
                    .expect("record failure")
            );
            let failed = storage
                .provider_profile(profile.id())
                .expect("failed profile query")
                .expect("failed profile");
            assert_eq!(failed.last_successful_health_check(), None);
            assert_eq!(
                failed.last_failure_category(),
                Some(ErrorKind::Authentication)
            );
            assert!(
                storage
                    .record_provider_health_success(profile.id(), 1_750_000_000)
                    .expect("record success")
            );
        }
        let storage = Storage::open(&path).expect("reopened storage");
        let restored = storage
            .provider_profile(profile.id())
            .expect("restored profile query")
            .expect("restored profile");
        assert_eq!(restored.last_successful_health_check(), Some(1_750_000_000));
        assert_eq!(restored.last_failure_category(), None);
    }

    #[test]
    fn provider_profile_user_notes_round_trip_without_secret_values() {
        let mut storage = Storage::in_memory().expect("storage");
        let profile = profile("notes-profile", None, Some("notes-model"))
            .with_user_notes(Some("Keep this profile for local review".to_owned()))
            .expect("notes");
        storage
            .save_and_activate_provider(&profile)
            .expect("save profile");

        let restored = storage
            .provider_profile(profile.id())
            .expect("profile query")
            .expect("profile");
        assert_eq!(restored.user_notes(), profile.user_notes());
        assert_eq!(restored.selected_model(), Some("notes-model"));
    }

    #[test]
    fn provider_profile_organization_round_trip_without_secret_values() {
        let mut storage = Storage::in_memory().expect("storage");
        let profile = profile("organization-profile", None, Some("organization-model"))
            .with_organization(Some("org-local".to_owned()))
            .expect("organization");
        storage
            .save_and_activate_provider(&profile)
            .expect("save profile");

        let restored = storage
            .provider_profile(profile.id())
            .expect("profile query")
            .expect("profile");
        assert_eq!(restored.organization(), profile.organization());
        assert_eq!(restored.selected_model(), Some("organization-model"));
    }

    #[test]
    fn provider_profile_project_round_trip_without_secret_values() {
        let mut storage = Storage::in_memory().expect("storage");
        let profile = profile("project-profile", None, Some("project-model"))
            .with_project(Some("project-local".to_owned()))
            .expect("project");
        storage
            .save_and_activate_provider(&profile)
            .expect("save profile");

        let restored = storage
            .provider_profile(profile.id())
            .expect("profile query")
            .expect("profile");
        assert_eq!(restored.project(), profile.project());
        assert_eq!(restored.selected_model(), Some("project-model"));
    }

    #[test]
    fn provider_profile_region_and_account_round_trip_without_secret_values() {
        let mut storage = Storage::in_memory().expect("storage");
        let profile = profile("region-account-profile", None, Some("region-model"))
            .with_region(Some("eu-west-1".to_owned()))
            .expect("region")
            .with_account_identifier(Some("tenant-42".to_owned()))
            .expect("account");
        storage
            .upsert_provider_profile(&profile)
            .expect("save profile");
        let restored = storage
            .provider_profile(profile.id())
            .expect("load profile")
            .expect("profile");
        assert_eq!(restored.region(), profile.region());
        assert_eq!(restored.account_identifier(), profile.account_identifier());
        assert_eq!(restored.selected_model(), Some("region-model"));
    }

    #[test]
    fn provider_profile_custom_headers_round_trip_without_secret_values() {
        let mut storage = Storage::in_memory().expect("storage");
        let profile = profile("headers-profile", None, Some("headers-model"))
            .with_custom_headers(Some(
                r#"{"X-Trace-Mode":"local","X-Feature":"draft"}"#.to_owned(),
            ))
            .expect("headers");
        storage
            .upsert_provider_profile(&profile)
            .expect("save profile");
        let restored = storage
            .provider_profile(profile.id())
            .expect("load profile")
            .expect("profile");
        assert_eq!(restored.custom_headers(), profile.custom_headers());
        assert_eq!(restored.selected_model(), Some("headers-model"));
    }

    #[test]
    fn provider_profile_proxy_round_trip_without_credentials() {
        let mut storage = Storage::in_memory().expect("storage");
        let profile = profile("proxy-profile", None, Some("proxy-model"))
            .with_proxy_url(Some("http://127.0.0.1:8080".to_owned()))
            .expect("proxy");
        storage
            .upsert_provider_profile(&profile)
            .expect("save profile");
        let restored = storage
            .provider_profile(profile.id())
            .expect("load profile")
            .expect("profile");
        assert_eq!(restored.proxy_url(), Some("http://127.0.0.1:8080/"));
        assert_eq!(restored.selected_model(), Some("proxy-model"));
    }

    #[test]
    fn provider_profile_request_timeout_round_trip() {
        let mut storage = Storage::in_memory().expect("storage");
        let profile = profile("timeout-profile", None, Some("timeout-model"))
            .with_request_timeout_secs(120)
            .expect("timeout");
        storage
            .upsert_provider_profile(&profile)
            .expect("save profile");
        let restored = storage
            .provider_profile(profile.id())
            .expect("load profile")
            .expect("profile");
        assert_eq!(restored.request_timeout_secs(), 120);
        assert_eq!(restored.selected_model(), Some("timeout-model"));
    }

    #[test]
    fn provider_profile_connection_timeout_round_trip() {
        let mut storage = Storage::in_memory().expect("storage");
        let profile = profile("connection-timeout-profile", None, Some("connection-model"))
            .with_connection_timeout_secs(45)
            .expect("connection timeout");
        storage
            .upsert_provider_profile(&profile)
            .expect("save profile");
        let restored = storage
            .provider_profile(profile.id())
            .expect("load profile")
            .expect("profile");
        assert_eq!(restored.connection_timeout_secs(), 45);
        assert_eq!(restored.selected_model(), Some("connection-model"));
    }

    #[test]
    fn provider_profile_streaming_idle_timeout_round_trip() {
        let mut storage = Storage::in_memory().expect("storage");
        let profile = profile("streaming-idle-timeout-profile", None, Some("stream-model"))
            .with_streaming_idle_timeout_secs(90)
            .expect("streaming idle timeout");
        storage
            .upsert_provider_profile(&profile)
            .expect("save profile");
        let restored = storage
            .provider_profile(profile.id())
            .expect("load profile")
            .expect("profile");
        assert_eq!(restored.streaming_idle_timeout_secs(), 90);
        assert_eq!(restored.selected_model(), Some("stream-model"));
    }

    #[test]
    fn provider_profile_trusted_certificates_round_trip() {
        let mut storage = Storage::in_memory().expect("storage");
        let pem = "-----BEGIN CERTIFICATE-----\nMIIB\n-----END CERTIFICATE-----";
        let profile = profile("trusted-certificates-profile", None, Some("cert-model"))
            .with_trusted_certificates_pem(Some(pem.to_owned()))
            .expect("trusted certificates");
        storage
            .upsert_provider_profile(&profile)
            .expect("save profile");
        let restored = storage
            .provider_profile(profile.id())
            .expect("load profile")
            .expect("profile");
        assert_eq!(restored.trusted_certificates_pem(), Some(pem));
        assert_eq!(restored.selected_model(), Some("cert-model"));
    }

    #[test]
    fn provider_profile_proxy_auth_round_trip_as_reference_only() {
        let mut storage = Storage::in_memory().expect("storage");
        let secret_ref = SecretRef::new(SecretRefNamespace::SecretService);
        let profile = profile("proxy-auth-profile", None, Some("proxy-model"))
            .with_proxy_auth_ref(Some(secret_ref.clone()));
        storage
            .upsert_provider_profile(&profile)
            .expect("save profile");
        let restored = storage
            .provider_profile(profile.id())
            .expect("load profile")
            .expect("profile");
        assert_eq!(restored.proxy_auth_ref(), Some(&secret_ref));
    }

    #[test]
    fn provider_profile_client_certificate_identity_round_trip_as_reference_only() {
        let mut storage = Storage::in_memory().expect("storage");
        let secret_ref = SecretRef::new(SecretRefNamespace::SecretService);
        let profile = profile("client-identity-profile", None, Some("identity-model"))
            .with_client_certificate_identity_ref(Some(secret_ref.clone()));
        storage
            .upsert_provider_profile(&profile)
            .expect("save profile");
        let restored = storage
            .provider_profile(profile.id())
            .expect("load profile")
            .expect("profile");
        assert_eq!(
            restored.client_certificate_identity_ref(),
            Some(&secret_ref)
        );
    }

    #[test]
    fn provider_profile_secret_custom_headers_round_trip_as_reference_only() {
        let mut storage = Storage::in_memory().expect("storage");
        let secret_ref = SecretRef::new(SecretRefNamespace::SecretService);
        let profile = profile("secret-headers-profile", None, Some("secret-model"))
            .with_secret_custom_headers_ref(Some(secret_ref.clone()));
        storage
            .upsert_provider_profile(&profile)
            .expect("save profile");
        let restored = storage
            .provider_profile(profile.id())
            .expect("load profile")
            .expect("profile");
        assert_eq!(restored.secret_custom_headers_ref(), Some(&secret_ref));
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
    fn session_proxy_auth_reference_cannot_be_persisted() {
        let mut storage = Storage::in_memory().expect("storage");
        let secret_ref = SecretRef::parse(SESSION_SECRET_REF).expect("session ref");
        let profile = profile("session-proxy-auth", None, Some("model"))
            .with_proxy_auth_ref(Some(secret_ref));
        let error = storage
            .upsert_provider_profile(&profile)
            .expect_err("session proxy reference must be rejected");
        assert_eq!(error.kind, ErrorKind::InvalidConfiguration);
        assert!(
            storage
                .provider_profile(profile.id())
                .expect("profile query")
                .is_none()
        );
    }

    #[test]
    fn session_client_certificate_identity_reference_cannot_be_persisted() {
        let mut storage = Storage::in_memory().expect("storage");
        let secret_ref = SecretRef::parse(SESSION_SECRET_REF).expect("session ref");
        let profile = profile("session-client-identity", None, Some("model"))
            .with_client_certificate_identity_ref(Some(secret_ref));
        let error = storage
            .upsert_provider_profile(&profile)
            .expect_err("session reference must be rejected");
        assert_eq!(error.kind, ErrorKind::InvalidConfiguration);
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
