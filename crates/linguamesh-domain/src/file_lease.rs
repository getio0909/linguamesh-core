use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};
use thiserror::Error;
use uuid::Uuid;

const ACTIVE: u8 = 0;
const EXPIRED: u8 = 1;
const REVOKED: u8 = 2;
const MAX_LOCATION_BYTES: usize = 4096;

/// 描述跨平台宿主授予核心的文件资源。
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FileLeaseResource {
    /// 桌面文件路径或门户返回的文件 URI。
    DesktopPath(String),
    /// POSIX 文件描述符。
    PosixDescriptor { fd: i64 },
    /// Android `ParcelFileDescriptor` 导出的描述符。
    AndroidParcelDescriptor { fd: i64 },
    /// Windows 宿主复制给核心的句柄值。
    WindowsHandle { value: u64 },
    /// 核心或宿主创建的临时文件路径。
    TemporaryPath(String),
    /// 输出目标路径或宿主输出 lease。
    OutputPath(String),
}

/// 描述文件 lease 失效或资源参数错误。
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum FileLeaseError {
    /// 路径或 URI 为空、过长或包含不可接受的 NUL 字节。
    #[error("The file lease location is invalid.")]
    InvalidLocation,
    /// 描述符为负数或超出宿主 ABI 可表达的范围。
    #[error("The file lease descriptor is invalid.")]
    InvalidDescriptor,
    /// Windows 句柄值为零。
    #[error("The file lease handle is invalid.")]
    InvalidHandle,
    /// lease 已到期，不能继续借用。
    #[error("The file lease has expired.")]
    Expired,
    /// lease 已被宿主明确撤销，不能继续借用。
    #[error("The file lease has been revoked.")]
    Revoked,
}

/// 表示由宿主授予、可显式撤销且不可在失效后继续借用的文件 lease。
#[derive(Clone, Debug)]
pub struct FileLease {
    lease_id: String,
    resource: FileLeaseResource,
    state: Arc<AtomicU8>,
}

impl FileLease {
    /// 创建桌面路径或门户文件 URI lease。
    pub fn desktop_path(value: impl Into<String>) -> Result<Self, FileLeaseError> {
        Self::location(FileLeaseResource::DesktopPath, value)
    }

    /// 创建 POSIX 文件描述符 lease。
    pub fn posix_descriptor(fd: i64) -> Result<Self, FileLeaseError> {
        Self::descriptor(FileLeaseResource::PosixDescriptor { fd }, fd)
    }

    /// 创建 Android `ParcelFileDescriptor` 导出的 lease。
    pub fn android_parcel_descriptor(fd: i64) -> Result<Self, FileLeaseError> {
        Self::descriptor(FileLeaseResource::AndroidParcelDescriptor { fd }, fd)
    }

    /// 创建 Windows 复制句柄 lease。
    pub fn windows_handle(value: u64) -> Result<Self, FileLeaseError> {
        if value == 0 {
            return Err(FileLeaseError::InvalidHandle);
        }
        Ok(Self::new(FileLeaseResource::WindowsHandle { value }))
    }

    /// 创建临时文件 lease。
    pub fn temporary_path(value: impl Into<String>) -> Result<Self, FileLeaseError> {
        Self::location(FileLeaseResource::TemporaryPath, value)
    }

    /// 创建输出目标 lease。
    pub fn output_path(value: impl Into<String>) -> Result<Self, FileLeaseError> {
        Self::location(FileLeaseResource::OutputPath, value)
    }

    /// 返回不透明 lease 标识，不包含路径、描述符或句柄。
    #[must_use]
    pub fn lease_id(&self) -> &str {
        &self.lease_id
    }

    /// 判断宿主是否仍允许核心借用该资源。
    #[must_use]
    pub fn is_active(&self) -> bool {
        self.state.load(Ordering::Acquire) == ACTIVE
    }

