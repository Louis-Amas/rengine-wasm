use proc_macro::TokenStream;
use quote::{quote, ToTokens};
use syn::{
    parse::{Parse, ParseStream},
    parse_macro_input,
    spanned::Spanned,
    visit::{self, Visit},
    Block, Expr, Ident, Token,
};

struct SifInput {
    condition: Expr,
    _comma1: Token![,],
    then_block: Block,
    _comma2: Token![,],
    else_block: Block,
}

impl Parse for SifInput {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        Ok(Self {
            condition: input.parse()?,
            _comma1: input.parse()?,
            then_block: input.parse()?,
            _comma2: input.parse()?,
            else_block: input.parse()?,
        })
    }
}

#[proc_macro]
pub fn sif(input: TokenStream) -> TokenStream {
    let SifInput {
        condition,
        then_block,
        else_block,
        ..
    } = parse_macro_input!(input as SifInput);

    // Get line number from condition span (syn::spanned::Spanned required!)
    let line = condition.span().unwrap().start().line();

    let cond_tokens = condition.to_token_stream();
    let cond_string = cond_tokens.to_string();

    let mut vars = vec![];
    collect_vars(&condition, &mut vars);
    vars.sort();
    vars.dedup();

    let var_values = vars.iter().map(|ident| {
        let name = ident.to_string();
        quote! {
            (#name, format!("{:?}", #ident))
        }
    });

    let gen = quote! {{
        let condition = (#cond_tokens);
        let mut map = ::std::collections::HashMap::new();
        #(map.insert(#var_values.0, #var_values.1);)*

        let log = #cond_string
            .split_whitespace()
            .map(|tok| {
                if let Some(val) = map.get(tok) {
                    format!("({tok} = {val})")
                } else {
                    tok.to_string()
                }
            })
            .collect::<Vec<_>>()
            .join(" ");

        let logs = format!("[line {}] cond = {} {}", #line, condition, log);

        let result = rengine_types::SifResult {
            condition,
            logs,
        };

        if result.condition {
            #then_block
        } else {
            #else_block
        }

        result
    }};

    gen.into()
}

fn collect_vars(expr: &Expr, out: &mut Vec<syn::Ident>) {
    match expr {
        Expr::Path(p) if p.path.segments.len() == 1 => {
            if let Some(ident) = p.path.get_ident() {
                out.push(ident.clone());
            }
        }
        _ => {
            for child in expr.clone().into_token_stream() {
                if let Ok(parsed) = syn::parse2::<Expr>(quote!(#child)) {
                    collect_vars(&parsed, out);
                }
            }
        }
    }
}

#[allow(dead_code)]
struct SAssignInput {
    maybe_let: Option<Token![let]>,
    lhs: Ident,
    _eq_token: Token![=],
    rhs: Expr,
}

impl Parse for SAssignInput {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let maybe_let = input.parse().ok();
        Ok(Self {
            maybe_let,
            lhs: input.parse()?,
            _eq_token: input.parse()?,
            rhs: input.parse()?,
        })
    }
}

#[proc_macro]
pub fn sassign(input: TokenStream) -> TokenStream {
    let SAssignInput { lhs, rhs, .. } = parse_macro_input!(input as SAssignInput);

    let lhs_name = lhs.to_string();
    let line = rhs.span().unwrap().start().line();
    let rhs_tokens = rhs.to_token_stream();

    let mut vars = sassign_collect_vars(&rhs);
    vars.sort_by_key(|v| v.to_string());
    vars.dedup();
    vars.retain(|v| v != &lhs);

    let gen = quote! {
        let #lhs = (#rhs_tokens);

        #(let _ = &#vars;)*

        let rhs_expr_string = stringify!(#rhs_tokens).to_string();

        let mut rhs_values = rhs_expr_string.clone();
        #(
            let name = stringify!(#vars);
            let val = format!("{:?}", #vars);
            rhs_values = rhs_values.replace(name, &format!("({name} = {val})"));
        )*

        let logs = format!(
            "[line {}] ({} = {:?}) = {}",
            #line,
            #lhs_name,
            #lhs,
            rhs_values
        );

        let __sassign_result = rengine_types::SifResult {
            condition: true,
            logs: logs.clone(),
        };
    };

    gen.into()
}

struct SAssignVarCollector {
    vars: Vec<Ident>,
}

impl<'ast> Visit<'ast> for SAssignVarCollector {
    fn visit_expr_path(&mut self, node: &'ast syn::ExprPath) {
        if node.path.segments.len() == 1 {
            if let Some(ident) = node.path.get_ident() {
                self.vars.push(ident.clone());
            }
        }
        visit::visit_expr_path(self, node);
    }
}

fn sassign_collect_vars(expr: &Expr) -> Vec<Ident> {
    let mut collector = SAssignVarCollector { vars: vec![] };
    collector.visit_expr(expr);
    collector.vars
}

#[proc_macro_derive(ToIndicators, attributes(indicator))]
pub fn derive_to_indicators(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as syn::DeriveInput);
    let name = &input.ident;

    let mut prefix = String::new();
    for attr in &input.attrs {
        if attr.path().is_ident("indicator") {
            let _ = attr.parse_nested_meta(|meta| {
                if meta.path.is_ident("prefix") {
                    let content: syn::LitStr = meta.value()?.parse()?;
                    prefix = content.value();
                    Ok(())
                } else {
                    Err(meta.error("unsupported attribute"))
                }
            });
        }
    }

    let fields = match input.data {
        syn::Data::Struct(ref data) => &data.fields,
        _ => {
            return syn::Error::new_spanned(input, "ToIndicators only supports structs")
                .to_compile_error()
                .into()
        }
    };

    let mut set_indicators = Vec::new();

    for field in fields {
        let field_name = &field.ident;
        let ty = &field.ty;

        // Check if type is Decimal. This is a heuristic.
        let is_decimal = if let syn::Type::Path(type_path) = ty {
            if let Some(segment) = type_path.path.segments.last() {
                segment.ident == "Decimal"
            } else {
                false
            }
        } else {
            false
        };

        if is_decimal {
            if let Some(ident) = field_name {
                let field_str = ident.to_string();
                set_indicators.push(quote! {
                    requests.push(rengine_types::ExecutionRequest::SetIndicator(format!("{}{}{}", #prefix, market_prefix, #field_str).into(), self.#ident));
                });
            }
        }
    }

    let expanded = quote! {
        impl #name {
            pub fn indicators(&self) -> Vec<rengine_types::ExecutionRequest> {
                self.indicators_with_market("")
            }

            pub fn indicators_with_market(&self, market: &str) -> Vec<rengine_types::ExecutionRequest> {
                let mut requests = Vec::new();
                let market_prefix = if market.is_empty() {
                    String::new()
                } else {
                    format!("{}_", market)
                };
                #(#set_indicators)*
                requests
            }
        }
    };

    TokenStream::from(expanded)
}

#[proc_macro_derive(FromIndicators, attributes(indicator))]
pub fn derive_from_indicators(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as syn::DeriveInput);
    let name = &input.ident;

    let mut prefix = String::new();
    for attr in &input.attrs {
        if attr.path().is_ident("indicator") {
            let _ = attr.parse_nested_meta(|meta| {
                if meta.path.is_ident("prefix") {
                    let content: syn::LitStr = meta.value()?.parse()?;
                    prefix = content.value();
                    Ok(())
                } else {
                    Err(meta.error("unsupported attribute"))
                }
            });
        }
    }

    let fields = match input.data {
        syn::Data::Struct(ref data) => &data.fields,
        _ => {
            return syn::Error::new_spanned(input, "FromIndicators only supports structs")
                .to_compile_error()
                .into()
        }
    };

    let mut field_inits = Vec::new();

    for field in fields {
        let field_name = &field.ident;
        let ty = &field.ty;

        // Check if type is Decimal
        let is_decimal = if let syn::Type::Path(type_path) = ty {
            if let Some(segment) = type_path.path.segments.last() {
                segment.ident == "Decimal"
            } else {
                false
            }
        } else {
            false
        };

        if let Some(ident) = field_name {
            if is_decimal {
                let field_str = ident.to_string();
                field_inits.push(quote! {
                    #ident: strategy_api::get_indicator(&format!("{}{}{}", #prefix, market_prefix, #field_str)).unwrap_or(rust_decimal_macros::dec!(0))
                });
            } else {
                // For non-Decimal fields, use Default
                field_inits.push(quote! {
                    #ident: Default::default()
                });
            }
        }
    }

    let expanded = quote! {
        impl #name {
            pub fn from_indicators() -> Self {
                Self::from_indicators_with_market("")
            }

            pub fn from_indicators_with_market(market: &str) -> Self {
                let market_prefix = if market.is_empty() {
                    String::new()
                } else {
                    format!("{}_", market)
                };
                Self {
                    #(#field_inits),*
                }
            }
        }
    };

    TokenStream::from(expanded)
}
