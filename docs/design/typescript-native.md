# Native TypeScript: server or compiler API

Status: implemented 2026-11-12 after the user confirmed the native LSP,
behind `lsp.typescript.native = true`. The current default is unchanged.

## Two different integrations

Microsoft released TypeScript 7.0 on July 8, 2026. It is the native port,
supports LSP directly and is distributed as `typescript`; the release note
also explains the transition from the earlier `@typescript/native-preview`
package. A native language server can therefore be evaluated through
Kawoosh's existing generic LSP transport. Its installed executable and
launch arguments were verified against an isolated 7.0.2 package install:
`tsc --lsp --stdio` (`--lsp` alone exits with “only stdio is supported”).
Source: [TypeScript 7.0 announcement](https://devblogs.microsoft.com/typescript/announcing-typescript-7-0/).

The TypeScript 7.1 beta announcement, dated October 20, describes a separate
programmatic compiler API. Its entry points include synchronous and
asynchronous clients, filesystems and AST types; it also describes content
mappers. That is a different integration from selecting an LSP server.
Source: [TypeScript 7.1 beta announcement](https://devblogs.microsoft.com/typescript/announcing-typescript-7-1-beta/).

## Kawoosh today

The [released tool catalog](../../kawoosh/tools/manifest.json) defines the
existing `typescript-language-server` recipe and its TypeScript dependency,
and the optional `typescript-native` recipe (`typescript@7.0.2`). The Vue
recipe uses the legacy TypeScript integration as well. The generic pool
and rule selection are in [`lsp_rules.rs`](../../kawoosh/src/lsp_rules.rs)
and [`systems/src/lsp.rs`](../../systems/src/lsp.rs).

## Opting in

Set this in user or project settings, then run `:lsp install typescript-native`:

```lua
return { lsp = { typescript = { native = true } } }
```

For TypeScript, TSX and JavaScript, selection uses only the native server
plus configured conditional companions such as ESLint. Explicit
`lsp.languages` selections take precedence. Vue retains `vue` plus the
legacy `typescript` server and its tsserver request relay; enabling native
TypeScript does not send Vue's relay requests to the native server.
Turning the flag off restores the existing provider selection. Registered
custom servers and project overrides keep their existing interfaces.

Managed executable discovery is restricted to the recipe's package, so
the native server cannot accidentally launch the TypeScript 5 `tsc`
installed as a dependency of `typescript-language-server`. An explicit
command path still takes precedence; an unmanaged executable can use PATH.

## Verification and limits

The isolated Windows package smoke initialized `typescript-go` 7.0.2 over
stdio, opened a TypeScript document, checked definition, rename and
completion, changed the document incrementally and received diagnostics,
then shut down and repeated the handshake with a fresh process. The
selection regression checks default-off, native language coverage,
conditional companions, Vue's unchanged legacy bridge, explicit order and
turning the flag off. Package lookup has a regression with both legacy and
native `tsc` binaries present.

This adds no compiler-API or content-mapper integration. Startup and memory
comparisons, other-platform package smoke tests and a default change remain
future evaluation work.
