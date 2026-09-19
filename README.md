# buzz

A `yay`/`paru`-style package helper for Debian and Debian-based distros
(Ubuntu, Mint, etc.). Named after **Debian 1.1 "Buzz"**, the distro's
first ever release.

`buzz` wraps `apt` with:

- an interactive search + numbered install picker (`buzz firefox`)
- a "download binary first, only compile from source if needed" install
  flow — it checks apt for a ready-made package before ever building
  anything from source
- **Flatpak support** — if `flatpak` is installed, search, install, AND
  remove all work across both apt and Flathub. Results are tagged
  `[flatpak]` so you can tell them apart. If apt has nothing for a
  package but Flatpak does, `buzz install` offers that before ever
  building from source
- `buzz info <name>` to see full package details before installing,
  instead of committing based on a one-line search description
- `buzz list` to see everything buzz has built from source (with whether
  it's still installed) plus all your installed Flatpak apps in one place
- **smart name matching** — you don't need the exact package name.
  `buzz remove obs` finds `com.obsproject.Studio`; search results are
  narrowed to packages whose *name* actually relates to what you typed,
  instead of every package that merely mentions it in its description
- **automatic `deb-src` setup** — if source builds need it and it isn't
  enabled, buzz offers to fix your apt sources instead of failing with
  apt's cryptic error
- automatic retries for anything that talks to the network (fetching
  source, cloning, installing via Flatpak) — a flaky connection won't
  kill the whole command on the first hiccup
- **clear, unmissable "this worked" / "this failed" messages** at the
  end of every command — no guessing whether something actually
  finished after a wall of apt/dpkg output
- a local build cache, so rebuilding the same version twice is instant
- confirmation prompts before anything destructive happens, with
  `--noconfirm` to skip them
- full passthrough to `apt-get` for anything it doesn't own (`update`,
  `autoremove`, etc.), so it can fully replace typing `apt` day-to-day

Written in plain Rust, no external crates — just the standard library.

## Usage

```bash
buzz firefox                  # search + pick from a numbered list
buzz install nginx            # installs the apt binary if one exists
buzz install nginx --build    # force a build from source instead
buzz install gimp --flatpak   # force installing via Flatpak, skip apt entirely
buzz search nginx             # plain search, no prompt
buzz info nginx                # see full details before installing (apt + flatpak)
buzz upgrade                  # apt upgrade + rebuilds tracked source packages
buzz remove nginx             # remove a package (checks apt, then flatpak)
buzz remove nginx --purge     # remove + purge configs (apt only)
buzz clean                    # wipe the build cache
buzz list                      # show what buzz has built + installed flatpaks
buzz self-update               # rebuild + reinstall buzz itself (run from its checkout)
buzz autoremove                # anything unrecognized passes through to apt-get
```

Add `--noconfirm` (or the shorthand `-y`) to any command to skip confirmation prompts.

If a package is available from **both** apt and Flatpak, `buzz install` will
show a quick numbered choice instead of silently picking one for you:

```
'gimp' is available from multiple sources:
  1) apt (system package)
  2) Flatpak (org.gimp.GIMP)
Choose a source (Enter = apt):
```

## Installing

