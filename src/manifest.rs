// SPDX-License-Identifier: MIT
//! 插件清单 — 描述动态插件的元信息与配置
//! Plugin manifest — metadata and configuration for dynamic plugins.
//!
//! 每个动态插件目录下需要一个 `manifest.toml` 文件：
//!
//! Each dynamic plugin lives in its own directory with a `manifest.toml`:
//!
//! ```toml
//! [plugin]
//! name = "echo"
//! version = "1.0.0"
//! description = "Echo plugin example"
//! author = "you"
//!
//! # 动态库文件名（仅 stem，平台扩展自动添加）
//! # 如不指定，默认为 lib<name>.so / lib<name>.dylib / <name>.dll
//! # Library stem; platform extension is appended automatically.
//! # Defaults to lib<name>.so / lib<name>.dylib / <name>.dll
//! library = "libecho"
//!
//! # 插件专属配置（传递给 on_load 的 JSON）
//! # Plugin-specific config (serialized to JSON, passed to on_load)
//! [plugin.config]
//! prefix = "[echo]"
//! max_length = 1024
//! ```

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::error::PluginError;

/// 插件清单
/// Plugin manifest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginManifest {
    pub plugin: PluginMeta,
}

/// 插件元信息
/// Plugin metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginMeta {
    /// 插件名称（唯一标识）/ Plugin name (unique identifier)
    pub name: String,
    /// 插件版本（semver）/ Plugin version (semver)
    pub version: String,
    /// 插件描述 / Plugin description
    #[serde(default)]
    pub description: String,
    /// 插件作者 / Plugin author
    #[serde(default)]
    pub author: String,
    /// 动态库路径（相对于清单文件所在目录）
    /// 如不指定，按平台约定自动推断
    ///
    /// Library path (relative to the manifest directory).
    /// Auto-inferred per-platform convention when absent.
    #[serde(default)]
    pub library: Option<String>,
    /// 插件专属配置（任意 TOML 值，序列化为 JSON 传给 on_load）
    /// Plugin-specific config (arbitrary TOML value, serialized to JSON for on_load)
    #[serde(default)]
    pub config: Option<toml::Value>,
}

impl PluginManifest {
    /// 从文件加载清单 / Load a manifest from file.
    pub fn load(path: &Path) -> Result<Self, PluginError> {
        let content = std::fs::read_to_string(path)?;
        toml::from_str(&content).map_err(|e| PluginError::ManifestError(e.to_string()))
    }

    /// 从 TOML 字符串解析清单 / Parse a manifest from a TOML string.
    pub fn parse_toml(content: &str) -> Result<Self, PluginError> {
        toml::from_str(content).map_err(|e| PluginError::ManifestError(e.to_string()))
    }

    /// 获取动态库的完整路径
    /// Resolve the full dynamic-library path.
    ///
    /// `base_dir` 为清单文件所在目录（发现扫描时的插件子目录）。
    /// 如果 `library` 字段已指定，直接使用（相对于 `base_dir`）；
    /// 否则按平台约定推断：Linux/macOS → `lib<name>.so`/`.dylib`，Windows → `<name>.dll`。
    ///
    /// `base_dir` is the directory containing the manifest (the plugin's
    /// own subdirectory during discovery). If `library` is set it is used
    /// relative to `base_dir`; otherwise inferred by platform convention:
    /// `lib<name>.so`/`.dylib` on Linux/macOS, `<name>.dll` on Windows.
    pub fn library_path(&self, base_dir: &Path) -> PathBuf {
        if let Some(ref lib) = self.plugin.library {
            // 如果是绝对路径，直接使用；否则相对于 base_dir
            // Absolute paths are used as-is; otherwise relative to base_dir
            let lib_path = PathBuf::from(lib);
            if lib_path.is_absolute() {
                lib_path
            } else {
                base_dir.join(lib)
            }
        } else {
            // 自动推断：<base_dir>/<name>.dll 或 lib<name>.so
            // Auto-infer: <base_dir>/<name>.dll or lib<name>.so
            let ext = platform_library_extension();
            let filename = if cfg!(target_os = "windows") {
                format!("{}{}", self.plugin.name, ext)
            } else {
                format!("lib{}{}", self.plugin.name, ext)
            };
            base_dir.join(filename)
        }
    }

    /// 将插件配置序列化为 JSON 字符串（传给 on_load）
    /// Serialize plugin config to a JSON string (passed to on_load).
    pub fn config_json(&self) -> String {
        match &self.plugin.config {
            Some(value) => serde_json::to_string(value).unwrap_or_else(|_| "{}".to_string()),
            None => "{}".to_string(),
        }
    }
}

