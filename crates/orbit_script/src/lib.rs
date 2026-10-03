//! OrbitEngine scripting (ADR 0015, M8-01): a sandboxed [Rhai] engine that
//! implements the VM's [`orbit_vm::ScriptHost`] hook.
//!
//! * **Sandbox.** `Engine::new_raw` plus the standard packages only: no file,
//!   network or clock access (`no_time`), no `eval`, no module loaded from
//!   disk (modules come from memory, see below), `print` / `debug` captured
//!   as logs, no capturing closures and no `curry` (values nested in
//!   function pointers escape the size limits). Limits ([`Limits`]):
//!   operations per call (modules and callbacks included), call depth,
//!   expression depth, string / array / map sizes (which also bound how
//!   deeply values nest), number of modules, import depth, no cyclic
//!   import. Security review: `docs/security/rhai-review.md`.
//! * **API** ([`API_VERSION`] 1): `get_var(name)`, `has_var(name)`,
//!   `set_var(name, value)` (queued, applied by the VM after the script, all
//!   or nothing, with the type checks of `VarOp::set`), `rand_int(a, b)`,
//!   `rand_float()`, `chance(p)` (the VM's seeded RNG), `branch(name)` (or
//!   return the branch name), `log(message)`, `api_version()`. Rhai
//!   conditions (`Expr::Rhai`) may only read: `set_var`, `branch` and the
//!   random functions are errors there.
//! * **Modules.** Shared sources (`scripts/<name>.rhai` of a project, the
//!   same entries in the pak) are given to [`ScriptEngine::new`] as a map
//!   and imported by name: `import "shop" as shop; shop::buy("sword")`.
//!   They are evaluated afresh at every import: a module keeps no state
//!   between two scripts (saves and rollback only see variables and RNG).
//! * **Determinism.** Same variables and RNG state give the same result,
//!   native and web (Rhai maps are ordered `BTreeMap`s; no clock, no
//!   unseeded random).
//!
//! [Rhai]: https://rhai.rs

use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use orbit_core::{ScalarType, Value, ValueType, VarOp};
use orbit_vm::{PluginContext, Rng, ScriptFailure, ScriptHost, ScriptOutcome};
use rhai::packages::{Package, StandardPackage};
use rhai::{
    Dynamic, Engine, EvalAltResult, ImmutableString, Module, NativeCallContext, Position, Scope, AST,
};

/// Version of the script API (ADR 0015 §2). Additions are additive.
pub const API_VERSION: u32 = 1;

pub use orbit_core::scripting::{
    SCRIPT_MODULE_DIR as MODULE_DIR, SCRIPT_MODULE_EXT as MODULE_EXT, is_valid_module_name,
    module_name, module_path, rhai_imports as imports, script_sources, story_uses_scripts,
};

/// Sandbox limits (ADR 0015 §3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// Rhai operations per script call (a loop that never ends stops here),
    /// counted across the modules it imports and the callbacks it gives to
    /// `sort`, `map`, `filter`… (Rhai counts those apart).
    pub max_operations: u64,
    pub max_call_levels: usize,
    /// Maximum nesting of expressions (global, in functions).
    pub max_expr_depth: usize,
    pub max_fn_expr_depth: usize,
    /// Bytes of one string.
    pub max_string_size: usize,
    pub max_array_size: usize,
    pub max_map_size: usize,
    /// Modules evaluated by one script call, counting the modules that
    /// modules import (an import in a loop counts every time). Imports
    /// also nest at most [`MAX_IMPORT_DEPTH`] deep and never in a cycle.
    pub max_modules: usize,
    /// Array / map elements of all the variables in scope together.
    pub max_scope_elements: usize,
    /// Bytes of the strings of all the variables in scope together.
    pub max_scope_string_bytes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_operations: 100_000,
            max_call_levels: 32,
            max_expr_depth: 64,
            max_fn_expr_depth: 32,
            max_string_size: 64 * 1024,
            // Rhai counts the elements of nested arrays / maps together:
            // these sizes also bound how deeply values can nest, and so the
            // native stack used to compare, print or drop them (an array
            // nested 10 000 deep overflowed a 2 MiB stack; see
            // docs/security/rhai-review.md).
            max_array_size: 1_024,
            max_map_size: 1_024,
            max_modules: 16,
            max_scope_elements: 32 * 1024,
            max_scope_string_bytes: 4 * 1024 * 1024,
        }
    }
}

/// How bad a [`Diagnostic`] is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
}

/// A problem found in a script without running it ([`ScriptEngine::check`]).
/// `line` / `column` are 1-based (0 when unknown).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    pub severity: Severity,
    pub line: usize,
    pub column: usize,
    pub message: String,
}

impl std::fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.line > 0 {
            write!(f, "{}:{}: {}", self.line, self.column, self.message)
        } else {
            f.write_str(&self.message)
        }
    }
}

// ------------------------------------------------------------------ state

/// Draws from the RNG copy and counts them (the VM's context is advanced by
/// the same count after a success).
struct CountingRng {
    rng: Rng,
    draws: u64,
}