You'll need Rust installed. If you don't have it, `install.sh` will offer
to install it for you via [rustup](https://rustup.rs).

```bash
git clone https://github.com/Yaptitude/buzz.git
cd buzz
chmod +x install.sh
./install.sh
```

This builds a release binary and installs it to `/usr/local/bin/buzz`
(you'll be asked for your sudo password for that last step).

Don't want to touch system directories? Install to your user directory
instead — no `sudo` needed:

```bash
./install.sh --user
```

(Make sure `~/.local/bin` is on your `PATH` if you use this option — the
script will warn you if it isn't.)

### Uninstalling

```bash
./install.sh --uninstall
```

## Project structure

```
buzz/
├── .gitignore
├── Cargo.toml
├── README.md
├── install.sh
└── src/
    └── main.rs
```

## Requirements

- A Debian-based Linux distro (Debian, Ubuntu, Mint, etc.)
- Rust (the install script can set this up for you)
- For `apt-get source` to work (used when building from source), you need
  `deb-src` lines enabled in your apt sources. **buzz detects this
  automatically** and offers to enable it for you — it handles both the
  classic `/etc/apt/sources.list` format and the newer deb822
  `/etc/apt/sources.list.d/*.sources` format (Ubuntu 24.04+ and
  derivatives). You can still do it by hand if you prefer.

## How it decides binary vs. source

`buzz install <name>` checks `apt-cache policy <name>` first. If apt
already has a candidate version, it just runs `apt-get install` directly
— no compiling. It only builds from source when:

- the target is a git URL, or
- apt has no binary candidate for the package, or
- you pass `--build` to force it

## Flatpak

If you have `flatpak` installed, `buzz` will automatically search and
offer Flathub apps too — no extra flags needed. Search results tag them
`[flatpak]` so you can tell them apart from apt/buzz-tracked entries.

**⚠️ Important: `buzz` always installs Flatpak apps system-wide (with
`sudo`), not per-user.** Every install in `buzz` — apt or Flatpak — runs
through the same `sudo` path, for consistency. This means:

- Flatpak apps installed via `buzz` go into `/var/lib/flatpak` (shared,
  available to every user on the machine), **not** `~/.local/share/flatpak`
  (the usual per-user Flatpak default).
- If you specifically want a **per-user** Flatpak install instead, don't
  use `buzz` for that app — install it yourself with:
  ```bash
  flatpak install --user flathub <app-id>
  ```

`buzz` does **not** install Flatpak for you. Install it first with your
distro's normal tools, e.g.:

```bash
sudo apt install flatpak
```

The first time `buzz` needs to install something via Flatpak, it'll offer
to add the Flathub remote automatically if it isn't already set up.

`buzz remove <name>` checks apt first (via `dpkg -s`); if the package
isn't installed through apt, it checks currently-installed Flatpak apps
by app ID or display name and removes it that way instead. `--purge`
only applies to the apt path — Flatpak doesn't have an equivalent
concept in the same sense.

`buzz upgrade` shows a color-coded version diff (old version in red, new
version in green) before rebuilding a tracked source package, so the
change is obvious at a glance instead of squinting at two similar-looking
version strings.

## Updating buzz itself

```bash
cd buzz              # your git checkout
buzz self-update
```

This pulls the latest changes (if it's a git checkout), rebuilds in
release mode, and reinstalls to `/usr/local/bin/buzz`. Must be run from
inside the checkout, or pass the path explicitly:

```bash
buzz self-update /path/to/buzz
```

## Why not Snap or raw .deb downloads?

`buzz` deliberately sticks to three sources: apt, Flatpak, and building
from source. Two things it does **not** do on purpose:

**Snap** isn't integrated. Snap packages come from Canonical's own store
and run through `snapd`, a separate background service with its own
versioning, update, and sandboxing model that doesn't map onto apt or
Flatpak's package model at all. Supporting it properly would mean a
third completely different subprocess integration (channels, confinement
levels, its own search/install/remove semantics) for a store that's
narrower and more centrally controlled than Flathub, and largely
overlaps with what apt + Flatpak already cover. Not worth the added
complexity for what it'd actually add.

**Downloading and installing a raw `.deb` from a random URL** isn't
supported either, and this one's a deliberate safety line, not just a
scope decision. A `.deb` from apt or a source build both come through a
chain you can reason about — apt's own signing and repository trust, or
code you fetched and compiled yourself. A `.deb` grabbed from an
arbitrary vendor URL has none of that: no signature check `buzz` can
verify, no repository to pull future security updates from, and running
`dpkg -i` on it means executing whatever install scripts are bundled
inside with full root privileges, sight unseen. If you specifically need
a vendor's `.deb` (e.g. software that only ships that way), download and
install it yourself deliberately — that's a decision worth making with
your own judgement in the moment, not something `buzz` should do for you
automatically.

## License

MIT
