//! Closed `$runtime` helper catalog shared by validation and evaluation.

/// The argument-count contract for a runtime helper.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeHelperArity {
    Exact(usize),
    Minimum(usize),
    Range { min: usize, max: usize },
}

impl RuntimeHelperArity {
    pub const fn accepts(self, count: usize) -> bool {
        match self {
            Self::Exact(expected) => count == expected,
            Self::Minimum(minimum) => count >= minimum,
            Self::Range { min, max } => count >= min && count <= max,
        }
    }

    pub fn expected(self) -> String {
        match self {
            Self::Exact(1) => "exactly 1 argument".to_string(),
            Self::Exact(count) => format!("exactly {count} arguments"),
            Self::Minimum(1) => "at least 1 argument".to_string(),
            Self::Minimum(count) => format!("at least {count} arguments"),
            Self::Range { min: 1, max: 1 } => "exactly 1 argument".to_string(),
            Self::Range { min, max } => format!("between {min} and {max} arguments"),
        }
    }
}

macro_rules! runtime_helpers {
    ($(($variant:ident, $name:literal, $arity:expr, $authored:literal)),+ $(,)?) => {
        /// A built-in helper accepted by the v1 expression language.
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum RuntimeHelper {
            $($variant),+
        }

        impl RuntimeHelper {
            pub const ALL: &'static [Self] = &[$(Self::$variant),+];

            pub fn named(name: &str) -> Option<Self> {
                Self::ALL
                    .iter()
                    .copied()
                    .find(|helper| helper.name() == name)
            }

            pub const fn name(self) -> &'static str {
                match self {
                    $(Self::$variant => $name),+
                }
            }

            pub const fn arity(self) -> RuntimeHelperArity {
                match self {
                    $(Self::$variant => $arity),+
                }
            }

            pub const fn is_authored(self) -> bool {
                match self {
                    $(Self::$variant => $authored),+
                }
            }
        }
    };
}

runtime_helpers!(
    (Uuid, "uuid", RuntimeHelperArity::Exact(0), true),
    (Now, "now", RuntimeHelperArity::Exact(0), true),
    (Format, "format", RuntimeHelperArity::Minimum(1), true),
    (Concat, "concat", RuntimeHelperArity::Minimum(0), true),
    (Len, "len", RuntimeHelperArity::Exact(1), true),
    (Contains, "contains", RuntimeHelperArity::Exact(2), true),
    (Matches, "matches", RuntimeHelperArity::Exact(2), true),
    (Any, "any", RuntimeHelperArity::Exact(3), true),
    (Lower, "lower", RuntimeHelperArity::Exact(1), true),
    (Upper, "upper", RuntimeHelperArity::Exact(1), true),
    (Trim, "trim", RuntimeHelperArity::Exact(1), true),
    (Replace, "replace", RuntimeHelperArity::Exact(3), true),
    (Default, "default", RuntimeHelperArity::Exact(2), true),
    (Exists, "exists", RuntimeHelperArity::Exact(1), false),
    (TypeCheck, "type_check", RuntimeHelperArity::Exact(2), false),
);

pub fn authored_runtime_helper_names() -> impl Iterator<Item = &'static str> {
    RuntimeHelper::ALL
        .iter()
        .copied()
        .filter(|helper| helper.is_authored())
        .map(RuntimeHelper::name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn range_arity_is_inclusive() {
        let arity = RuntimeHelperArity::Range { min: 1, max: 3 };
        assert!(!arity.accepts(0));
        assert!(arity.accepts(1));
        assert!(arity.accepts(3));
        assert!(!arity.accepts(4));
    }
}
