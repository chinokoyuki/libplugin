# libplugin

[![crates.io](https://img.shields.io/crates/v/libplugin.svg)](https://crates.io/crates/libplugin)
[![docs.rs](https://docs.rs/libplugin/badge.svg)](https://docs.rs/libplugin)
[![CI](https://github.com/chinokoyuki/libplugin/actions/workflows/ci.yml/badge.svg)](https://github.com/chinokoyuki/libplugin/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

[中文说明](README_ZH.md)

A minimal plugin framework for Rust: static registration plus runtime-loaded native plugins, one lifecycle, one small C-ABI vtable between the host and everything else.

```
PluginManager
├── static plugins   — Box<dyn Plugin>, compiled in, zero loading cost
└── dynamic plugins  — .so / .dll / .dylib found via manifest.toml,
                       called through a #[repr(C)] vtable
```

No macros, no code generation, no async runtime. A plugin is a trait impl; a dynamic plugin is a `cdylib` exporting one C function. That's the whole design.

## Why this shape

WASM plugin systems (extism and friends) are great when you need sandboxing and language portability. They cost you a runtime boundary and a serialization hop on every call.

`libplugin` is the other trade: native code, direct function pointers, zero overhead — for when plugins are things you trust (your own builds, optional feature packs, mod loading for a game or bot). The ABI is 7 function pointers in `#[repr(C)]` layout; any language that can produce a C-ABI shared library can write one.

## Install

```toml
[dependencies]
libplugin = "0.1"
```

## Static plugins in 30 seconds

```rust
use libplugin::{Plugin, PluginManager};

struct Uptime;

impl Plugin for Uptime {
    fn name(&self) -> &str { "uptime" }
    fn on_message(&self, msg: &str) -> Option<String> {
        msg.contains("time").then(|| "it is tick o'clock".into())
    }
}

let mut mgr = PluginManager::new_static_only();
mgr.register(Box::new(Uptime));
mgr.load_all()?;
let replies = mgr.on_message("what time is it?");
// [("uptime", "it is tick o'clock")]
mgr.unload_all()?;
```

Default trait methods mean a plugin can implement just `name()` and be valid — the rest is opt-in.

## Dynamic plugins

A dynamic plugin is a `cdylib` exporting `libplugin_entry() -> PluginVTable`. Complete working example in [`examples/echo_plugin.rs`](examples/echo_plugin.rs) (~60 lines), host side in [`examples/manager.rs`](examples/manager.rs):

```rust
#[no_mangle]
pub extern "C" fn libplugin_entry() -> PluginVTable {
    PluginVTable { name, version, on_load, on_unload, on_message, on_tick, on_shutdown }
}
```

Discovery layout:

```
plugins/
└── echo_plugin/
    ├── manifest.toml
    └── echo_plugin.dll        (libecho_plugin.so on Linux)
```

```toml
# manifest.toml
[plugin]
name = "echo_plugin"
version = "1.0.0"

# optional: everything under [plugin.config] reaches on_load as JSON
[plugin.config]
prefix = "[echo]"
```

Build the plugin, point `PluginManager::new("plugins")` at the directory, call `discover_and_load()` — the host resolves each manifest, loads the library, and runs `on_load` with the config JSON. The `Library` handle lives inside `DynamicPlugin`, so function pointers stay valid for the plugin's lifetime. Verified end-to-end: the example dll loads, initializes, and echoes messages back across the ABI on both CI platforms.

## The lifecycle

| Call | Who gets it | Notes |
|---|---|---|
| `register` | static plugins | compile-time, same-name overwrites |
| `discover_and_load` | dynamic plugins | one bad plugin never blocks the rest |
| `load_all` | registered, not yet loaded | static plugins get `"{}"` config |
| `on_message` | loaded plugins | broadcast, collects `(name, response)` pairs |
| `on_tick` | loaded plugins | your scheduler drives the cadence |
| `unload_all` / `shutdown_all` | loaded / all | best-effort, failures recorded in state |

Per-plugin state is queryable: `state(name)` → `Registered` / `Loaded` / `Errored(..)` / `Unloaded`, and `health_status()` renders a one-line report per plugin.

## Notes

- **Silent-skip discovery**: an unreadable manifest, a missing library, or a failed load skips that plugin; the rest proceed. Failures land in the plugin's `Errored` state where they're inspectable.
- **ABI stability**: the vtable layout is the contract. The entry symbol is `libplugin_entry`; adding host features means new optional pointers, not breaking old ones.
- **Thread safety**: `Plugin: Send + Sync`; the host may call plugins from any thread. Dynamic plugins assert this on trust — the `unsafe impl Send/Sync` is only as good as the plugin author.
- Config crosses the boundary as a JSON string; message responses as UTF-8 written into a host-provided 64 KB buffer (truncated, never overflowed).

## License

MIT