    /// 创建一次受保护的借用视图；视图在每次访问资源时重新检查状态。
    pub fn acquire(&self) -> Result<FileLeaseGuard<'_>, FileLeaseError> {
        self.ensure_active()?;
        Ok(FileLeaseGuard { lease: self })
    }

    /// 检查 lease 是否仍然有效。
    pub fn ensure_active(&self) -> Result<(), FileLeaseError> {
        match self.state.load(Ordering::Acquire) {
            ACTIVE => Ok(()),
            EXPIRED => Err(FileLeaseError::Expired),
            _ => Err(FileLeaseError::Revoked),
        }
    }

    /// 模拟宿主 lease 到期；重复调用保持第一次失效原因。
    pub fn expire(&self) {
        let _ = self
            .state
            .compare_exchange(ACTIVE, EXPIRED, Ordering::AcqRel, Ordering::Acquire);
    }

    /// 由宿主明确撤销 lease；重复调用不会恢复资源。
    pub fn revoke(&self) {
        let _ = self
            .state
            .compare_exchange(ACTIVE, REVOKED, Ordering::AcqRel, Ordering::Acquire);
    }

    fn new(resource: FileLeaseResource) -> Self {
        Self {
            lease_id: Uuid::new_v4().to_string(),
            resource,
            state: Arc::new(AtomicU8::new(ACTIVE)),
        }
    }

    fn location(
        constructor: fn(String) -> FileLeaseResource,
        value: impl Into<String>,
    ) -> Result<Self, FileLeaseError> {
        let value = value.into();
        if value.is_empty() || value.len() > MAX_LOCATION_BYTES || value.contains('\0') {
            return Err(FileLeaseError::InvalidLocation);
        }
        Ok(Self::new(constructor(value)))
    }

    fn descriptor(resource: FileLeaseResource, fd: i64) -> Result<Self, FileLeaseError> {
        if fd < 0 {
            return Err(FileLeaseError::InvalidDescriptor);
        }
        Ok(Self::new(resource))
    }
}

/// 表示一次只能在 lease 有效期间读取资源的借用视图。
#[derive(Debug)]
pub struct FileLeaseGuard<'a> {
    lease: &'a FileLease,
}

impl FileLeaseGuard<'_> {
    /// 返回不透明 lease 标识。
    #[must_use]
    pub fn lease_id(&self) -> &str {
        self.lease.lease_id()
    }

    /// 返回资源描述；lease 到期或撤销后拒绝访问。
    pub fn resource(&self) -> Result<&FileLeaseResource, FileLeaseError> {
        self.lease.ensure_active()?;
        Ok(&self.lease.resource)
    }
}

#[cfg(test)]
mod tests {
    use super::{FileLease, FileLeaseError, FileLeaseResource};

    #[test]
    fn supports_all_platform_resource_shapes_without_exposing_lease_identity() {
        let desktop = FileLease::desktop_path("file:///tmp/input.txt").expect("desktop lease");
        assert!(!desktop.lease_id().is_empty());
        assert_eq!(
            desktop.acquire().expect("desktop borrow").resource(),
            Ok(&FileLeaseResource::DesktopPath(
                "file:///tmp/input.txt".into()
            ))
        );
        assert!(FileLease::posix_descriptor(0).is_ok());
        assert!(FileLease::android_parcel_descriptor(3).is_ok());
        assert!(FileLease::windows_handle(1).is_ok());
        assert!(FileLease::temporary_path("/tmp/lease-input").is_ok());
        assert!(FileLease::output_path("/tmp/lease-output").is_ok());
    }

    #[test]
    fn rejects_invalid_locations_and_handles_before_work() {
        assert_eq!(
            FileLease::desktop_path("").expect_err("empty location"),
            FileLeaseError::InvalidLocation
        );
        assert_eq!(
            FileLease::desktop_path("bad\0path").expect_err("NUL location"),
            FileLeaseError::InvalidLocation
        );
        assert_eq!(
            FileLease::posix_descriptor(-1).expect_err("negative descriptor"),
            FileLeaseError::InvalidDescriptor
        );
        assert_eq!(
            FileLease::windows_handle(0).expect_err("zero handle"),
            FileLeaseError::InvalidHandle
        );
    }

    #[test]
    fn an_expired_or_revoked_lease_cannot_be_borrowed_again() {
        let expired = FileLease::temporary_path("/tmp/expired").expect("lease");
        let guard = expired.acquire().expect("borrow");
        expired.expire();
        assert_eq!(guard.resource(), Err(FileLeaseError::Expired));
        assert_eq!(
            expired.acquire().expect_err("expired borrow"),
            FileLeaseError::Expired
        );

        let revoked = FileLease::output_path("/tmp/revoked").expect("lease");
        let clone = revoked.clone();
        revoked.revoke();
        assert!(!clone.is_active());
        assert_eq!(clone.ensure_active(), Err(FileLeaseError::Revoked));
    }
}
