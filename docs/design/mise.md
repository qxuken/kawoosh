# mise projects

Built 2026-11-12. The bundled `mise.lua` plugin contributes tasks to the
compile picker and a project environment for tools, compile processes,
plugin children and language servers, on local and connected domains.

`mise tasks ls --json` discovers task names, descriptions, source files,
declared commands and usage without running tasks. Hidden and global
tasks are omitted. A selected task runs as argv `{ mise, run, NAME }`
through the existing direct-process launcher, independent of shell
quoting, including custom program paths or task names containing spaces.
Its displayed line stays `mise run NAME`. It runs from the directory
where it was discovered; mise applies its own `dir` and inherited task
configuration. The plugin keeps discovery by directory, paints the last
answer while a new one is pending, and refreshes the open compile picker
without replacing its query or calling pane. Configuration/task watches,
working-directory changes, settings changes and a five-second freshness
limit invalidate discovery. The cache holds at most 64 directories.

`mise env --json` supplies the project's variables, including PATH.
Discovery never uses `mise exec`, installs a tool, or grants trust.
Provider processes have no terminal or stdin; mise's existing trust
records decide whether executable configuration may load. Discovery sets
`MISE_YES=0`, `CI=0`, `MISE_PARANOID=1` and `MISE_NO_HOOKS=1`, so entering a project cannot
silently trust it through an inherited CI policy or run a directory hook.
Paranoid mode also rejects a trusted file whose content changed, and
requires mise trust for otherwise inert tool/task configuration too.
Missing mise or an untrusted configuration leaves the inherited child
environment intact. Review and trust configuration with mise itself.
Optional discovery uses the generic `spawn` option `quiet = true`:
startup failures still reach its stderr/done callbacks, but cannot
replace a newer foreground command's message. Explicit spawns keep
their normal visible startup errors.

The engine adds two generic extension doors:

- `compile_kind` can set `context = true`: its commands function is
  called with the current directory, including on a remote domain,
  without marker-file reads on the window thread. It returns cached
  rows and may start background work through `kawoosh.spawn`.
  `kawoosh.compile_changed()` republishes rows while the compile picker
  is open. A row may name its own `cwd` and source `file`; optional
  `argv` executes it without a shell and is retained with recent/rerun
  steps. Required-argument and `<C-e>` prompts retain that argv prefix
  and original cwd; appended words use the editor's existing quoted-word
  parser and stay literal arguments. Changing the displayed command
  prefix returns to an ordinary shell command. The original prompt's
  view and accepted command line anchor this handoff, so a canceled
  prompt cannot supply argv/cwd to a later command.
- `kawoosh.project_environment(name, { command, markers, watch, env })`
  registers an argv that prints a JSON object of environment strings.
  The engine resolves it per working directory/domain on child or LSP
  worker threads, with explicit child environment values taking
  precedence. `markers` limits local application to a matching ancestor;
  `watch` names config files/directories whose stamps invalidate the
  cache. `project_environment_changed(name, dir)` invalidates explicitly.
  A nil definition removes the provider.

The environment cache holds at most 64 provider/directory pairs. Local
configuration stamps invalidate it immediately on the next launch;
remote answers expire after five seconds, and plugin watches invalidate
them sooner. Provider execution has a five-second deadline and a 4 MiB
output cap. Remote providers use an owned bounded SSH channel or process
rather than holding the shared runner's serialized socket. A host
watchdog bounds the remote job too; timed-out jobs are cancelled.
A terminal's environment is prepared asynchronously
before its first shell starts, preserving its reserved terminal number,
directory, selected shell and tool metadata; closing its pane cancels
adoption. Keys, pasted text and terminal sends wait in a bounded 64 KiB
queue and are delivered once after spawn; an overflowing input chunk is
rejected with a message. Recording starts before queued input or the
first process output is consumed, and sessions keep the pending cwd.
Running servers and shells retain the environment they started with; their next
launch uses the newly resolved environment.

`mise.enabled = false` disables both contributions. `mise.command`
selects the program on the relevant domain (default `mise`).

Task JSON/environment/trust semantics were verified against mise
v2026.11.7 primary documentation/source and an installed Windows
v2026.11.4 trust probe. Automated process coverage uses a fake mise to
verify discovery, child/LSP/terminal environments and cache invalidation.
Remote metadata/domain isolation is covered by the Lua fixture, and a
fake SSH process verifies timeout cancellation and recovery. A real
SSH/WSL mise installation was not available for an end-to-end smoke test.

Primary references:
[task listing](https://mise.jdx.dev/cli/tasks/ls.html),
[environment output](https://mise.jdx.dev/cli/env.html),
[configuration trust](https://mise.jdx.dev/security.html#configuration-trust),
and the [task JSON implementation](https://github.com/jdx/mise/blob/v2026.11.7/src/cli/tasks/ls.rs).
