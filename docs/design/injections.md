# Injections: a language inside a string

Status: built 2026-10-19 from the ask to "recognize syntax in" a
WebGPU shader written as `` code: /* wgsl */ `…` `` in a TypeScript file.
The calls below are taken here, each the user's to overturn.
Companion to [grammars.md](grammars.md) (where an installed grammar
comes from), [markdown.md](markdown.md) (the injections it was built
for: the inline grammar, a fence's language) and
[comments.md](comments.md) (`gc` asks the injection at the caret for
its token).

## What there was

The ts thread paints a grammar's injections query over the span it
answers for (`systems/src/ts.rs`, `paint_injections`): each
`@injection.content` node reaching into the span is parsed by its
language's grammar over that node alone, its captures painted over
the host's, its own injections under it, three deep. That serves
markdown (the inline grammar in every paragraph, a fence's language),
JSDoc in a javascript comment and regex in a regex literal.

A `#set! injection.combined` pattern was passed over. tree-sitter
means by it every match of the pattern in the file as one document —
php's text between its tags as one html page — which a painter of a
span cannot see. The javascript crate's one such pattern is the
tagged template, `` sql`select 1` ``: its language the tag, its
document the template's pieces between the `${…}`. So it was never
followed, and nothing named a string's language with a comment.

## Decisions

### 1. A block comment of one word before a string names its language

```typescript
code: /* wgsl */ `@vertex fn vs() -> @builtin(position) vec4f { … }`,
const q = /* sql */ 'select 1';
```

`languages/queries/ecma/injections.scm`, after the javascript crate's
query, for javascript, typescript and tsx: a `(comment)` matching
`/* WORD */` right before a template string or a string, under any
parent. tree-sitter hangs a comment, an extra, on whichever node is
being built, beside the string: a `pair`'s key and value, an
`arguments`, a `variable_declarator`, a `return_statement`, an
`array`, an `assignment_expression` — one pattern with a wildcard
parent and an anchor reads them all. The word is the language: any
grammar the build links or the user installed (`wgsl`, `glsl`, `sql`,
`html`, `css`, `graphql` once there is one), none named in the query.
A comment of more words — `/* the shader */` — injects nothing.

Beaten: **nvim's list of places** (a declarator, a call's argument, a
pair): the wildcard covers those and the ones it lacks (a return, an
array, an assignment). **Zed's list of languages** (a pattern each
for html, css, sql, glsl, wgsl…): a grammar installed tomorrow would
need a pattern added; the one-word guard is what keeps prose out.
**A line comment** (`// wgsl` on the line above, JetBrains'
`// language=sql`): a line comment above a string is as often prose
about it; not built.

### 2. A template's `${…}` stay javascript: one document per naming node

A `#set! injection.combined` pattern is now followed when a node of it
names the language: its matches that share that node — the tag of
one tagged template, the comment before one string — are one
document, its pieces their content nodes. The pieces are parsed
together as the injected grammar's one document (tree-sitter's
included ranges, `parse_range` taking several), so the shader reads
as one shader across `${count}`, and the substitution keeps the
host's colours: its `${` `}` punctuation, its expression javascript.
A combined pattern naming its language by `#set!` is still passed
over; that one is the whole file's.

Beaten: **the template whole as the content** (nvim's
`injection.include-children` with `#offset!` cutting the backticks):
the injected grammar reads `${count}` as an error, and paints over
javascript's colours there. **Each piece its own document** (Zed's
reading, and what kawoosh did for a non-combined pattern): after the
first `${…}` the grammar starts over mid-function, in error recovery.
**A quantified capture**, `(template_string (string_fragment)*
@injection.content)`, to have the match hold every piece: tree-sitter
answers it as a match a piece, tried in a scratch test.

What the hole costs: the injected grammar reads the text with the
substitution gone, so `where id = ${id} and b = 1` is `where id = and
b = 1` to SQL, which takes `and` for a column. A template whose
substitutions stand where a value would have to be is painted by the
grammar's error recovery there.

The same rule follows the javascript crate's tagged templates, which
it already listed: `` sql`…` ``, `` css`…` ``, lit's `` html`…` ``, with
the grammar there. A tag naming no grammar (`` dedent`…` ``,
`` String.raw`…` ``) injects nothing, as before.