impl CountingRng {
    fn next_u64(&mut self) -> u64 {
        self.draws += 1;
        self.rng.next_u64()
    }

    /// Uniform in `lo..=hi`, without modulo bias (as `PluginContext`).
    fn range(&mut self, lo: i64, hi: i64) -> i64 {
        let (lo, hi) = if lo <= hi { (lo, hi) } else { (hi, lo) };
        let span = (hi as i128 - lo as i128 + 1) as u128;
        if span > u64::MAX as u128 {
            return self.next_u64() as i64;
        }
        let span = span as u64;
        let zone = u64::MAX - (u64::MAX % span);
        loop {
            let r = self.next_u64();
            if r < zone {
                return (lo as i128 + (r % span) as i128) as i64;
            }
        }
    }

    fn float(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }
}

/// What one script call sees and produces.
struct CallState {
    /// `false` for a Rhai condition: reads only.
    writable: bool,
    vars: BTreeMap<String, Value>,
    writes: Vec<(String, Value)>,
    rng: CountingRng,
    branch: Option<String>,
    logs: Vec<String>,
    /// Modules being imported, outermost first (cycle and depth checks).
    importing: Vec<String>,
    /// Modules evaluated so far by this call.
    modules_loaded: usize,
}

impl CallState {
    fn new(writable: bool, vars: BTreeMap<String, Value>, rng: Rng) -> Self {
        Self {
            writable,
            vars,
            writes: Vec::new(),
            rng: CountingRng { rng, draws: 0 },
            branch: None,
            logs: Vec::new(),
            importing: Vec::new(),
            modules_loaded: 0,
        }
    }
}

/// How deeply imports may nest (`a` imports `b` imports `c`…). Each level
/// evaluates a module on the native stack.
pub const MAX_IMPORT_DEPTH: usize = 8;

type Slot = Arc<Mutex<Option<CallState>>>;

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

type RhaiResult<T> = Result<T, Box<EvalAltResult>>;

fn with_state<T>(slot: &Slot, f: impl FnOnce(&mut CallState) -> RhaiResult<T>) -> RhaiResult<T> {
    let mut guard = lock(slot);
    match guard.as_mut() {
        Some(state) => f(state),
        None => Err("the script API is only available while a script runs".into()),
    }
}

fn writable(state: &CallState, what: &str) -> RhaiResult<()> {
    if state.writable {
        Ok(())
    } else {
        Err(format!("`{what}` cannot be used in a condition (conditions only read variables)").into())
    }
}

// -------------------------------------------------------------- values

/// A story value as a Rhai value.
pub fn to_dynamic(value: &Value) -> Dynamic {
    match value {
        Value::Bool(b) => Dynamic::from_bool(*b),
        Value::Int(i) => Dynamic::from_int(*i),
        Value::Float(f) => Dynamic::from_float(*f),
        Value::Str(s) => Dynamic::from(ImmutableString::from(s.as_str())),
        Value::List(_, items) => {
            Dynamic::from_array(items.iter().map(to_dynamic).collect())
        }
        Value::Map(_, map) => {
            let m: rhai::Map = map
                .iter()
                .map(|(k, v)| (k.as_str().into(), to_dynamic(v)))
                .collect();
            Dynamic::from_map(m)
        }
    }
}

fn scalar_of(d: &Dynamic) -> Result<Value, String> {
    if let Ok(b) = d.as_bool() {
        Ok(Value::Bool(b))
    } else if let Ok(i) = d.as_int() {
        Ok(Value::Int(i))
    } else if let Ok(f) = d.as_float() {
        // NaN / infinity would make comparisons of the story lie (NaN is
        // never equal to itself) and spread through every computation.
        if f.is_finite() {
            Ok(Value::Float(f))
        } else {
            Err(format!("{f} is not a finite number"))
        }
    } else if d.is_string() {
        Ok(Value::Str(d.clone().into_string().unwrap_or_default()))
    } else if let Ok(c) = d.as_char() {
        Ok(Value::Str(c.to_string()))
    } else {
        Err(format!("a {} cannot be stored in a story variable", d.type_name()))
    }
}

fn common_scalar(values: &[Value], hint: Option<ScalarType>) -> Result<ScalarType, String> {
    let mut found: Option<ScalarType> = None;
    for v in values {
        let t = match v.value_type() {
            ValueType::Bool => ScalarType::Bool,
            ValueType::Int => ScalarType::Int,
            ValueType::Float => ScalarType::Float,
            ValueType::Str => ScalarType::Str,
            _ => return Err("collections cannot be nested in a story variable".into()),
        };
        found = Some(match (found, t) {
            (None, t) => t,
            (Some(a), b) if a == b => a,
            (Some(ScalarType::Int), ScalarType::Float) | (Some(ScalarType::Float), ScalarType::Int) => {
                ScalarType::Float
            }
            (Some(a), b) => return Err(format!("a collection cannot mix {a:?} and {b:?}")),
        });
    }
    Ok(found.or(hint).unwrap_or(ScalarType::Int))
}