/// 返回当前平台的动态库扩展名
/// Return the dynamic-library extension for the current platform.
pub fn platform_library_extension() -> &'static str {
    if cfg!(target_os = "linux") {
        ".so"
    } else if cfg!(target_os = "macos") {
        ".dylib"
    } else if cfg!(target_os = "windows") {
        ".dll"
    } else {
        ".so" // fallback
    }
}

/// 在指定目录下发现所有插件清单
/// Discover all plugin manifests under a directory.
///
/// 扫描策略：遍历 `plugin_dir` 下的每个子目录，查找 `manifest.toml`。
/// 目录不存在时返回空 Vec（静态插件场景无需目录）。
///
/// Scans each subdirectory of `plugin_dir` for a `manifest.toml`.
/// A missing directory yields an empty Vec (static-only setups need no dir).
pub fn discover_plugins(plugin_dir: &Path) -> Vec<Result<(PathBuf, PluginManifest), PluginError>> {
    let mut results = Vec::new();

    let entries = match std::fs::read_dir(plugin_dir) {
        Ok(entries) => entries,
        Err(_) => return results,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let manifest_path = path.join("manifest.toml");
        if !manifest_path.exists() {
            continue;
        }
        results.push(PluginManifest::load(&manifest_path).map(|m| (path.clone(), m)));
    }

    results
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_minimal_manifest() {
        let toml_str = r#"
[plugin]
name = "echo"
version = "1.0.0"
"#;
        let manifest = PluginManifest::parse_toml(toml_str).unwrap();
        assert_eq!(manifest.plugin.name, "echo");
        assert_eq!(manifest.plugin.version, "1.0.0");
        assert!(manifest.plugin.description.is_empty());
        assert!(manifest.plugin.library.is_none());
        assert!(manifest.plugin.config.is_none());
    }

    #[test]
    fn test_parse_full_manifest() {
        let toml_str = r#"
[plugin]
name = "sentiment"
version = "2.1.0"
description = "Sentiment analysis plugin"
author = "Someone"
library = "libsentiment_ext.so"

[plugin.config]
model = "distilbert"
threshold = 0.7
"#;
        let manifest = PluginManifest::parse_toml(toml_str).unwrap();
        assert_eq!(manifest.plugin.name, "sentiment");
        assert_eq!(manifest.plugin.version, "2.1.0");
        assert_eq!(manifest.plugin.description, "Sentiment analysis plugin");
        assert_eq!(
            manifest.plugin.library.as_deref(),
            Some("libsentiment_ext.so")
        );
        assert!(manifest.plugin.config.is_some());
    }

    #[test]
    fn test_config_json() {
        let toml_str = r#"
[plugin]
name = "test"
version = "0.1.0"

[plugin.config]
key = "value"
count = 42
"#;
        let manifest = PluginManifest::parse_toml(toml_str).unwrap();
        let json = manifest.config_json();
        assert!(json.contains("key"));
        assert!(json.contains("value"));
    }

    #[test]
    fn test_config_json_empty() {
        let toml_str = r#"
[plugin]
name = "test"
version = "0.1.0"
"#;
        let manifest = PluginManifest::parse_toml(toml_str).unwrap();
        assert_eq!(manifest.config_json(), "{}");
    }

    #[test]
    fn test_library_path_auto_infer() {
        let toml_str = r#"
[plugin]
name = "echo"
version = "1.0.0"
"#;
        let manifest = PluginManifest::parse_toml(toml_str).unwrap();
        // base_dir = 清单所在目录（插件自己的子目录）/ base_dir = the manifest's own dir
        let dir = PathBuf::from("/opt/app/plugins/echo");
        let lib_path = manifest.library_path(&dir);

        // 自动推断不应重复拼接插件名：期望 <dir>/<lib 文件名>，而非 <dir>/<name>/<lib 文件名>
        // Auto-inference must not double-join the plugin name
        if cfg!(target_os = "windows") {
            assert!(lib_path.ends_with("echo.dll"));
            assert_eq!(lib_path, PathBuf::from("/opt/app/plugins/echo/echo.dll"));
        } else {
            assert!(lib_path.ends_with("libecho.so"));
            assert_eq!(lib_path, PathBuf::from("/opt/app/plugins/echo/libecho.so"));
        }
    }

    #[test]
    fn test_library_path_explicit() {
        let toml_str = r#"
[plugin]
name = "custom"
version = "1.0.0"
library = "my_custom_lib.so"
"#;
        let manifest = PluginManifest::parse_toml(toml_str).unwrap();
        let dir = PathBuf::from("/opt/app/plugins");
        let lib_path = manifest.library_path(&dir);
        assert!(lib_path.to_str().unwrap().contains("my_custom_lib.so"));
    }

    #[test]
    fn test_discover_missing_dir() {
        // 目录不存在 → 空 Vec，不 panic / Missing dir → empty Vec, no panic
        let results = discover_plugins(Path::new("/nonexistent/path/xyz"));
        assert!(results.is_empty());
    }
}
