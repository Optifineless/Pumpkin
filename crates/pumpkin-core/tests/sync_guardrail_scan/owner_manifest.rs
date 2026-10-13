//! Restrictive AST/call-edge manifest for the M1 leaf boundary, independent of raw-lock selection.

use std::{collections::BTreeSet, fs, path::PathBuf};
use syn::{Expr, Item, Member, visit::Visit};

const SOURCE: &str = "src/world/owner_service.rs";
const LEAVES: &[(&str, &[&str])] = &[
    (
        "OwnerServiceState::new",
        &["AtomicU8::new", "AtomicU64::new"],
    ),
    (
        "OwnerServiceState::clock_ready",
        &["self.startup.get_mut", "self.startup.store"],
    ),
    (
        "OwnerServiceState::public_ingress",
        &["self.startup.get_mut", "self.startup.store"],
    ),
    (
        "OwnerServiceState::enter",
        &[
            "self.startup.load",
            "self.active.compare_exchange",
            "self.epoch.fetch_add",
            "self.active.store",
            "self.active.compare_exchange.is_err",
            "Err",
            "Ok",
        ],
    ),
    (
        "OwnerServiceState::release",
        &["self.active.compare_exchange", "released.is_ok"],
    ),
    ("ServiceReceipt::drop", &["self.state.release"]),
];
// Callbacks occur only at this declared guard-free effect boundary, never in a leaf.
const EFFECT_BOUNDARIES: &[&str] = &[
    "World::initialize_owner_service",
    "World::with_service",
    "World::with_normal_service",
    "World::with_frozen_service",
    "World::report_service_violation",
    "World::tick",
];

fn edge(expr: &Expr) -> Option<String> {
    match expr {
        Expr::Path(path) if path.qself.is_none() => Some(
            path.path
                .segments
                .iter()
                .map(|part| part.ident.to_string())
                .collect::<Vec<_>>()
                .join("::"),
        ),
        Expr::Field(field) => {
            let Member::Named(name) = &field.member else {
                return None;
            };
            Some(format!("{}.{}", edge(&field.base)?, name))
        }
        Expr::MethodCall(call) => Some(format!("{}.{}", edge(&call.receiver)?, call.method)),
        _ => None,
    }
}

struct LeafScan<'a> {
    allowed: &'a [&'a str],
    errors: Vec<String>,
}

impl LeafScan<'_> {
    fn check_edge(&mut self, call: Option<&str>) {
        if call.is_none_or(|call| !self.allowed.contains(&call)) {
            self.errors.push(format!("unreviewed call: {call:?}"));
        }
    }
}

impl<'ast> Visit<'ast> for LeafScan<'_> {
    fn visit_expr_call(&mut self, call: &'ast syn::ExprCall) {
        self.check_edge(edge(&call.func).as_deref());
        syn::visit::visit_expr_call(self, call);
    }

    fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
        self.check_edge(edge(&Expr::MethodCall(call.clone())).as_deref());
        syn::visit::visit_expr_method_call(self, call);
    }

    fn visit_expr_await(&mut self, _: &'ast syn::ExprAwait) {
        self.errors.push("await in leaf".into());
    }

    fn visit_expr_closure(&mut self, _: &'ast syn::ExprClosure) {
        self.errors.push("closure in leaf".into());
    }

    fn visit_expr_async(&mut self, _: &'ast syn::ExprAsync) {
        self.errors.push("async body in leaf".into());
    }

    fn visit_expr_unsafe(&mut self, _: &'ast syn::ExprUnsafe) {
        self.errors.push("unsafe body in leaf".into());
    }

    fn visit_macro(&mut self, mac: &'ast syn::Macro) {
        // These reviewed standard macros evaluate their expression before a guard-free panic.
        // Parse all arguments too: assert!(callback()) must still fail.
        if !mac.path.is_ident("assert") && !mac.path.is_ident("debug_assert") {
            self.errors.push("unreviewed macro in leaf".into());
            return;
        }
        let parsed = mac
            .parse_body_with(syn::punctuated::Punctuated::<Expr, syn::Token![,]>::parse_terminated);
        match parsed {
            Ok(args) => {
                if args.len() > 2 {
                    self.errors
                        .push("format arguments in leaf assertion".into());
                }
                if let Some(message) = args.iter().nth(1) {
                    let plain_message = matches!(message, Expr::Lit(lit)
                        if matches!(&lit.lit, syn::Lit::Str(text) if !text.value().contains(['{', '}'])));
                    if !plain_message {
                        self.errors.push("unreviewed assertion message".into());
                    }
                }
                for arg in &args {
                    self.visit_expr(arg);
                }
            }
            Err(error) => self.errors.push(format!("unresolved macro body: {error}")),
        }
    }
}

