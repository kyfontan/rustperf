#![feature(rustc_private)]

extern crate rustc_ast;
extern crate rustc_driver;
extern crate rustc_lint;
extern crate rustc_session;
extern crate rustc_span;

use rustc_ast::ast::{
    Block, Expr, ExprKind, FieldDef, Item, ItemKind, Local, LocalKind, PatKind, Stmt, StmtKind,
    Ty, TyKind, VariantData,
};
use rustc_ast::token::LitKind as TokenLitKind;
use rustc_lint::{EarlyContext, EarlyLintPass, LintContext};
use rustc_session::{declare_lint, impl_lint_pass};
use rustc_span::{Span, Symbol};
use serde::Deserialize;
use std::ffi::CString;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

const DYLINT_VERSION: &str = "0.1.0";
const DYLINT_TOML_ENV: &str = "DYLINT_TOML";
static CONFIG: OnceLock<LoadedConfig> = OnceLock::new();

#[doc(hidden)]
#[unsafe(no_mangle)]
pub extern "C" fn dylint_version() -> *mut std::os::raw::c_char {
    CString::new(DYLINT_VERSION).unwrap().into_raw()
}

const VEC_WITH_CAPACITY_PATHS: &[&[&str]] = &[
    &["Vec", "with_capacity"],
    &["vec", "Vec", "with_capacity"],
];
const VEC_NEW_PATHS: &[&[&str]] = &[&["Vec", "new"], &["vec", "Vec", "new"]];
const HASH_MAP_NEW_PATHS: &[&[&str]] = &[
    &["HashMap", "new"],
    &["collections", "HashMap", "new"],
    &["std", "collections", "HashMap", "new"],
];
const HASH_SET_NEW_PATHS: &[&[&str]] = &[
    &["HashSet", "new"],
    &["collections", "HashSet", "new"],
    &["std", "collections", "HashSet", "new"],
];
const BTREE_MAP_NEW_PATHS: &[&[&str]] = &[
    &["BTreeMap", "new"],
    &["collections", "BTreeMap", "new"],
    &["std", "collections", "BTreeMap", "new"],
];
const BTREE_SET_NEW_PATHS: &[&[&str]] = &[
    &["BTreeSet", "new"],
    &["collections", "BTreeSet", "new"],
    &["std", "collections", "BTreeSet", "new"],
];
const LINKED_LIST_NEW_PATHS: &[&[&str]] = &[
    &["LinkedList", "new"],
    &["collections", "LinkedList", "new"],
    &["std", "collections", "LinkedList", "new"],
];
const STRING_NEW_PATHS: &[&[&str]] =
    &[&["String", "new"], &["string", "String", "new"], &["std", "string", "String", "new"]];

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
struct Config {
    small_vec_capacity_threshold: u64,
    vec_new_then_push_min_pushes: usize,
    hash_map_new_then_insert_min_inserts: usize,
    hash_set_new_then_insert_min_inserts: usize,
    string_new_then_push_str_min_calls: usize,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            small_vec_capacity_threshold: 64,
            vec_new_then_push_min_pushes: 2,
            hash_map_new_then_insert_min_inserts: 2,
            hash_set_new_then_insert_min_inserts: 2,
            string_new_then_push_str_min_calls: 2,
        }
    }
}

declare_lint! {
    pub SMALL_VEC_WITH_CAPACITY,
    Warn,
    "Vec::with_capacity(N) with a small compile-time constant; review array/SmallVec/ArrayVec instead"
}

declare_lint! {
    pub VEC_NEW_THEN_PUSH,
    Warn,
    "Vec::new() followed by immediate consecutive pushes; reserve capacity up front"
}

declare_lint! {
    pub LINKED_LIST_NEW,
    Warn,
    "construction of LinkedList, which is usually hostile to cache locality"
}

declare_lint! {
    pub FIELD_ORDER_BY_SIZE,
    Warn,
    "struct fields are not ordered by decreasing size, which can introduce padding"
}

