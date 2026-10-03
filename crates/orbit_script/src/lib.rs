//! OrbitEngine scripting (ADR 0015, M8-01): a sandboxed [Rhai] engine that
//! implements the VM's [`orbit_vm::ScriptHost`] hook.
//!
//! * **Sandbox.** `Engine::new_raw` plus the standard packages only: no file,
//!   network or clock access (`no_time`), no `eval`, no module loaded from
//!   disk (modules come from memory, see below), `print` / `debug` captured
//!   as logs. Limits ([`Limits`]): operations per call, call depth,
//!   expression depth, string / array / map sizes, number of modules.
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
use std::sync::{Arc, Mutex, MutexGuard};

use orbit_core::{ScalarType, Value, ValueType, VarOp};
use orbit_vm::{PluginContext, Rng, ScriptFailure, ScriptHost, ScriptOutcome};
use rhai::packages::{Package, StandardPackage};
use rhai::{Dynamic, Engine, EvalAltResult, ImmutableString, Module, Position, Scope, AST};

/// Version of the script API (ADR 0015 §2). Additions are additive.
pub const API_VERSION: u32 = 1;

pub use orbit_core::scripting::{
    SCRIPT_MODULE_DIR as MODULE_DIR, SCRIPT_MODULE_EXT as MODULE_EXT, is_valid_module_name,
    module_name, module_path, rhai_imports as imports, script_sources, story_uses_scripts,
};

/// Sandbox limits (ADR 0015 §3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// Rhai operations per script call (a loop that never ends stops here).
    pub max_operations: u64,
    pub max_call_levels: usize,
    /// Maximum nesting of expressions (global, in functions).
    pub max_expr_depth: usize,
    pub max_fn_expr_depth: usize,
    /// Bytes of one string.
    pub max_string_size: usize,
    pub max_array_size: usize,
    pub max_map_size: usize,
    /// Modules imported by one script.
    pub max_modules: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_operations: 100_000,
            max_call_levels: 32,
            max_expr_depth: 64,
            max_fn_expr_depth: 32,
            max_string_size: 64 * 1024,
            max_array_size: 10_000,
            max_map_size: 10_000,
            max_modules: 16,
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
        }
    }
}

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
        Ok(Value::Float(f))
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
        // Evaluated at every import: a module keeps no state between calls.
        let module = Module::eval_ast_as_new(Scope::new(), &ast, engine)
            .map_err(|e| Box::new(EvalAltResult::ErrorInModule(path.into(), e, pos)))?;
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
    let key = if expression {
        format!("\u{0}expr\u{0}{source}")
    } else {
        source.to_owned()
    };
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
        engine.set_module_resolver(MemoryResolver {
            modules: Arc::clone(&modules),
            asts: Arc::clone(&asts),
        });
        register_api(&mut engine, &slot);
        Self {
            engine,
            modules,
            asts,
            slot,
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
        *lock(&self.slot) = Some(state);
        let result = run(&self.engine).map_err(|e| failure(&e));
        let state = lock(&self.slot).take().expect("the call state is set");
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
    let message = match e {
        EvalAltResult::ErrorTooManyOperations(_) => "the script ran too long (operation limit)".to_owned(),
        EvalAltResult::ErrorStackOverflow(_) => "the script calls itself too deeply (call depth limit)".to_owned(),
        EvalAltResult::ErrorDataTooLarge(what, _) => format!("{what} is too large (size limit)"),
        EvalAltResult::ErrorTooManyModules(_) => "the script imports too many modules".to_owned(),
        other => other.to_string(),
    };
    ScriptFailure {
        message,
        line: (line > 0).then_some(line),
        column: (column > 0).then_some(column),
    }
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