fn coerce_items(values: Vec<Value>, t: ScalarType) -> Vec<Value> {
    values
        .into_iter()
        .map(|v| orbit_core::coerce(v.clone(), t.value_type()).unwrap_or(v))
        .collect()
}

/// A Rhai value as a story value. `hint` is the type of the variable it is
/// stored in (gives the element type of an empty collection).
pub fn from_dynamic(d: &Dynamic, hint: Option<ValueType>) -> Result<Value, String> {
    let elem_hint = match hint {
        Some(ValueType::List(t)) | Some(ValueType::Map(t)) => Some(t),
        _ => None,
    };
    if d.is_array() {
        let arr = d.clone().into_array().map_err(|e| e.to_string())?;
        let values = arr.iter().map(scalar_of).collect::<Result<Vec<_>, _>>()?;
        let t = common_scalar(&values, elem_hint)?;
        Ok(Value::list(t, coerce_items(values, t)))
    } else if d.is_map() {
        let map = d.clone().cast::<rhai::Map>();
        let mut entries = BTreeMap::new();
        let mut values = Vec::new();
        for (k, v) in &map {
            let v = scalar_of(v)?;
            values.push(v.clone());
            entries.insert(k.to_string(), v);
        }
        let t = common_scalar(&values, elem_hint)?;
        let entries: BTreeMap<String, Value> = entries
            .into_iter()
            .map(|(k, v)| (k, orbit_core::coerce(v.clone(), t.value_type()).unwrap_or(v)))
            .collect();
        Ok(Value::map(t, entries))
    } else {
        scalar_of(d)
    }
}

// -------------------------------------------------------------- modules

/// Resolves `import "name"` from the in-memory module sources.
struct MemoryResolver {
    modules: Arc<BTreeMap<String, String>>,
    asts: Arc<Mutex<HashMap<String, Arc<AST>>>>,
    slot: Slot,
    max_modules: usize,
}

impl MemoryResolver {
    /// Records that `path` starts being imported: refuses a cycle, too deep
    /// a nesting and too many modules for this call. Rhai evaluates a
    /// module with a fresh runtime state, so its own module and call-depth
    /// counters restart at every level: without this, `a` importing `b`
    /// importing `a` recursed until the native stack overflowed (a crash
    /// of the player, not a script error).
    fn enter(&self, path: &str) -> Result<(), String> {
        let mut guard = lock(&self.slot);
        let Some(st) = guard.as_mut() else {
            return Err("modules can only be imported while a script runs".into());
        };
        if st.importing.iter().any(|m| m == path) {
            let mut chain = st.importing.clone();
            chain.push(path.to_owned());
            return Err(format!("cyclic import: {}", chain.join(" -> ")));
        }
        if st.importing.len() >= MAX_IMPORT_DEPTH {
            return Err(format!("imports nest more than {MAX_IMPORT_DEPTH} deep"));
        }
        if st.modules_loaded >= self.max_modules {
            return Err(format!("more than {} modules imported by one script", self.max_modules));
        }
        st.modules_loaded += 1;
        st.importing.push(path.to_owned());
        Ok(())
    }

    fn leave(&self) {
        if let Some(st) = lock(&self.slot).as_mut() {
            st.importing.pop();
        }
    }
}

impl rhai::ModuleResolver for MemoryResolver {
    fn resolve(
        &self,
        engine: &Engine,
        _source: Option<&str>,
        path: &str,
        pos: Position,
    ) -> RhaiResult<rhai::Shared<Module>> {
        let Some(source) = self.modules.get(path) else {
            return Err(Box::new(EvalAltResult::ErrorModuleNotFound(path.into(), pos)));
        };
        let ast = compiled(engine, &self.asts, source, false).map_err(|e| {
            Box::new(EvalAltResult::ErrorInModule(
                path.into(),
                Box::new(EvalAltResult::ErrorParsing(*e.0, e.1)),
                pos,
            ))
        })?;
        let mut ast = (*ast).clone();
        ast.set_source(path);
        self.enter(path).map_err(|message| {
            Box::new(EvalAltResult::ErrorInModule(path.into(), message.into(), pos))
        })?;
        // Evaluated at every import: a module keeps no state between calls.
        let module = Module::eval_ast_as_new(Scope::new(), &ast, engine);
        self.leave();
        let module =
            module.map_err(|e| Box::new(EvalAltResult::ErrorInModule(path.into(), e, pos)))?;
        Ok(module.into())
    }
}

const AST_CACHE_CAP: usize = 512;

/// The AST of `source`, from the cache when it was compiled before.
fn compiled(
    engine: &Engine,
    cache: &Mutex<HashMap<String, Arc<AST>>>,
    source: &str,
    expression: bool,
) -> Result<Arc<AST>, rhai::ParseError> {
    // Both kinds are tagged: an untagged script key could equal the key of
    // an expression and run the other kind's AST.
    let key = format!("{}\u{0}{source}", if expression { 'e' } else { 's' });
    if let Some(ast) = lock(cache).get(&key) {
        return Ok(Arc::clone(ast));
    }
    let ast = Arc::new(if expression {
        engine.compile_expression(source)?
    } else {
        engine.compile(source)?
    });
    let mut cache = lock(cache);
    if cache.len() >= AST_CACHE_CAP {
        cache.clear();
    }
    cache.insert(key, Arc::clone(&ast));
    Ok(ast)
}

