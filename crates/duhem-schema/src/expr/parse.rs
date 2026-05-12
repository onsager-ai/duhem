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

        let string_lit = just('"')
            .ignore_then(none_of('"').repeated().to_slice())
            .then_ignore(just('"'))
            .map(|s: &str| Literal::Str(s.to_string()));

        let literal = choice((bool_lit, float_lit, int_lit, string_lit)).map(Expr::Lit);

        // ---- path / call ----
        let ident = text::ident().to_slice().map(|s: &str| s.to_string());

        let root =
            just('$')
                .ignore_then(text::ident().to_slice())
                .try_map(|s: &str, span| match s {
                    "steps" => Ok(PathRoot::Steps),
                    "setup" => Ok(PathRoot::Setup),
                    "inputs" => Ok(PathRoot::Inputs),
                    "env" => Ok(PathRoot::Env),
                    "runtime" => Ok(PathRoot::Runtime),
                    other => Err(Rich::custom(
                        span,
                        format!(
                            "unknown scope `${other}` (expected `$steps`, `$setup`, `$inputs`, `$env`, or `$runtime`)"
                        ),
                    )),
                });

        let segments = just('.').ignore_then(ident).repeated().collect::<Vec<_>>();

        let path = root
            .then(segments)
            .map(|(root, segments)| Path { root, segments });

        let call_args = expr
            .clone()
            .padded()
            .separated_by(just(','))
            .allow_trailing()
            .collect::<Vec<_>>()
            .delimited_by(just('('), just(')'));

        // Function-call syntax (`(args)`) is only legal under
        // `$runtime` per `docs/duhem-spec.md` §10.7. Reject it on
        // other roots at parse time so structural validation isn't
        // asked to enforce a grammar rule.
        let path_or_call = path
            .then(call_args.or_not())
            .try_map(|(path, args), span| match (args, path.root) {
                (Some(args), PathRoot::Runtime) => Ok(Expr::Call { path, args }),
                (Some(_), other) => Err(Rich::custom(
                    span,
                    format!(
                        "function-call syntax `(...)` is only valid on `$runtime`, not `${}`",
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
                segments: vec!["api".into(), "outputs".into(), "status".into()],
            })
        );
        assert_eq!(
            p("$inputs.workspace_name"),
            Expr::Path(Path {
                root: PathRoot::Inputs,
                segments: vec!["workspace_name".into()],
            })
        );
    }

    #[test]
    fn rejects_unknown_scope() {
        let err = parse("$nope.x").unwrap_err();
        assert!(
            err.message.contains("unknown scope"),
            "got: {}",
            err.message
        );
    }

    #[test]
    fn parses_env_scope() {
        assert_eq!(
            p("$env.DATABASE_URL"),
            Expr::Path(Path {
                root: PathRoot::Env,
                segments: vec!["DATABASE_URL".into()],
            })
        );
    }

    #[test]
    fn rejects_call_on_non_runtime_root() {
        let err = parse("$inputs.x()").unwrap_err();
        assert!(
            err.message.contains("only valid on `$runtime`"),
            "got: {}",
            err.message
        );
        let err = parse("$steps.a.outputs.x()").unwrap_err();
        assert!(err.message.contains("only valid on `$runtime`"));
        let err = parse("$env.X()").unwrap_err();
        assert!(err.message.contains("only valid on `$runtime`"));
    }

    #[test]
    fn parses_call_no_args() {
        assert_eq!(
            p("$runtime.uuid()"),
            Expr::Call {
                path: Path {
                    root: PathRoot::Runtime,
                    segments: vec!["uuid".into()],
                },
                args: vec![],
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