declare_lint! {
    pub HASH_MAP_NEW_THEN_INSERT,
    Warn,
    "HashMap::new() followed by immediate inserts; reserve buckets up front"
}

declare_lint! {
    pub HASH_SET_NEW_THEN_INSERT,
    Warn,
    "HashSet::new() followed by immediate inserts; reserve buckets up front"
}

declare_lint! {
    pub STRING_NEW_THEN_PUSH_STR,
    Warn,
    "String::new() followed by immediate push_str calls; reserve capacity up front"
}

declare_lint! {
    pub VEC_NEW_THEN_RESERVE,
    Warn,
    "Vec::new() followed immediately by reserve/reserve_exact; use Vec::with_capacity instead"
}

declare_lint! {
    pub VEC_REMOVE_FIRST,
    Warn,
    "Vec::remove(0) shifts the whole tail and is usually hostile to cache-efficient queues"
}

declare_lint! {
    pub VEC_INSERT_FRONT,
    Warn,
    "Vec::insert(0, value) shifts the whole tail and is usually hostile to cache-efficient queues"
}

declare_lint! {
    pub BTREE_MAP_NEW,
    Warn,
    "construction of BTreeMap, which often loses to contiguous sorted storage on traversal-heavy paths"
}

declare_lint! {
    pub BTREE_SET_NEW,
    Warn,
    "construction of BTreeSet, which often loses to contiguous sorted storage on traversal-heavy paths"
}

#[derive(Default)]
struct MachineOrientedLints {
    config: Config,
}

impl MachineOrientedLints {
    fn new() -> Self {
        Self {
            config: config_or_default(env!("CARGO_PKG_NAME")),
        }
    }
}

impl_lint_pass!(MachineOrientedLints => [
    SMALL_VEC_WITH_CAPACITY,
    VEC_NEW_THEN_PUSH,
    LINKED_LIST_NEW,
    FIELD_ORDER_BY_SIZE,
    HASH_MAP_NEW_THEN_INSERT,
    HASH_SET_NEW_THEN_INSERT,
    STRING_NEW_THEN_PUSH_STR,
    VEC_NEW_THEN_RESERVE,
    VEC_REMOVE_FIRST,
    VEC_INSERT_FRONT,
    BTREE_MAP_NEW,
    BTREE_SET_NEW,
]);

#[unsafe(no_mangle)]
pub fn register_lints(sess: &rustc_session::Session, lint_store: &mut rustc_lint::LintStore) {
    init_config(sess);

    lint_store.register_lints(&[
        SMALL_VEC_WITH_CAPACITY,
        VEC_NEW_THEN_PUSH,
        LINKED_LIST_NEW,
        FIELD_ORDER_BY_SIZE,
        HASH_MAP_NEW_THEN_INSERT,
        HASH_SET_NEW_THEN_INSERT,
        STRING_NEW_THEN_PUSH_STR,
        VEC_NEW_THEN_RESERVE,
        VEC_REMOVE_FIRST,
        VEC_INSERT_FRONT,
        BTREE_MAP_NEW,
        BTREE_SET_NEW,
    ]);

    lint_store.register_early_pass(|| Box::new(MachineOrientedLints::new()));
}

fn init_config(sess: &rustc_session::Session) {
    if CONFIG.get().is_some() {
        return;
    }

    let config = match load_config(sess) {
        Ok(config) => config,
        Err(err) => sess.dcx().fatal(format!("could not read configuration file: {err}")),
    };

    let _ = CONFIG.set(config);
}

fn config_or_default(name: &str) -> Config {
    CONFIG
        .get()
        .and_then(|config| config.section(name))
        .unwrap_or_default()
}