// -------------------------------------------------------------- engine

/// The sandboxed engine of a game: one per game, shared by its VMs.
pub struct ScriptEngine {
    engine: Engine,
    modules: Arc<BTreeMap<String, String>>,
    asts: Arc<Mutex<HashMap<String, Arc<AST>>>>,
    slot: Slot,
    /// Operations of the call being run, all runtime states together.
    operations: Arc<AtomicU64>,
    /// Set when a value too deep was printed during the call (it fails).
    too_deep: Arc<AtomicBool>,
    /// Serializes calls: the API functions work on the one `slot`.
    running: Mutex<()>,
    limits: Limits,
}

impl std::fmt::Debug for ScriptEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScriptEngine")
            .field("modules", &self.modules.keys().collect::<Vec<_>>())
            .field("limits", &self.limits)
            .finish()
    }
}

impl Default for ScriptEngine {
    fn default() -> Self {
        Self::new(BTreeMap::new())
    }
}

impl ScriptEngine {
    /// An engine with these shared modules (name -> Rhai source) and the
    /// default [`Limits`].
    pub fn new(modules: BTreeMap<String, String>) -> Self {
        Self::with_limits(modules, Limits::default())
    }

    pub fn with_limits(modules: BTreeMap<String, String>, limits: Limits) -> Self {
        let modules = Arc::new(modules);
        let asts: Arc<Mutex<HashMap<String, Arc<AST>>>> = Arc::default();
        let slot: Slot = Arc::default();
        let mut engine = Engine::new_raw();
        engine.register_global_module(StandardPackage::new().as_shared_module());
        engine
            .set_max_operations(limits.max_operations)
            .set_max_call_levels(limits.max_call_levels)
            .set_max_expr_depths(limits.max_expr_depth, limits.max_fn_expr_depth)
            .set_max_string_size(limits.max_string_size)
            .set_max_array_size(limits.max_array_size)
            .set_max_map_size(limits.max_map_size)
            .set_max_modules(limits.max_modules)
            .set_strict_variables(true)
            .set_fail_on_invalid_map_property(true);
        // No code from strings at run time.
        engine.disable_symbol("eval");
        // No curried function pointers: `Fn("f").curry(x)` nests values
        // inside function pointers, where the size limits do not see them.
        engine.disable_symbol("curry");
        // Rhai counts operations per runtime state, and evaluates modules
        // and the callbacks of `sort` / `map` / `filter`… with fresh ones
        // (a comparator could spend `max_operations` at every comparison).
        // This counts every operation of the call, whatever the state.
        let operations = Arc::new(AtomicU64::new(0));
        let too_deep = Arc::new(AtomicBool::new(false));
        let ops = Arc::clone(&operations);
        let deep = Arc::clone(&too_deep);
        let max_operations = limits.max_operations;
        engine.on_progress(move |_| {
            if ops.fetch_add(1, Ordering::Relaxed) >= max_operations || deep.load(Ordering::Relaxed) {
                Some(Dynamic::UNIT)
            } else {
                None
            }
        });
        engine.set_module_resolver(MemoryResolver {
            modules: Arc::clone(&modules),
            asts: Arc::clone(&asts),
            slot: Arc::clone(&slot),
            max_modules: limits.max_modules,
        });
        register_hardening(&mut engine, limits, &too_deep);
        register_api(&mut engine, &slot);
        Self {
            engine,
            modules,
            asts,
            slot,
            operations,
            too_deep,
            running: Mutex::new(()),
            limits,
        }
    }

    /// An engine ready to be attached to a VM.
    pub fn shared(modules: BTreeMap<String, String>) -> orbit_vm::SharedScriptHost {
        Arc::new(Self::new(modules))
    }

    pub fn limits(&self) -> Limits {
        self.limits
    }

    /// Names of the shared modules.
    pub fn module_names(&self) -> impl Iterator<Item = &str> {
        self.modules.keys().map(String::as_str)
    }

    /// Syntax problems of a script (`expression`: a Rhai condition), and
    /// imports of modules that do not exist. Does not run anything.
    pub fn check(&self, source: &str, expression: bool) -> Vec<Diagnostic> {
        let mut out = Vec::new();
        let result = if expression {
            self.engine.compile_expression(source)
        } else {
            self.engine.compile(source)
        };
        if let Err(e) = result {
            let (line, column) = position(e.1);
            out.push(Diagnostic {
                severity: Severity::Error,
                line,
                column,
                message: e.0.to_string(),
            });
        }
        if !expression {
            for (name, line, column) in imports(source) {
                if !self.modules.contains_key(&name) {
                    out.push(Diagnostic {
                        severity: Severity::Error,
                        line,
                        column,
                        message: format!("unknown module `{name}` (no {})", module_path(&name)),
                    });
                }
            }
        }
        out
    }

