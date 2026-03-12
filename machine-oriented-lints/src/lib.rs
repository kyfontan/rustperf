#![feature(rustc_private)]

extern crate rustc_ast;
extern crate rustc_lint;
extern crate rustc_session;
extern crate rustc_span;

use rustc_ast::ast::{Block, Expr, ExprKind, GenericArg, GenericArgs, Item, ItemKind, Local, Stmt, StmtKind, Ty, TyKind};
use rustc_lint::{EarlyContext, EarlyLintPass};
use rustc_session::{declare_lint, impl_lint_pass};
use rustc_span::{Span, Symbol};
use serde::Deserialize;

dylint_linting::dylint_library!();

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
struct Config {
    /// Warn when `Vec::with_capacity(N)` uses a compile-time constant `N <= small_vec_capacity_threshold`.
    small_vec_capacity_threshold: u128,
    /// Warn when `Vec::new()` is followed by at least this many consecutive pushes in the same block.
    vec_new_then_push_min_pushes: usize,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            small_vec_capacity_threshold: 64,
            vec_new_then_push_min_pushes: 2,
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

#[derive(Default)]
struct MachineOrientedLints {
    config: Config,
}

impl MachineOrientedLints {
    fn new() -> Self {
        Self {
            config: dylint_linting::config_or_default(env!("CARGO_PKG_NAME")),
        }
    }
}

impl_lint_pass!(MachineOrientedLints => [
    SMALL_VEC_WITH_CAPACITY,
    VEC_NEW_THEN_PUSH,
    LINKED_LIST_NEW,
]);

#[unsafe(no_mangle)]
pub fn register_lints(sess: &rustc_session::Session, lint_store: &mut rustc_lint::LintStore) {
    dylint_linting::init_config(sess);
    lint_store.register_lints(&[
        SMALL_VEC_WITH_CAPACITY,
        VEC_NEW_THEN_PUSH,
        LINKED_LIST_NEW,
    ]);
    lint_store.register_early_pass(|| Box::new(MachineOrientedLints::new()));
}

impl EarlyLintPass for MachineOrientedLints {
    fn check_expr(&mut self, cx: &EarlyContext<'_>, expr: &Expr) {
        if let Some(capacity) = small_vec_with_capacity_literal(expr) {
            if capacity <= self.config.small_vec_capacity_threshold {
                cx.opt_span_lint(SMALL_VEC_WITH_CAPACITY, Some(expr.span), |diag| {
                    diag.primary_message(format!(
                        "small constant capacity ({capacity}) in `Vec::with_capacity`"
                    ));
                    diag.help(
                        "for tiny fixed-capacity collections, review `[T; N]`, `SmallVec<[T; N]>`, or `ArrayVec<T, N>` to reduce heap traffic and improve locality"
                    );
                });
            }
        }

        if is_constructor_path(expr, &["LinkedList", "new"]) || is_constructor_path(expr, &["collections", "LinkedList", "new"]) {
            cx.opt_span_lint(LINKED_LIST_NEW, Some(expr.span), |diag| {
                diag.primary_message("`LinkedList::new()` used here");
                diag.help(
                    "prefer contiguous storage (`Vec`, array, `SmallVec`, `VecDeque`) unless you benchmarked and proved a linked list is better"
                );
            });
        }
    }

    fn check_block(&mut self, cx: &EarlyContext<'_>, block: &Block) {
        let min_pushes = self.config.vec_new_then_push_min_pushes;
        let stmts = &block.stmts;

        for i in 0..stmts.len() {
            let Some((binding, span)) = local_vec_new_binding(&stmts[i]) else {
                continue;
            };

            let pushes = consecutive_push_count(binding, &stmts[i + 1..]);
            if pushes >= min_pushes {
                cx.opt_span_lint(VEC_NEW_THEN_PUSH, Some(span), |diag| {
                    diag.primary_message(format!(
                        "`Vec::new()` is followed by {pushes} consecutive `push` calls"
                    ));
                    diag.help(format!(
                        "prefer `Vec::with_capacity({pushes})` or a fixed-capacity stack-backed representation when size is known"
                    ));
                });
            }
        }
    }

