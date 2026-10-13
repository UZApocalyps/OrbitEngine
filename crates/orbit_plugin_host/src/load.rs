//! The only `unsafe` code of the editor's plugin support: opening a library,
//! reading its descriptor and calling its constructor.
//!
//! Soundness rests on the checks done before [`OpenLibrary::create`]: the
//! descriptor's magic, layout version and size, then the build fingerprint
//! (same `orbit_editor_api` sources, rustc, egui, target, profile and type
//! layouts as this editor), checked again by the plugin itself in its
//! constructor. A library is never unloaded (its code may still be referenced
//! by boxed closures, egui memory, threads): it stays mapped until the process
//! exits.
#![allow(unsafe_code)]

use std::ffi::{CStr, c_char, c_void};
use std::path::Path;

use orbit_editor_api::native::{
    CREATE_SYMBOL, DESCRIPTOR_LAYOUT_VERSION, DESCRIPTOR_SYMBOL, DescriptorInfo, Fingerprint,
    LAYOUT_HASH, MAGIC, NATIVE_ABI_VERSION, NativePlugin, PluginDescriptor, TYPE_HASH_SYMBOL,
};

use crate::verify::{Problem, ProbeOutput};

type DescriptorFn = unsafe extern "C" fn() -> *const PluginDescriptor;
type TypeHashFn = unsafe extern "C" fn() -> u64;
type CreateFn = unsafe extern "C" fn(u32, u64, u64, u64) -> *mut c_void;

/// A library opened and whose descriptor was read.
pub struct OpenLibrary {
    library: libloading::Library,
    pub info: DescriptorInfo,
}

fn symbol_name(name: &str) -> Vec<u8> {
    let mut v = name.as_bytes().to_vec();
    v.push(0);
    v
}

/// Reads a NUL-terminated string of the descriptor.
///
/// # Safety
/// `p` is null or points to a NUL-terminated string living as long as the
/// library.
unsafe fn text(p: *const c_char, field: &str) -> Result<String, Problem> {
    if p.is_null() {
        return Err(Problem::BadDescriptor(format!("`{field}` is missing")));
    }
    // SAFETY: see the function's contract.
    let s = unsafe { CStr::from_ptr(p) };
    s.to_str()
        .map(str::to_owned)
        .map_err(|_| Problem::BadDescriptor(format!("`{field}` is not UTF-8")))
}

/// Opens `path` and reads its descriptor. Loading a library runs its
/// initialisation code: call this in the probe process first (see
/// `verify::ProcessProber`) unless the library was probed already.
pub fn open(path: &Path) -> Result<OpenLibrary, Problem> {
    // SAFETY: loading runs the library's initialisers. Plugins are trusted
    // native code by design (the manager warns on install), and an unknown
    // library is first opened in a throw-away probe process.
    let library = unsafe { libloading::Library::new(path) }
        .map_err(|e| Problem::LoadFailed(e.to_string()))?;
    // SAFETY: the symbol, when present, has this signature (export_plugin!).
    let descriptor: DescriptorFn = unsafe {
        *library
            .get::<DescriptorFn>(&symbol_name(DESCRIPTOR_SYMBOL))
            .map_err(|_| Problem::BadDescriptor(format!("no `{DESCRIPTOR_SYMBOL}` function")))?
    };
    // SAFETY: an `extern "C"` function without arguments returning a pointer.
    let ptr = unsafe { descriptor() };
    if ptr.is_null() {
        return Err(Problem::BadDescriptor("null descriptor".into()));
    }
    // SAFETY: the first three fields (magic, layout version, size) are the
    // same in every descriptor layout; nothing else is read before they are
    // checked.
    let (magic, layout, size) = unsafe { ((*ptr).magic, (*ptr).layout_version, (*ptr).size) };
    if magic != MAGIC {
        return Err(Problem::BadDescriptor("wrong magic number".into()));
    }
    if layout != DESCRIPTOR_LAYOUT_VERSION || (size as usize) < std::mem::size_of::<PluginDescriptor>() {
        return Err(Problem::BadDescriptor(format!(
            "descriptor layout {layout} (size {size}), this editor reads layout {DESCRIPTOR_LAYOUT_VERSION}"
        )));
    }
    // SAFETY: the symbol, when present, has this signature (export_plugin!).
    let type_hash_fn: TypeHashFn = unsafe {
        *library
            .get::<TypeHashFn>(&symbol_name(TYPE_HASH_SYMBOL))
            .map_err(|_| Problem::BadDescriptor(format!("no `{TYPE_HASH_SYMBOL}` function")))?
    };
    // SAFETY: an `extern "C"` function without arguments returning a number
    // (it hashes `TypeId`s; nothing that can fail).
    let type_hash = unsafe { type_hash_fn() };
    // SAFETY: layout checked above: the descriptor has this exact type.
    let d = unsafe { &*ptr };
    // SAFETY: string fields point to NUL-terminated literals of the library.
    let info = unsafe {
        DescriptorInfo {
            id: text(d.id, "id")?,
            name: text(d.name, "name")?,
            version: text(d.version, "version")?,
            description: text(d.description, "description")?,
            author: text(d.author, "author")?,
            fingerprint: Fingerprint {
                api_version: text(d.api_version, "api_version")?,
                abi_version: d.abi_version,
                rustc: text(d.rustc, "rustc")?,
                egui: text(d.egui, "egui")?,
                target: text(d.target, "target")?,
                profile: text(d.profile, "profile")?,
                api_hash: format!("{:016x}", d.api_hash),
                layout_hash: format!("{:016x}", d.layout_hash),
                type_hash: format!("{type_hash:016x}"),
            },
        }
    };
    Ok(OpenLibrary { library, info })
}