    /// Syntax problems of a shared module.
    pub fn check_module(&self, source: &str) -> Vec<Diagnostic> {
        self.check(source, false)
    }

    fn call(
        &self,
        state: CallState,
        run: impl FnOnce(&Engine) -> RhaiResult<Dynamic>,
    ) -> (Result<Dynamic, ScriptFailure>, CallState) {
        let _running = lock(&self.running);
        self.operations.store(0, Ordering::Relaxed);
        self.too_deep.store(false, Ordering::Relaxed);
        let fallback = CallState::new(state.writable, BTreeMap::new(), state.rng.rng);
        *lock(&self.slot) = Some(state);
        // A panic inside Rhai (or the API) must not take the game down: it
        // fails the script like any error (where unwinding exists).
        let mut result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(&self.engine)))
            .unwrap_or_else(|_| Err("the script engine failed (internal error)".into()))
            .map_err(|e| failure(&e));
        if self.too_deep.load(Ordering::Relaxed) {
            let (line, column) = match &result {
                Err(e) => (e.line, e.column),
                Ok(_) => (None, None),
            };
            result = Err(ScriptFailure {
                message: format!(
                    "arrays / maps nest more than {MAX_VALUE_DEPTH} deep (size limit)"
                ),
                line,
                column,
            });
        }
        let state = lock(&self.slot).take().unwrap_or(fallback);
        (result, state)
    }
}

/// 1-based line / column of a position (0 when unknown).
fn position(pos: Position) -> (usize, usize) {
    (pos.line().unwrap_or(0), pos.position().unwrap_or(0))
}

fn failure(e: &EvalAltResult) -> ScriptFailure {
    // The innermost error says what went wrong; the outer position says
    // where in the script being run.
    let (line, column) = position(e.position());
    let mut inner = e;
    while let EvalAltResult::ErrorInModule(_, err, _) | EvalAltResult::ErrorInFunctionCall(_, _, err, _) =
        inner
    {
        inner = err;
    }
    let message = match inner {
        EvalAltResult::ErrorTooManyOperations(_) | EvalAltResult::ErrorTerminated(..) => {
            "the script ran too long (operation limit)".to_owned()
        }
        EvalAltResult::ErrorStackOverflow(_) => "the script calls itself too deeply (call depth limit)".to_owned(),
        EvalAltResult::ErrorDataTooLarge(what, _) => format!("{what} is too large (size limit)"),
        EvalAltResult::ErrorTooManyModules(_) => "the script imports too many modules".to_owned(),
        // Other errors keep the whole chain ("… in module `shop`").
        _ => e.to_string(),
    };
    ScriptFailure {
        message,
        line: (line > 0).then_some(line),
        column: (column > 0).then_some(column),
    }
}

// ------------------------------------------------------------ hardening

/// How deeply arrays and maps may nest in a script value. Rhai prints,
/// compares and hashes values recursively: an array nested ~500 deep
/// overflowed a 1 MiB stack (release build) in `to_string` / `==`.
pub const MAX_VALUE_DEPTH: usize = 64;

/// Every that many variable reads, the variables of the running scope are
/// measured ([`audit`]).
const AUDIT_EVERY: u64 = 32;

fn data_too_large(what: &str) -> Box<EvalAltResult> {
    Box::new(EvalAltResult::ErrorDataTooLarge(what.into(), Position::NONE))
}

/// Whether `v` nests arrays / maps deeper than `left` more levels. Stops
/// descending past the limit, so it never recurses more than
/// [`MAX_VALUE_DEPTH`] + 1 deep.
fn nests_deeper(v: &Dynamic, left: usize) -> bool {
    if let Ok(a) = v.as_array_ref() {
        left == 0 || a.iter().any(|x| nests_deeper(x, left - 1))
    } else if let Ok(m) = v.as_map_ref() {
        left == 0 || m.values().any(|x| nests_deeper(x, left - 1))
    } else {
        false
    }
}

fn check_items<'a>(items: impl IntoIterator<Item = &'a Dynamic>) -> RhaiResult<()> {
    if items
        .into_iter()
        .any(|x| nests_deeper(x, MAX_VALUE_DEPTH - 1))
    {
        Err(data_too_large("Nesting of arrays / maps"))
    } else {
        Ok(())
    }
}

/// Totals of the variables in scope.
#[derive(Default)]
struct Footprint {
    elements: usize,
    string_bytes: usize,
}

impl Footprint {
    fn add(&mut self, v: &Dynamic, depth: usize, limits: &Limits) -> RhaiResult<()> {
        if depth > MAX_VALUE_DEPTH {
            return Err(data_too_large("Nesting of arrays / maps"));
        }
        if let Ok(a) = v.as_array_ref() {
            self.elements += a.len();
            self.check(limits)?;
            for x in a.iter() {
                self.add(x, depth + 1, limits)?;
            }
        } else if let Ok(m) = v.as_map_ref() {
            self.elements += m.len();
            self.check(limits)?;
            for (k, x) in m.iter() {
                self.string_bytes += k.len();
                self.add(x, depth + 1, limits)?;
            }
        } else if let Ok(s) = v.as_immutable_string_ref() {
            self.string_bytes += s.len();
        } else if let Ok(b) = v.as_blob_ref() {
            self.elements += b.len();
        }
        self.check(limits)
    }

