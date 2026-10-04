-- kawoosh's own project code, run once trusted (`:trust`).
-- `:compile install` installs the app: a release build put where the
-- system opens it from, by this system's script. A Kawoosh running from
-- there offers to relaunch into it. `settings.lua` is data and cannot
-- ask which system it is on, so the command is named here.
local install = {
  windows = {
    cmd = "nu scripts/windows-app.nu --install",
    doc = "%LOCALAPPDATA%\\Programs\\Kawoosh, and a Start menu shortcut",
  },
  mac = {
    cmd = "nu scripts/macos-app.nu /Applications",
    doc = "/Applications/Kawoosh.app",
  },
}

local here = install[kawoosh.os]
if here then
  here.cwd = kawoosh.project.root
  kawoosh.opt("compile.commands.install", here)
end
