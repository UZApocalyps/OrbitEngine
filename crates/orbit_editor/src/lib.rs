//! The OrbitEngine editor (native, Windows and Linux).
//!
//! `EditorCore` is the editor without window or GPU (plugin registry,
//! project session, dock, palette, dialogs): it is tested headlessly.
//! `app` is the thin window / wgpu shell around it.

pub mod app;
pub mod args;
pub mod assets;
pub mod bench;
pub mod bench_graph;
pub mod builtin;
pub mod config;
pub mod core;
pub mod diagnostics;
pub mod dock;
pub mod fuzzy;
pub mod i18n;
pub mod live;
pub mod logging;
pub mod palette;
pub mod perf;
pub mod preview_host;
pub mod rig_host;
pub mod scene3d_host;
pub mod search;
pub mod session;
pub mod storage;
pub mod templates;
pub mod theme;
pub mod thumbs;
pub mod ui;
pub mod watch;
