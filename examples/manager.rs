// SPDX-License-Identifier: MIT
//! Host demo — static registration, lifecycle, and dynamic loading.
//!
//! Static part runs out of the box.
//! Dynamic part needs the echo plugin built first:
//!
//!     cargo build --example echo_plugin
//!     cargo run --example manager

use libplugin::{Plugin, PluginManager};

struct UptimePlugin {
    label: &'static str,
}

impl Plugin for UptimePlugin {
    fn name(&self) -> &str {
        "uptime"
    }
    fn version(&self) -> &str {
        "1.0.0"
    }
    fn on_message(&self, msg: &str) -> Option<String> {
        if msg.contains("time") {
            Some(format!("{}: it is tick o'clock", self.label))
        } else {
            None
        }
    }
}

fn main() {
    // ── Static registration ──
    let mut mgr = PluginManager::new_static_only();
    mgr.register(Box::new(UptimePlugin { label: "clock" }));
    mgr.load_all().unwrap();

    let responses = mgr.on_message("what time is it?");
    for (name, response) in &responses {
        println!("static {name}: {response}");
    }

    // ── Dynamic discovery (if the plugin dir is set up) ──
    // Build the example plugin, then arrange:
    //   plugins/echo_plugin/manifest.toml
    //   plugins/echo_plugin/libecho_plugin.{so,dll,dylib}
    let dir = std::env::temp_dir().join("libplugin_demo_plugins");
    if dir.exists() {
        let mut dyn_mgr = PluginManager::new(&dir);
        match dyn_mgr.discover_and_load() {
            Ok(()) => {
                for (name, status) in dyn_mgr.health_status() {
                    println!("dynamic {name}: {status}");
                }
                for (name, response) in dyn_mgr.on_message("hello from the host") {
                    println!("dynamic {name}: {response}");
                }
            }
            Err(e) => println!("discovery failed: {e}"),
        }
    } else {
        println!(
            "(dynamic part skipped: {} not set up — see the README)",
            dir.display()
        );
    }

    let _ = mgr.unload_all();
    let _ = mgr.shutdown_all();
}
