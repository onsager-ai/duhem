//! Hand-grammar-built `chumsky` parser for the v0.1 expression
//! language. Lives behind `super::parse` so callers see a string-in,
//! `Expr`-out function and never touch chumsky directly — keeping the
//! parser library a swappable implementation detail.

use chumsky::prelude::*;

use super::{BinOp, Expr, Literal, Path, PathRoot, UnaryOp};

/// Parse failure with a single human-readable message. The full
/// `chumsky::error::Rich` diagnostics are flattened here at the API
/// boundary; richer error rendering can be added when the CLI grows a
/// proper `validate` UX.
#[derive(Debug, Clone, thiserror::Error)]
#[error("expression parse error: {message}")]
pub struct ParseError {
    pub message: String,
}

/// Parse a source string into an `Expr`. Whitespace is permitted around
/// every token; the entire input must be consumed.
pub fn parse(src: &str) -> Result<Expr, ParseError> {
    expr_parser().parse(src).into_result().map_err(|errs| {
        let message = errs
            .into_iter()
            .map(|e| format!("{e}"))
            .collect::<Vec<_>>()
            .join("; ");
        ParseError { message }
    })
}

type Err<'src> = extra::Err<Rich<'src, char>>;

/// Parse a path at the start of a string and return the unconsumed suffix.
pub(crate) fn parse_path_prefix(src: &str) -> Option<(Path, &str)> {
    path_parser()
        .then(any().repeated().to_slice())
        .parse(src)
        .into_result()
        .ok()
}

fn path_parser<'src>() -> impl Parser<'src, &'src str, Path, Err<'src>> + Clone {
    let ident = text::ident().to_slice().map(|s: &str| s.to_string());

    // The six fixed roots are a closed keyword set. Any other
    // identifier is not a parse error: it is provisionally a
    // `for_each` loop-variable reference (`as: row` → `$row`),
    // carried as the first path segment rather than the root
    // itself (see `PathRoot::Loop`'s doc comment for why). Whether
    // that identifier is actually a declared, in-scope binding is
    // resolved by the validator, not here — same deferral the
    // parser already gives an undeclared `$steps.<id>`.
    let root = just('$')
        .ignore_then(text::ident().to_slice())
        .map(|s: &str| match s {
            "steps" => (PathRoot::Steps, None),
            "setup" => (PathRoot::Setup, None),
            "inputs" => (PathRoot::Inputs, None),
            "pages" => (PathRoot::Pages, None),
            "env" => (PathRoot::Env, None),
            "runtime" => (PathRoot::Runtime, None),
            other => (PathRoot::Loop, Some(other.to_string())),
        });

    // Keep quoted-key positions so even a numeric key cannot index an array.
    let key_seg = just('.').ignore_then(ident).map(|s| (s, false));
    let index_seg = text::int(10)
        .to_slice()
        .map(|s: &str| (s.to_string(), false))
        .delimited_by(just('['), just(']'));
    let escaped = just('\\').ignore_then(choice((just('\\'), just('\''))));
    let quoted_key = choice((escaped, none_of("\\'")))
        .repeated()
        .at_least(1)
        .collect::<String>()
        .delimited_by(just("['"), just("']"))
        .labelled("non-empty quoted key (only \\' and \\\\ escapes are allowed)")
        .map(|s| (s, true));
    let segments = choice((key_seg, quoted_key, index_seg))
        .repeated()
        .collect::<Vec<_>>();

    root.then(segments).map(|((root, loop_name), segments)| {
        let mut names = Vec::new();
        let mut quoted_keys = Vec::new();
        if let Some(name) = loop_name {
            names.push(name);
        }
        for (name, quoted) in segments {
            if quoted && name.bytes().all(|b| b.is_ascii_digit()) {
                quoted_keys.push(names.len());
            }
            names.push(name);
        }
        Path {
            root,
            segments: names,
            quoted_keys,
        }
    })
}

