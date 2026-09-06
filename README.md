# buzz

A `yay`/`paru`-style package helper for Debian and Debian-based distros
(Ubuntu, Mint, etc.). Named after **Debian 1.1 "Buzz"**, the distro's
first ever release.

`buzz` wraps `apt` with:

- an interactive search + numbered install picker (`buzz firefox`)
- a "download binary first, only compile from source if needed" install
  flow — it checks apt for a ready-made package before ever building
  anything from source
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
buzz search nginx             # plain search, no prompt
buzz upgrade                  # apt upgrade + rebuilds tracked source packages
buzz remove nginx             # remove a package
buzz remove nginx --purge     # remove + purge configs
buzz clean                    # wipe the build cache
buzz autoremove                # anything unrecognized passes through to apt-get
```

Add `--noconfirm` to any command to skip confirmation prompts.

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
  `deb-src` lines enabled in your apt sources. On classic
  `/etc/apt/sources.list` systems, uncomment or add matching `deb-src`
  lines. On newer deb822-format systems (`/etc/apt/sources.list.d/*.sources`),
  change `Types: deb` to `Types: deb deb-src`.

## How it decides binary vs. source

`buzz install <name>` checks `apt-cache policy <name>` first. If apt
already has a candidate version, it just runs `apt-get install` directly
— no compiling. It only builds from source when:

- the target is a git URL, or
- apt has no binary candidate for the package, or
- you pass `--build` to force it

## License

MIT