fn load_config(sess: &rustc_session::Session) -> Result<LoadedConfig, String> {
    if let Ok(raw) = std::env::var(DYLINT_TOML_ENV) {
        return parse_config_toml(&raw);
    }

    let Some(source_file) = local_crate_source_file(sess) else {
        return Ok(LoadedConfig::default());
    };

    let Some(dylint_toml) = find_upwards(source_file.parent().unwrap_or_else(|| Path::new(".")), "dylint.toml") else {
        return Ok(LoadedConfig::default());
    };

    let raw = fs::read_to_string(&dylint_toml)
        .map_err(|err| format!("{}: {err}", dylint_toml.display()))?;
    parse_config_toml(&raw)
}

fn parse_config_toml(raw: &str) -> Result<LoadedConfig, String> {
    toml::from_str(raw).map_err(|err| err.to_string())
}

#[derive(Debug, Default, Deserialize)]
struct LoadedConfig {
    #[serde(default)]
    machine_oriented_lints: Config,
}

impl LoadedConfig {
    fn section(&self, name: &str) -> Option<Config> {
        (name == env!("CARGO_PKG_NAME")).then(|| self.machine_oriented_lints.clone())
    }
}

fn find_upwards(start: &Path, file_name: &str) -> Option<PathBuf> {
    let mut current = Some(start);

    while let Some(dir) = current {
        let candidate = dir.join(file_name);
        if candidate.is_file() {
            return Some(candidate);
        }
        current = dir.parent();
    }

    None
}

fn local_crate_source_file(sess: &rustc_session::Session) -> Option<PathBuf> {
    sess.local_crate_source_file()
        .and_then(|real| real.local_path().map(|path| path.to_path_buf()))
}

impl EarlyLintPass for MachineOrientedLints {
    fn check_expr(&mut self, cx: &EarlyContext<'_>, expr: &Expr) {
        lint_small_vec_with_capacity(cx, expr, &self.config);
        lint_linked_list_new(cx, expr);
        lint_btree_map_new(cx, expr);
        lint_btree_set_new(cx, expr);
        lint_vec_remove_first(cx, expr);
        lint_vec_insert_front(cx, expr);
    }

    fn check_block(&mut self, cx: &EarlyContext<'_>, block: &Block) {
        lint_vec_new_then_push(cx, block, &self.config);
        lint_hash_map_new_then_insert(cx, block, &self.config);
        lint_hash_set_new_then_insert(cx, block, &self.config);
        lint_string_new_then_push_str(cx, block, &self.config);
        lint_vec_new_then_reserve(cx, block);
    }

    fn check_item(&mut self, cx: &EarlyContext<'_>, item: &Item) {
        lint_field_order_by_size(cx, item);
    }
}

fn lint_small_vec_with_capacity(cx: &EarlyContext<'_>, expr: &Expr, config: &Config) {
    let Some(capacity) = small_vec_with_capacity_literal(expr) else {
        return;
    };

    if capacity > config.small_vec_capacity_threshold {
        return;
    }

    let mut diag = cx.sess().dcx().struct_span_warn(
        expr.span,
        format!("small constant capacity ({capacity}) in `Vec::with_capacity`"),
    );
    diag.help(
        "for tiny fixed-capacity collections, review `[T; N]`, `SmallVec<[T; N]>`, or `ArrayVec<T, N>` to reduce heap traffic and improve locality",
    );
    diag.emit();
}

fn lint_linked_list_new(cx: &EarlyContext<'_>, expr: &Expr) {
    if !is_linked_list_new(expr) {
        return;
    }

    let mut diag = cx
        .sess()
        .dcx()
        .struct_span_warn(expr.span, "`LinkedList::new()` used here");
    diag.help(
        "prefer contiguous storage (`Vec`, array, `SmallVec`, `VecDeque`) unless you benchmarked and proved a linked list is better",
    );
    diag.emit();
}

