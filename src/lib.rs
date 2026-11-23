// SPDX-License-Identifier: MIT
//! # libplugin — 轻量插件框架 / Minimal plugin framework
//!
//! 静态注册 + 运行时动态加载，一套生命周期，两种来源：
//! Static registration + runtime loading, one lifecycle, two sources:
//!
//! - **静态插件**：编译时注册，零开销 / **Static plugins**: registered at compile time, zero overhead
//! - **动态插件**：运行时通过 `libloading` 加载 .so/.dll/.dylib / **Dynamic plugins**: .so/.dll/.dylib loaded at runtime via `libloading`
//! - **插件发现**：扫描插件目录下的 manifest.toml / **Discovery**: scan plugin dirs for manifest.toml
//! - **生命周期**：on_load → on_message/on_tick → on_unload/on_shutdown / **Lifecycle**: on_load → on_message/on_tick → on_unload/on_shutdown
//! - **per-plugin 配置**：manifest.toml 中的 `[plugin.config]` 序列化为 JSON 传给 on_load / **Per-plugin config**: `[plugin.config]` in the manifest reaches on_load as JSON
//!
//! ## 快速开始 / Quick start
//!
//! ```rust,ignore
//! use libplugin::{Plugin, PluginManager};
//!
//! let mut mgr = PluginManager::new("/path/to/plugins");
//!
//! // 注册静态插件 / Register a static plugin
//! mgr.register(Box::new(MyPlugin));
//!
//! // 发现并加载动态插件 / Discover and load dynamic plugins
//! mgr.discover_and_load()?;
//!
//! // 调用生命周期 / Drive the lifecycle
//! mgr.load_all()?;           // 初始化所有已注册插件 / initialize all registered plugins
//! mgr.on_message("hello");   // 广播消息给所有插件 / broadcast to all plugins
//! mgr.on_tick()?;            // 周期 tick / periodic tick
//! mgr.unload_all()?;         // 卸载所有插件 / unload everything
//! ```
//!
//! ## 动态插件 ABI
//!
//! 动态插件必须导出 C 函数 `libplugin_entry() -> PluginVTable`。
//! 详见 [`vtable::PluginVTable`]。
//!
//! Dynamic plugins must export the C function
//! `libplugin_entry() -> PluginVTable`. See [`vtable::PluginVTable`].

pub mod dynamic;
pub mod error;
pub mod manifest;
pub mod vtable;

pub use error::Result;

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use error::PluginError;

// ── 插件 trait / Plugin trait ──

/// 插件特质 — 所有插件（静态或动态）需实现此 trait
/// Plugin trait — all plugins (static or dynamic) must implement this.
pub trait Plugin: Send + Sync {
    /// 插件名称 / Plugin name
    fn name(&self) -> &str;

    /// 插件版本（默认 "0.1.0"）/ Plugin version (defaults to "0.1.0")
    fn version(&self) -> &str {
        "0.1.0"
    }

    /// 初始化插件。config 为 JSON 配置字符串（来自 manifest.toml 的 [plugin.config]）。
    /// 默认实现忽略配置。
    ///
    /// Initialize the plugin. `config` is a JSON string (from the manifest's
    /// [plugin.config]). The default implementation ignores it.
    fn on_load(&self, _config: &str) -> Result<()> {
        Ok(())
    }

    /// 反初始化插件。默认实现无操作。
    /// Deinitialize the plugin. Default is a no-op.
    fn on_unload(&self) -> Result<()> {
        Ok(())
    }

    /// 处理消息事件。返回 Some(response) 表示插件产生了输出（由宿主决定如何消费）。
    /// 默认实现忽略消息。
    ///
    /// Handle a message event. `Some(response)` marks plugin output
    /// (the host decides what to do with it). Default ignores the message.
    fn on_message(&self, _msg: &str) -> Option<String> {
        None
    }

    /// 周期性 tick 回调（由宿主的调度器驱动）。
    /// 默认实现无操作。
    ///
    /// Periodic tick callback (driven by the host's scheduler).
    /// Default is a no-op.
    fn on_tick(&self) -> Result<()> {
        Ok(())
    }

    /// 关机通知（进程退出前调用）。
    /// 默认实现无操作。
    ///
    /// Shutdown notification (called before process exit).
    /// Default is a no-op.
    fn on_shutdown(&self) -> Result<()> {
        Ok(())
    }
}

// ── 插件状态 / Plugin state ──

/// 插件运行状态 / Plugin runtime state.
#[derive(Debug, Clone)]
pub enum PluginState {
    /// 已注册但尚未初始化 / Registered but not yet initialized
    Registered,
    /// 已成功加载（on_load 成功）/ Successfully loaded (on_load succeeded)
    Loaded,
    /// 加载或运行时出错 / Load or runtime error
    Errored(String),
    /// 已卸载 / Unloaded
    Unloaded,
}

