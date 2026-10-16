# Kawoosh as the default editor, asked

Status: decided and built 2026-10-16, the calls taken here, each the
user's to overturn. Asked (todo): "let's become opt-in default editor".

## What there was

Kawoosh.app declares a document type for every language the build knows
(`kawoosh --languages` → `CFBundleDocumentTypes`, scripts/macos-app.nu),
all `LSHandlerRank` `Alternate`: it is in every file's Open With and
opens what no other app claims, and takes nothing from an app that is a
type's default. On Windows `scripts/windows-app.nu --install` registers
the same types under HKCU — Open with, a `Kawoosh.File` ProgID and
`RegisteredApplications` — again taking nothing. Linux has no `.desktop`
file. Nothing made Kawoosh the default for a type, and nothing should
unasked: the install stays as it is.

What macOS makes of the extensions (read 2026-10-16 on the user's Mac,
`LSCopyDefaultRoleHandlerForContentType` for each, nothing changed):
of 184 extensions, about half are dynamic types (`dyn.…`: `rs`, `go`,
`lua`, `nu` — no app declares them; Kawoosh is already the default for
most of those nobody else opens), most of the rest text types other
editors hold (`public.json`, `public.python-script`, …), and a handful
are not text at all to macOS: `ts` is `public.mpeg-2-transport-stream`,
a video; `mts` AVCHD video, `dts` DTS audio, `dot` a Word template,
`exs` a Logic sampler file, `pem` a certificate, `plist` a property
list. `html`, `svg` and `csv` are text that another app shows (Safari,
an image editor, Numbers).

## Decisions

### 1. `kawoosh default-editor`, and `:default-editor` in the window

```text
kawoosh default-editor [status]           what Kawoosh is the default for
kawoosh default-editor on [EXT…] [--dry-run]
                                          take the types; EXT… only those
kawoosh default-editor off [EXT…] [--dry-run]
                                          give back what `on` took
```

A verb, as `lsp` and `format` are, not a flag: it has verbs of its own.
`:default-editor`, `:default-editor on`, `:default-editor off` run the
same in the window and show what it said in a `*default editor*` pane.
The CLI is the first way in — `Kawoosh.app/Contents/MacOS/kawoosh
default-editor on` from a terminal — and the `:` command is so it need
not be looked up. No key: it is done once.

`status` is the bare form: the safe one is what a word left off does.
`--dry-run` prints what `on` or `off` would do and changes nothing; the
tests run on it, and so can the user before the real thing.

Beaten: **a setting** (`editor.default = true`, applied at launch).
A default handler is the system's state, not kawoosh's; a setting would
re-take types the user gave to another app since, at every launch.
**Doing it at install** (`macos-app.nu --default`): the install is
re-run every build, and the ask is opt-in.

### 2. Which types: the text ones, by the system's own word

The candidates are what `--languages` lists — the extensions the plist
declares — looked up as the system types them (a UTI on macOS, a MIME
type on Linux, the extension on Windows), each type claimed once
however many extensions share it. A type is **claimed** when it is:

- **unknown** to the system (`dyn.…` on macOS): only Kawoosh says what
  it is;
- **text** (conforms to `public.text`; `text/*` or a subclass of
  `text/plain` on Linux) and not **shown**: an image, audio or video,
  HTML, or delimited values (`csv`) — text a browser, an image editor
  or a spreadsheet is the one to open on a double-click.

The rest is **left out** and said so, with why: `ts — a video to macOS
(public.mpeg-2-transport-stream)`. Naming one claims it whatever it is
(`on ts`), and the line says what that means ("macOS opens every
public.mpeg-2-transport-stream with Kawoosh, videos too"). That is "never
a type another app owns unless asked": a type another *editor* is the
default for is taken — being the default editor is that — but not one
whose files are something else.

Beaten: **every type** — then a `.ts` video and a `.svg` picture open in
an editor. **A curated list** — it falls behind the languages, which is
why the plist is compiled from `--languages`.

### 3. On macOS: LaunchServices, the `All` role, the bundle running

`LSSetDefaultRoleHandlerForContentType(uti, kLSRolesAll, bundle_id)`
per type — what Finder's Get Info → "Change All…" writes. `Editor`
alone is not enough: a type whose default someone set with Get Info has
an `All` handler, which a double-click goes on taking. The bundle id is
the running bundle's (`CFBundleGetMainBundle`), so a test bundle claims
for itself; a bare binary (`cargo run`) has none and stands for
`dev.qxuken.kawoosh` when LaunchServices knows an app by it, else it
refuses: LaunchServices hands a file to an app, not to an executable.

The call is deprecated since macOS 12 for `NSWorkspace
setDefaultApplicationAtURL:toOpenContentType:`, which takes an app's URL
and answers on a block — nothing it does that the C call does not, and
it would need objc2 in kawoosh. Content types do not ask the user to
confirm (the http scheme does); in case a macOS does, each type is read
back after it is set, and one that did not take is said: "macOS kept
com.tasqer.editor for public.json".

### 4. Undo gives back what was there, recorded at `on`

`on` records, for each type it takes, the handler it took it from —
`LSCopyDefaultRoleHandlerForContentType`, the default as it stood, the
system's pick or the user's — in `default-editor.json` beside the state
db. A second `on` keeps the first record of a type: what was there
before Kawoosh. `off`, for each recorded type:

- Kawoosh still its default, another app before: that app again;
- Kawoosh still its default, nothing (or Kawoosh) before: on macOS it
  **stays** — LaunchServices has no "no default", only another app —
  and is said ("kept: no other app had rs"); on Linux the line comes
  out of mimeapps.list and the system's pick is back;
- another app its default now: left alone and forgotten — the user moved
  it since.

`off EXT…` gives back only those. A type `on` finds Kawoosh already
the default for — on macOS most of the `dyn.…` ones, by its
`Alternate` rank — is left as it is and not recorded: `off` has
nothing to give back for it.

Beaten: **resetting to the system's pick** (deleting the
LSHandlers entries from the secure plist and `lsregister -kill`): it
undoes the user's own choices for those types too, and writes a file
LaunchServices owns.