fn lint_btree_map_new(cx: &EarlyContext<'_>, expr: &Expr) {
    if !is_btree_map_new(expr) {
        return;
    }

    let mut diag = cx
        .sess()
        .dcx()
        .struct_span_warn(expr.span, "`BTreeMap::new()` used here");
    diag.help(
        "for traversal-heavy hot paths, review `Vec<(K, V)>` kept sorted, dense tables, or other contiguous layouts before defaulting to a tree",
    );
    diag.emit();
}

fn lint_btree_set_new(cx: &EarlyContext<'_>, expr: &Expr) {
    if !is_btree_set_new(expr) {
        return;
    }

    let mut diag = cx
        .sess()
        .dcx()
        .struct_span_warn(expr.span, "`BTreeSet::new()` used here");
    diag.help(
        "for traversal-heavy hot paths, review sorted `Vec<T>` or bitsets when those layouts fit the workload and preserve locality",
    );
    diag.emit();
}

fn lint_vec_new_then_push(cx: &EarlyContext<'_>, block: &Block, config: &Config) {
    let min_pushes = config.vec_new_then_push_min_pushes;
    let stmts = &block.stmts;

    if min_pushes == 0 {
        return;
    }

    for (index, stmt) in stmts.iter().enumerate() {
        let Some((binding, span)) = local_binding_with_init(stmt, is_vec_new_expr) else {
            continue;
        };

        let pushes = consecutive_push_count(binding, &stmts[index + 1..]);
        if pushes < min_pushes {
            continue;
        }

        let mut diag = cx.sess().dcx().struct_span_warn(
            span,
            format!("`Vec::new()` is followed by {pushes} consecutive `push` calls"),
        );
        diag.help(format!(
            "prefer `Vec::with_capacity({pushes})` or a fixed-capacity stack-backed representation when size is known"
        ));
        diag.emit();
    }
}

fn lint_hash_map_new_then_insert(cx: &EarlyContext<'_>, block: &Block, config: &Config) {
    let min_inserts = config.hash_map_new_then_insert_min_inserts;
    let stmts = &block.stmts;

    if min_inserts == 0 {
        return;
    }

    for (index, stmt) in stmts.iter().enumerate() {
        let Some((binding, span)) = local_binding_with_init(stmt, is_hash_map_new_expr) else {
            continue;
        };

        let inserts = consecutive_method_count(binding, &stmts[index + 1..], &["insert"], &[2]);
        if inserts < min_inserts {
            continue;
        }

        let mut diag = cx.sess().dcx().struct_span_warn(
            span,
            format!("`HashMap::new()` is followed by {inserts} consecutive `insert` calls"),
        );
        diag.help(format!(
            "prefer `HashMap::with_capacity({inserts})` when approximate size is known to reduce rehashing and bucket churn"
        ));
        diag.emit();
    }
}

fn lint_hash_set_new_then_insert(cx: &EarlyContext<'_>, block: &Block, config: &Config) {
    let min_inserts = config.hash_set_new_then_insert_min_inserts;
    let stmts = &block.stmts;

    if min_inserts == 0 {
        return;
    }

    for (index, stmt) in stmts.iter().enumerate() {
        let Some((binding, span)) = local_binding_with_init(stmt, is_hash_set_new_expr) else {
            continue;
        };

        let inserts = consecutive_method_count(binding, &stmts[index + 1..], &["insert"], &[1]);
        if inserts < min_inserts {
            continue;
        }

        let mut diag = cx.sess().dcx().struct_span_warn(
            span,
            format!("`HashSet::new()` is followed by {inserts} consecutive `insert` calls"),
        );
        diag.help(format!(
            "prefer `HashSet::with_capacity({inserts})` when approximate size is known to reduce rehashing and bucket churn"
        ));
        diag.emit();
    }
}