    fn check(&self, limits: &Limits) -> RhaiResult<()> {
        if self.elements > limits.max_scope_elements {
            Err(data_too_large("Total size of the script's arrays / maps"))
        } else if self.string_bytes > limits.max_scope_string_bytes {
            Err(data_too_large("Total size of the script's strings"))
        } else {
            Ok(())
        }
    }
}

/// Measures every variable of the running scope. Rhai checks the size of
/// a value after most operations, but not after `map[new_key] = value`:
/// a loop of such assignments grew a map without bound (a gigabyte in a
/// fraction of a second). Reading the map's variable comes first, so
/// checking the scope at variable reads bounds that growth (to
/// [`AUDIT_EVERY`] assignments past the limit).
fn audit(ctx: &rhai::EvalContext, limits: &Limits) -> RhaiResult<()> {
    let mut total = Footprint::default();
    for v in ctx.scope().iter_raw().map(|(_, _, v)| v).chain(ctx.this_ptr()) {
        let before = (total.elements, total.string_bytes);
        total.add(v, 0, limits)?;
        // One value: the limits Rhai itself applies to a value.
        if total.elements - before.0 > limits.max_array_size + limits.max_map_size {
            return Err(data_too_large("Size of array / object map"));
        }
        if total.string_bytes - before.1 > limits.max_string_size {
            return Err(data_too_large("Length of string"));
        }
    }
    Ok(())
}

fn debug_item(ctx: &NativeCallContext, x: &mut Dynamic) -> String {
    let name = x.type_name();
    match ctx.call_native_fn_raw("to_debug", true, &mut [x]) {
        Ok(r) if r.is_string() => r.into_string().unwrap_or_default(),
        Ok(r) => ctx.engine().map_type_name(r.type_name()).to_owned(),
        Err(_) => ctx.engine().map_type_name(name).to_owned(),
    }
}

/// Rhai's own format of an array (`[1, "a"]`), after the depth check.
///
/// Too deep, it does not fail: string interpolation and `+` swallow an
/// error of `to_string` and fall back to a recursive `Display` of the
/// value. It flags the call instead (`too_deep`), which the next operation
/// turns into an error, and gives a placeholder.
fn format_array(ctx: NativeCallContext, a: &mut rhai::Array, too_deep: &AtomicBool) -> ImmutableString {
    if check_items(a.iter()).is_err() {
        too_deep.store(true, Ordering::Relaxed);
        return "[…]".into();
    }
    let items: Vec<String> = a.iter_mut().map(|x| debug_item(&ctx, x)).collect();
    format!("[{}]", items.join(", ")).into()
}

/// Rhai's own format of a map (`#{"a": 1}`), after the depth check (see
/// [`format_array`]).
fn format_map(ctx: NativeCallContext, m: &mut rhai::Map, too_deep: &AtomicBool) -> ImmutableString {
    if check_items(m.values()).is_err() {
        too_deep.store(true, Ordering::Relaxed);
        return "#{…}".into();
    }
    let items: Vec<String> = m
        .iter_mut()
        .map(|(k, v)| format!("{:?}: {}", k.as_str(), debug_item(&ctx, v)))
        .collect();
    format!("#{{{}}}", items.join(", ")).into()
}

fn item_eq(ctx: &NativeCallContext, a: &mut Dynamic, b: &mut Dynamic) -> RhaiResult<bool> {
    let same_type = a.type_id() == b.type_id();
    match ctx.call_native_fn_raw("==", true, &mut [a, b]) {
        Ok(r) => Ok(r.as_bool().unwrap_or(false)),
        Err(e) => match *e {
            EvalAltResult::ErrorFunctionNotFound(ref sig, ..) if sig.starts_with("==") && !same_type => {
                Ok(false)
            }
            _ => Err(e),
        },
    }
}

