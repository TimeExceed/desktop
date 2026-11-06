//! fedora-setup 的共享逻辑：策略结构与文件操作。
//!
//! 备份器（`backup`）与恢复器（`restore`）是两个独立二进制，
//! 共用这里的解析与文件操作。

mod r#impl;
mod policy;

pub use self::r#impl::*;
pub use self::policy::*;
