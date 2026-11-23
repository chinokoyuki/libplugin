# libplugin

[![crates.io](https://img.shields.io/crates/v/libplugin.svg)](https://crates.io/crates/libplugin)
[![docs.rs](https://docs.rs/libplugin/badge.svg)](https://docs.rs/libplugin)
[![CI](https://github.com/chinokoyuki/libplugin/actions/workflows/ci.yml/badge.svg)](https://github.com/chinokoyuki/libplugin/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

[English](README.md)

Rust 的极简插件框架：静态注册 + 运行时加载原生库，一套生命周期，宿主与插件之间只隔一个小巧的 C-ABI vtable。

```
PluginManager
├── 静态插件   — Box<dyn Plugin>，编译进宿主，零加载开销
└── 动态插件   — 通过 manifest.toml 发现 .so / .dll / .dylib，
                 经 #[repr(C)] vtable 调用
```

没有宏、没有代码生成、没有异步运行时。插件就是一个 trait 实现；动态插件就是一个导出单个 C 函数的 `cdylib`。设计就这么多。

## 为什么是这个形态

WASM 插件系统（extism 一类）在需要沙箱和跨语言时可移植时很好，代价是每次调用都过一层运行时边界和序列化。

`libplugin` 是另一个取舍：原生代码、直接函数指针、零开销——适用于你信任插件的场景（自己的构建产物、可选功能包、游戏或机器人的 mod 加载）。ABI 就是 7 个 `#[repr(C)]` 布局的函数指针；任何能产出 C-ABI 动态库的语言都能写插件。

## 安装

```toml
[dependencies]
libplugin = "0.1"
```

## 30 秒上手静态插件

```rust
use libplugin::{Plugin, PluginManager};

struct Uptime;

impl Plugin for Uptime {
    fn name(&self) -> &str { "uptime" }
    fn on_message(&self, msg: &str) -> Option<String> {
        msg.contains("time").then(|| "到整点了".into())
    }
}

let mut mgr = PluginManager::new_static_only();
mgr.register(Box::new(Uptime));
mgr.load_all()?;
let replies = mgr.on_message("现在几点了？");
// [("uptime", "到整点了")]
mgr.unload_all()?;
```

trait 方法全有默认实现——一个只实现 `name()` 的插件就是合法插件，其余全靠按需覆盖。

## 动态插件

动态插件是导出 `libplugin_entry() -> PluginVTable` 的 `cdylib`。完整可运行示例见 [`examples/echo_plugin.rs`](examples/echo_plugin.rs)（约 60 行），宿主侧见 [`examples/manager.rs`](examples/manager.rs)：

```rust
#[no_mangle]
pub extern "C" fn libplugin_entry() -> PluginVTable {
    PluginVTable { name, version, on_load, on_unload, on_message, on_tick, on_shutdown }
}
```

发现目录结构：

```
plugins/
└── echo_plugin/
    ├── manifest.toml
    └── echo_plugin.dll        （Linux 上是 libecho_plugin.so）
```

```toml
# manifest.toml
[plugin]
name = "echo_plugin"
version = "1.0.0"

# 可选：[plugin.config] 下的所有内容序列化为 JSON 传给 on_load
[plugin.config]
prefix = "[echo]"
```

编译好插件后，把 `PluginManager::new("plugins")` 指向该目录，调用 `discover_and_load()`——宿主解析清单、加载动态库、用配置 JSON 执行 `on_load`。`Library` 句柄由 `DynamicPlugin` 持有，函数指针在插件生命周期内始终有效。已端到端验证：示例 dll 能加载、初始化、并跨 ABI 回声消息。

## 生命周期

| 调用 | 作用对象 | 说明 |
|---|---|---|
| `register` | 静态插件 | 编译期注册，同名覆盖 |
| `discover_and_load` | 动态插件 | 单个插件失败不阻断其余 |
| `load_all` | 已注册未加载 | 静态插件收到 `"{}"` 配置 |
| `on_message` | 已加载插件 | 广播，收集 `(名称, 响应)` |
| `on_tick` | 已加载插件 | 频率由你的调度器决定 |
| `unload_all` / `shutdown_all` | 已加载 / 全部 | 尽力而为，失败记入状态 |

每个插件的状态可查：`state(name)` → `Registered` / `Loaded` / `Errored(..)` / `Unloaded`，`health_status()` 生成逐插件的一行报告。

## 说明

- **静默跳过的发现策略**：清单解析失败、库文件缺失、加载失败都会跳过该插件，其余继续；失败落在该插件的 `Errored` 状态里可查。
- **ABI 稳定性**：vtable 布局就是契约。入口符号是 `libplugin_entry`；宿主侧新特性以新增可选指针的方式演进，不破坏旧插件。
- **线程安全**：`Plugin: Send + Sync`，宿主可在任意线程调用。动态插件的线程安全靠插件作者保证——`unsafe impl Send/Sync` 的可信度等同于插件作者。
- 配置以 JSON 字符串过 ABI；消息响应由插件写入宿主提供的 64 KB 缓冲区（超长截断，绝不溢出）。

## 许可

MIT