### 5. Windows: registered already; Settings is where a default is given

Since Windows 8 a type's default is a `UserChoice` key under HKCU whose
hash a program cannot make, on purpose: only Settings or the Open with
dialog give one. So on Windows `on` and `off` check the registration
`windows-app.nu --install` writes (`RegisteredApplications\Kawoosh`),
say which types would be Kawoosh's, and open Settings at Kawoosh's
Default apps page (`ms-settings:defaultapps?registeredAppUser=Kawoosh`),
where each is one click; not registered, they say to install first.
`status` reads each extension's `UserChoice` ProgId (`Kawoosh.File` or
`Applications\kawoosh.exe` are Kawoosh) and `PerceivedType`, one
PowerShell for all of them. Nothing is recorded: Settings is the undo.

### 6. Linux: mimeapps.list, and a .desktop file when there is none

The spec's user file, `$XDG_CONFIG_HOME/mimeapps.list` `[Default
Applications]`, is written directly rather than through `xdg-mime
default`, which does the same with a shell script and cannot take a
line out. A type is the MIME type shared-mime-info's `globs2` gives the
extension (`$XDG_DATA_HOME` and `$XDG_DATA_DIRS`), text by its
`subclasses`. The handler is `kawoosh.desktop`; when no data dir has
one, `on` writes `$XDG_DATA_HOME/applications/kawoosh.desktop` (Exec
the running executable with `--reuse %f`, the types it claims) and
records that it did, so `off` removes it with the last type. The
previous handler recorded is the user's own line for the type, when
there was one: taking ours out gives back exactly what was there.

### 7. What it says

One report, grouped, the extensions named rather than the types —
`rs`, not `dyn.ah62d4rv4ge81e62` — a type's id only where it explains
(a left-out type). `status`:

```text
Kawoosh (dev.qxuken.kawoosh) is the default for 102 of the 153 types it would take on macOS.
  Kawoosh's: toml cfg cjs jsx cts inl tpp pyi pyw scm ss sld sls sps sch
    …
  com.tasqer.editor's: txt text rs css js mjs tsx go lua sh nu c h cpp cc
    …
  com.apple.dt.Xcode's: bash zsh geojson glsl vert frag geom mk mak m r gpx
    …
Left out (`kawoosh default-editor on EXT` takes one all the same):
  ts — media to macOS: a picture, a sound, a model (public.mpeg-2-transport-stream)
  csv — shown by com.apple.Numbers (public.comma-separated-values-text)
  html htm — shown by com.apple.Safari (public.html)
  pem — not text to macOS (public.x509-certificate)
```

`on` says what it took from whom and how to give it back; `off` what
went back to whom and what stayed.

## Deliberately not

- Folders (`public.folder`): Finder's.
- File names (`Dockerfile`, `Makefile`): not types to any of the three.
- URL schemes.
- Doing anything at install or launch.

## Built

2026-10-16. `kawoosh/src/default_editor.rs`: the plan over a `System`
trait — an extension's type, a type's default, set, clear, the
handler's `.desktop` file, Settings' page — with the record and the
report, tested against a fake system. LaunchServices is called through
CoreServices' C functions declared there (no new crate); Linux's files
are tested in a temp dir standing for HOME and the XDG dirs; Windows's
registry read is a PowerShell's output, parsed and tested as text. No
test touches the real LaunchServices database, registry or
mimeapps.list. The candidates are `grammars::languages()`, which
`--languages` now prints.

A type is looked up as Finder does, among those conforming to
`public.data`: with no conformance `fs` was
`com.apple.file-system-plug-in`, a bundle. `status` and `on --dry-run`
were run on the user's Mac (read-only): 153 types to take, 102 of them
Kawoosh's already, 13 left out (`ts mts dts exs svg dae` media, `html
htm csv` shown, `edn dot pem plist` not text). The real `on`/`off`
were not run here: they change the user's defaults, theirs to run.

Not checked: whether a macOS asks the user to confirm a content type's
default (no known version does; the read-back says so if one
does), and the Windows and Linux halves on their systems — the Linux
`system()` branch and the PowerShell call compile there only.