fn scan_manifest(source: &str, leaves: &[(&str, &[&str])], effects: &[&str]) -> Vec<String> {
    let file = match syn::parse_file(source) {
        Ok(file) => file,
        Err(error) => return vec![format!("unparsed source: {error}")],
    };
    let mut found = BTreeSet::new();
    let mut errors = Vec::new();
    for item in &file.items {
        let Item::Impl(implementation) = item else {
            if matches!(item, Item::Fn(_) | Item::Macro(_)) {
                errors.push("unmapped function or macro item".into());
            }
            continue;
        };
        if implementation.unsafety.is_some() {
            errors.push("unsafe impl at owner boundary".into());
        }
        let syn::Type::Path(ty) = implementation.self_ty.as_ref() else {
            errors.push("unresolved impl type".into());
            continue;
        };
        let Some(type_name) = ty.path.segments.last() else {
            errors.push("unnamed impl type".into());
            continue;
        };
        for item in &implementation.items {
            let syn::ImplItem::Fn(function) = item else {
                errors.push("unreviewed non-function impl item".into());
                continue;
            };
            let key = format!("{}::{}", type_name.ident, function.sig.ident);
            if !found.insert(key.clone()) {
                errors.push(format!("duplicate method: {key}"));
            }
            if let Some((_, allowed)) = leaves.iter().find(|(name, _)| *name == key) {
                let mut scan = LeafScan {
                    allowed,
                    errors: Vec::new(),
                };
                if function.sig.asyncness.is_some()
                    || matches!(function.sig.safety, syn::Safety::Unsafe(_))
                {
                    scan.errors.push("async/unsafe leaf declaration".into());
                }
                scan.visit_block(&function.block);
                errors.extend(
                    scan.errors
                        .into_iter()
                        .map(|error| format!("{key}: {error}")),
                );
            } else if !effects.contains(&key.as_str()) {
                errors.push(format!("unmapped method: {key}"));
            }
        }
    }
    for name in leaves.iter().map(|(name, _)| name).chain(effects.iter()) {
        if !found.contains(*name) {
            errors.push(format!("missing mapped method: {name}"));
        }
    }
    errors
}

#[test]
fn owner_service_leaves_are_restrictive_and_unconditionally_scanned() {
    let source = fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(SOURCE));
    assert!(
        source.is_ok(),
        "manifest source unavailable: {:?}",
        source.as_ref().err()
    );
    let errors = scan_manifest(&source.unwrap_or_default(), LEAVES, EFFECT_BOUNDARIES);
    assert!(
        errors.is_empty(),
        "owner leaf manifest violations: {errors:?}"
    );
}

#[test]
fn keyword_free_mapped_callbacks_and_awaits_are_rejected() {
    let mapped: &[(&str, &[&str])] = &[("Probe::leaf", &[])];
    for source in [
        "impl Probe { fn leaf(&self) { callback(); } }",
        "impl Probe { async fn leaf(&self) { future.await; } }",
        "impl Probe { fn leaf(&self) { (|| callback())(); } }",
        "impl Probe { fn leaf(&self) { opaque!(); } }",
        "impl Probe { fn leaf(&self) { assert!(callback()); } }",
        "impl Probe { fn leaf(&self) { assert!(true, \"{custom}\"); } }",
    ] {
        assert!(super::scan_source("fixture.rs", source).is_empty());
        assert!(!scan_manifest(source, mapped, &[]).is_empty(), "{source}");
    }
    assert!(scan_manifest("impl Probe { fn leaf(&self) {} }", mapped, &[]).is_empty());
    assert!(!scan_manifest("", mapped, &[]).is_empty());
}