impl std::fmt::Display for PluginState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PluginState::Registered => write!(f, "registered"),
            PluginState::Loaded => write!(f, "loaded"),
            PluginState::Errored(e) => write!(f, "error: {}", e),
            PluginState::Unloaded => write!(f, "unloaded"),
        }
    }
}

// ── 插件条目 / Plugin entry ──

/// 已加载的插件条目（内部使用）/ A loaded plugin entry (internal).
struct PluginEntry {
    plugin: Box<dyn Plugin>,
    state: PluginState,
    /// 来源：static 或 dynamic / Source: static or dynamic
    source: PluginSource,
}

/// 插件来源 / Plugin origin.
#[derive(Debug, Clone, Copy)]
pub enum PluginSource {
    /// 编译时静态注册 / Registered at compile time
    Static,
    /// 运行时动态加载 / Loaded at runtime
    Dynamic,
}

// ── 插件管理器 / Plugin manager ──

/// 插件管理器 — 负责注册、发现、加载、卸载插件
/// Plugin manager — registration, discovery, loading, unloading.
pub struct PluginManager {
    /// 已注册/已加载的插件（按名称索引）/ Registered/loaded plugins, indexed by name
    entries: HashMap<String, PluginEntry>,
    /// 插件目录（用于动态插件发现）/ Plugin directory (for dynamic discovery)
    plugin_dir: PathBuf,
}

impl PluginManager {
    /// 创建插件管理器
    /// Create a plugin manager.
    ///
    /// # Arguments
    /// * `plugin_dir` - 插件目录路径，用于动态插件发现 / Plugin dir, used for dynamic discovery
    pub fn new<P: AsRef<Path>>(plugin_dir: P) -> Self {
        Self {
            entries: HashMap::new(),
            plugin_dir: plugin_dir.as_ref().to_path_buf(),
        }
    }

    /// 创建无插件目录的管理器（仅支持静态注册）
    /// Create a manager without a plugin dir (static registration only).
    pub fn new_static_only() -> Self {
        Self {
            entries: HashMap::new(),
            plugin_dir: PathBuf::new(),
        }
    }

    // ── 静态注册 / Static registration ──

    /// 注册一个静态插件（编译时已知）。同名插件会被覆盖。
    /// Register a static plugin (known at compile time).
    /// Same-name plugins overwrite the previous registration.
    pub fn register(&mut self, plugin: Box<dyn Plugin>) {
        let name = plugin.name().to_string();
        self.entries.insert(
            name,
            PluginEntry {
                plugin,
                state: PluginState::Registered,
                source: PluginSource::Static,
            },
        );
    }

    // ── 动态发现与加载 / Dynamic discovery & loading ──

    /// 扫描插件目录，发现并加载所有动态插件
    /// Discover and load all dynamic plugins in the plugin dir.
    ///
    /// 遍历 `plugin_dir` 下的子目录，查找 `manifest.toml`，
    /// 加载对应的动态库并调用 on_load。
    /// 单个插件失败只记录状态，不影响其余插件。
    ///
    /// Scans subdirectories of `plugin_dir` for `manifest.toml`,
    /// loads the referenced library and calls on_load.
    /// A single plugin failing only affects itself.
    pub fn discover_and_load(&mut self) -> std::result::Result<(), PluginError> {
        let discoveries = manifest::discover_plugins(&self.plugin_dir);

        for result in discoveries {
            match result {
                Ok((dir, manifest)) => {
                    let lib_path = manifest.library_path(&dir);
                    let plugin_name = manifest.plugin.name.clone();

                    if self.entries.contains_key(&plugin_name) {
                        continue;
                    }

                    if !lib_path.exists() {
                        continue;
                    }

                    match dynamic::DynamicPlugin::load_from_file(&lib_path) {
                        Ok(dyn_plugin) => {
                            let config_json = manifest.config_json();
                            let name = dyn_plugin.name().to_string();

                            // 注册到 entries / Register
                            self.entries.insert(
                                name.clone(),
                                PluginEntry {
                                    plugin: Box::new(dyn_plugin),
                                    state: PluginState::Registered,
                                    source: PluginSource::Dynamic,
                                },
                            );

                            // 调用 on_load / Initialize
                            if let Some(entry) = self.entries.get_mut(&name) {
                                match entry.plugin.on_load(&config_json) {
                                    Ok(()) => {
                                        entry.state = PluginState::Loaded;
                                    }
                                    Err(e) => {
                                        entry.state = PluginState::Errored(e.to_string());
                                    }
                                }
                            }
                        }
                        Err(_) => {
                            // 加载失败的库跳过；详细错误可从 discover 结果获取
                            // Skip unloadable libraries; details available via the error
                        }
                    }
                }
                Err(_) => {
                    // 清单解析失败的目录跳过 / Skip dirs with unparseable manifests
                }
            }
        }

        Ok(())
    }