/// Opens a library just to read its descriptor (it stays loaded: this is
/// meant for the probe process, or a library about to be loaded anyway).
pub fn read_descriptor(path: &Path) -> Result<DescriptorInfo, Problem> {
    let open = open(path)?;
    let info = open.info.clone();
    // Never unloaded (see the module docs).
    std::mem::forget(open.library);
    Ok(info)
}

impl OpenLibrary {
    /// Builds the plugin object. The caller has verified [`Self::info`]
    /// (fingerprint equal to this editor's): the plugin checks the ABI
    /// number and hashes again and returns nothing on a mismatch. The
    /// library is leaked on purpose: it stays loaded until the editor exits.
    pub fn create(self) -> Result<Box<dyn NativePlugin>, Problem> {
        // SAFETY: the symbol has this signature (export_plugin!).
        let create: CreateFn = unsafe {
            *self
                .library
                .get::<CreateFn>(&symbol_name(CREATE_SYMBOL))
                .map_err(|_| Problem::BadDescriptor(format!("no `{CREATE_SYMBOL}` function")))?
        };
        // SAFETY: plain values in, a pointer out. A panic in the plugin's
        // constructor is caught by the plugin (it returns null).
        let raw = unsafe {
            create(
                NATIVE_ABI_VERSION,
                orbit_editor_api::native::API_HASH,
                LAYOUT_HASH,
                orbit_editor_api::native::type_hash(),
            )
        };
        std::mem::forget(self.library);
        if raw.is_null() {
            return Err(Problem::LoadFailed(
                "the plugin refused this editor or its constructor failed".into(),
            ));
        }
        // SAFETY: `raw` is `Box::into_raw(Box<Box<dyn NativePlugin>>)` made
        // by `orbit_editor_api::native::create_plugin` in a library whose
        // fingerprint equals ours: same type layout, same global allocator
        // (the system one; neither side may set its own).
        let plugin = unsafe { Box::from_raw(raw.cast::<Box<dyn NativePlugin>>()) };
        Ok(*plugin)
    }
}

/// Body of `--probe-plugin <file>`: one JSON line (see [`ProbeOutput`]),
/// and whether the library is a readable plugin.
pub fn probe_main(path: &Path) -> (String, bool) {
    let result = crate::binary::check_library_file(path)
        .map_err(Problem::Binary)
        .and_then(|_| read_descriptor(path));
    let output = match result {
        Ok(info) => ProbeOutput { ok: true, info: Some(info), error: None },
        Err(problem) => ProbeOutput { ok: false, info: None, error: Some(problem.to_string()) },
    };
    let ok = output.ok;
    (serde_json::to_string(&output).unwrap_or_else(|_| "{\"ok\":false}".into()), ok)
}
