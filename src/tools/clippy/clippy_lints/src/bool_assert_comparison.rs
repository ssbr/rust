use clippy_utils::diagnostics::span_lint_and_then;
use clippy_utils::macros::{find_assert_eq_args, root_macro_call_first_node};
use clippy_utils::ty::{implements_trait, is_copy};
use rustc_ast::ast::LitKind;
use rustc_errors::Applicability;
use rustc_hir::{Expr, ExprKind, Lit};
use rustc_lint::{LateContext, LateLintPass, LintContext};
use rustc_middle::ty::{self, Ty};
use rustc_session::{declare_lint_pass, declare_tool_lint};
use rustc_span::symbol::Ident;

declare_clippy_lint! {
    /// ### What it does
    /// This lint warns about boolean comparisons in assert-like macros.
    ///
    /// ### Why is this bad?
    /// It is shorter to use the equivalent.
    ///
    /// ### Example
    /// ```rust
    /// assert_eq!("a".is_empty(), false);
    /// assert_ne!("a".is_empty(), true);
    /// ```
    ///
    /// Use instead:
    /// ```rust
    /// assert!(!"a".is_empty());
    /// ```
    #[clippy::version = "1.53.0"]
    pub BOOL_ASSERT_COMPARISON,
    style,
    "Using a boolean as comparison value in an assert_* macro when there is no need"
}

declare_lint_pass!(BoolAssertComparison => [BOOL_ASSERT_COMPARISON]);

fn is_bool_lit(e: &Expr<'_>) -> bool {
    matches!(
        e.kind,
        ExprKind::Lit(Lit {
            node: LitKind::Bool(_),
            ..
        })
    ) && !e.span.from_expansion()
}

fn is_impl_not_trait_with_bool_out<'tcx>(cx: &LateContext<'tcx>, ty: Ty<'tcx>) -> bool {
    cx.tcx
        .lang_items()
        .not_trait()
        .filter(|trait_id| implements_trait(cx, ty, *trait_id, &[]))
        .and_then(|trait_id| {
            cx.tcx.associated_items(trait_id).find_by_name_and_kind(
                cx.tcx,
                Ident::from_str("Output"),
                ty::AssocKind::Type,
                trait_id,
            )
        })
        .map_or(false, |assoc_item| {
            let proj = cx.tcx.mk_projection(assoc_item.def_id, cx.tcx.mk_substs_trait(ty, []));
            let nty = cx.tcx.normalize_erasing_regions(cx.param_env, proj);

            nty.is_bool()
        })
}

impl<'tcx> LateLintPass<'tcx> for BoolAssertComparison {
    fn check_expr(&mut self, cx: &LateContext<'tcx>, expr: &'tcx Expr<'_>) {
        let Some(macro_call) = root_macro_call_first_node(cx, expr) else { return };
        let macro_name = cx.tcx.item_name(macro_call.def_id);
        if !matches!(
            macro_name.as_str(),
            "assert_eq" | "debug_assert_eq" | "assert_ne" | "debug_assert_ne"
        ) {
            return;
        }
        let Some ((a, b, _)) = find_assert_eq_args(cx, expr, macro_call.expn) else { return };

        let a_span = a.span.source_callsite();
        let b_span = b.span.source_callsite();

        let (lit_span, non_lit_expr) = match (is_bool_lit(a), is_bool_lit(b)) {
            // assert_eq!(true, b)
            //            ^^^^^^
            (true, false) => (a_span.until(b_span), b),
            // assert_eq!(a, true)
            //             ^^^^^^
            (false, true) => (b_span.with_lo(a_span.hi()), a),
            // If there are two boolean arguments, we definitely don't understand
            // what's going on, so better leave things as is...
            //
            // Or there is simply no boolean and then we can leave things as is!
            _ => return,
        };

        let non_lit_ty = cx.typeck_results().expr_ty(non_lit_expr);

        if !is_impl_not_trait_with_bool_out(cx, non_lit_ty) {
            // At this point the expression which is not a boolean
            // literal does not implement Not trait with a bool output,
            // so we cannot suggest to rewrite our code
            return;
        }

        if !is_copy(cx, non_lit_ty) {
            // Only lint with types that are `Copy` because `assert!(x)` takes
            // ownership of `x` whereas `assert_eq(x, true)` does not
            return;
        }

        let macro_name = macro_name.as_str();
        let non_eq_mac = &macro_name[..macro_name.len() - 3];
        span_lint_and_then(
            cx,
            BOOL_ASSERT_COMPARISON,
            macro_call.span,
            &format!("used `{macro_name}!` with a literal bool"),
            |diag| {
                // assert_eq!(...)
                // ^^^^^^^^^
                let name_span = cx.sess().source_map().span_until_char(macro_call.span, '!');

                diag.multipart_suggestion(
                    format!("replace it with `{non_eq_mac}!(..)`"),
                    vec![(name_span, non_eq_mac.to_string()), (lit_span, String::new())],
                    Applicability::MachineApplicable,
                );
            },
        );
    }
}