    // ── 生命周期管理 / Lifecycle ──

    /// 初始化所有已注册但未加载的插件（调用 on_load，配置为空 JSON）
    /// Initialize all registered-but-not-loaded plugins (on_load with empty config).
    pub fn load_all(&mut self) -> Result<()> {
        let names: Vec<String> = self
            .entries
            .iter()
            .filter(|(_, e)| matches!(e.state, PluginState::Registered))
            .map(|(name, _)| name.clone())
            .collect();

        for name in names {
            if let Some(entry) = self.entries.get_mut(&name) {
                match entry.plugin.on_load("{}") {
                    Ok(()) => {
                        entry.state = PluginState::Loaded;
                    }
                    Err(e) => {
                        entry.state = PluginState::Errored(e.to_string());
                    }
                }
            }
        }
        Ok(())
    }

    /// 卸载所有已加载的插件（调用 on_unload）
    /// Unload all loaded plugins (calls on_unload).
    pub fn unload_all(&mut self) -> Result<()> {
        let names: Vec<String> = self
            .entries
            .iter()
            .filter(|(_, e)| matches!(e.state, PluginState::Loaded))
            .map(|(name, _)| name.clone())
            .collect();

        for name in &names {
            if let Some(entry) = self.entries.get_mut(name) {
                match entry.plugin.on_unload() {
                    Ok(()) => {
                        entry.state = PluginState::Unloaded;
                    }
                    Err(e) => {
                        entry.state = PluginState::Errored(e.to_string());
                    }
                }
            }
        }
        Ok(())
    }

    /// 通知所有插件关机（调用 on_shutdown）
    /// Notify all plugins of shutdown (calls on_shutdown).
    pub fn shutdown_all(&self) -> Result<()> {
        for entry in self.entries.values() {
            // 关机路径尽力而为：单个失败不阻断其余插件
            // Best effort: one failure never blocks the others
            let _ = entry.plugin.on_shutdown();
        }
        Ok(())
    }

    // ── 消息广播 / Broadcast ──

    /// 向所有已加载插件广播消息，收集插件响应
    /// Broadcast a message to all loaded plugins and collect responses.
    ///
    /// 返回所有非 None 的插件响应，顺序不保证。
    /// Returns every non-None response; order is unspecified.
    pub fn on_message(&self, msg: &str) -> Vec<(String, String)> {
        let mut responses = Vec::new();
        for (name, entry) in &self.entries {
            if !matches!(entry.state, PluginState::Loaded) {
                continue;
            }
            if let Some(response) = entry.plugin.on_message(msg) {
                responses.push((name.clone(), response));
            }
        }
        responses
    }

    /// 周期 tick 所有已加载插件。单个失败不影响其余。
    /// Tick all loaded plugins. A single failure never blocks the rest.
    pub fn on_tick(&self) -> Result<()> {
        for entry in self.entries.values() {
            if !matches!(entry.state, PluginState::Loaded) {
                continue;
            }
            let _ = entry.plugin.on_tick();
        }
        Ok(())
    }

    // ── 查询 / Queries ──

    /// 获取插件总数 / Total plugin count.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// 是否无插件 / Whether there are no plugins.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// 获取指定插件的状态 / State of a specific plugin.
    pub fn state(&self, name: &str) -> Option<&PluginState> {
        self.entries.get(name).map(|e| &e.state)
    }

    /// 获取所有插件名称 / Names of all plugins.
    pub fn plugin_names(&self) -> Vec<&str> {
        self.entries.keys().map(|s| s.as_str()).collect()
    }

    /// 获取所有已加载插件的名称 / Names of loaded plugins.
    pub fn loaded_names(&self) -> Vec<&str> {
        self.entries
            .iter()
            .filter(|(_, e)| matches!(e.state, PluginState::Loaded))
            .map(|(name, _)| name.as_str())
            .collect()
    }

    /// 生成健康状态报告 / Produce a health status report.
    ///
    /// 格式：`name v{version} [{source}] {state}`
    /// Format: `name v{version} [{source}] {state}`
    pub fn health_status(&self) -> HashMap<String, String> {
        self.entries
            .iter()
            .map(|(name, entry)| {
                let status = format!(
                    "{} v{} [{:?}] {}",
                    name,
                    entry.plugin.version(),
                    entry.source,
                    entry.state
                );
                (name.clone(), status)
            })
            .collect()
    }
}