fn expr_parser<'src>() -> impl Parser<'src, &'src str, Expr, Err<'src>> {
    recursive(|expr| {
        // ---- literals ----
        let bool_lit = choice((
            text::keyword("true").to(Literal::Bool(true)),
            text::keyword("false").to(Literal::Bool(false)),
        ));

        let digits = text::int(10);

        let float_lit = just('-')
            .or_not()
            .then(digits)
            .then(just('.'))
            .then(text::digits(10).to_slice())
            .to_slice()
            .try_map(|s: &str, span| {
                s.parse::<f64>()
                    .map(Literal::Float)
                    .map_err(|e| Rich::custom(span, format!("bad float `{s}`: {e}")))
            });

        let int_lit = just('-')
            .or_not()
            .then(digits)
            .to_slice()
            .try_map(|s: &str, span| {
                s.parse::<i64>()
                    .map(Literal::Int)
                    .map_err(|e| Rich::custom(span, format!("bad int `{s}`: {e}")))
            });

        let double_quoted = just('"')
            .ignore_then(none_of('"').repeated().to_slice())
            .then_ignore(just('"'))
            .map(|s: &str| Literal::Str(s.to_string()));
        let single_quoted = just('\'')
            .ignore_then(none_of('\'').repeated().to_slice())
            .then_ignore(just('\''))
            .map(|s: &str| Literal::Str(s.to_string()));
        let string_lit = choice((double_quoted, single_quoted));

        let literal = choice((bool_lit, float_lit, int_lit, string_lit)).map(Expr::Lit);

        // ---- path / call ----
        let path = path_parser();

        let call_args = expr
            .clone()
            .padded()
            .separated_by(just(','))
            .allow_trailing()
            .collect::<Vec<_>>()
            .delimited_by(just('('), just(')'));

        // Function-call syntax (`(args)`) is legal for runtime helpers and
        // parameterized two-segment page references (spec #495). Page shape
        // and placeholder arity remain structural-validation concerns.
        let path_or_call = path
            .then(call_args.or_not())
            .try_map(|(path, args), span| match (args, path.root) {
                (Some(args), PathRoot::Runtime | PathRoot::Pages) => {
                    Ok(Expr::Call { path, args })
                }
                (Some(_), PathRoot::Loop) => Err(Rich::custom(
                    span,
                    format!(
                        "function-call syntax `(...)` is only valid on `$runtime` or `$pages`, not `${}`",
                        path.segments.first().map(String::as_str).unwrap_or("?")
                    ),
                )),
                (Some(_), other) => Err(Rich::custom(
                    span,
                    format!(
                        "function-call syntax `(...)` is only valid on `$runtime` or `$pages`, not `${}`",
                        other.as_str()
                    ),
                )),
                (None, _) => Ok(Expr::Path(path)),
            });

        // ---- primary ----
        let parens = expr.clone().padded().delimited_by(just('('), just(')'));

        let atom = choice((literal, path_or_call, parens)).padded();

        // ---- unary not ----
        let not = recursive(|not| {
            just('!')
                .padded()
                .ignore_then(not)
                .map(|expr| Expr::UnaryOp {
                    op: UnaryOp::Not,
                    expr: Box::new(expr),
                })
                .or(atom.clone())
        });

        // ---- comparisons (non-associative; at most one) ----
        let cmp_op = choice((
            just("==").to(BinOp::Eq),
            just("!=").to(BinOp::Ne),
            just("<=").to(BinOp::Le),
            just(">=").to(BinOp::Ge),
            just("<").to(BinOp::Lt),
            just(">").to(BinOp::Gt),
        ))
        .padded();

        let cmp =
            not.clone()
                .then(cmp_op.then(not.clone()).or_not())
                .map(|(lhs, rest)| match rest {
                    Some((op, rhs)) => Expr::BinOp {
                        op,
                        lhs: Box::new(lhs),
                        rhs: Box::new(rhs),
                    },
                    None => lhs,
                });

        // ---- logical ----
        let and = cmp.clone().foldl(
            just("&&").padded().ignore_then(cmp.clone()).repeated(),
            |lhs, rhs| Expr::BinOp {
                op: BinOp::And,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
            },
        );

        and.clone().foldl(
            just("||").padded().ignore_then(and.clone()).repeated(),
            |lhs, rhs| Expr::BinOp {
                op: BinOp::Or,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
            },
        )
    })
    .padded()
    .then_ignore(end())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> Expr {
        parse(s).unwrap_or_else(|e| panic!("parse {s:?}: {e}"))
    }

    #[test]
    fn parses_int_literal() {
        assert_eq!(p("200"), Expr::Lit(Literal::Int(200)));
        assert_eq!(p("-7"), Expr::Lit(Literal::Int(-7)));
    }

    #[test]
    fn parses_float_literal() {
        assert_eq!(p("2.5"), Expr::Lit(Literal::Float(2.5)));
    }

    #[test]
    fn parses_string_literal() {
        assert_eq!(p("\"hi\""), Expr::Lit(Literal::Str("hi".to_string())));
    }

    #[test]
    fn parses_bool_literal() {
        assert_eq!(p("true"), Expr::Lit(Literal::Bool(true)));
        assert_eq!(p("false"), Expr::Lit(Literal::Bool(false)));
    }

    #[test]
    fn parses_path() {
        assert_eq!(
            p("$steps.api.outputs.status"),
            Expr::Path(Path {
                root: PathRoot::Steps,
                quoted_keys: vec![],
                segments: vec!["api".into(), "outputs".into(), "status".into()],
            })
        );
        assert_eq!(
            p("$inputs.workspace_name"),
            Expr::Path(Path {
                root: PathRoot::Inputs,
                quoted_keys: vec![],
                segments: vec!["workspace_name".into()],
            })
        );
    }

    #[test]
    fn parses_nested_navigation_keys_and_index() {
        assert_eq!(
            p("$steps.api.outputs.body.app.id"),
            Expr::Path(Path {
                root: PathRoot::Steps,
                quoted_keys: vec![],
                segments: vec![
                    "api".into(),
                    "outputs".into(),
                    "body".into(),
                    "app".into(),
                    "id".into(),
                ],
            })
        );
        // `[N]` lowers to a decimal-string segment alongside keys.
        assert_eq!(
            p("$steps.api.outputs.body.items[0].id"),
            Expr::Path(Path {
                root: PathRoot::Steps,
                quoted_keys: vec![],
                segments: vec![
                    "api".into(),
                    "outputs".into(),
                    "body".into(),
                    "items".into(),
                    "0".into(),
                    "id".into(),
                ],
            })
        );
    }

    #[test]
    fn parses_quoted_keys_and_escapes() {
        let Expr::Path(path) = p(r"$steps.r.outputs.items[0]['a-b']['it\'s\\ok']['200']") else {
            panic!("expected path");
        };
        assert_eq!(
            path.segments,
            ["r", "outputs", "items", "0", "a-b", "it's\\ok", "200"]
        );
        assert_eq!(path.quoted_keys, [6]);
        assert_eq!(p("$inputs.x['name']"), p("$inputs.x.name"));
    }

    #[test]
    fn rejects_invalid_quoted_keys() {
        for src in [
            "$inputs.x['']",
            "$inputs.x['missing]",
            r"$inputs.x['bad\q']",
        ] {
            let err = parse(src).unwrap_err();
            assert!(!err.message.is_empty(), "{src}");
        }
    }

    #[test]
    fn unrecognized_root_parses_as_a_loop_reference() {
        // Not one of the six fixed keywords, so it's provisionally a
        // `for_each` loop-variable reference (#443 Tier 1): `$nope`
        // becomes `PathRoot::Loop` with `nope` as the first segment.
        // Whether `nope` is an actual, in-scope `as:` binding is a
        // validator question, not a parser one.
        assert_eq!(
            p("$nope.x"),
            Expr::Path(Path {
                root: PathRoot::Loop,
                quoted_keys: vec![],
                segments: vec!["nope".into(), "x".into()],
            })
        );
        assert_eq!(
            p("$row"),
            Expr::Path(Path {
                root: PathRoot::Loop,
                quoted_keys: vec![],
                segments: vec!["row".into()],
            })
        );
    }

    #[test]
    fn loop_reference_rejects_call_syntax() {
        let err = parse("$row(1)").unwrap_err();
        assert!(err.message.contains("$row"), "got: {}", err.message);
    }

    #[test]
    fn parses_env_scope() {
        assert_eq!(
            p("$env.DATABASE_URL"),
            Expr::Path(Path {
                root: PathRoot::Env,
                quoted_keys: vec![],
                segments: vec!["DATABASE_URL".into()],
            })
        );
    }

    #[test]
    fn rejects_call_on_non_callable_root() {
        let err = parse("$inputs.x()").unwrap_err();
        assert!(
            err.message.contains("only valid on `$runtime` or `$pages`"),
            "got: {}",
            err.message
        );
        let err = parse("$steps.a.outputs.x()").unwrap_err();
        assert!(err.message.contains("only valid on `$runtime` or `$pages`"));
        let err = parse("$env.X()").unwrap_err();
        assert!(err.message.contains("only valid on `$runtime` or `$pages`"));
    }

    #[test]
    fn parses_call_no_args() {
        assert_eq!(
            p("$runtime.uuid()"),
            Expr::Call {
                path: Path {
                    root: PathRoot::Runtime,
                    quoted_keys: vec![],
                    segments: vec!["uuid".into()],
                },
                args: vec![],
            }
        );
    }

    #[test]
    fn parses_page_locator_path() {
        assert_eq!(
            p("$pages.login.submit"),
            Expr::Path(Path {
                root: PathRoot::Pages,
                quoted_keys: vec![],
                segments: vec!["login".into(), "submit".into()],
            })
        );
    }

    #[test]
    fn parses_parameterized_page_locator() {
        assert_eq!(
            p("$pages.chat.history_item($inputs.index, 2)"),
            Expr::Call {
                path: Path {
                    root: PathRoot::Pages,
                    quoted_keys: vec![],
                    segments: vec!["chat".into(), "history_item".into()],
                },
                args: vec![
                    Expr::Path(Path {
                        root: PathRoot::Inputs,
                        quoted_keys: vec![],
                        segments: vec!["index".into()],
                    }),
                    Expr::Lit(Literal::Int(2)),
                ],
            }
        );
    }

    #[test]
    fn parses_call_with_args() {
        assert_eq!(
            p("$runtime.format(\"x\", 1)"),
            Expr::Call {
                path: Path {
                    root: PathRoot::Runtime,
                    quoted_keys: vec![],
                    segments: vec!["format".into()],
                },
                args: vec![
                    Expr::Lit(Literal::Str("x".into())),
                    Expr::Lit(Literal::Int(1)),
                ],
            }
        );
    }

    #[test]
    fn parses_both_string_quote_forms() {
        assert_eq!(
            p(r#"$runtime.format("{}", "value")"#),
            p("$runtime.format('{}', 'value')")
        );
    }

    #[test]
    fn each_string_quote_form_may_contain_the_other_quote() {
        assert_eq!(
            p(r#"$runtime.format("{}", "it's")"#),
            Expr::Call {
                path: Path {
                    root: PathRoot::Runtime,
                    quoted_keys: vec![],
                    segments: vec!["format".into()],
                },
                args: vec![
                    Expr::Lit(Literal::Str("{}".into())),
                    Expr::Lit(Literal::Str("it's".into())),
                ],
            }
        );
        assert_eq!(
            p(r#"$runtime.format('{}', 'say "hello"')"#),
            Expr::Call {
                path: Path {
                    root: PathRoot::Runtime,
                    quoted_keys: vec![],
                    segments: vec!["format".into()],
                },
                args: vec![
                    Expr::Lit(Literal::Str("{}".into())),
                    Expr::Lit(Literal::Str("say \"hello\"".into())),
                ],
            }
        );
    }

    #[test]
    fn parses_eq_comparison() {
        let e = p("$steps.api.outputs.status == 200");
        match e {
            Expr::BinOp { op: BinOp::Eq, .. } => {}
            other => panic!("expected ==, got {other:?}"),
        }
    }

    #[test]
    fn parses_all_comparisons() {
        for op in ["==", "!=", "<", "<=", ">", ">="] {
            let src = format!("$inputs.x {op} 1");
            assert!(parse(&src).is_ok(), "failed for {op}: {src}");
        }
    }

    #[test]
    fn parses_logical_and_or() {
        let e = p("$inputs.a == 1 && $inputs.b == 2 || $inputs.c");
        // top-level should be Or
        match e {
            Expr::BinOp { op: BinOp::Or, .. } => {}
            other => panic!("expected ||, got {other:?}"),
        }
    }

    #[test]
    fn parses_not() {
        let e = p("!$inputs.flag");
        match e {
            Expr::UnaryOp {
                op: UnaryOp::Not, ..
            } => {}
            other => panic!("expected !, got {other:?}"),
        }
    }

    #[test]
    fn parses_parens() {
        let e = p("($inputs.a || $inputs.b) && $inputs.c");
        match e {
            Expr::BinOp { op: BinOp::And, .. } => {}
            other => panic!("expected && at top, got {other:?}"),
        }
    }

    #[test]
    fn rejects_trailing_garbage() {
        assert!(parse("1 garbage").is_err());
    }

    #[test]
    fn walk_paths_visits_all() {
        let e = p("$steps.a.outputs.x == 1 && $inputs.y");
        let mut roots: Vec<PathRoot> = Vec::new();
        e.walk_paths(|p| roots.push(p.root));
        assert_eq!(roots, vec![PathRoot::Steps, PathRoot::Inputs]);
    }
}
