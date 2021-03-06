use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::{fold::Fold, visit_mut::VisitMut, *};

mod utils;

use crate::utils::SignatureExtensions;

#[proc_macro_attribute]
pub fn recursive(_attr: TokenStream, item_fn: TokenStream) -> TokenStream {
    let item_fn = parse_macro_input!(item_fn as ItemFn);
    let mut transformer = RecursionTransformer::new(item_fn);
    let item_fn = transformer.transform_item_fn();

    // println!("{}", quote! { #item_fn });

    TokenStream::from(quote!(#item_fn))
}

macro_rules! verbatim {
    (some, $($tokens:tt)*) => {
        Some(Box::new(Expr::Verbatim(quote! { $($tokens)* })))
    };
    (boxed, $($tokens:tt)*) => {
        Box::new(Expr::Verbatim(quote! { $($tokens)* }))
    };
    ($($tokens:tt)*) => {
        Expr::Verbatim(quote! { $($tokens)* })
    };
}

struct RecursionTransformer {
    item_fn: ItemFn,
}

impl Fold for RecursionTransformer {
    fn fold_item_fn(&mut self, item_fn: ItemFn) -> ItemFn {
        let ItemFn { sig, block, .. } = item_fn;

        let fn_inner = format_ident!("{}_inner", sig.ident);
        let (input_pats, input_types) = sig.split_inputs();
        let return_type = sig.extract_return_type();

        let block = parse_quote! {{
            enum Action<C, R> {
                Continue(C),
                Return(R),
            }

            fn #fn_inner((#(#input_pats),*): (#(#input_types),*))
                -> Action<(#(#input_types),*), #return_type> #block

            let mut acc = (#(#input_pats),*);
            loop {
                match #fn_inner(acc) {
                    Action::Return(r) => return r,
                    Action::Continue(c) => acc = c,
                }
            }
        }};

        ItemFn {
            sig,
            block,
            ..item_fn
        }
    }

    fn fold_expr_return(&mut self, expr_return: ExprReturn) -> ExprReturn {
        let ExprReturn { expr, .. } = expr_return;

        let expr = match expr {
            Some(expr) => self.fold_expr(*expr),
            None => verbatim! { Action::Return(()) },
        };

        ExprReturn {
            expr: Some(Box::new(expr)),
            ..expr_return
        }
    }

    fn fold_expr(&mut self, expr: Expr) -> Expr {
        match expr {
            Expr::Call(expr_call) => {
                let func = &*expr_call.func;
                let func_ident: Ident = parse_quote!(#func);

                if func_ident != self.item_fn.sig.ident {
                    verbatim! { Action::Return(#expr_call) }
                } else {
                    let args = &expr_call.args;
                    verbatim! { Action::Continue((#args)) }
                }
            },
            Expr::MethodCall(expr_method_call) => {
                let func_ident = &expr_method_call.method;

                if *func_ident != self.item_fn.sig.ident {
                    verbatim! { Action::Return(#expr_method_call) }
                } else {
                    let args = &expr_method_call.args;
                    verbatim! { Action::Continue((#args)) }
                }
            },
            Expr::Verbatim(_) => expr,
            _ => verbatim! { Action::Return(#expr) }
        }
    }
}

impl RecursionTransformer {
    fn new(item_fn: ItemFn) -> Self {
        RecursionTransformer { item_fn }
    }

    fn transform_item_fn(&mut self) -> ItemFn {
        let mut item_fn = self.item_fn.clone();

        // transform `return` expression
        self.visit_item_fn_mut(&mut item_fn);

        // transform last expression
        let mut last_stmt = item_fn.block.stmts.last().unwrap().clone();
        self.visit_stmt_mut(&mut last_stmt);
        let fn_body_last_stmt = item_fn.block.stmts.last_mut().unwrap();
        *fn_body_last_stmt = last_stmt;

        self.fold_item_fn(item_fn)
    }

    fn transform_expr_return(&self, node: &mut ExprReturn) {
        if let Some(ref mut some_expr) = node.expr {
            self.transform_expr(some_expr);
        } else {
            node.expr = verbatim!(some, Action::Return(()));
        }
    }

    fn transform_expr(&self, expr: &mut Expr) {
        let fn_name = &self.item_fn.sig.ident;

        match expr {
            Expr::Call(expr_call) => {
                let func = expr_call.func.clone();
                let func_id: Ident = parse_quote!(#func);

                if func_id != *fn_name {
                    *expr = verbatim! { Action::Return(#expr_call) };
                } else {
                    let args = expr_call.args.clone();
                    *expr = verbatim! { Action::Continue((#args)) };
                }
            }
            Expr::MethodCall(expr_method_call) => {
                let func_id = expr_method_call.method.clone();
                if func_id != *fn_name {
                    *expr = verbatim! { Action::Return(#expr_method_call) };
                } else {
                    let args = expr_method_call.args.clone();
                    *expr = verbatim! { Action::Continue((#args)) };
                }
            }
            Expr::Match(expr) => expr.arms.iter_mut().for_each(|arm| {
                self.transform_expr(&mut arm.body);
            }),
            Expr::If(expr) => {
                if let Some(last_stmt) = expr.then_branch.stmts.last_mut() {
                    if let Stmt::Expr(expr) = last_stmt {
                        self.transform_expr(expr);
                    }
                }
                if let Some((_, ref mut expr)) = &mut expr.else_branch {
                    self.transform_expr(expr);
                }
            }
            Expr::Block(expr) => {
                if let Some(last_stmt) = expr.block.stmts.last_mut() {
                    if let Stmt::Expr(expr) = last_stmt {
                        self.transform_expr(expr);
                    }
                }
            }
            Expr::Return(_) | Expr::Verbatim(_) => {
                // Ignore return expressions as they are handled separately.
                // And, ignore verbatim!{} expressions generated by this macro.
            }
            _ => {
                *expr = verbatim! { Action::Return(#expr) };
            }
        }
    }
}

impl VisitMut for RecursionTransformer {
    fn visit_expr_return_mut(&mut self, node: &mut ExprReturn) {
        self.transform_expr_return(node);
    }

    fn visit_expr_mut(&mut self, node: &mut Expr) {
        self.transform_expr(node);
    }
}