impl Default for PluginManager {
    fn default() -> Self {
        Self::new_static_only()
    }
}

// ── 测试 / Tests ──

#[cfg(test)]
mod tests {
    use super::*;

    /// 测试用静态插件 / Static test plugin
    struct TestPlugin {
        name_str: String,
    }

    impl TestPlugin {
        fn new(name: &str) -> Self {
            Self {
                name_str: name.to_string(),
            }
        }
    }

    impl Plugin for TestPlugin {
        fn name(&self) -> &str {
            &self.name_str
        }
        fn version(&self) -> &str {
            "1.0.0"
        }
        fn on_load(&self, _config: &str) -> Result<()> {
            Ok(())
        }
        fn on_unload(&self) -> Result<()> {
            Ok(())
        }
        fn on_message(&self, msg: &str) -> Option<String> {
            Some(format!("[echo] {}", msg))
        }
        fn on_tick(&self) -> Result<()> {
            Ok(())
        }
        fn on_shutdown(&self) -> Result<()> {
            Ok(())
        }
    }

    #[test]
    fn test_register_and_load() {
        let mut mgr = PluginManager::new_static_only();
        assert!(mgr.is_empty());

        mgr.register(Box::new(TestPlugin::new("echo")));
        assert_eq!(mgr.len(), 1);
        assert!(mgr.state("echo").is_some());

        assert!(mgr.load_all().is_ok());
        assert!(matches!(mgr.state("echo"), Some(PluginState::Loaded)));
    }

    #[test]
    fn test_unload_all() {
        let mut mgr = PluginManager::new_static_only();
        mgr.register(Box::new(TestPlugin::new("echo")));
        assert!(mgr.load_all().is_ok());
        assert!(mgr.unload_all().is_ok());
        assert!(matches!(mgr.state("echo"), Some(PluginState::Unloaded)));
    }

    #[test]
    fn test_on_message() {
        let mut mgr = PluginManager::new_static_only();
        mgr.register(Box::new(TestPlugin::new("echo")));
        assert!(mgr.load_all().is_ok());

        let responses = mgr.on_message("hello");
        assert_eq!(responses.len(), 1);
        assert_eq!(responses[0].0, "echo");
        assert_eq!(responses[0].1, "[echo] hello");
    }

    #[test]
    fn test_on_tick() {
        let mut mgr = PluginManager::new_static_only();
        mgr.register(Box::new(TestPlugin::new("echo")));
        assert!(mgr.load_all().is_ok());
        assert!(mgr.on_tick().is_ok());
    }

    #[test]
    fn test_multiple_plugins() {
        let mut mgr = PluginManager::new_static_only();
        mgr.register(Box::new(TestPlugin::new("echo")));
        mgr.register(Box::new(TestPlugin::new("sentiment")));
        assert_eq!(mgr.len(), 2);

        assert!(mgr.load_all().is_ok());
        let loaded = mgr.loaded_names();
        assert_eq!(loaded.len(), 2);

        let responses = mgr.on_message("test");
        assert_eq!(responses.len(), 2);
    }

    #[test]
    fn test_health_status() {
        let mut mgr = PluginManager::new_static_only();
        mgr.register(Box::new(TestPlugin::new("echo")));
        assert!(mgr.load_all().is_ok());

        let status = mgr.health_status();
        assert!(status.contains_key("echo"));
        assert!(status["echo"].contains("loaded"));
    }

    #[test]
    fn test_default() {
        let mgr = PluginManager::default();
        assert!(mgr.is_empty());
    }

    #[test]
    fn test_plugin_state_display() {
        assert_eq!(format!("{}", PluginState::Registered), "registered");
        assert_eq!(format!("{}", PluginState::Loaded), "loaded");
        assert_eq!(format!("{}", PluginState::Unloaded), "unloaded");
        assert_eq!(
            format!("{}", PluginState::Errored("fail".into())),
            "error: fail"
        );
    }

    #[test]
    fn test_discover_on_missing_dir() {
        // 目录不存在时 discover_and_load 不应失败（静态插件场景）
        // discover_and_load must not fail on a missing dir (static-only setups)
        let mut mgr = PluginManager::new("/nonexistent/path/xyz");
        assert!(mgr.discover_and_load().is_ok());
        assert!(mgr.is_empty());
    }

    #[test]
    fn test_register_overwrites_same_name() {
        let mut mgr = PluginManager::new_static_only();
        mgr.register(Box::new(TestPlugin::new("echo")));
        mgr.register(Box::new(TestPlugin::new("echo")));
        assert_eq!(mgr.len(), 1);
    }
}