### 3. The language is the first word

The name read off a query (`#set!` or the `@injection.language`
node's text) is its first run of letters, digits, `_` `-` `+` `#`
(`language_word`): `rust` of a fence's `rust,ignore`, `python` of
pandoc's `{.python}`, `wgsl` of `/* wgsl */`, `objective-c` and `c++`
whole. It was the text up to a comma or a space, which read `/*`.
Case is kept: `/* WGSL */` names nothing, as `:syntax WGSL` does not; a
language's other spellings are its aliases.

### 4. Painted within the pieces

The injected tree's captures are painted piece by piece, each clipped
to its piece and the answered span: a node of the shader that reaches
over a `${…}` (a statement around it, an error) leaves the host's
colours inside the substitution. A piece's own injections are read
over that piece.

### 5. JSDoc in a doc comment only; the query is kawoosh's

Asked the same day: `// @ts-expect-error "webgpu" does not have context
type` was painted a keyword only as far as `@ts`. The javascript
crate's query injects JSDoc into every comment, line comments too, and
JSDoc's tag is `@` and letters (`/@[a-zA-Z_]+/` in tree-sitter-jsdoc),
so the `-` ended it and the rest read as the tag's description. JSDoc
is now a `/** … */` comment's alone (`#match? "^/[*][*][^*]"`, nvim's
rule): a line comment and a plain `/* … */` are the comment's colour
whole.

`languages/queries/ecma/injections.scm` is therefore the whole query,
in place of the crate's: its tagged-template and regex patterns
copied, its `hbs` one left out (no Glimmer grammar here reads it),
then Decision 1's. Beaten: **the crate's query with a pattern of
kawoosh's after it**: a later pattern cannot take back an earlier one's
injection, and a text substitution on the crate's query would break
quietly at a crate update. The cost is that a crate update's new
injections are not picked up by themselves.

A `@ts-…` directive inside a `/** … */` is still a `@ts` tag and the
rest description: the tag's extent is the jsdoc grammar's, which only a
change to it would move. TypeScript's directives are line comments in
practice.

## Build notes

2026-10-19, on the branch `claude/xenodochial-archimedes-640302`.
`systems/src/ts.rs`: `injected` reads a query's documents (the
grouping of Decision 2), shared by the painter and `injection_at`,
which now answers the pieces, so `gc` and the indenter at a caret in a
shader ask WGSL's tokens and parse the whole template; `parse_range`
takes the pieces. `languages/src/javascript.rs` holds `INJECTIONS`,
the crate's query then kawoosh's, for typescript and tsx too.

Tests in `systems/src/ts.rs`: `a_comment_names_a_strings_language`
(javascript, typescript and tsx; an object's value, an argument, a
declaration, a plain string, a tagged template, a comment of two words
left alone; the substitution javascript's; `injection_at`'s pieces),
`a_combined_injection_survives_a_reparse` (an edit after the `${…}`
answers for a span and reads as a whole parse), `a_language_is_the_first_word`.
`jsdoc_is_a_doc_comments_alone` (Decision 5: `// @ts-expect-error` all
comment, `/* @internal */` too, `/** @param {number} x */` JSDoc's).
`kawoosh/tests/comment.rs`: `gcc` in a `/* sql */` template takes `--`.
The tests inject `sql`, which the build links. WGSL is an installed
grammar (kawoosh-grammars' `wgsl`, szebniok/tree-sitter-wgsl), so the
asked snippet was checked through the harness with the installed
grammar copied into a scratch data directory: `@vertex` attribute,
`fn` keyword, `vs` function, `u32` and `vec4f` types, numbers,
`// top center` a comment, and a `${scale}` put in the `return`
javascript's punctuation and variable. That grammar predates
module-scope `const` (its `grammar.toml` says so); a shader using one
has an error there.

## Not built

- A line comment naming the language (Decision 1).
- Other hosts: python's `# language=sql`, rust's `r#"…"#` after a
  comment. The rule is javascript's query; another grammar's would be
  a query of its own in the same shape.
- A combined pattern of the whole file (php's text as html).