fn lint_string_new_then_push_str(cx: &EarlyContext<'_>, block: &Block, config: &Config) {
    let min_calls = config.string_new_then_push_str_min_calls;
    let stmts = &block.stmts;

    if min_calls == 0 {
        return;
    }

    for (index, stmt) in stmts.iter().enumerate() {
        let Some((binding, span)) = local_binding_with_init(stmt, is_string_new_expr) else {
            continue;
        };

        let pushes = consecutive_method_count(binding, &stmts[index + 1..], &["push_str"], &[1]);
        if pushes < min_calls {
            continue;
        }

        let mut diag = cx.sess().dcx().struct_span_warn(
            span,
            format!("`String::new()` is followed by {pushes} consecutive `push_str` calls"),
        );
        diag.help(
            "prefer `String::with_capacity(...)` when approximate final length is known to reduce reallocations and copies",
        );
        diag.emit();
    }
}

fn lint_vec_new_then_reserve(cx: &EarlyContext<'_>, block: &Block) {
    let stmts = &block.stmts;

    for (index, stmt) in stmts.iter().enumerate() {
        let Some((binding, span)) = local_binding_with_init(stmt, is_vec_new_expr) else {
            continue;
        };

        let Some(reserve_call) = stmts.get(index + 1) else {
            continue;
        };

        let Some(method) = method_call_for_stmt(reserve_call, binding) else {
            continue;
        };

        if !matches!(method.name, "reserve" | "reserve_exact") || method.arg_len != 1 {
            continue;
        }

        let mut diag = cx.sess().dcx().struct_span_warn(
            span,
            format!("`Vec::new()` is followed immediately by `{}`", method.name),
        );
        diag.help("prefer `Vec::with_capacity(n)` when reserving right after construction");
        diag.emit();
    }
}

fn lint_vec_remove_first(cx: &EarlyContext<'_>, expr: &Expr) {
    let Some(method_call) = method_call(expr) else {
        return;
    };

    if method_call.name != "remove" || method_call.arg_len != 1 {
        return;
    }

    let Some(index) = method_call.first_arg.and_then(integer_literal) else {
        return;
    };

    if index != 0 {
        return;
    }

    let mut diag = cx
        .sess()
        .dcx()
        .struct_span_warn(expr.span, "`Vec::remove(0)` used here");
    diag.help(
        "removing from the front shifts every later element; review `VecDeque`, swap-based removal, or a different queue layout",
    );
    diag.emit();
}

fn lint_vec_insert_front(cx: &EarlyContext<'_>, expr: &Expr) {
    let Some(method_call) = method_call(expr) else {
        return;
    };

    if method_call.name != "insert" || method_call.arg_len != 2 {
        return;
    }

    let Some(index) = method_call.first_arg.and_then(integer_literal) else {
        return;
    };

    if index != 0 {
        return;
    }

    let mut diag = cx
        .sess()
        .dcx()
        .struct_span_warn(expr.span, "`Vec::insert(0, ...)` used here");
    diag.help(
        "inserting at the front shifts every later element; review `VecDeque`, batching, or an append-oriented layout",
    );
    diag.emit();
}

fn lint_field_order_by_size(cx: &EarlyContext<'_>, item: &Item) {
    let ItemKind::Struct(_, _, variant) = &item.kind else {
        return;
    };

    let VariantData::Struct { fields, .. } = variant else {
        return;
    };

    if fields.len() < 2 {
        return;
    }

    let Some(sized_fields) = sized_named_fields(fields) else {
        return;
    };

    let Some((previous, previous_size, current, current_size)) =
        first_field_order_violation(&sized_fields)
    else {
        return;
    };

    let previous_name = field_name(previous);
    let current_name = field_name(current);

    let mut diag = cx.sess().dcx().struct_span_warn(
        current.span,
        format!(
            "field `{current_name}` ({current_size} bytes) comes after larger field `{previous_name}` ({previous_size} bytes)"
        ),
    );
    diag.help(
        "reorder fields from larger fixed-size primitives to smaller ones when representation and API constraints allow it",
    );
    diag.note(
        "this lint is intentionally conservative and currently checks only named structs made entirely of known primitive scalar fields",
    );
    diag.emit();
}