    fn check_item(&mut self, cx: &EarlyContext<'_>, item: &Item) {
        // Lightweight extra detection for type aliases like `type Q = std::collections::LinkedList<u8>;`
        if let ItemKind::TyAlias(boxed_ty, ..) = &item.kind {
            if is_type_path_suffix(boxed_ty, &["LinkedList"]) || is_type_path_suffix(boxed_ty, &["collections", "LinkedList"]) {
                cx.opt_span_lint(LINKED_LIST_NEW, Some(item.span), |diag| {
                    diag.primary_message("type alias to `LinkedList` detected");
                    diag.help(
                        "linked lists usually hurt spatial locality because every node dereference can miss cache"
                    );
                });
            }
        }
    }
}

fn small_vec_with_capacity_literal(expr: &Expr) -> Option<u128> {
    let ExprKind::Call(callee, args) = &expr.kind else {
        return None;
    };
    if args.len() != 1 {
        return None;
    }
    if !path_suffix_of_expr(callee, &["Vec", "with_capacity"]) && !path_suffix_of_expr(callee, &["vec", "Vec", "with_capacity"]) {
        return None;
    }
    integer_literal(&args[0])
}

fn is_constructor_path(expr: &Expr, suffix: &[&str]) -> bool {
    let ExprKind::Call(callee, _) = &expr.kind else {
        return false;
    };
    path_suffix_of_expr(callee, suffix)
}

fn local_vec_new_binding(stmt: &Stmt) -> Option<(Symbol, Span)> {
    let StmtKind::Let(local) = &stmt.kind else {
        return None;
    };

    let binding = local_binding_name(local)?;
    let init = local.init.as_ref()?;

    if is_vec_new_expr(init) {
        Some((binding, stmt.span))
    } else {
        None
    }
}

fn consecutive_push_count(binding: Symbol, stmts: &[Stmt]) -> usize {
    let mut count = 0;
    for stmt in stmts {
        if is_push_stmt_for(stmt, binding) {
            count += 1;
        } else {
            break;
        }
    }
    count
}

fn is_push_stmt_for(stmt: &Stmt, binding: Symbol) -> bool {
    match &stmt.kind {
        StmtKind::Expr(expr) | StmtKind::Semi(expr) => is_push_call_for(expr, binding),
        _ => false,
    }
}

fn is_push_call_for(expr: &Expr, binding: Symbol) -> bool {
    let ExprKind::MethodCall(segment, receiver, args, _) = &expr.kind else {
        return false;
    };

    if segment.ident.name != Symbol::intern("push") {
        return false;
    }

    if args.len() != 1 {
        return false;
    }

    is_path_expr_named(receiver, binding)
}

fn is_vec_new_expr(expr: &Expr) -> bool {
    let ExprKind::Call(callee, args) = &expr.kind else {
        return false;
    };
    args.is_empty() && (path_suffix_of_expr(callee, &["Vec", "new"]) || path_suffix_of_expr(callee, &["vec", "Vec", "new"]))
}

fn integer_literal(expr: &Expr) -> Option<u128> {
    let ExprKind::Lit(token_lit) = &expr.kind else {
        return None;
    };
    token_lit.kind.as_str().parse().ok()
}

fn local_binding_name(local: &Local) -> Option<Symbol> {
    let rustc_ast::ast::PatKind::Ident(_, ident, _) = &local.pat.kind else {
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

fn path_suffix_of_expr(expr: &Expr, suffix: &[&str]) -> bool {
    let ExprKind::Path(_, path) = &expr.kind else {
        return false;
    };
    path_suffix(path.segments.iter().map(|seg| seg.ident.name.as_str()), suffix)
}

fn is_type_path_suffix(ty: &Ty, suffix: &[&str]) -> bool {
    let TyKind::Path(None, path) = &ty.kind else {
        return false;
    };
    path_suffix(path.segments.iter().map(|seg| seg.ident.name.as_str()), suffix)
}

fn path_suffix<'a>(segments: impl Iterator<Item = &'a str>, suffix: &[&str]) -> bool {
    let collected: Vec<&str> = segments.collect();
    collected.len() >= suffix.len() && &collected[collected.len() - suffix.len()..] == suffix
}
