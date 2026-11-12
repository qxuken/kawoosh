# Language tools released independently

Decided 2026-11-12: server and formatter recipes have their own catalog and
release cycle, as grammars do. This replaces lsp-servers.md's earlier choice
to keep all definitions in the application release.

## Catalog and installed tools

[`qxuken/kawoosh-tools`](https://github.com/qxuken/kawoosh-tools) owns the
Lua recipe tables grouped by language or tool, their validation and
the schema-versioned JSON manifest generated for each release. The bundled
[r2](https://github.com/qxuken/kawoosh-tools/releases/tag/r2) preserves the
first release's recipes and order while moving their source into Lua.
Kawoosh owns process execution, package installation, installed-version
records and settings overrides. Moving a recipe does not move an installed
tool's version. The TypeScript native recipe is available but remains behind
the explicit opt-in described in [typescript-native.md](typescript-native.md).

The manifest has `schema`, `release`, `servers` and `formatters`. Each recipe
has a stable name and command; the remaining data uses the existing settings
shape. Server ordering is preserved with explicit recipe order, independent
of source filenames. A small release-time Lua runner loads and validates
the tables before producing the JSON asset; generated JSON is not the
authoring source. Nushell provides the verification/build entrypoints.
Kawoosh consumes data from the manifest and does not execute catalog Lua.
Contributors can verify one file with
`nu scripts/verify.nu recipes/typescript.lua`, run all checks with
`nu scripts/test.nu`, and build with `nu scripts/build.nu --release rN`
inside the catalog repository. Existing runtime placeholders stay literal
in the recipes; the release runner adds no interpolation layer.
The shared parser in `editor/src/tool_catalog.rs` supplies the offline
formatter defaults as well as the host's server data. The former embedded
`servers.lua` and Rust formatter recipe table are no longer authoritative.

## Offline snapshot and explicit refresh

`kawoosh/tools/manifest.json` is the bundled snapshot. `source.json` records
its release URL and SHA-256; Git attributes preserve LF bytes so verification
also works on Windows. A malformed or unsupported cached manifest falls back
to the bundled snapshot. Server definitions are validated through their
normal decoder before a cache replacement is promoted.

`kawoosh catalog update` or `:catalog update` fetches the current released
manifest explicitly, validates it, then atomically replaces the cached file
under `tools/` beside the state database. `KAWOOSH_TOOLS_CATALOG` overrides
that path. Running processes keep their selected definitions; relaunching
uses the refreshed catalog without reinstalling Kawoosh. Installed tools
keep their recorded versions until their normal explicit update command.
`:catalog info` identifies the effective and bundled releases.
On Windows, the UI refresh uses the console companion executable so its
result is visible in the terminal pane as well as in the cache.

## Updating source snapshots

There is no implicit download at build or release time. Both catalogs use
the same explicit Nushell scripts:

```nu
nu scripts/verify-catalog.nu tools
nu scripts/update-catalog.nu tools --release r2
nu scripts/verify-catalog.nu grammars
nu scripts/update-catalog.nu grammars --release rN
```

The updater requires a release name, downloads to a temporary file, validates
before replacing the snapshot and records its digest. A maintainer reviews
the source diff and runs the regular checks. `scripts/verify.nu` verifies
both snapshots. The standalone catalog repository validates recipes in CI;
a release tag builds and publishes its manifest from the Lua sources.

This keeps an offline fallback and reproducible application builds while
allowing recipe changes independently. It costs a separate release and
maintains schema compatibility across catalog and application versions.