/// Rhai's array `==`, after the depth check.
fn array_eq(ctx: NativeCallContext, a: &mut rhai::Array, mut b: rhai::Array) -> RhaiResult<bool> {
    check_items(a.iter().chain(b.iter()))?;
    if a.len() != b.len() {
        return Ok(false);
    }
    for (x, y) in a.iter_mut().zip(b.iter_mut()) {
        if !item_eq(&ctx, x, y)? {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Rhai's map `==`, after the depth check.
fn map_eq(ctx: NativeCallContext, a: &mut rhai::Map, mut b: rhai::Map) -> RhaiResult<bool> {
    check_items(a.values().chain(b.values()))?;
    if a.len() != b.len() {
        return Ok(false);
    }
    for (k, x) in a.iter_mut() {
        let Some(y) = b.get_mut(k) else {
            return Ok(false);
        };
        let equal = ctx
            .call_native_fn_raw("==", true, &mut [x, y])?
            .as_bool()
            .unwrap_or(false);
        if !equal {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Overrides of the standard package (functions registered on the engine
/// come before packages): no blocking `sleep`, and depth-checked printing
/// and comparison of arrays / maps (the standard ones recurse without
/// bound).
fn register_hardening(engine: &mut Engine, limits: Limits, too_deep: &Arc<AtomicBool>) {
    let no_sleep = || -> RhaiResult<()> { Err("`sleep` is not available in game scripts".into()) };
    engine.register_fn("sleep", move |_: rhai::INT| no_sleep());
    engine.register_fn("sleep", move |_: rhai::FLOAT| no_sleep());
    for name in ["to_string", "to_debug", "print", "debug"] {
        let flag = Arc::clone(too_deep);
        engine.register_fn(name, move |ctx: NativeCallContext, a: &mut rhai::Array| {
            format_array(ctx, a, &flag)
        });
        let flag = Arc::clone(too_deep);
        engine.register_fn(name, move |ctx: NativeCallContext, m: &mut rhai::Map| {
            format_map(ctx, m, &flag)
        });
    }
    engine.register_fn("==", array_eq);
    engine.register_fn("!=", |ctx: NativeCallContext, a: &mut rhai::Array, b: rhai::Array| {
        array_eq(ctx, a, b).map(|r| !r)
    });
    engine.register_fn("==", map_eq);
    engine.register_fn("!=", |ctx: NativeCallContext, a: &mut rhai::Map, b: rhai::Map| {
        map_eq(ctx, a, b).map(|r| !r)
    });
    let reads = AtomicU64::new(0);
    // `on_var` is flagged "volatile" (deprecated-as-a-warning) by Rhai: it
    // is stable in 1.x; a change would show up in the adversarial tests.
    #[allow(deprecated)]
    engine.on_var(move |_, _, ctx| {
        if reads.fetch_add(1, Ordering::Relaxed).is_multiple_of(AUDIT_EVERY) {
            audit(&ctx, &limits)?;
        }
        Ok(None)
    });
}

fn register_api(engine: &mut Engine, slot: &Slot) {
    engine.register_fn("api_version", || API_VERSION as rhai::INT);

    let s = Arc::clone(slot);
    engine.register_fn("get_var", move |name: &str| -> RhaiResult<Dynamic> {
        with_state(&s, |st| match st.vars.get(name) {
            Some(v) => Ok(to_dynamic(v)),
            None => Err(format!("unknown variable `{name}`").into()),
        })
    });

    let s = Arc::clone(slot);
    engine.register_fn("has_var", move |name: &str| -> RhaiResult<bool> {
        with_state(&s, |st| Ok(st.vars.contains_key(name)))
    });

    let s = Arc::clone(slot);
    engine.register_fn("set_var", move |name: &str, value: Dynamic| -> RhaiResult<()> {
        with_state(&s, |st| {
            writable(st, "set_var")?;
            let Some(current) = st.vars.get(name) else {
                return Err(format!("unknown variable `{name}` (declare it in the project)").into());
            };
            let hint = current.value_type();
            let value = from_dynamic(&value, Some(hint)).map_err(|e| format!("set_var(`{name}`): {e}"))?;
            st.vars.insert(name.to_owned(), value.clone());
            st.writes.push((name.to_owned(), value));
            Ok(())
        })
    });

    let s = Arc::clone(slot);
    engine.register_fn("rand_int", move |lo: rhai::INT, hi: rhai::INT| -> RhaiResult<rhai::INT> {
        with_state(&s, |st| {
            writable(st, "rand_int")?;
            Ok(st.rng.range(lo, hi))
        })
    });

    let s = Arc::clone(slot);
    engine.register_fn("rand_float", move || -> RhaiResult<rhai::FLOAT> {
        with_state(&s, |st| {
            writable(st, "rand_float")?;
            Ok(st.rng.float())
        })
    });

    let s = Arc::clone(slot);
    engine.register_fn("chance", move |p: rhai::FLOAT| -> RhaiResult<bool> {
        with_state(&s, |st| {
            writable(st, "chance")?;
            Ok(st.rng.float() < p)
        })
    });

    let s = Arc::clone(slot);
    engine.register_fn("branch", move |branch: &str| -> RhaiResult<()> {
        with_state(&s, |st| {
            writable(st, "branch")?;
            st.branch = Some(branch.to_owned());
            Ok(())
        })
    });

    let s = Arc::clone(slot);
    engine.register_fn("log", move |message: &str| -> RhaiResult<()> {
        with_state(&s, |st| {
            push_log(st, message);
            Ok(())
        })
    });

    let s = Arc::clone(slot);
    engine.on_print(move |message| {
        if let Some(st) = lock(&s).as_mut() {
            push_log(st, message);
        }
    });
    let s = Arc::clone(slot);
    engine.on_debug(move |message, _, _| {
        if let Some(st) = lock(&s).as_mut() {
            push_log(st, message);
        }
    });
}

/// Logs are capped so that a looping script cannot flood the console.
const MAX_LOGS: usize = 64;

fn push_log(st: &mut CallState, message: &str) {
    if st.logs.len() < MAX_LOGS {
        st.logs.push(message.chars().take(1024).collect());
    }
}

impl ScriptHost for ScriptEngine {
    fn run(&self, source: &str, ctx: &mut PluginContext<'_>) -> Result<ScriptOutcome, ScriptFailure> {
        let ast = compiled(&self.engine, &self.asts, source, false).map_err(|e| {
            let (line, column) = position(e.1);
            ScriptFailure {
                message: e.0.to_string(),
                line: (line > 0).then_some(line),
                column: (column > 0).then_some(column),
            }
        })?;
        let state = CallState::new(true, ctx.variables(), ctx.rng());
        let (result, state) = self.call(state, |engine| {
            engine.eval_ast_with_scope::<Dynamic>(&mut Scope::new(), &ast)
        });
        let value = result?;
        // Commit the draws and the writes to the context; the VM applies
        // them (all or nothing) after this returns.
        for _ in 0..state.rng.draws {
            ctx.random_u64();
        }
        for (name, value) in state.writes {
            if let Err(e) = ctx.apply(VarOp::set(&name, value)) {
                return Err(ScriptFailure::new(format!("set_var(`{name}`): {e}")));
            }
        }
        let branch = if value.is_string() {
            value.into_string().ok().filter(|s| !s.is_empty())
        } else {
            None
        }
        .or(state.branch);
        Ok(ScriptOutcome {
            branch,
            logs: state.logs,
        })
    }

    fn eval(
        &self,
        source: &str,
        vars: &dyn Fn(&str) -> Option<Value>,
    ) -> Result<Value, ScriptFailure> {
        let ast = compiled(&self.engine, &self.asts, source, true).map_err(|e| {
            let (line, column) = position(e.1);
            ScriptFailure {
                message: e.0.to_string(),
                line: (line > 0).then_some(line),
                column: (column > 0).then_some(column),
            }
        })?;
        // A condition can only read the variables it names with `get_var(…)`:
        // collect them from the source to snapshot just those.
        let mut snapshot = BTreeMap::new();
        for name in var_names(source) {
            if let Some(v) = vars(&name) {
                snapshot.insert(name, v);
            }
        }
        let state = CallState::new(false, snapshot, Rng::new(0));
        let (result, _) = self.call(state, |engine| {
            engine.eval_ast_with_scope::<Dynamic>(&mut Scope::new(), &ast)
        });
        let value = result?;
        from_dynamic(&value, None).map_err(ScriptFailure::new)
    }
}

/// String literals given to `get_var(…)` / `has_var(…)` in a condition.
fn var_names(source: &str) -> Vec<String> {
    let mut out = Vec::new();
    for prefix in ["get_var(", "has_var("] {
        let mut rest = source;
        while let Some(at) = rest.find(prefix) {
            rest = &rest[at + prefix.len()..];
            let lit = rest.trim_start();
            if let Some(lit) = lit.strip_prefix('"')
                && let Some(end) = lit.find('"')
            {
                out.push(lit[..end].to_owned());
            }
        }
    }
    out
}

/// What a test run in the sandbox produced ([`ScriptEngine::test_run`]).
#[derive(Clone, Debug, PartialEq)]
pub struct TestRun {
    /// `Err`: the script failed (nothing would be applied).
    pub result: Result<Option<String>, ScriptFailure>,
    /// Variables the script would change: name, before, after.
    pub changes: Vec<(String, Value, Value)>,
    pub logs: Vec<String>,
    /// Random numbers drawn.
    pub draws: u64,
}

impl ScriptEngine {
    /// Runs a script in the sandbox against these variables (the story's
    /// initial values, typically) without any VM: what the editor's "Run in
    /// sandbox" button and the MCP `script_test` tool show. Nothing is
    /// applied anywhere.
    pub fn test_run(&self, source: &str, vars: &BTreeMap<String, Value>, seed: u64) -> TestRun {
        let empty = BTreeMap::new();
        let mut ctx = PluginContext::new(vars, &empty, Rng::new(seed));
        let before = ctx.rng();
        let result = self.run(source, &mut ctx);
        let mut after_rng = before;
        let mut draws = 0;
        while after_rng != ctx.rng() && draws < 1_000_000 {
            after_rng.next_u64();
            draws += 1;
        }
        match result {
            Ok(outcome) => {
                let changes = ctx
                    .queued()
                    .iter()
                    .map(|op| op.var.clone())
                    .collect::<std::collections::BTreeSet<_>>()
                    .into_iter()
                    .filter_map(|name| {
                        let old = vars.get(&name)?.clone();
                        let new = ctx.var(&name)?;
                        (old != new).then_some((name, old, new))
                    })
                    .collect();
                TestRun {
                    result: Ok(outcome.branch),
                    changes,
                    logs: outcome.logs,
                    draws,
                }
            }
            Err(e) => TestRun {
                result: Err(e),
                changes: Vec::new(),
                logs: Vec::new(),
                draws: 0,
            },
        }
    }
}

#[cfg(test)]
mod tests;
