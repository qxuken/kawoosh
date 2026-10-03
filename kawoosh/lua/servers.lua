-- The builtin language servers (docs/design/lsp-servers.md Decision
-- 1): the ones every editor's LSP setup reaches for, each run as its
-- project installs it. Data, read as a settings file is — the pure
-- library and nothing that reaches out, no `kawoosh` — before any
-- other Lua runs, and by `kawoosh lsp` on the command line.
--
-- A row is what `lsp.NAME` in the settings says, with its `name`: the
-- language it is first for and its settings' key. `lsp.NAME` (and
-- `kawoosh.lsp.server`) is read over the row of that name, field by
-- field:
--
--   cmd        the program, found on the PATH or in kawoosh's servers
--   args       its arguments
--   roots      files that mark a workspace root
--   languages  every language it serves; none for its name alone
--   when       files one of which must be at or above a file for it to
--              run there (eslint's configs); none for everywhere
--   install    a line for a manager kawoosh does not drive; a
--              { mac =, linux =, windows = } of lines, this machine's
--              taken; or a package kawoosh installs itself into its own
--              directory — { npm = "name" }, { pip = { "a", "b<2" } },
--              { cargo = "x", args = { "--locked" } } (also go, dotnet)
--   settings   what the server reads as its configuration
--   answers    requests of its own and the result each is answered
--              with, before kawoosh's own answers:
--              { ["eslint/confirmESLintExecution"] = 4 }
--
-- The order is the asking order: a language's servers, when
-- `lsp.languages` does not say, are asked first to last — typescript's
-- before eslint's and biome's, which run beside it.

--- Homebrew's line, on macOS and Linux; none known on Windows.
local function brew(formula)
  local line = "brew install " .. formula
  return { mac = line, linux = line }
end

--- One program's npm package, for the many servers published there.
local vscode_extracted = { npm = "vscode-langservers-extracted" }

--- The files that say a project lints with eslint: flat configs and the
--- older `.eslintrc`s.
local eslint_configs = {
  "eslint.config.js",
  "eslint.config.mjs",
  "eslint.config.cjs",
  "eslint.config.ts",
  "eslint.config.mts",
  "eslint.config.cts",
  ".eslintrc",
  ".eslintrc.js",
  ".eslintrc.cjs",
  ".eslintrc.json",
  ".eslintrc.yaml",
  ".eslintrc.yml",
}

--- What vscode-eslint-language-server reads as its configuration: the
--- editor extension's defaults, as nvim-lspconfig sends them. A
--- `workspaceFolder` of "root" is answered with the server's root.
local eslint_settings = {
  validate = "on",
  useESLintClass = false,
  experimental = { useFlatConfig = false },
  codeActionOnSave = { enable = false, mode = "all" },
  format = false,
  quiet = false,
  onIgnoredFiles = "off",
  run = "onType",
  problems = { shortenToSingleLine = false },
  nodePath = "",
  workingDirectory = { mode = "location" },
  workspaceFolder = "root",
  codeAction = {
    disableRuleComment = { enable = true, location = "separateLine" },
    showDocumentation = { enable = true },
  },
}

return {
  {
    name = "rust",
    cmd = "rust-analyzer",
    roots = { "Cargo.toml" },
    install = "rustup component add rust-analyzer",
  },
  {
    name = "typescript",
    languages = { "typescript", "tsx", "javascript" },
    cmd = "typescript-language-server",
    args = { "--stdio" },
    roots = { "tsconfig.json", "jsconfig.json", "package.json" },
    install = { npm = { "typescript-language-server", "typescript@5" } },
  },
  {
    name = "lua",
    cmd = "lua-language-server",
    roots = { ".luarc.json", ".luarc.jsonc" },
    install = brew("lua-language-server"),
  },
  {
    name = "python",
    cmd = "pyright-langserver",
    args = { "--stdio" },
    roots = { "pyproject.toml", "pyrightconfig.json", "setup.py", "requirements.txt" },
    install = { npm = "pyright" },
  },
  {
    name = "go",
    languages = { "go", "gomod" },
    cmd = "gopls",
    roots = { "go.work", "go.mod" },
    install = { go = "golang.org/x/tools/gopls" },
  },
  {
    name = "c",
    languages = { "c", "cpp", "objc" },
    cmd = "clangd",
    roots = { "compile_commands.json", ".clangd", "CMakeLists.txt", "Makefile" },
    install = { mac = "xcode-select --install", windows = "winget install LLVM.LLVM" },
  },
  {
    name = "bash",
    cmd = "bash-language-server",
    args = { "start" },
    install = { npm = "bash-language-server" },
  },
  {
    name = "fish",
    cmd = "fish-lsp",
    args = { "start" },
    install = { npm = "fish-lsp" },
  },
  {
    name = "nu",
    cmd = "nu",
    args = { "--lsp" },
    install = { mac = "brew install nushell", linux = "cargo install nu --locked", windows = "winget install nushell" },
  },
  {
    name = "html",
    cmd = "vscode-html-language-server",
    args = { "--stdio" },
    roots = { "package.json" },
    install = vscode_extracted,
  },
  {
    name = "css",
    languages = { "css", "scss" },
    cmd = "vscode-css-language-server",
    args = { "--stdio" },
    roots = { "package.json" },
    install = vscode_extracted,
  },
  {
    name = "json",
    languages = { "json", "jsonc" },
    cmd = "vscode-json-language-server",
    args = { "--stdio" },
    install = vscode_extracted,
  },
  {
    name = "yaml",
    cmd = "yaml-language-server",
    args = { "--stdio" },
    install = { npm = "yaml-language-server" },
  },
  {
    name = "toml",
    cmd = "taplo",
    args = { "lsp", "stdio" },
    roots = { "taplo.toml", ".taplo.toml" },
    install = { cargo = "taplo-cli", args = { "--locked", "--features", "lsp" } },
  },
  {
    name = "markdown",
    cmd = "marksman",
    args = { "server" },
    roots = { ".marksman.toml" },
    install = brew("marksman"),
  },
  {
    name = "dockerfile",
    cmd = "docker-langserver",
    args = { "--stdio" },
    install = { npm = "dockerfile-language-server-nodejs" },
  },
  {
    name = "svelte",
    cmd = "svelteserver",
    args = { "--stdio" },
    roots = { "svelte.config.js", "package.json" },
    install = { npm = "svelte-language-server" },
  },
  {
    name = "php",
    cmd = "intelephense",
    args = { "--stdio" },
    roots = { "composer.json" },
    install = { npm = "intelephense" },
  },
  {
    name = "ruby",
    cmd = "ruby-lsp",
    roots = { "Gemfile", ".ruby-version" },
    install = "gem install ruby-lsp",
  },
  {
    name = "java",
    cmd = "jdtls",
    roots = { "pom.xml", "build.gradle", "build.gradle.kts", "settings.gradle", "settings.gradle.kts" },
    install = brew("jdtls"),
  },
  {
    name = "kotlin",
    cmd = "kotlin-language-server",
    roots = { "settings.gradle.kts", "settings.gradle", "build.gradle.kts", "build.gradle", "pom.xml" },
    install = brew("kotlin-language-server"),
  },
  {
    name = "scala",
    cmd = "metals",
    roots = { "build.sbt", "build.sc", "build.mill" },
    install = "cs install metals",
  },
  {
    name = "csharp",
    cmd = "csharp-ls",
    roots = { "global.json", "Directory.Build.props" },
    install = { dotnet = "csharp-ls" },
  },
  {
    name = "fsharp",
    cmd = "fsautocomplete",
    roots = { "global.json", "Directory.Build.props" },
    install = { dotnet = "fsautocomplete" },
  },
  {
    name = "dart",
    cmd = "dart",
    args = { "language-server", "--protocol=lsp" },
    roots = { "pubspec.yaml" },
    install = brew("dart-lang/dart/dart"),
  },
  {
    name = "zig",
    cmd = "zls",
    roots = { "build.zig", "build.zig.zon" },
    install = brew("zls"),
  },
  {
    name = "haskell",
    cmd = "haskell-language-server-wrapper",
    args = { "--lsp" },
    roots = { "hie.yaml", "stack.yaml", "cabal.project", "package.yaml" },
    install = "ghcup install hls",
  },
  {
    name = "ocaml",
    cmd = "ocamllsp",
    roots = { "dune-project", "dune-workspace" },
    install = "opam install ocaml-lsp-server",
  },
  {
    name = "elixir",
    cmd = "elixir-ls",
    roots = { "mix.exs" },
    install = brew("elixir-ls"),
  },
  {
    name = "erlang",
    cmd = "erlang_ls",
    roots = { "rebar.config", "erlang.mk" },
    install = brew("erlang_ls"),
  },
  {
    name = "gleam",
    cmd = "gleam",
    args = { "lsp" },
    roots = { "gleam.toml" },
    install = brew("gleam"),
  },
  {
    name = "elm",
    cmd = "elm-language-server",
    roots = { "elm.json" },
    install = { npm = "@elm-tooling/elm-language-server" },
  },
  {
    name = "purescript",
    cmd = "purescript-language-server",
    args = { "--stdio" },
    roots = { "spago.yaml", "spago.dhall" },
    install = { npm = "purescript-language-server" },
  },
  {
    name = "clojure",
    cmd = "clojure-lsp",
    roots = { "deps.edn", "project.clj", "bb.edn", "shadow-cljs.edn" },
    install = brew("clojure-lsp/brew/clojure-lsp-native"),
  },
  {
    name = "racket",
    cmd = "racket",
    args = { "-l", "racket-langserver" },
    roots = { "info.rkt" },
    install = "raco pkg install --auto racket-langserver",
  },
  {
    name = "nix",
    cmd = "nil",
    roots = { "flake.nix" },
    install = "nix profile install nixpkgs#nil",
  },
  {
    name = "cmake",
    cmd = "cmake-language-server",
    roots = { "CMakeLists.txt" },
    -- pygls 2 took away the class it imports (2026-10).
    install = { pip = { "cmake-language-server", "pygls<2" } },
  },
  {
    name = "fortran",
    cmd = "fortls",
    roots = { ".fortls" },
    install = { pip = "fortls" },
  },
  {
    name = "r",
    cmd = "R",
    args = { "--no-echo", "-e", "languageserver::run()" },
    roots = { "DESCRIPTION", ".Rprofile" },
    install = [[R -e 'install.packages("languageserver", repos = "https://cloud.r-project.org")']],
  },
  {
    name = "prisma",
    cmd = "prisma-language-server",
    args = { "--stdio" },
    roots = { "package.json" },
    install = { npm = "@prisma/language-server" },
  },
  {
    name = "proto",
    cmd = "protols",
    roots = { "protols.toml", "buf.yaml" },
    install = { cargo = "protols" },
  },
  {
    name = "dot",
    cmd = "dot-language-server",
    args = { "--stdio" },
    install = { npm = "dot-language-server" },
  },
  {
    name = "awk",
    cmd = "awk-language-server",
    install = { npm = "awk-language-server" },
  },
  {
    name = "wgsl",
    cmd = "wgsl-analyzer",
    install = { cargo = "wgsl-analyzer", args = { "--git", "https://github.com/wgsl-analyzer/wgsl-analyzer" } },
  },
  {
    name = "glsl",
    cmd = "glsl_analyzer",
  },
  {
    name = "odin",
    cmd = "ols",
    roots = { "ols.json", "odinfmt.json" },
  },
  {
    name = "luau",
    cmd = "luau-lsp",
    args = { "lsp" },
    roots = { ".luaurc" },
  },
  -- Beside a language's own server, where the project says so
  -- (lsp-installs.md Decision 7): its config, at or above the file.
  {
    name = "eslint",
    languages = { "typescript", "tsx", "javascript" },
    cmd = "vscode-eslint-language-server",
    args = { "--stdio" },
    roots = { "package.json" },
    when = eslint_configs,
    install = vscode_extracted,
    settings = eslint_settings,
    -- It asks before it runs a project's eslint; 4 is "approved", as
    -- the editor extension answers once the user has allowed it.
    answers = { ["eslint/confirmESLintExecution"] = 4 },
  },
  {
    name = "biome",
    languages = { "typescript", "tsx", "javascript", "json", "jsonc", "css" },
    cmd = "biome",
    args = { "lsp-proxy" },
    roots = { "biome.json", "biome.jsonc" },
    when = { "biome.json", "biome.jsonc" },
    install = { npm = "@biomejs/biome" },
  },
  {
    name = "ruff",
    languages = { "python" },
    cmd = "ruff",
    args = { "server" },
    roots = { "pyproject.toml", "ruff.toml", ".ruff.toml" },
    when = { "ruff.toml", ".ruff.toml" },
    install = { pip = "ruff" },
  },
}
