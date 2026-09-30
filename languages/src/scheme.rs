//! Scheme, with its crate's query and kawoosh's additions after it: the
//! R7RS forms the crate's keywords miss, and a list's head that is a name
//! bound rather than a procedure called.

use crate::Language;

pub static LANGUAGE: Language = Language {
    name: "scheme",
    aliases: &["scm"],
    extensions: &["scm", "ss", "sld", "sls", "sps", "sch"],
    filenames: &[],
    shebangs: &[
        "scheme",
        "guile",
        "chez",
        "petite",
        "csi",
        "gosh",
        "chibi-scheme",
        "gsi",
    ],
    grammar: crate::grammar!("scheme", grammar),
};

#[cfg(feature = "scheme")]
fn grammar() -> Result<crate::Grammar, String> {
    crate::Grammar::new(
        tree_sitter_scheme::LANGUAGE.into(),
        &[tree_sitter_scheme::HIGHLIGHTS_QUERY, HIGHLIGHTS].concat(),
        None,
    )
    .and_then(|g| g.with_outline(OUTLINE))
    .and_then(|g| g.with_indents(include_str!("../queries/scheme/indents.scm")))
}

/// After the crate's query, so each pattern here is the later one a node
/// is painted by. The crate paints every list's head a call; in a
/// lambda's formals, a `let`'s or a `do`'s bindings, the head is a name
/// bound, and `@none` takes the call's colour off it — plain, as every
/// theme paints a variable (a later `@variable` would not win over the
/// call: the highlighter keeps the specific class).
#[cfg(feature = "scheme")]
const HIGHLIGHTS: &str = r#"
(list
  .
  (symbol) @keyword
  (#match? @keyword
   "^(letrec\\*|case-lambda|define-record-type|define-values|parameterize|guard|delay-force|cond-expand|include|include-ci|define-library|syntax-case|with-syntax|receive|fluid-let|define\\*|define-public|define-module|use-modules)$"))

; (lambda (x y) …), (define-values (a b) …), (receive (a b) …)
(list
  .
  (symbol) @_form
  (#match? @_form "^(lambda|λ|define-values|receive)$")
  .
  (list . (symbol) @none))

; (case-lambda ((x) …) ((x y) …))
(list
  .
  (symbol) @_form
  (#eq? @_form "case-lambda")
  (list . (list . (symbol) @none)))

; (let ((x 1) (y 2)) …), (do ((i 0 (+ i 1))) …)
(list
  .
  (symbol) @_form
  (#match? @_form "^(let|let\\*|letrec|letrec\\*|let-syntax|letrec-syntax|fluid-let|parameterize|do)$")
  .
  (list (list . (symbol) @none)))

; (let loop ((i 0)) …): the loop a procedure, its bindings names.
(list
  .
  (symbol) @_form
  (#eq? @_form "let")
  .
  (symbol) @function
  .
  (list (list . (symbol) @none)))

; (let-values (((a b) (values 1 2))) …)
(list
  .
  (symbol) @_form
  (#match? @_form "^(let-values|let\\*-values)$")
  .
  (list (list . (list . (symbol) @none))))

; (define-record-type point (make-point x y) point? (x point-x)): a
; field's name.
(list
  .
  (symbol) @_form
  (#eq? @_form "define-record-type")
  .
  (_)
  .
  (_)
  .
  (_)
  (list . (symbol) @property))
"#;

/// The outline: what `symbols` lists without a server (docs/design/marks.md).
/// A definition's first pattern names it, so a `define` of a lambda is
/// a function before it is a variable.
#[cfg(feature = "scheme")]
const OUTLINE: &str = r#"(list . (symbol) @_d (#match? @_d "^(define|define\\*|define-public|define-inline)$") . (list . (symbol) @name)) @definition.function
(list . (symbol) @_d (#match? @_d "^(define|define\\*|define-public|define-inline)$") . (symbol) @name . (list . (symbol) @_l (#match? @_l "^(lambda|λ|case-lambda)$"))) @definition.function
(list . (symbol) @_d (#match? @_d "^(define-syntax|define-macro|define-syntax-rule)$") . (symbol) @name) @definition.macro
(list . (symbol) @_d (#match? @_d "^(define-syntax|define-macro|define-syntax-rule)$") . (list . (symbol) @name)) @definition.macro
(list . (symbol) @_d (#eq? @_d "define-record-type") . (symbol) @name) @definition.struct
(list . (symbol) @_d (#match? @_d "^(define-library|define-module|library)$") . (list) @name) @definition.module
; Variables, after every pattern that names a definition better.
(list . (symbol) @_d (#match? @_d "^(define|define\\*|define-public)$") . (symbol) @name) @definition.variable
"#;