fn small_vec_with_capacity_literal(expr: &Expr) -> Option<u64> {
    let ExprKind::Call(callee, args) = &expr.kind else {
        return None;
    };

    if args.len() != 1 || !matches_any_path_suffix(callee, VEC_WITH_CAPACITY_PATHS) {
        return None;
    }

    integer_literal(&args[0])
}

fn is_linked_list_new(expr: &Expr) -> bool {
    let ExprKind::Call(callee, args) = &expr.kind else {
        return false;
    };

    args.is_empty() && matches_any_path_suffix(callee, LINKED_LIST_NEW_PATHS)
}

fn is_btree_map_new(expr: &Expr) -> bool {
    let ExprKind::Call(callee, args) = &expr.kind else {
        return false;
    };

    args.is_empty() && matches_any_path_suffix(callee, BTREE_MAP_NEW_PATHS)
}

fn is_btree_set_new(expr: &Expr) -> bool {
    let ExprKind::Call(callee, args) = &expr.kind else {
        return false;
    };

    args.is_empty() && matches_any_path_suffix(callee, BTREE_SET_NEW_PATHS)
}

fn is_hash_map_new_expr(expr: &Expr) -> bool {
    let ExprKind::Call(callee, args) = &expr.kind else {
        return false;
    };

    args.is_empty() && matches_any_path_suffix(callee, HASH_MAP_NEW_PATHS)
}

fn is_hash_set_new_expr(expr: &Expr) -> bool {
    let ExprKind::Call(callee, args) = &expr.kind else {
        return false;
    };

    args.is_empty() && matches_any_path_suffix(callee, HASH_SET_NEW_PATHS)
}

fn is_string_new_expr(expr: &Expr) -> bool {
    let ExprKind::Call(callee, args) = &expr.kind else {
        return false;
    };

    args.is_empty() && matches_any_path_suffix(callee, STRING_NEW_PATHS)
}

fn local_binding_with_init(
    stmt: &Stmt,
    predicate: impl FnOnce(&Expr) -> bool,
) -> Option<(Symbol, Span)> {
    let StmtKind::Let(local) = &stmt.kind else {
        return None;
    };

    let binding = local_binding_name(local)?;
    let init = local_init_expr(local)?;

    predicate(init).then_some((binding, stmt.span))
}

fn local_init_expr(local: &Local) -> Option<&Expr> {
    match &local.kind {
        LocalKind::Decl => None,
        LocalKind::Init(expr) => Some(expr),
        LocalKind::InitElse(expr, _) => Some(expr),
    }
}

fn consecutive_push_count(binding: Symbol, stmts: &[Stmt]) -> usize {
    consecutive_method_count(binding, stmts, &["push"], &[1])
}

fn consecutive_method_count(
    binding: Symbol,
    stmts: &[Stmt],
    method_names: &[&str],
    arg_lens: &[usize],
) -> usize {
    stmts
        .iter()
        .take_while(|stmt| is_method_stmt_for(stmt, binding, method_names, arg_lens))
        .count()
}

fn is_method_stmt_for(
    stmt: &Stmt,
    binding: Symbol,
    method_names: &[&str],
    arg_lens: &[usize],
) -> bool {
    match &stmt.kind {
        StmtKind::Expr(expr) | StmtKind::Semi(expr) => {
            is_method_call_for(expr, binding, method_names, arg_lens)
        }
        _ => false,
    }
}

fn is_method_call_for(
    expr: &Expr,
    binding: Symbol,
    method_names: &[&str],
    arg_lens: &[usize],
) -> bool {
    let Some(method_call) = method_call(expr) else {
        return false;
    };

    method_names.contains(&method_call.name)
        && arg_lens.contains(&method_call.arg_len)
        && is_path_expr_named(method_call.receiver, binding)
}

struct MethodCallInfo<'a> {
    name: &'a str,
    receiver: &'a Expr,
    first_arg: Option<&'a Expr>,
    arg_len: usize,
}

fn method_call(expr: &Expr) -> Option<MethodCallInfo<'_>> {
    let ExprKind::MethodCall(method_call) = &expr.kind else {
        return None;
    };

    Some(MethodCallInfo {
        name: method_call.seg.ident.name.as_str(),
        receiver: &method_call.receiver,
        first_arg: method_call.args.first().map(|arg| arg.as_ref()),
        arg_len: method_call.args.len(),
    })
}

fn method_call_for_stmt(stmt: &Stmt, binding: Symbol) -> Option<MethodCallInfo<'_>> {
    let expr = match &stmt.kind {
        StmtKind::Expr(expr) | StmtKind::Semi(expr) => expr,
        _ => return None,
    };

    let method_call = method_call(expr)?;
    is_path_expr_named(method_call.receiver, binding).then_some(method_call)
}

fn is_vec_new_expr(expr: &Expr) -> bool {
    let ExprKind::Call(callee, args) = &expr.kind else {
        return false;
    };

    args.is_empty() && matches_any_path_suffix(callee, VEC_NEW_PATHS)
}

fn integer_literal(expr: &Expr) -> Option<u64> {
    let ExprKind::Lit(token_lit) = &expr.kind else {
        return None;
    };

    match token_lit.kind {
        TokenLitKind::Integer => token_lit.symbol.as_str().replace('_', "").parse::<u64>().ok(),
        _ => None,
    }
}

fn local_binding_name(local: &Local) -> Option<Symbol> {
    let PatKind::Ident(_, ident, _) = &local.pat.kind else {
        return None;
    };

    Some(ident.name)
}

fn is_path_expr_named(expr: &Expr, name: Symbol) -> bool {
    let ExprKind::Path(_, path) = &expr.kind else {
        return false;
    };

    path.segments.len() == 1 && path.segments[0].ident.name == name
}

fn matches_any_path_suffix(expr: &Expr, candidates: &[&[&str]]) -> bool {
    candidates.iter().any(|suffix| path_suffix_of_expr(expr, suffix))
}

fn path_suffix_of_expr(expr: &Expr, suffix: &[&str]) -> bool {
    let ExprKind::Path(_, path) = &expr.kind else {
        return false;
    };

    let collected: Vec<&str> = path.segments.iter().map(|seg| seg.ident.name.as_str()).collect();
    collected.len() >= suffix.len() && &collected[collected.len() - suffix.len()..] == suffix
}

fn field_name(field: &FieldDef) -> String {
    field
        .ident
        .as_ref()
        .map(|ident| ident.name.to_string())
        .unwrap_or_else(|| "<field>".to_string())
}

fn sized_named_fields(fields: &[FieldDef]) -> Option<Vec<(&FieldDef, usize)>> {
    fields
        .iter()
        .map(|field| Some((field, primitive_type_size(&field.ty)?)))
        .collect()
}

fn first_field_order_violation<'a>(
    fields: &'a [(&'a FieldDef, usize)],
) -> Option<(&'a FieldDef, usize, &'a FieldDef, usize)> {
    let mut previous = *fields.first()?;

    for current in fields.iter().copied().skip(1) {
        if current.1 > previous.1 {
            return Some((previous.0, previous.1, current.0, current.1));
        }

        previous = current;
    }

    None
}

fn primitive_type_size(ty: &Ty) -> Option<usize> {
    let TyKind::Path(_, path) = &ty.kind else {
        return None;
    };

    let segment = path.segments.last()?;
    match segment.ident.name.as_str() {
        "u128" | "i128" => Some(16),
        "u64" | "i64" | "f64" => Some(8),
        "u32" | "i32" | "f32" => Some(4),
        "u16" | "i16" => Some(2),
        "u8" | "i8" | "bool" => Some(1),
        _ => None,
    }
}
